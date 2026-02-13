use std::any::Any;
use std::path::PathBuf;

use clap::Args;

use crate::error::RalphError;
use crate::Result;

pub mod assertions;
pub mod harness;
pub mod mock_scripts;
pub mod runner;

mod tests_commands;
mod tests_daemon;
mod tests_init;
mod tests_mcp;
mod tests_project;
mod tests_prompt_review;
mod tests_qa;
mod tests_run;
mod tests_tail;

pub use runner::{ConformanceTest, TestResult, TestRunner};

#[derive(Debug, Args, Clone)]
pub struct ValidateArgs {
    #[arg(long, value_name = "PATH")]
    pub bin: PathBuf,
    #[arg(long, value_name = "PATTERN")]
    pub filter: Option<String>,
    #[arg(long)]
    pub list: bool,
    #[arg(long)]
    pub verbose: bool,
}

pub fn execute(args: ValidateArgs) -> Result<()> {
    // Resolve --bin to an absolute path so relative paths work regardless of
    // per-test harness cwd changes.
    let ralph_bin = std::fs::canonicalize(&args.bin).map_err(|e| {
        RalphError::Validation(format!(
            "--bin path '{}' does not exist or is not accessible: {e}",
            args.bin.display()
        ))
    })?;

    // Verify the resolved path is executable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(&ralph_bin).map_err(|e| {
            RalphError::Validation(format!(
                "--bin path '{}' cannot be stat'd: {e}",
                ralph_bin.display()
            ))
        })?;
        if meta.permissions().mode() & 0o111 == 0 {
            return Err(RalphError::Validation(format!(
                "--bin path '{}' is not executable",
                ralph_bin.display()
            )));
        }
    }

    let tests = register_tests();
    let runner = TestRunner::new(tests, ralph_bin, args.filter, args.verbose);
    let success = runner.run(args.list)?;

    if success {
        Ok(())
    } else {
        Err(RalphError::Orchestration(
            "conformance validation failed".to_owned(),
        ))
    }
}

fn register_tests() -> Vec<ConformanceTest> {
    let mut tests = Vec::new();
    tests.extend(tests_init::tests());
    tests.extend(tests_mcp::tests());
    tests.extend(tests_project::tests());
    tests.extend(tests_run::tests());
    tests.extend(tests_prompt_review::tests());
    tests.extend(tests_qa::tests());
    tests.extend(tests_commands::tests());
    tests.extend(tests_tail::tests());
    tests.extend(tests_daemon::tests());
    tests
}

pub(crate) fn panic_message(e: Box<dyn Any + Send>) -> String {
    if let Some(s) = e.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = e.downcast_ref::<&str>() {
        s.to_string()
    } else {
        "unknown panic".to_owned()
    }
}
