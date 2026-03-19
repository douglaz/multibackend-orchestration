use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::daemon::github::{self, OpenPrInfo};
use crate::daemon::process;
use crate::daemon::runtime::{truncate_for_github, DaemonRuntimeConfig, GITHUB_COMMENT_LIMIT};
use crate::error::RalphError;
use crate::Result;

const ORACLE_SYSTEM_PROMPT: &str = "You are a senior code reviewer. Review this PR diff for bugs, security issues, performance problems, and code quality. Be concise and actionable. Focus on substantive issues, not style nits.";
const ORACLE_USER_PROMPT: &str = "Review the attached PR diff.";
const ORACLE_SYSTEM_WRAPPER: &str = r#"import { createRequire } from 'node:module';
import { pathToFileURL } from 'node:url';

const cliPath = process.env.RALPH_ORACLE_CLI_PATH;
if (!cliPath) {
  console.error('oracle spawn: missing RALPH_ORACLE_CLI_PATH');
  process.exit(70);
}

const cliUrl = pathToFileURL(cliPath);
const require = createRequire(cliUrl);

let commander;
try {
  commander = require('commander');
} catch (error) {
  const message = error instanceof Error ? error.message : String(error);
  console.error(`oracle spawn: failed to load commander: ${message}`);
  process.exit(70);
}

const { Command, Option } = commander;
const originalOption = Command.prototype.option;

Command.prototype.option = function patchedOption(flags, description, ...args) {
  const result = originalOption.call(this, flags, description, ...args);
  if (
    flags === '-p, --prompt <text>' &&
    !this.options.some((option) => option.long === '--system')
  ) {
    result.addOption(
      new Option('--system <text>', 'System prompt to send to the model.').hideHelp(),
    );
  }
  return result;
};

try {
  await import(cliUrl.href);
} catch (error) {
  const message =
    error instanceof Error ? error.stack ?? error.message : String(error);
  console.error(`oracle spawn: failed to launch oracle cli: ${message}`);
  process.exit(70);
}
"#;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OracleReviewState {
    pub reviewed: HashMap<String, String>,
}

impl OracleReviewState {
    pub fn load(workspace_root: &Path) -> Result<Self> {
        let path = state_path(workspace_root);
        let content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(err) => {
                return Err(RalphError::Orchestration(format!(
                    "failed to read oracle review state {}: {err}",
                    path.display()
                )))
            }
        };

        serde_json::from_str(&content).map_err(|err| {
            RalphError::Orchestration(format!(
                "corrupted oracle review state at {} (refusing to reset to empty): {err}",
                path.display()
            ))
        })
    }

    pub fn save(&self, workspace_root: &Path) -> Result<()> {
        let path = state_path(workspace_root);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                RalphError::Orchestration(format!(
                    "failed to create oracle-review-state dir {}: {err}",
                    parent.display()
                ))
            })?;
        }

        let json = serde_json::to_string_pretty(self).map_err(|err| {
            RalphError::Orchestration(format!("failed to serialize oracle review state: {err}"))
        })?;
        let tmp_path = path.with_extension("json.tmp");
        fs::write(&tmp_path, json).map_err(|err| {
            RalphError::Orchestration(format!(
                "failed to write oracle review state tmp {}: {err}",
                tmp_path.display()
            ))
        })?;
        fs::rename(&tmp_path, &path).map_err(|err| {
            RalphError::Orchestration(format!(
                "failed to rename oracle review state {} -> {}: {err}",
                tmp_path.display(),
                path.display()
            ))
        })?;
        Ok(())
    }

    fn reviewed_sha_matches(&self, pr_number: u32, head_sha: &str) -> bool {
        self.reviewed
            .get(&pr_number.to_string())
            .map(|saved_sha| saved_sha == head_sha)
            .unwrap_or(false)
    }

    fn mark_reviewed(&mut self, pr_number: u32, head_sha: &str) {
        self.reviewed
            .insert(pr_number.to_string(), head_sha.to_owned());
    }
}

