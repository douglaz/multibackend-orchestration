pub mod amendments;
pub mod artifacts;
pub mod lifecycle;
pub mod state;

use std::path::Path;

use crate::config::ProjectConfig;
use crate::Result;

pub fn load_project_config_if_exists(project_dir: &Path) -> Result<Option<ProjectConfig>> {
    let path = project_dir.join("config.toml");
    if !path.exists() {
        return Ok(None);
    }

    let cfg = ProjectConfig::load(&path)?;
    Ok(Some(cfg))
}
