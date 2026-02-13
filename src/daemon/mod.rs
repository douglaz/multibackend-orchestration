pub mod github;
pub mod process;
pub mod refine;
pub mod runtime;
pub mod worktree;

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::error::RalphError;
use crate::util::time::now_iso8601;
use crate::Result;

use self::process as daemon_process;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Pending,
    InProgress,
    Completed,
    Failed,
    Aborted,
}

impl TaskState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Aborted => "aborted",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Aborted)
    }
}

impl std::fmt::Display for TaskState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonTask {
    pub task_id: String,
    pub state: TaskState,
    pub issue_number: u32,
    pub owner: String,
    pub repo: String,
    #[serde(default)]
    pub raw_idea: Option<String>,
    pub child_pid: Option<u32>,
    pub child_pgid: Option<u32>,
    pub branch: Option<String>,
    pub pr_url: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub fn format_task_id(owner: &str, repo: &str, issue_number: u32) -> String {
    format!("{owner}-{repo}-{issue_number}")
}

#[derive(Debug, Clone)]
pub struct TaskStore {
    tasks_path: PathBuf,
}

impl TaskStore {
    pub fn new(workspace_root: &Path) -> Self {
        Self {
            tasks_path: workspace_root.join("daemon").join("tasks.json"),
        }
    }

    pub fn path(&self) -> &Path {
        &self.tasks_path
    }

    pub fn load(&self) -> Result<Vec<DaemonTask>> {
        let mut file = self.open_lock_file()?;
        file.lock_shared()?;
        let tasks = read_tasks_from_file(&mut file)?;
        file.unlock()?;
        Ok(tasks)
    }

    pub fn save(&self, tasks: &[DaemonTask]) -> Result<()> {
        let mut file = self.open_lock_file()?;
        file.lock_exclusive()?;
        write_tasks_to_file(&mut file, tasks)?;
        file.unlock()?;
        Ok(())
    }

    pub fn update_task<F>(&self, task_id: &str, mut update: F) -> Result<DaemonTask>
    where
        F: FnMut(&mut DaemonTask) -> Result<()>,
    {
        self.with_exclusive_tasks(|tasks| {
            let task = tasks
                .iter_mut()
                .find(|task| task.task_id == task_id)
                .ok_or_else(|| RalphError::Validation(format!("task not found: {task_id}")))?;
            update(task)?;
            task.updated_at = now_iso8601();
            Ok(task.clone())
        })
    }

    pub fn with_exclusive_tasks<R, F>(&self, op: F) -> Result<R>
    where
        F: FnOnce(&mut Vec<DaemonTask>) -> Result<R>,
    {
        let mut file = self.open_lock_file()?;
        file.lock_exclusive()?;
        let mut tasks = read_tasks_from_file(&mut file)?;
        let result = op(&mut tasks)?;
        write_tasks_to_file(&mut file, &tasks)?;
        file.unlock()?;
        Ok(result)
    }

    fn open_lock_file(&self) -> Result<File> {
        if let Some(parent) = self.tasks_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&self.tasks_path)?;

        Ok(file)
    }
}

pub fn abort_task(store: &TaskStore, selector: &str) -> Result<DaemonTask> {
    let tasks = store.load()?;
    let index = resolve_task_index(&tasks, selector)?;
    let selected = tasks[index].clone();

    if selected.state.is_terminal() {
        return Err(RalphError::Validation(format!(
            "task {} is already terminal ({})",
            selected.task_id, selected.state
        )));
    }

    terminate_process_group_if_present(selected.child_pid, selected.child_pgid, &selected.task_id);

    let updated = store.update_task(&selected.task_id, |task| {
        if task.state.is_terminal() {
            return Err(RalphError::Validation(format!(
                "task {} is already terminal ({})",
                task.task_id, task.state
            )));
        }

        task.state = TaskState::Aborted;
        task.child_pid = None;
        task.child_pgid = None;
        Ok(())
    })?;

    update_abort_labels_best_effort(&updated);
    Ok(updated)
}

pub fn resolve_task_index(tasks: &[DaemonTask], selector: &str) -> Result<usize> {
    if let Ok(issue_number) = selector.parse::<u32>() {
        let matches = tasks
            .iter()
            .enumerate()
            .filter_map(|(idx, task)| (task.issue_number == issue_number).then_some((idx, task)))
            .collect::<Vec<_>>();

        if matches.is_empty() {
            return Err(RalphError::Validation(format!(
                "no task found for issue number {issue_number}"
            )));
        }

        if matches.len() > 1 {
            let ids = matches
                .iter()
                .map(|(_, task)| task.task_id.clone())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(RalphError::Validation(format!(
                "issue number {issue_number} is ambiguous; matching tasks: {ids}"
            )));
        }

        return Ok(matches[0].0);
    }

    tasks
        .iter()
        .position(|task| task.task_id == selector)
        .ok_or_else(|| RalphError::Validation(format!("task not found: {selector}")))
}

