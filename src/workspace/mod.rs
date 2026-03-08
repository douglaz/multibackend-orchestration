pub mod active;
pub mod discovery;
pub mod summary;

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use tracing::{info, warn};

use crate::config::GlobalConfig;
use crate::error::RalphError;
use crate::project::lifecycle::reconstruct_project_state;
use crate::Result;

use self::summary::{summarize_project, ProjectSummary};

#[derive(Debug, Clone)]
pub struct Workspace {
    pub root: PathBuf,
    pub config: GlobalConfig,
}

impl Workspace {
    pub fn discover() -> Result<Self> {
        let root = discovery::discover_workspace_root(None)?;
        Self::load(root)
    }

    /// Discover a workspace by walking up from `start` instead of ambient CWD.
    pub fn discover_from(start: &Path) -> Result<Self> {
        let root = discovery::discover_workspace_root(Some(start))?;
        Self::load(root)
    }

    pub fn load(root: PathBuf) -> Result<Self> {
        let config_path = root.join("ralph.toml");
        let config = GlobalConfig::load(&config_path)?;

        let ws = Self { root, config };

        // One-time migration: seed worktree-local active project from legacy
        // index.json if the local file doesn't exist yet.
        ws.migrate_active_project_from_index();

        Ok(ws)
    }

    /// If the worktree-local active-project file is absent and legacy
    /// `index.json` contains an `active_project` for an existing project,
    /// copy it to local storage. Errors are silently ignored.
    fn migrate_active_project_from_index(&self) {
        #[derive(Deserialize)]
        struct LegacyIndexFile {
            active_project: Option<String>,
        }

        // Only migrate if no local active project file exists yet.
        if self.active_project_id().is_some() {
            return;
        }
        // Check if the local file itself exists (even if empty/invalid).
        let local_path = active::active_project_file_path(&self.root);
        if local_path.exists() {
            return;
        }

        let index_path = self.root.join("index.json");
        let raw = match fs::read_to_string(&index_path) {
            Ok(raw) => raw,
            Err(_) => return,
        };
        let legacy = match serde_json::from_str::<LegacyIndexFile>(&raw) {
            Ok(legacy) => legacy,
            Err(_) => return,
        };

        if let Some(legacy_id) = legacy.active_project {
            if !self.project_exists(&legacy_id) {
                return;
            }
            if active::write_active_project(&self.root, &legacy_id).is_ok() {
                info!(
                    "migrated active project '{}' from index.json to worktree-local storage",
                    legacy_id
                );
            }
        }
    }

    pub fn init(root: &Path) -> Result<Self> {
        fs::create_dir_all(root.join("projects"))?;
        fs::create_dir_all(root.join("templates"))?;

        let config = GlobalConfig::default();
        config.save(&root.join("ralph.toml"))?;

        Ok(Self {
            root: root.to_path_buf(),
            config,
        })
    }

    pub fn save_config(&self) -> Result<()> {
        self.config.save(&self.root.join("ralph.toml"))
    }

    pub fn project_dir(&self, id: &str) -> PathBuf {
        self.root.join("projects").join(id)
    }

    pub fn list_projects(&self) -> Result<Vec<ProjectSummary>> {
        let projects_root = self.root.join("projects");
        if !projects_root.exists() {
            return Ok(Vec::new());
        }

        let mut projects = Vec::new();
        for entry in fs::read_dir(&projects_root)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            if !path.join("prompt.md").is_file() {
                continue;
            }

            let project_id = entry.file_name().to_string_lossy().to_string();
            let state = match reconstruct_project_state(self, &project_id) {
                Ok(state) => state,
                Err(err) => {
                    warn!(
                        "warning: skipping project directory '{}' because state derivation failed: {}",
                        path.display(),
                        err
                    );
                    continue;
                }
            };

            projects.push(summarize_project(&project_id, &state, &path));
        }

