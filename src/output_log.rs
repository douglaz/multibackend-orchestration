use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub fn log_path_for_role(project_dir: &Path, loop_number: Option<u32>, role: &str) -> PathBuf {
    let filename = format!("agent-output-{role}.log");
    match loop_number {
        Some(loop_number) => project_dir
            .join("loops")
            .join(format!("{loop_number:03}"))
            .join(filename),
        None => project_dir.join(filename),
    }
}

pub fn ensure_log_parent(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}

pub fn sanitize_for_filename(label: &str) -> String {
    let mut sanitized = String::with_capacity(label.len());
    let mut last_was_underscore = false;

    for ch in label.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' {
            sanitized.push(ch);
            last_was_underscore = false;
        } else if !last_was_underscore {
            sanitized.push('_');
            last_was_underscore = true;
        }
    }

    sanitized.trim_matches('_').to_owned()
}

#[cfg(test)]
mod tests {
    use super::{ensure_log_parent, log_path_for_role, sanitize_for_filename};
    use std::path::Path;
    use tempfile::tempdir;

    #[test]
    fn derives_loop_scoped_log_path() {
        let project_dir = Path::new("/tmp/project");
        let path = log_path_for_role(project_dir, Some(7), "implementer");
        assert_eq!(
            path,
            Path::new("/tmp/project/loops/007/agent-output-implementer.log")
        );
    }

    #[test]
    fn derives_prompt_reviewer_root_log_path_when_loop_is_none() {
        let project_dir = Path::new("/tmp/project");
        let path = log_path_for_role(project_dir, None, "prompt-reviewer");
        assert_eq!(
            path,
            Path::new("/tmp/project/agent-output-prompt-reviewer.log")
        );
    }

    #[test]
    fn formats_loop_number_edges_with_three_digits() {
        let project_dir = Path::new("/tmp/project");
        let loop_zero = log_path_for_role(project_dir, Some(0), "planner");
        let loop_max = log_path_for_role(project_dir, Some(999), "planner");

        assert_eq!(
            loop_zero,
            Path::new("/tmp/project/loops/000/agent-output-planner.log")
        );
        assert_eq!(
            loop_max,
            Path::new("/tmp/project/loops/999/agent-output-planner.log")
        );
    }

    #[test]
    fn creates_missing_parent_directories() {
        let temp = tempdir().expect("tempdir");
        let log_path = temp.path().join("loops/004/agent-output-reviewer.log");

        ensure_log_parent(&log_path).expect("parent directories should be created");

        assert!(
            log_path
                .parent()
                .expect("parent")
                .try_exists()
                .expect("exists check"),
            "parent path should exist after ensure_log_parent"
        );
    }

    #[test]
    fn sanitizes_unsafe_filename_characters() {
        assert_eq!(sanitize_for_filename("../"), "");
        assert_eq!(sanitize_for_filename("; rm -rf"), "rm_-rf");
        assert_eq!(sanitize_for_filename("backend label with spaces"), "backend_label_with_spaces");
        assert_eq!(sanitize_for_filename("日本語"), "");
    }

    #[test]
    fn sanitization_collapses_and_trims_underscores() {
        assert_eq!(sanitize_for_filename("___alpha___beta___"), "alpha_beta");
        assert_eq!(sanitize_for_filename("a////b"), "a_b");
    }

    #[test]
    fn sanitization_handles_empty_input() {
        assert_eq!(sanitize_for_filename(""), "");
    }
}
