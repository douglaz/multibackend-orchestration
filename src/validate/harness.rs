use std::ffi::OsStr;
use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
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
        let repo_root = temp_dir.path().join("repo");
        let ralph_bin = bin.as_ref().to_path_buf();
        initialize_git_repo(&repo_root, true)?;

        Ok(Self {
            temp_dir,
            repo_root,
            ralph_bin,
        })
    }

    /// Create a harness with a git repo that has zero commits (only `git init`).
    pub fn new_zero_commit_repo<P: AsRef<Path>>(bin: P) -> Result<Self> {
        let temp_dir = TempDir::new()?;
        let repo_root = temp_dir.path().join("repo");
        let ralph_bin = bin.as_ref().to_path_buf();
        initialize_git_repo(&repo_root, false)?;

        Ok(Self {
            temp_dir,
            repo_root,
            ralph_bin,
        })
    }

    /// Create a harness with repo_root at `<temp_dir>/<owner>/<repo>` to model
    /// a daemon data-dir layout.
    pub fn new_daemon<P: AsRef<Path>>(bin: P, owner: &str, repo: &str) -> Result<Self> {
        let temp_dir = TempDir::new()?;
        let repo_root = temp_dir.path().join(owner).join(repo);
        let ralph_bin = bin.as_ref().to_path_buf();
        initialize_git_repo(&repo_root, true)?;

        Ok(Self {
            temp_dir,
            repo_root,
            ralph_bin,
        })
    }

    pub fn data_dir(&self) -> &Path {
        self.temp_dir.path()
    }

    pub fn data_dir_str(&self) -> String {
        self.data_dir().to_string_lossy().into_owned()
    }

    pub fn ralph<I, S>(&self, args: I) -> Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args = self.prepare_cli_args(args);
        let output = Command::new(&self.ralph_bin)
            .args(args)
            .current_dir(&self.repo_root)
            .output()?;
        Ok(output)
    }

    pub fn ralph_with_stdin<I, S>(&self, args: I, input: &str) -> Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        use std::io::Write;
        use std::process::Stdio;

        let args = self.prepare_cli_args(args);
        let mut child = Command::new(&self.ralph_bin)
            .args(args)
            .current_dir(&self.repo_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(input.as_bytes())?;
        }

        let output = child.wait_with_output()?;
        Ok(output)
    }

    pub fn ralph_env<I, S>(&self, args: I, env_vars: &[(&str, &str)]) -> Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args = self.prepare_cli_args(args);
        let mut command = Command::new(&self.ralph_bin);
        command.args(args).current_dir(&self.repo_root);
        for (key, value) in env_vars {
            command.env(key, value);
        }
        Ok(command.output()?)
    }

    pub fn daemon_env<I, S>(&self, args: I, env_vars: &[(&str, &str)]) -> Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args = self.prepare_cli_args(args);
        let mut command = Command::new(&self.ralph_bin);
        command.args(args).current_dir(self.data_dir());
        for (key, value) in env_vars {
            command.env(key, value);
        }
        Ok(command.output()?)
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

    /// Read the worktree-local active-project file (`.git/ralph-active-project`).
    /// Returns `Ok(None)` if the file is absent, empty, or whitespace-only.
    pub fn load_active_project(&self) -> Result<Option<String>> {
        let git_dir = self.repo_root.join(".git");
        let path = git_dir.join("ralph-active-project");
        match fs::read_to_string(&path) {
            Ok(content) => {
                let trimmed = content.trim().to_owned();
                if trimmed.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(trimmed))
                }
            }
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    /// Assert that `.ralph/index.json` does NOT exist.
    pub fn assert_no_index_json(&self) {
        let path = self.repo_root.join(".ralph").join("index.json");
        assert!(
            !path.exists(),
            "index.json should not exist at {}",
            path.display()
        );
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

    /// Interpreter-stable mock backend setup that avoids reliance on
    /// `/usr/bin/env` (absent in Nix sandboxes). Creates a thin `/bin/sh`
    /// wrapper that execs `bash <script>`, ignoring any extra CLI args
    /// (e.g. `--model opus`) that the backend registry may inject. The
    /// wrapper is set as the backend command with empty args so stale
    /// defaults (e.g. codex `exec -`) don't interfere.
    pub fn setup_mock_backends_stable<P: AsRef<Path>>(&self, script: P) -> Result<()> {
        let script = script.as_ref().to_string_lossy().into_owned();
        // Write a POSIX shell wrapper that calls bash with the real script.
        // The wrapper ignores all positional args (model flags, etc.) because
        // it does not pass "$@" through to bash.
        let wrapper_content = format!("#!/bin/sh\nexec bash \"{script}\"\n");
        let wrapper = self.write_mock_script("mock-wrapper.sh", &wrapper_content)?;
        let wrapper_str = wrapper.to_string_lossy().into_owned();

        for backend in &["claude", "codex"] {
            self.ralph_ok(vec![
                "config".to_owned(),
                "set".to_owned(),
                format!("backends.{backend}.command"),
                wrapper_str.clone(),
            ])?;
            self.ralph_ok(vec![
                "config".to_owned(),
                "set".to_owned(),
                format!("backends.{backend}.args"),
                "[]".to_owned(),
            ])?;
        }
        Ok(())
    }

    pub fn setup_separate_mock_backends<P: AsRef<Path>, Q: AsRef<Path>>(
        &self,
        claude_script: P,
        codex_script: Q,
    ) -> Result<()> {
        let claude = claude_script.as_ref().to_string_lossy().into_owned();
        let codex = codex_script.as_ref().to_string_lossy().into_owned();
        self.ralph_ok(vec![
            "config".to_owned(),
            "set".to_owned(),
            "backends.claude.command".to_owned(),
            claude,
        ])?;
        self.ralph_ok(vec![
            "config".to_owned(),
            "set".to_owned(),
            "backends.codex.command".to_owned(),
            codex,
        ])?;
        Ok(())
    }

    fn prepare_cli_args<I, S>(&self, args: I) -> Vec<OsString>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut prepared: Vec<OsString> = args
            .into_iter()
            .map(|arg| arg.as_ref().to_os_string())
            .collect();
        inject_daemon_data_dir_arg(self.data_dir(), &mut prepared);
        prepared
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

    pub fn project_dir(&self, project_id: &str) -> PathBuf {
        self.repo_root
            .join(".ralph")
            .join("projects")
            .join(project_id)
    }

    pub fn loop_dir(&self, project_id: &str, loop_number: u32) -> Result<Option<PathBuf>> {
        let loops_dir = self.project_dir(project_id).join("loops");
        let entries = match fs::read_dir(&loops_dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err.into()),
        };

        let prefix = format!("{loop_number:03}-");
        for entry in entries {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let name = entry.file_name();
            if name.to_string_lossy().starts_with(&prefix) {
                return Ok(Some(entry.path()));
            }
        }

        Ok(None)
    }

    pub fn list_artifacts(&self, project_id: &str, loop_number: u32) -> Result<Vec<PathBuf>> {
        let Some(loop_dir) = self.loop_dir(project_id, loop_number)? else {
            return Ok(Vec::new());
        };

        let mut files = Vec::new();
        for entry in fs::read_dir(loop_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                files.push(entry.path());
            }
        }
        files.sort();
        Ok(files)
    }
}