        projects.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(projects)
    }

    pub fn project_exists(&self, id: &str) -> bool {
        let project_dir = self.project_dir(id);
        project_dir.is_dir() && project_dir.join("prompt.md").is_file()
    }

    pub fn load_project_summary(&self, id: &str) -> Result<ProjectSummary> {
        if !self.project_exists(id) {
            return Err(RalphError::ProjectNotFound(id.to_owned()));
        }

        let project_dir = self.project_dir(id);
        let state = reconstruct_project_state(self, id)?;
        Ok(summarize_project(id, &state, &project_dir))
    }

    pub fn active_project_id(&self) -> Option<String> {
        active::read_active_project(&self.root)
    }

    /// Resolve the project ID from an explicit flag or the active-project file.
    /// If the active-project file points to a nonexistent project (stale),
    /// returns `ActiveProjectNotSet` with a user-facing hint.
    pub fn resolve_project_id(&self, explicit: Option<&str>) -> Result<String> {
        if let Some(id) = explicit {
            return Ok(id.to_owned());
        }
        let id = self
            .active_project_id()
            .ok_or(RalphError::ActiveProjectNotSet)?;
        if !self.project_exists(&id) {
            return Err(RalphError::Validation(format!(
                "active project '{}' no longer exists; run `ralph project use <id>` to set a new active project",
                id
            )));
        }
        Ok(id)
    }

    pub fn set_active_project_id(&self, id: &str) -> Result<()> {
        if !self.project_exists(id) {
            return Err(RalphError::ProjectNotFound(id.to_owned()));
        }
        active::write_active_project(&self.root, id)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::Workspace;
    use crate::error::RalphError;
    use crate::project::state::{
        CompletionLoopArtifacts, CompletionLoopBackends, CompletionLoopState, CompletionVerdict,
        FeatureLoopBackends, LoopStatus, LoopType, ProjectState, ProjectStatus,
    };

    fn create_workspace() -> (tempfile::TempDir, Workspace) {
        let temp = tempdir().expect("temp dir");
        let workspace_root = temp.path().join(".ralph");
        let workspace = Workspace::init(&workspace_root).expect("init workspace");
        (temp, workspace)
    }

    fn write_state(workspace: &Workspace, id: &str, mut state: ProjectState) {
        let project_dir = workspace.project_dir(id);
        fs::create_dir_all(project_dir.join("loops")).expect("create loops dir");
        state.project_id = id.to_owned();
        fs::write(project_dir.join("prompt.md"), "prompt").expect("write prompt");
        fs::write(
            project_dir.join("project.toml"),
            format!("name = {:?}\n", state.project_name),
        )
        .expect("write metadata");

        for loop_state in &state.loops {
            let loop_dir = project_dir
                .join("loops")
                .join(format!("{:03}-{}", loop_state.loop_number, loop_state.slug));
            fs::create_dir_all(&loop_dir).expect("create loop dir");
            fs::write(
                loop_dir.join("20260101000000-spec.md"),
                format!(
                    "---\nartifact: spec\nloop: {}\nbackend: planner\nrole: planner\ncreated_at: 2026-01-01T00:00:00Z\n---\n\n# Feature: {}\n",
                    loop_state.loop_number, loop_state.feature_name
                ),
            )
            .expect("write spec");

            if loop_state.status == LoopStatus::Completed {
                fs::write(
                    loop_dir.join("20260101000100-review-approved.md"),
                    format!(
                        "---\nartifact: review-approved\nloop: {}\nbackend: reviewer\nrole: reviewer\ncreated_at: 2026-01-01T00:01:00Z\n---\n\n# Review: APPROVED\n",
                        loop_state.loop_number
                    ),
                )
                .expect("write approval");
            }
        }

        for completion in &state.completion_attempts {
            let loop_dir = project_dir
                .join("loops")
                .join(format!("{:03}-completion", completion.loop_number));
            fs::create_dir_all(&loop_dir).expect("create completion loop dir");
            fs::write(
                loop_dir.join("20260101000200-termination-request.md"),
                format!(
                    "---\nartifact: termination-request\nloop: {}\nbackend: planner\nrole: planner\ncreated_at: 2026-01-01T00:02:00Z\n---\n\n# Project Completion Request\n",
                    completion.loop_number
                ),
            )
            .expect("write termination request");
            if completion.status == LoopStatus::Completed {
                let verdict_label = match completion.verdict {
                    Some(crate::project::state::CompletionVerdict::Complete) => "COMPLETE",
                    _ => "CONTINUE",
                };
                fs::write(
                    loop_dir.join("20260101000300-completer-verdict.md"),
                    format!(
                        "---\nartifact: completer-verdict\nloop: {}\nbackend: completer\nrole: completer\ncreated_at: 2026-01-01T00:03:00Z\n---\n\n# Verdict: {verdict_label}\n",
                        completion.loop_number
                    ),
                )
                .expect("write completer verdict");
            }
        }
    }

    fn demo_state(name: &str) -> ProjectState {
        let mut state = ProjectState::new("placeholder", name, "hash", None);
        state.status = ProjectStatus::Pending;
        state
    }

    #[test]
    fn list_projects_returns_empty_for_empty_projects_dir() {
        let (_temp, workspace) = create_workspace();
        let projects = workspace.list_projects().expect("list projects");
        assert!(projects.is_empty());
    }

    #[test]
    fn list_projects_scans_and_sorts_by_project_id() {
        let (_temp, workspace) = create_workspace();
        write_state(&workspace, "b-project", demo_state("Bravo"));
        write_state(&workspace, "a-project", demo_state("Alpha"));

        let projects = workspace.list_projects().expect("list projects");
        let ids: Vec<String> = projects.iter().map(|project| project.id.clone()).collect();
        assert_eq!(ids, vec!["a-project".to_owned(), "b-project".to_owned()]);
    }

    #[test]
    fn list_projects_skips_non_dirs_and_missing_prompt() {
        let (_temp, workspace) = create_workspace();
        write_state(&workspace, "valid", demo_state("Valid"));

        let non_dir_entry = workspace.root.join("projects").join("README.txt");
        fs::write(non_dir_entry, "ignore me").expect("write non-dir entry");

        let missing_prompt_dir = workspace.project_dir("missing");
        fs::create_dir_all(&missing_prompt_dir).expect("create missing prompt dir");

        let projects = workspace.list_projects().expect("list projects");
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].id, "valid");
    }

    #[test]
    fn project_exists_checks_for_prompt_file() {
        let (_temp, workspace) = create_workspace();
        write_state(&workspace, "exists", demo_state("Exists"));
        assert!(workspace.project_exists("exists"));
        assert!(!workspace.project_exists("missing"));
    }

    #[test]
    fn load_project_summary_returns_summary_for_existing_project() {
        let (_temp, workspace) = create_workspace();
        let mut state = demo_state("Demo");
        state.register_feature_loop(
            1,
            "demo-loop".to_owned(),
            "Demo Feature".to_owned(),
            FeatureLoopBackends {
                planner: "planner".to_owned(),
                implementer: "implementer".to_owned(),
                reviewer: "reviewer".to_owned(),
                qa: "qa".to_owned(),
            },
            "loops/001-demo/spec.md".to_owned(),
            chrono::Utc::now(),
        );
        if let Some(loop_state) = state.current_feature_loop_mut() {
            loop_state.status = LoopStatus::Completed;
            loop_state.completed_at = Some(chrono::Utc::now());
        }
        // Add a completion attempt with COMPLETE verdict so reconstruction
        // derives ProjectStatus::Completed.
        state.completion_attempts.push(CompletionLoopState {
            loop_number: 2,
            slug: "completion".to_owned(),
            loop_type: LoopType::Completion,
            status: LoopStatus::Completed,
            backends: CompletionLoopBackends::new(
                "planner".to_owned(),
                vec!["completer".to_owned()],
            ),
            artifacts: CompletionLoopArtifacts {
                termination_request: "loops/002-completion/termination-request.md".to_owned(),
                verdict: Some("loops/002-completion/completer-verdict.md".to_owned()),
                acceptance_results: vec![],
                acceptance_result: None,
                acceptance_passed: None,
            },
            verdict: Some(CompletionVerdict::Complete),
            started_at: chrono::Utc::now(),
            completed_at: Some(chrono::Utc::now()),
        });
        state.status = ProjectStatus::Completed;
        write_state(&workspace, "demo", state);

        let summary = workspace
            .load_project_summary("demo")
            .expect("load project summary");
        assert_eq!(summary.id, "demo");
        assert_eq!(summary.name, "Demo");
        assert_eq!(summary.total_feature_loops, 1);
        assert_eq!(summary.last_loop_number, 2);
        assert!(summary.completed_at.is_some());
    }

    #[test]
    fn load_project_summary_returns_not_found_for_missing_project() {
        let (_temp, workspace) = create_workspace();
        let err = workspace
            .load_project_summary("missing")
            .expect_err("missing project should error");
        assert!(matches!(err, RalphError::ProjectNotFound(id) if id == "missing"));
    }

    #[test]
    fn active_project_set_and_read_roundtrip() {
        let (_temp, workspace) = create_workspace();
        write_state(&workspace, "demo", demo_state("Demo"));

        workspace
            .set_active_project_id("demo")
            .expect("set active project");
        assert_eq!(workspace.active_project_id().as_deref(), Some("demo"));
    }

    #[test]
    fn set_active_project_rejects_missing_project() {
        let (_temp, workspace) = create_workspace();
        let err = workspace
            .set_active_project_id("missing")
            .expect_err("missing project should error");
        assert!(matches!(err, RalphError::ProjectNotFound(id) if id == "missing"));
    }
}
