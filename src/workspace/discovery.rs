use std::env;
use std::path::{Path, PathBuf};

use crate::error::RalphError;
use crate::Result;

pub fn discover_workspace_root(start: Option<&Path>) -> Result<PathBuf> {
    let mut current = if let Some(path) = start {
        path.to_path_buf()
    } else {
        env::current_dir()?
    };

    if current.file_name().is_some_and(|name| name == ".ralph") {
        return Ok(current);
    }

    loop {
        let candidate = current.join(".ralph");
        if candidate.is_dir() && candidate.join("ralph.toml").is_file() {
            return Ok(candidate);
        }

        if !current.pop() {
            return Err(RalphError::WorkspaceNotFound);
        }
    }
}
