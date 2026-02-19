use std::path::Path;

use crate::error::RalphError;
use crate::git::{ensure_git_repo, run_git, run_git_status};
use crate::project::state::Phase;
use crate::Result;

const RALPH_SUBJECT_PREFIX: &str = "ralph(";
const RALPH_SUBJECT_LOOP_DELIMITER: &str = "): loop ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RalphCommitInfo {
    pub project_id: String,
    pub loop_number: u32,
    pub phase: Phase,
    pub from_phase: Phase,
    pub commit_hash: Option<String>,
}

pub fn build_ralph_commit_message(
    project_id: &str,
    loop_number: u32,
    from_phase: Phase,
    to_phase: Phase,
) -> String {
    format!(
        "ralph({project_id}): loop {loop_number} {} -> {}\n\nRalph-Project: {project_id}\nRalph-Loop: {loop_number}\nRalph-Phase: {}",
        phase_label(&from_phase),
        phase_label(&to_phase),
        phase_label(&to_phase)
    )
}

pub fn parse_ralph_commit(
    raw_subject: &str,
    raw_body: &str,
    commit_hash: Option<&str>,
) -> Result<RalphCommitInfo> {
    let (subject_project_id, subject_loop_number, from_phase, to_phase) =
        parse_subject(raw_subject)?;
    let (trailer_project_id, trailer_loop_number, trailer_phase) = parse_trailers(raw_body)?;

    if subject_project_id != trailer_project_id {
        return Err(RalphError::ParseError(format!(
            "subject project id '{subject_project_id}' does not match Ralph-Project trailer '{trailer_project_id}'"
        )));
    }

    if subject_loop_number != trailer_loop_number {
        return Err(RalphError::ParseError(format!(
            "subject loop number '{subject_loop_number}' does not match Ralph-Loop trailer '{trailer_loop_number}'"
        )));
    }

    if to_phase != trailer_phase {
        return Err(RalphError::ParseError(format!(
            "subject to-phase '{}' does not match Ralph-Phase trailer '{}'",
            phase_label(&to_phase),
            phase_label(&trailer_phase)
        )));
    }

    Ok(RalphCommitInfo {
        project_id: trailer_project_id,
        loop_number: trailer_loop_number,
        phase: trailer_phase,
        from_phase,
        commit_hash: commit_hash.map(|hash| hash.to_owned()),
    })
}

pub fn parse_last_ralph_commit(repo_root: &Path, branch: &str) -> Result<Option<RalphCommitInfo>> {
    ensure_git_repo(repo_root)?;
    let remote_branch = format!("origin/{branch}");

    let status = run_git_status(
        repo_root,
        &["rev-parse", "--verify", "--quiet", remote_branch.as_str()],
    )?;
    if !status.success() {
        return Ok(None);
    }

    let log = run_git(
        repo_root,
        &["log", remote_branch.as_str(), "--format=%H%x1f%s%x1f%b%x1e"],
    )?;

    for record in log.split('\x1e').filter(|entry| !entry.trim().is_empty()) {
        let mut fields = record.splitn(3, '\x1f');
        let hash = fields.next().unwrap_or("").trim();
        let subject = fields.next().unwrap_or("").trim();
        let body = fields.next().unwrap_or("");

        if !subject.starts_with(RALPH_SUBJECT_PREFIX) {
            continue;
        }

        let parsed = parse_ralph_commit(subject, body, Some(hash)).map_err(|err| {
            RalphError::ParseError(format!("malformed Ralph checkpoint commit {hash}: {err}"))
        })?;
        return Ok(Some(parsed));
    }

    Ok(None)
}

pub fn derive_position(repo_root: &Path, branch: &str) -> Result<(u32, Phase)> {
    match parse_last_ralph_commit(repo_root, branch)? {
        Some(info) => Ok((info.loop_number, info.phase)),
        None => Ok((1, Phase::Planning)),
    }
}