fn initialize_git_repo(repo_root: &Path, with_initial_commit: bool) -> Result<()> {
    fs::create_dir_all(repo_root)?;

    run_git(repo_root, &["init"])?;
    run_git(repo_root, &["config", "user.email", "validate@example.com"])?;
    run_git(repo_root, &["config", "user.name", "Validate Harness"])?;
    let origin_bare = repo_root.parent().unwrap_or(repo_root).join("origin.git");
    run_git(
        repo_root,
        &["init", "--bare", origin_bare.to_string_lossy().as_ref()],
    )?;
    run_git(
        repo_root,
        &[
            "remote",
            "add",
            "origin",
            origin_bare.to_string_lossy().as_ref(),
        ],
    )?;

    if with_initial_commit {
        fs::write(repo_root.join(".gitkeep"), "")?;
        run_git(repo_root, &["add", ".gitkeep"])?;
        run_git(repo_root, &["commit", "-m", "chore: initial commit"])?;
        run_git(repo_root, &["branch", "-M", "master"])?;
        run_git(repo_root, &["push", "-u", "origin", "master"])?;
    }

    Ok(())
}

fn inject_daemon_data_dir_arg(data_dir: &Path, args: &mut Vec<OsString>) {
    if args.len() < 2 {
        return;
    }
    if args[0] != OsStr::new("daemon") {
        return;
    }

    let subcommand = args[1].as_os_str();
    let needs_data_dir = subcommand == OsStr::new("start")
        || subcommand == OsStr::new("status")
        || subcommand == OsStr::new("abort");
    if !needs_data_dir {
        return;
    }

    let has_data_dir = args.iter().any(|arg| arg == OsStr::new("--data-dir"));
    if has_data_dir {
        return;
    }

    args.insert(2, data_dir.as_os_str().to_os_string());
    args.insert(2, OsString::from("--data-dir"));
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
