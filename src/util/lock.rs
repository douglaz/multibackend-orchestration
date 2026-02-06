use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use fs2::FileExt;

use crate::error::RalphError;
use crate::Result;

pub struct ProjectLock {
    file: File,
    path: PathBuf,
}

impl ProjectLock {
    pub fn acquire(project_dir: &Path, project_id: &str) -> Result<Self> {
        let lock_path = project_dir.join(".lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)?;

        if file.try_lock_exclusive().is_err() {
            return Err(RalphError::StateLocked {
                project_id: project_id.to_owned(),
                lock_path,
            });
        }

        Ok(Self {
            file,
            path: project_dir.join(".lock"),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ProjectLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}
