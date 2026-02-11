use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

use crate::error::RalphError;
use crate::validate::assertions::assert_exit_code;
use crate::Result;

#[derive(Debug)]
pub struct RalphHarness {
    pub temp_dir: TempDir,
    pub repo_root: PathBuf,
    pub ralph_bin: PathBuf,
}

impl RalphHarness {
    pub fn new<P: AsRef<Path>>(bin: P) -> Result<Self> {
        let temp_dir = TempDir::new()?;
        let repo_root = temp_dir.path().to_path_buf();
        let ralph_bin = bin.as_ref().to_path_buf();

        run_git(&repo_root, &["init"])?;
        run_git(
            &repo_root,
            &["config", "user.email", "validate@example.com"],
        )?;
        run_git(&repo_root, &["config", "user.name", "Validate Harness"])?;

        fs::write(repo_root.join(".gitkeep"), "")?;
        run_git(&repo_root, &["add", ".gitkeep"])?;
        run_git(&repo_root, &["commit", "-m", "chore: initial commit"])?;
        run_git(&repo_root, &["branch", "-M", "master"])?;

        Ok(Self {
            temp_dir,
            repo_root,
            ralph_bin,
        })
    }

    pub fn ralph<I, S>(&self, args: I) -> Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = Command::new(&self.ralph_bin)
            .args(args)
            .current_dir(&self.repo_root)
            .output()?;
        Ok(output)
    }

    pub fn ralph_ok<I, S>(&self, args: I) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.ralph(args)?;
        assert_exit_code(&output, 0);
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    pub fn ralph_exit<I, S>(&self, args: I, code: i32) -> Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.ralph(args)?;
        assert_exit_code(&output, code);
        Ok(output)
    }

    pub fn load_state(&self, project_id: &str) -> Result<Value> {
        let path = self
            .repo_root
            .join(".ralph")
            .join("projects")
            .join(project_id)
            .join("state.json");
        load_json(&path)
    }

    pub fn load_index(&self) -> Result<Value> {
        load_json(&self.repo_root.join(".ralph").join("index.json"))
    }

    pub fn init_workspace(&self) -> Result<()> {
        self.ralph_ok(["init"])?;
        Ok(())
    }

    pub fn write_mock_script(&self, name: &str, content: &str) -> Result<PathBuf> {
        let path = self.temp_dir.path().join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, content)?;

        let mut perms = fs::metadata(&path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms)?;
        Ok(path)
    }

    pub fn setup_mock_backends<P: AsRef<Path>>(&self, script: P) -> Result<()> {
        let script = script.as_ref().to_string_lossy().into_owned();
        self.ralph_ok(vec![
            "config".to_owned(),
            "set".to_owned(),
            "backends.claude.command".to_owned(),
            script.clone(),
        ])?;
        self.ralph_ok(vec![
            "config".to_owned(),
            "set".to_owned(),
            "backends.codex.command".to_owned(),
            script,
        ])?;
        Ok(())
    }

    pub fn create_project(&self, id: &str, name: &str, prompt: &str) -> Result<()> {
        let prompt_path = self.temp_dir.path().join(format!("{id}-prompt.md"));
        fs::write(&prompt_path, prompt)?;

        self.ralph_ok(vec![
            "project".to_owned(),
            "new".to_owned(),
            "--id".to_owned(),
            id.to_owned(),
            "--name".to_owned(),
            name.to_owned(),
            "--prompt".to_owned(),
            prompt_path.to_string_lossy().into_owned(),
        ])?;
        Ok(())
    }
}

fn load_json(path: &Path) -> Result<Value> {
    let raw = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw)?)
}

fn run_git(repo_root: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()?;

    if output.status.success() {
        return Ok(());
    }

    Err(RalphError::Orchestration(format!(
        "git command failed in {}: git {}: {}",
        repo_root.display(),
        args.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}