fn parse_subject(raw_subject: &str) -> Result<(String, u32, Phase, Phase)> {
    let subject = raw_subject.trim();
    let rest = subject.strip_prefix(RALPH_SUBJECT_PREFIX).ok_or_else(|| {
        RalphError::ParseError(format!(
            "invalid Ralph commit subject '{subject}': expected prefix 'ralph('"
        ))
    })?;

    let (project_id, remainder) =
        rest.split_once(RALPH_SUBJECT_LOOP_DELIMITER)
            .ok_or_else(|| {
                RalphError::ParseError(format!(
                    "invalid Ralph commit subject '{subject}': expected '): loop ' delimiter"
                ))
            })?;

    let project_id = project_id.trim();
    if !is_valid_project_id(project_id) {
        return Err(RalphError::ParseError(format!(
            "invalid Ralph commit subject project id '{project_id}': expected issue-<number>"
        )));
    }

    let (loop_number_raw, transition_raw) = remainder.split_once(' ').ok_or_else(|| {
        RalphError::ParseError(format!(
            "invalid Ralph commit subject '{subject}': missing loop transition"
        ))
    })?;
    let loop_number = parse_loop_number(loop_number_raw, "subject")?;

    let (from_phase_raw, to_phase_raw) = transition_raw.split_once(" -> ").ok_or_else(|| {
        RalphError::ParseError(format!(
            "invalid Ralph commit subject '{subject}': expected '<from_phase> -> <to_phase>'"
        ))
    })?;
    let from_phase = parse_phase(from_phase_raw, "subject from-phase")?;
    let to_phase = parse_phase(to_phase_raw, "subject to-phase")?;

    Ok((project_id.to_owned(), loop_number, from_phase, to_phase))
}

fn parse_trailers(raw_body: &str) -> Result<(String, u32, Phase)> {
    let mut trailer_project_id: Option<String> = None;
    let mut trailer_loop_number: Option<u32> = None;
    let mut trailer_phase: Option<Phase> = None;

    for line in raw_body.lines().map(str::trim) {
        if let Some(value) = line.strip_prefix("Ralph-Project:") {
            if trailer_project_id.is_some() {
                return Err(RalphError::ParseError(
                    "duplicate Ralph-Project trailer".to_owned(),
                ));
            }
            let project_id = value.trim();
            if !is_valid_project_id(project_id) {
                return Err(RalphError::ParseError(format!(
                    "invalid Ralph-Project trailer '{project_id}': expected issue-<number>"
                )));
            }
            trailer_project_id = Some(project_id.to_owned());
            continue;
        }

        if let Some(value) = line.strip_prefix("Ralph-Loop:") {
            if trailer_loop_number.is_some() {
                return Err(RalphError::ParseError(
                    "duplicate Ralph-Loop trailer".to_owned(),
                ));
            }
            trailer_loop_number = Some(parse_loop_number(value.trim(), "Ralph-Loop trailer")?);
            continue;
        }

        if let Some(value) = line.strip_prefix("Ralph-Phase:") {
            if trailer_phase.is_some() {
                return Err(RalphError::ParseError(
                    "duplicate Ralph-Phase trailer".to_owned(),
                ));
            }
            trailer_phase = Some(parse_phase(value.trim(), "Ralph-Phase trailer")?);
        }
    }

    let project_id = trailer_project_id.ok_or_else(|| {
        RalphError::ParseError("missing required trailer: Ralph-Project".to_owned())
    })?;
    let loop_number = trailer_loop_number
        .ok_or_else(|| RalphError::ParseError("missing required trailer: Ralph-Loop".to_owned()))?;
    let phase = trailer_phase.ok_or_else(|| {
        RalphError::ParseError("missing required trailer: Ralph-Phase".to_owned())
    })?;

    Ok((project_id, loop_number, phase))
}

fn parse_loop_number(value: &str, source: &str) -> Result<u32> {
    value.parse::<u32>().map_err(|err| {
        RalphError::ParseError(format!("invalid loop number '{value}' in {source}: {err}"))
    })
}

