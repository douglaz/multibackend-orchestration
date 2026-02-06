use std::fs;

use crate::cli::InitArgs;
use crate::prompts::templates::{
    default_completer_template, default_implementer_template, default_planner_template,
    default_reviewer_template,
};
use crate::workspace::Workspace;
use crate::Result;

/// Execute the `ralph init` command, creating a workspace with default configuration,
/// index, and template files.
pub fn execute(args: InitArgs) -> Result<()> {
    let workspace = Workspace::init(&args.dir)?;

    fs::write(
        workspace.root.join("templates/planner.md"),
        default_planner_template(),
    )?;
    fs::write(
        workspace.root.join("templates/implementer.md"),
        default_implementer_template(),
    )?;
    fs::write(
        workspace.root.join("templates/reviewer.md"),
        default_reviewer_template(),
    )?;
    fs::write(
        workspace.root.join("templates/completer.md"),
        default_completer_template(),
    )?;

    println!("initialized workspace at {}", workspace.root.display());
    Ok(())
}