pub async fn oracle_review_phase(config: &DaemonRuntimeConfig) -> Result<()> {
    if !config.oracle_review_enabled {
        return Ok(());
    }

    let (open_prs, overflow) =
        match github::list_open_non_draft_prs(&config.owner, &config.repo, &config.gh_bin).await {
            Ok(result) => result,
            Err(err) => {
                eprintln!("warning: oracle review: PR list failed: {err}");
                return Ok(());
            }
        };

    if overflow {
        eprintln!("warning: oracle review: gh pr list returned 100 PRs, results may be truncated");
    }

    let allowed_authors = normalized_author_allowlist(&config.oracle_review_authors);
    let candidates: Vec<OpenPrInfo> = open_prs
        .into_iter()
        .filter(|pr| {
            allowed_authors.is_empty() || allowed_authors.contains(&pr.author.to_ascii_lowercase())
        })
        .collect();

    let mut state = match OracleReviewState::load(&config.workspace_root) {
        Ok(state) => state,
        Err(err) => {
            eprintln!("warning: oracle review: state load failed: {err}");
            return Ok(());
        }
    };

    let bot_login = match github::fetch_authenticated_login_with_gh_bin(&config.gh_bin).await {
        Ok(login) => login,
        Err(err) => {
            eprintln!("warning: oracle review: bot login resolve failed: {err}");
            return Ok(());
        }
    };

    let mut success_count = 0u32;
    for pr in candidates {
        if success_count >= config.oracle_review_max_per_cycle {
            break;
        }

        if state.reviewed_sha_matches(pr.number, &pr.head_sha) {
            continue;
        }

        let marker = oracle_review_marker(pr.number, &pr.head_sha);
        match github::find_bot_comment_with_marker_with_gh_bin(
            &config.gh_bin,
            &config.owner,
            &config.repo,
            pr.number,
            &marker,
            &bot_login,
        )
        .await
        {
            Ok(Some(_)) => {
                state.mark_reviewed(pr.number, &pr.head_sha);
                if let Err(err) = state.save(&config.workspace_root) {
                    eprintln!(
                        "warning: oracle review: PR #{} state save failed: {err}",
                        pr.number
                    );
                }
                continue;
            }
            Ok(None) => {}
            Err(err) => {
                eprintln!(
                    "warning: oracle review: PR #{} marker check failed: {err}",
                    pr.number
                );
                continue;
            }
        }

        let diff =
            match github::fetch_pr_diff(&config.owner, &config.repo, pr.number, &config.gh_bin)
                .await
            {
                Ok(diff) => diff,
                Err(err) => {
                    eprintln!(
                        "warning: oracle review: PR #{} diff fetch failed: {err}",
                        pr.number
                    );
                    continue;
                }
            };

        let review_text = match invoke_oracle(
            &config.workspace_root,
            pr.number,
            &pr.head_sha,
            diff,
            config.oracle_review_timeout_secs,
        )
        .await
        {
            Ok(output) => output,
            Err(err) => {
                eprintln!(
                    "warning: oracle review: PR #{} {}: {err}",
                    pr.number,
                    classify_oracle_error(&err)
                );
                continue;
            }
        };

        let available_body_chars = GITHUB_COMMENT_LIMIT
            .saturating_sub(marker.chars().count())
            .saturating_sub(1);
        let truncated_body = truncate_for_github(review_text.trim(), available_body_chars);

        match github::post_bot_comment_with_marker_with_gh_bin(
            &config.gh_bin,
            &config.owner,
            &config.repo,
            pr.number,
            &marker,
            &truncated_body,
            &bot_login,
        )
        .await
        {
            Ok(_) => {
                success_count = success_count.saturating_add(1);
                state.mark_reviewed(pr.number, &pr.head_sha);
                if let Err(err) = state.save(&config.workspace_root) {
                    eprintln!(
                        "warning: oracle review: PR #{} state save failed: {err}",
                        pr.number
                    );
                }
            }
            Err(err) => {
                eprintln!(
                    "warning: oracle review: PR #{} comment post failed: {err}",
                    pr.number
                );
            }
        }
    }

    Ok(())
}

fn normalized_author_allowlist(authors: &[String]) -> HashSet<String> {
    authors
        .iter()
        .map(|author| author.to_ascii_lowercase())
        .collect()
}

fn oracle_review_state_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join("daemon").join("oracle-review-state")
}

fn state_path(workspace_root: &Path) -> PathBuf {
    oracle_review_state_dir(workspace_root).join("state.json")
}

fn oracle_review_marker(pr_number: u32, head_sha: &str) -> String {
    format!("<!-- ralph:oracle-review:{pr_number}:{head_sha} -->")
}

fn diff_temp_path(workspace_root: &Path, temp_stem: &str) -> PathBuf {
    oracle_review_state_dir(workspace_root).join(format!("{temp_stem}.diff"))
}

fn oracle_output_temp_path(workspace_root: &Path, temp_stem: &str) -> PathBuf {
    oracle_review_state_dir(workspace_root).join(format!("{temp_stem}.out"))
}

fn oracle_wrapper_temp_path(workspace_root: &Path, temp_stem: &str) -> PathBuf {
    oracle_review_state_dir(workspace_root).join(format!("{temp_stem}.mjs"))
}

fn temp_file_stem(pr_number: u32, head_sha: &str) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sanitized_sha: String = head_sha
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect();
    format!("pr-{pr_number}-{sanitized_sha}-{now}")
}

fn resolve_executable_in_path(binary: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|dir| dir.join(binary))
        .find(|candidate| candidate.is_file())
        .map(|candidate| fs::canonicalize(&candidate).unwrap_or(candidate))
}

fn resolve_oracle_cli_entrypoint() -> Option<PathBuf> {
    let oracle_path = resolve_executable_in_path("oracle")?;
    let bin_dir = oracle_path.parent()?;
    let dist_dir = bin_dir.parent()?;

    if oracle_path.file_name()?.to_str()? != "oracle-cli.js" {
        return None;
    }
    if bin_dir.file_name()?.to_str()? != "bin" {
        return None;
    }
    if dist_dir.file_name()?.to_str()? != "dist" {
        return None;
    }

    Some(oracle_path)
}