fn read_tasks_from_file(file: &mut File) -> Result<Vec<DaemonTask>> {
    file.seek(SeekFrom::Start(0))?;
    let mut raw = String::new();
    file.read_to_string(&mut raw)?;

    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }

    Ok(serde_json::from_str(&raw)?)
}

fn write_tasks_to_file(file: &mut File, tasks: &[DaemonTask]) -> Result<()> {
    file.seek(SeekFrom::Start(0))?;
    file.set_len(0)?;
    let raw = serde_json::to_string_pretty(tasks)?;
    file.write_all(raw.as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

fn terminate_process_group_if_present(
    child_pid: Option<u32>,
    child_pgid: Option<u32>,
    _task_id: &str,
) {
    // Prefer killing by process group; fall back to single PID.
    if let Some(pgid) = child_pgid.filter(|v| *v > 0) {
        daemon_process::terminate_process_group(pgid, Duration::from_secs(10));
        return;
    }
    if let Some(pid) = child_pid.filter(|v| *v > 0) {
        // No PGID available — treat the single PID as a one-member "group".
        daemon_process::terminate_process_group(pid, Duration::from_secs(10));
    }
}

fn update_abort_labels_best_effort(task: &DaemonTask) {
    let repo = format!("{}/{}", task.owner, task.repo);
    let issue_number = task.issue_number.to_string();

    let output = Command::new("gh")
        .args([
            "issue",
            "edit",
            &issue_number,
            "--repo",
            &repo,
            "--remove-label",
            "ralph:in-progress",
            "--add-label",
            "ralph:aborted",
        ])
        .output();

    match output {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            eprintln!(
                "warning: failed to update labels for {}#{}: {}",
                repo,
                task.issue_number,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Err(err) => {
            eprintln!(
                "warning: failed to run gh for {}#{} label update: {}",
                repo, task.issue_number, err
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{resolve_task_index, DaemonTask, TaskState};

    fn task(task_id: &str, issue_number: u32) -> DaemonTask {
        DaemonTask {
            task_id: task_id.to_owned(),
            state: TaskState::Pending,
            issue_number,
            owner: "acme".to_owned(),
            repo: "widgets".to_owned(),
            raw_idea: None,
            child_pid: None,
            child_pgid: None,
            branch: None,
            pr_url: None,
            created_at: "2026-01-01T00:00:00Z".to_owned(),
            updated_at: "2026-01-01T00:00:00Z".to_owned(),
        }
    }

    #[test]
    fn resolve_task_index_by_full_id() {
        let tasks = vec![task("acme-widgets-1", 1), task("acme-widgets-2", 2)];
        let idx = resolve_task_index(&tasks, "acme-widgets-2").expect("task should resolve");
        assert_eq!(idx, 1);
    }

    #[test]
    fn resolve_task_index_by_bare_issue_number() {
        let tasks = vec![task("acme-widgets-1", 1), task("acme-widgets-2", 2)];
        let idx = resolve_task_index(&tasks, "2").expect("task should resolve");
        assert_eq!(idx, 1);
    }

    #[test]
    fn resolve_task_index_rejects_ambiguous_bare_issue_number() {
        let tasks = vec![task("acme-widgets-7", 7), task("other-api-7", 7)];
        let err = resolve_task_index(&tasks, "7").expect_err("should be ambiguous");
        assert!(err.to_string().contains("ambiguous"));
    }

    #[test]
    fn daemon_task_deserializes_without_raw_idea_for_backwards_compatibility() {
        let raw = r#"{
            "task_id":"acme-widgets-1",
            "state":"pending",
            "issue_number":1,
            "owner":"acme",
            "repo":"widgets",
            "child_pid":null,
            "child_pgid":null,
            "branch":null,
            "pr_url":null,
            "created_at":"2026-01-01T00:00:00Z",
            "updated_at":"2026-01-01T00:00:00Z"
        }"#;

        let task: DaemonTask = serde_json::from_str(raw).expect("legacy task json should parse");
        assert!(task.raw_idea.is_none());
    }

    #[test]
    fn daemon_task_round_trips_with_raw_idea() {
        let mut original = task("acme-widgets-2", 2);
        original.raw_idea = Some("Issue title\n\nIssue body".to_owned());

        let raw = serde_json::to_string(&original).expect("serialize task");
        let decoded: DaemonTask = serde_json::from_str(&raw).expect("deserialize task");
        assert_eq!(
            decoded.raw_idea.as_deref(),
            Some("Issue title\n\nIssue body")
        );
    }
}
