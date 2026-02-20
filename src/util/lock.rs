use std::fs;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use fs2::FileExt;

use crate::error::RalphError;
use crate::util::hash::sha256_hex;
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

pub struct DaemonLock {
    file: File,
    path: PathBuf,
    repo_root: PathBuf,
}

impl DaemonLock {
    pub fn acquire(repo_root: &Path) -> Result<Self> {
        let canonical_repo_root = fs::canonicalize(repo_root).map_err(|err| {
            RalphError::Orchestration(format!(
                "failed to canonicalize daemon repo root '{}': {err}",
                repo_root.display()
            ))
        })?;
        let digest = sha256_hex(canonical_repo_root.to_string_lossy().as_ref());
        let lock_path = PathBuf::from("/tmp").join(format!("ralph-daemon-{digest}.lock"));

        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)?;

        if file.try_lock_exclusive().is_err() {
            return Err(RalphError::DaemonLocked {
                repo_root: canonical_repo_root,
                lock_path,
            });
        }

        Ok(Self {
            file,
            path: lock_path,
            repo_root: canonical_repo_root,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }
}

impl Drop for DaemonLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}
