use std::fs;
use std::path::Path;

use chrono::Utc;

use crate::error::RalphError;
use crate::project::amendments::{
    enqueue_amendment, AmendmentPriority, AmendmentRequest, AmendmentSource,
};
use crate::util::time::format_timestamp_yyyymmddhhmmss;
use crate::workspace::Workspace;
use crate::Result;

use super::AmendArgs;

pub fn execute(args: AmendArgs) -> Result<()> {
    let workspace = Workspace::discover()?;
    let project_id = workspace.resolve_project_id(args.project.as_deref())?;

    if !workspace.project_exists(&project_id) {
        return Err(RalphError::ProjectNotFound(project_id));
    }

    let project_dir = workspace.project_dir(&project_id);

    let priority = parse_priority(&args.priority)?;
    let body = resolve_body(&args.body)?;

    let now = Utc::now();
    let id = args
        .id
        .unwrap_or_else(|| format!("EXT-{}", format_timestamp_yyyymmddhhmmss(now)));

    let request = AmendmentRequest {
        id,
        body,
        priority,
        source: AmendmentSource::Cli,
        source_detail: None,
        created_at: now,
    };

    let queue_path = enqueue_amendment(&project_dir, &request)?;
    println!("{}", queue_path.display());
    Ok(())
}

fn parse_priority(raw: &str) -> Result<AmendmentPriority> {
    match raw {
        "P0" => Ok(AmendmentPriority::P0),
        "P1" => Ok(AmendmentPriority::P1),
        "P2" => Ok(AmendmentPriority::P2),
        "P3" => Ok(AmendmentPriority::P3),
        _ => Err(RalphError::Validation(format!(
            "invalid priority '{raw}': must be one of P0, P1, P2, P3"
        ))),
    }
}

fn resolve_body(raw: &str) -> Result<String> {
    if let Some(path_str) = raw.strip_prefix('@') {
        let path = Path::new(path_str);
        let content = fs::read_to_string(path).map_err(|e| {
            RalphError::Validation(format!(
                "failed to read body from '{}': {e}",
                path.display()
            ))
        })?;
        Ok(content)
    } else {
        Ok(raw.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_priority_accepts_valid_values() {
        assert_eq!(parse_priority("P0").unwrap(), AmendmentPriority::P0);
        assert_eq!(parse_priority("P1").unwrap(), AmendmentPriority::P1);
        assert_eq!(parse_priority("P2").unwrap(), AmendmentPriority::P2);
        assert_eq!(parse_priority("P3").unwrap(), AmendmentPriority::P3);
    }

    #[test]
    fn parse_priority_rejects_invalid_values() {
        assert!(parse_priority("p0").is_err());
        assert!(parse_priority("P4").is_err());
        assert!(parse_priority("high").is_err());
        assert!(parse_priority("").is_err());
    }

    #[test]
    fn resolve_body_returns_inline_text() {
        let body = resolve_body("fix the bug").unwrap();
        assert_eq!(body, "fix the bug");
    }

    #[test]
    fn resolve_body_loads_file_content() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(temp.path(), "file body content").unwrap();
        let arg = format!("@{}", temp.path().display());
        let body = resolve_body(&arg).unwrap();
        assert_eq!(body, "file body content");
    }

    #[test]
    fn resolve_body_returns_error_for_missing_file() {
        let result = resolve_body("@/nonexistent/path/to/body.txt");
        assert!(result.is_err());
    }
}
