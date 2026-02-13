use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::Result;

pub fn resolve_git_dir(workspace_root: &Path) -> Option<PathBuf> {
    let repo_root = workspace_root.parent().unwrap_or(workspace_root);
    let output = Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(repo_root)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let git_dir = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if git_dir.is_empty() {
        return None;
    }

    let git_dir_path = PathBuf::from(&git_dir);
    if git_dir_path.is_absolute() {
        Some(git_dir_path)
    } else {
        Some(repo_root.join(git_dir_path))
    }
}

pub fn active_project_file_path(workspace_root: &Path) -> PathBuf {
    resolve_git_dir(workspace_root)
        .map(|git_dir| git_dir.join("ralph-active-project"))
        .unwrap_or_else(|| workspace_root.join(".active-project-local"))
}

pub fn read_active_project(workspace_root: &Path) -> Option<String> {
    let path = active_project_file_path(workspace_root);
    let raw = fs::read_to_string(&path).ok()?;
    let id = raw.trim();

    if id.is_empty() {
        return None;
    }

    if !is_valid_project_id(id) {
        eprintln!(
            "warning: ignoring invalid active project id '{}' from {}",
            id,
            path.display()
        );
        return None;
    }

    Some(id.to_owned())
}

pub fn write_active_project(workspace_root: &Path, id: &str) -> Result<()> {
    let path = active_project_file_path(workspace_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, format!("{id}\n"))?;
    Ok(())
}

fn is_valid_project_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use tempfile::tempdir;

    use super::{
        active_project_file_path, read_active_project, resolve_git_dir, write_active_project,
    };

    fn init_git_repo(dir: &Path) {
        let status = Command::new("git")
            .args(["init"])
            .current_dir(dir)
            .status()
            .expect("run git init");
        assert!(status.success());
    }

    #[test]
    fn read_write_roundtrip_non_git_workspace() {
        let temp = tempdir().expect("tempdir");
        let workspace_root = temp.path().join(".ralph");
        fs::create_dir_all(&workspace_root).expect("create workspace");

        write_active_project(&workspace_root, "demo_project").expect("write active project");
        let active = read_active_project(&workspace_root);

        assert_eq!(active.as_deref(), Some("demo_project"));
        assert_eq!(
            active_project_file_path(&workspace_root),
            workspace_root.join(".active-project-local")
        );
    }

    #[test]
    fn read_write_roundtrip_git_workspace() {
        let temp = tempdir().expect("tempdir");
        init_git_repo(temp.path());

        let workspace_root = temp.path().join(".ralph");
        fs::create_dir_all(&workspace_root).expect("create workspace");

        write_active_project(&workspace_root, "demo_project").expect("write active project");
        let active = read_active_project(&workspace_root);
        let path = active_project_file_path(&workspace_root);

        assert_eq!(active.as_deref(), Some("demo_project"));
        assert!(path.ends_with("ralph-active-project"));
        assert!(path.exists());
        assert!(resolve_git_dir(&workspace_root).is_some());
    }

    #[test]
    fn missing_active_project_file_returns_none() {
        let temp = tempdir().expect("tempdir");
        let workspace_root = temp.path().join(".ralph");
        fs::create_dir_all(&workspace_root).expect("create workspace");

        assert_eq!(read_active_project(&workspace_root), None);
    }

    #[test]
    fn empty_or_whitespace_active_project_file_returns_none() {
        let temp = tempdir().expect("tempdir");
        let workspace_root = temp.path().join(".ralph");
        fs::create_dir_all(&workspace_root).expect("create workspace");

        let path = active_project_file_path(&workspace_root);
        fs::write(&path, "\n").expect("write empty file");
        assert_eq!(read_active_project(&workspace_root), None);

        fs::write(&path, "   \t\n").expect("write whitespace file");
        assert_eq!(read_active_project(&workspace_root), None);
    }

    #[test]
    fn invalid_active_project_file_returns_none() {
        let temp = tempdir().expect("tempdir");
        let workspace_root = temp.path().join(".ralph");
        fs::create_dir_all(&workspace_root).expect("create workspace");

        let path = active_project_file_path(&workspace_root);
        fs::write(&path, "invalid id\n").expect("write invalid file");

        assert_eq!(read_active_project(&workspace_root), None);
    }
}
