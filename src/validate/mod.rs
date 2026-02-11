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
mod tests_init;
mod tests_project;
mod tests_run;

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
    let tests = register_tests();
    let runner = TestRunner::new(tests, args.bin, args.filter, args.verbose);
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
    tests.extend(tests_project::tests());
    tests.extend(tests_run::tests());
    tests.extend(tests_commands::tests());
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