fn parse_phase(value: &str, source: &str) -> Result<Phase> {
    match value {
        "planning" => Ok(Phase::Planning),
        "implementing" => Ok(Phase::Implementing),
        "qa" => Ok(Phase::QA),
        "reviewing" => Ok(Phase::Reviewing),
        "committing" => Ok(Phase::Committing),
        "completing" => Ok(Phase::Completing),
        _ => Err(RalphError::ParseError(format!(
            "invalid phase '{value}' in {source}; expected one of planning, implementing, qa, reviewing, committing, completing"
        ))),
    }
}

fn phase_label(phase: &Phase) -> &'static str {
    match phase {
        Phase::Planning => "planning",
        Phase::Implementing => "implementing",
        Phase::QA => "qa",
        Phase::Reviewing => "reviewing",
        Phase::Committing => "committing",
        Phase::Completing => "completing",
    }
}

fn is_valid_project_id(project_id: &str) -> bool {
    let Some(number) = project_id.strip_prefix("issue-") else {
        return false;
    };
    !number.is_empty() && number.chars().all(|ch| ch.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use tempfile::TempDir;

    use super::{
        build_ralph_commit_message, derive_position, parse_last_ralph_commit, parse_ralph_commit,
    };
    use crate::project::state::Phase;

    fn git_ok(repo: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(repo)
            .status()
            .expect("git command should execute");
        assert!(
            status.success(),
            "git command failed: git {}",
            args.join(" ")
        );
    }

    fn init_local_repo() -> TempDir {
        let temp = TempDir::new().expect("temp dir should be created");
        let repo = temp.path();
        git_ok(repo, &["init"]);
        git_ok(repo, &["config", "user.email", "test@example.com"]);
        git_ok(repo, &["config", "user.name", "Test User"]);
        fs::write(repo.join("README.md"), "# test\n").expect("README should be written");
        git_ok(repo, &["add", "-A"]);
        git_ok(repo, &["commit", "-m", "initial"]);
        temp
    }

    fn init_repo_with_remote() -> (TempDir, std::path::PathBuf) {
        let temp = TempDir::new().expect("temp dir should be created");
        let root = temp.path();
        let remote = root.join("remote.git");
        let work = root.join("work");

        git_ok(root, &["init", "--bare", remote.to_string_lossy().as_ref()]);
        git_ok(root, &["clone", remote.to_string_lossy().as_ref(), "work"]);
        git_ok(&work, &["config", "user.email", "test@example.com"]);
        git_ok(&work, &["config", "user.name", "Test User"]);

        fs::write(work.join("README.md"), "# test\n").expect("README should be written");
        git_ok(&work, &["add", "-A"]);
        git_ok(&work, &["commit", "-m", "initial"]);
        git_ok(&work, &["push", "origin", "HEAD:master"]);
        git_ok(&work, &["checkout", "-b", "ralph/issue-42"]);
        git_ok(&work, &["push", "-u", "origin", "ralph/issue-42"]);

        (temp, work)
    }

    fn commit_empty_with_message(repo: &Path, message: &str) {
        let message_file = repo.join(".git").join("RALPH_TEST_MSG");
        fs::write(&message_file, message).expect("commit message should be written");
        git_ok(
            repo,
            &[
                "commit",
                "--allow-empty",
                "--file",
                message_file.to_string_lossy().as_ref(),
            ],
        );
        fs::remove_file(message_file).expect("message file should be removed");
    }

    #[test]
    fn build_and_parse_round_trip() {
        let message =
            build_ralph_commit_message("issue-42", 3, Phase::Planning, Phase::Implementing);
        let (subject, body) = message
            .split_once("\n\n")
            .expect("message should include body");

        let parsed =
            parse_ralph_commit(subject, body, Some("abc123")).expect("commit should parse cleanly");
        assert_eq!(parsed.project_id, "issue-42");
        assert_eq!(parsed.loop_number, 3);
        assert_eq!(parsed.from_phase, Phase::Planning);
        assert_eq!(parsed.phase, Phase::Implementing);
        assert_eq!(parsed.commit_hash.as_deref(), Some("abc123"));
    }

    #[test]
    fn parse_rejects_malformed_subject() {
        let result = parse_ralph_commit(
            "ralph issue-42 loop 1 planning -> implementing",
            "Ralph-Project: issue-42\nRalph-Loop: 1\nRalph-Phase: implementing",
            None,
        );
        assert!(result.is_err(), "malformed subject should be rejected");
    }

    #[test]
    fn parse_rejects_missing_required_trailer() {
        let result = parse_ralph_commit(
            "ralph(issue-42): loop 1 planning -> implementing",
            "Ralph-Project: issue-42\nRalph-Loop: 1",
            None,
        );
        assert!(result.is_err(), "missing trailers should be rejected");
    }

    #[test]
    fn parse_rejects_subject_and_trailer_disagreement() {
        let project_mismatch = parse_ralph_commit(
            "ralph(issue-42): loop 1 planning -> implementing",
            "Ralph-Project: issue-99\nRalph-Loop: 1\nRalph-Phase: implementing",
            None,
        );
        assert!(
            project_mismatch.is_err(),
            "project id mismatch should be rejected"
        );

        let loop_mismatch = parse_ralph_commit(
            "ralph(issue-42): loop 1 planning -> implementing",
            "Ralph-Project: issue-42\nRalph-Loop: 2\nRalph-Phase: implementing",
            None,
        );
        assert!(loop_mismatch.is_err(), "loop mismatch should be rejected");
    }

    #[test]
    fn derive_position_defaults_when_no_ralph_commit_exists() {
        let repo = init_local_repo();
        let position = derive_position(repo.path(), "ralph/issue-42").expect("derive should work");
        assert_eq!(position, (1, Phase::Planning));
    }

    #[test]
    fn parse_last_ralph_commit_reads_latest_remote_checkpoint() {
        let (_temp, repo) = init_repo_with_remote();

        let first = build_ralph_commit_message("issue-42", 1, Phase::Planning, Phase::Implementing);
        commit_empty_with_message(&repo, &first);
        git_ok(&repo, &["push", "origin", "HEAD:ralph/issue-42"]);

        commit_empty_with_message(&repo, "chore: non-ralph commit");
        git_ok(&repo, &["push", "origin", "HEAD:ralph/issue-42"]);

        let second =
            build_ralph_commit_message("issue-42", 2, Phase::Implementing, Phase::Reviewing);
        commit_empty_with_message(&repo, &second);
        git_ok(&repo, &["push", "origin", "HEAD:ralph/issue-42"]);

        let parsed = parse_last_ralph_commit(&repo, "ralph/issue-42")
            .expect("parse should succeed")
            .expect("ralph commit should exist");
        assert_eq!(parsed.project_id, "issue-42");
        assert_eq!(parsed.loop_number, 2);
        assert_eq!(parsed.from_phase, Phase::Implementing);
        assert_eq!(parsed.phase, Phase::Reviewing);
        assert!(
            parsed.commit_hash.is_some(),
            "parsed commit should include hash"
        );
    }

    #[test]
    fn parse_last_ralph_commit_rejects_malformed_newest_checkpoint() {
        let (_temp, repo) = init_repo_with_remote();

        let valid = build_ralph_commit_message("issue-42", 1, Phase::Planning, Phase::Implementing);
        commit_empty_with_message(&repo, &valid);
        git_ok(&repo, &["push", "origin", "HEAD:ralph/issue-42"]);

        let malformed = "ralph(issue-42): loop 2 implementing -> reviewing\n\nRalph-Project: issue-999\nRalph-Loop: 2\nRalph-Phase: reviewing";
        commit_empty_with_message(&repo, malformed);
        git_ok(&repo, &["push", "origin", "HEAD:ralph/issue-42"]);

        let result = parse_last_ralph_commit(&repo, "ralph/issue-42");
        assert!(result.is_err(), "malformed newest checkpoint should error");
    }
}
