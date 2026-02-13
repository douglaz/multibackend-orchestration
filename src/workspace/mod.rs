pub mod active;
pub mod discovery;
pub mod index;
pub mod summary;

use std::fs;
use std::path::{Path, PathBuf};

use crate::config::GlobalConfig;
use crate::error::RalphError;
use crate::project::state::ProjectState;
use crate::util::time::now_utc;
use crate::Result;

use self::index::{ProjectRef, WorkspaceIndex};
use self::summary::{summarize_project, ProjectSummary};

#[derive(Debug, Clone)]
pub struct Workspace {
    pub root: PathBuf,
    pub config: GlobalConfig,
    pub index: WorkspaceIndex,
}

impl Workspace {
    pub fn discover() -> Result<Self> {
        let root = discovery::discover_workspace_root(None)?;
        Self::load(root)
    }

    pub fn load(root: PathBuf) -> Result<Self> {
        let config_path = root.join("ralph.toml");
        let config = GlobalConfig::load(&config_path)?;

        let index_path = root.join("index.json");
        let index = if index_path.is_file() {
            WorkspaceIndex::load(&index_path).unwrap_or_else(|_| {
                WorkspaceIndex::new(&config.workspace.version, now_utc())
            })
        } else {
            WorkspaceIndex::new(&config.workspace.version, now_utc())
        };

        let ws = Self {
            root,
            config,
            index,
        };

        // One-time migration: seed worktree-local active project from legacy
        // index.json if the local file doesn't exist yet.
        ws.migrate_active_project_from_index();

        Ok(ws)
    }

    /// If the worktree-local active-project file is absent and legacy
    /// `index.json` contains an `active_project` for an existing project,
    /// copy it to local storage. Errors are silently ignored.
    fn migrate_active_project_from_index(&self) {
        // Only migrate if no local active project file exists yet.
        if self.active_project_id().is_some() {
            return;
        }
        // Check if the local file itself exists (even if empty/invalid).
        let local_path = active::active_project_file_path(&self.root);
        if local_path.exists() {
            return;
        }

        if let Some(ref legacy_id) = self.index.active_project {
            if self.project_exists(legacy_id) {
                if active::write_active_project(&self.root, legacy_id).is_ok() {
                    eprintln!(
                        "migrated active project '{}' from index.json to worktree-local storage",
                        legacy_id
                    );
                }
            }
        }
    }

    pub fn init(root: &Path) -> Result<Self> {
        if root.exists() {
            let mut entries = fs::read_dir(root)?;
            if entries.next().is_some() {
                return Err(RalphError::Validation(format!(
                    "workspace directory '{}' already exists and is not empty",
                    root.display()
                )));
            }
        }

        fs::create_dir_all(root.join("projects"))?;
        fs::create_dir_all(root.join("templates"))?;

        let config = GlobalConfig::default();
        config.save(&root.join("ralph.toml"))?;

        let index = WorkspaceIndex::new(&config.workspace.version, now_utc());

        Ok(Self {
            root: root.to_path_buf(),
            config,
            index,
        })
    }

    pub fn save_index(&self) -> Result<()> {
        self.index.save(&self.root.join("index.json"))
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

            let state_path = path.join("state.json");
            if !state_path.is_file() {
                eprintln!(
                    "warning: skipping project directory '{}' because state.json is missing",
                    path.display()
                );
                continue;
            }

            let state = match ProjectState::load(&state_path) {
                Ok(state) => state,
                Err(err) => {
                    eprintln!(
                        "warning: skipping project directory '{}' because state.json is invalid: {}",
                        path.display(),
                        err
                    );
                    continue;
                }
            };

            let project_id = entry.file_name().to_string_lossy().to_string();
            projects.push(summarize_project(&project_id, &state));
        }

        projects.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(projects)
    }

    pub fn project_exists(&self, id: &str) -> bool {
        self.project_dir(id).join("state.json").is_file()
    }

    pub fn load_project_summary(&self, id: &str) -> Result<ProjectSummary> {
        if !self.project_exists(id) {
            return Err(RalphError::ProjectNotFound(id.to_owned()));
        }

        let state_path = self.project_dir(id).join("state.json");
        let state = ProjectState::load(&state_path)?;
        Ok(summarize_project(id, &state))
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

    pub fn active_project(&self) -> Option<&ProjectRef> {
        self.index.active_project_ref()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::Workspace;
    use crate::error::RalphError;
    use crate::project::state::{FeatureLoopBackends, LoopStatus, ProjectState, ProjectStatus};

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
        state
            .save(&project_dir.join("state.json"))
            .expect("save state");
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
    fn list_projects_skips_non_dirs_missing_state_and_malformed_state() {
        let (_temp, workspace) = create_workspace();
        write_state(&workspace, "valid", demo_state("Valid"));

        let non_dir_entry = workspace.root.join("projects").join("README.txt");
        fs::write(non_dir_entry, "ignore me").expect("write non-dir entry");

        let missing_state_dir = workspace.project_dir("missing");
        fs::create_dir_all(&missing_state_dir).expect("create missing state dir");

        let malformed_state_dir = workspace.project_dir("malformed");
        fs::create_dir_all(&malformed_state_dir).expect("create malformed dir");
        fs::write(malformed_state_dir.join("state.json"), "{").expect("write malformed state");

        let projects = workspace.list_projects().expect("list projects");
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].id, "valid");
    }

    #[test]
    fn project_exists_checks_for_state_json() {
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
        state.status = ProjectStatus::Completed;
        write_state(&workspace, "demo", state);

        let summary = workspace
            .load_project_summary("demo")
            .expect("load project summary");
        assert_eq!(summary.id, "demo");
        assert_eq!(summary.name, "Demo");
        assert_eq!(summary.total_feature_loops, 1);
        assert_eq!(summary.last_loop_number, 1);
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