async fn invoke_oracle(
    workspace_root: &Path,
    pr_number: u32,
    head_sha: &str,
    diff: String,
    timeout_secs: u64,
) -> Result<String> {
    let workspace_root = workspace_root.to_path_buf();
    let head_sha = head_sha.to_owned();

    tokio::task::spawn_blocking(move || {
        let state_dir = oracle_review_state_dir(&workspace_root);
        fs::create_dir_all(&state_dir).map_err(|err| {
            RalphError::Orchestration(format!(
                "failed to create oracle review temp dir {}: {err}",
                state_dir.display()
            ))
        })?;

        let temp_stem = temp_file_stem(pr_number, &head_sha);
        let diff_path = diff_temp_path(&workspace_root, &temp_stem);
        let output_path = oracle_output_temp_path(&workspace_root, &temp_stem);
        let wrapper_path = oracle_wrapper_temp_path(&workspace_root, &temp_stem);
        fs::write(&diff_path, diff).map_err(|err| {
            RalphError::Orchestration(format!(
                "failed to write oracle review diff temp file {}: {err}",
                diff_path.display()
            ))
        })?;

        let result = (|| -> Result<String> {
            let oracle_cli_entrypoint = resolve_oracle_cli_entrypoint();
            let mut command = if let Some(cli_entrypoint) = oracle_cli_entrypoint {
                fs::write(&wrapper_path, ORACLE_SYSTEM_WRAPPER).map_err(|err| {
                    RalphError::Orchestration(format!(
                        "failed to write oracle wrapper {}: {err}",
                        wrapper_path.display()
                    ))
                })?;

                let mut command = std::process::Command::new("node");
                command
                    .arg(&wrapper_path)
                    .env("RALPH_ORACLE_CLI_PATH", cli_entrypoint);
                command
            } else {
                std::process::Command::new("oracle")
            };
            command
                .arg("--system")
                .arg(ORACLE_SYSTEM_PROMPT)
                .arg("--prompt")
                .arg(ORACLE_USER_PROMPT)
                .arg("--file")
                .arg(&diff_path)
                .arg("--write-output")
                .arg(&output_path);

            let output =
                process::run_command_with_timeout(&mut command, Duration::from_secs(timeout_secs))?;
            if !output.status.success() {
                return Err(RalphError::Orchestration(format!(
                    "oracle exited with status {}: {}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr).trim()
                )));
            }

            match fs::read_to_string(&output_path) {
                Ok(review_text) => Ok(review_text.trim().to_owned()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
                }
                Err(err) => Err(RalphError::Orchestration(format!(
                    "failed to read oracle output temp file {}: {err}",
                    output_path.display()
                ))),
            }
        })();

        let _ = fs::remove_file(&diff_path);
        let _ = fs::remove_file(&output_path);
        let _ = fs::remove_file(&wrapper_path);
        result
    })
    .await
    .map_err(|err| {
        RalphError::Orchestration(format!("oracle review blocking task join failure: {err}"))
    })?
}

fn classify_oracle_error(err: &RalphError) -> &'static str {
    let message = err.to_string();
    if message.contains("command timed out") {
        "oracle timeout"
    } else if message.contains("failed to spawn command") || message.contains("oracle spawn:") {
        "oracle spawn"
    } else if message.contains("oracle exited with status") {
        "oracle exit"
    } else {
        "oracle failure"
    }
}

#[cfg(test)]
mod tests {
    use super::{oracle_review_marker, state_path, OracleReviewState};

    #[test]
    fn oracle_review_state_load_defaults_when_missing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let state = OracleReviewState::load(temp.path()).expect("load should default");
        assert!(state.reviewed.is_empty());
    }

    #[test]
    fn oracle_review_state_roundtrip_save_and_load() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut state = OracleReviewState::default();
        state.mark_reviewed(17, "abc123");
        state.save(temp.path()).expect("save should succeed");

        let loaded = OracleReviewState::load(temp.path()).expect("load should succeed");
        assert_eq!(loaded.reviewed.get("17"), Some(&"abc123".to_owned()));
    }

    #[test]
    fn oracle_review_state_load_rejects_corrupt_json() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = state_path(temp.path());
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, "{bad").expect("write corrupt state");

        let error = OracleReviewState::load(temp.path()).expect_err("corrupt state must fail");
        assert!(error.to_string().contains("corrupted oracle review state"));
    }

    #[test]
    fn oracle_review_state_dedups_same_sha_and_allows_changed_sha() {
        let mut state = OracleReviewState::default();
        state.mark_reviewed(23, "sha-a");

        assert!(state.reviewed_sha_matches(23, "sha-a"));
        assert!(!state.reviewed_sha_matches(23, "sha-b"));
    }

    #[test]
    fn oracle_review_marker_matches_spec() {
        assert_eq!(
            oracle_review_marker(12, "deadbeef"),
            "<!-- ralph:oracle-review:12:deadbeef -->"
        );
    }
}
