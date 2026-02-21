pub mod backend;
pub mod cli;
pub mod config;
pub mod daemon;
pub mod error;
pub mod git;
pub mod output_log;
pub mod prd;
pub mod project;
pub mod prompts;
pub mod util;
pub mod validate;
pub mod workflow;
pub mod workspace;

pub type Result<T> = std::result::Result<T, error::RalphError>;
