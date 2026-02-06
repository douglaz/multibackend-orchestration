pub mod discovery;
pub mod index;

use std::fs;
use std::path::{Path, PathBuf};

use crate::config::GlobalConfig;
use crate::error::RalphError;
use crate::util::time::now_utc;
use crate::Result;

use self::index::{ProjectRef, WorkspaceIndex};

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
        let index_path = root.join("index.json");
        let config = GlobalConfig::load(&config_path)?;
        let index = WorkspaceIndex::load(&index_path)?;

        Ok(Self {
            root,
            config,
            index,
        })
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
        index.save(&root.join("index.json"))?;

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

    pub fn active_project(&self) -> Option<&ProjectRef> {
        self.index.active_project_ref()
    }
}
