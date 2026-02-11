use std::fs;
use std::os::unix::fs as unix_fs;

use crate::cli::InitArgs;
use crate::prompts::templates::{
    default_completer_template, default_implementer_template, default_planner_template,
    default_qa_template, default_reviewer_template,
};
use crate::workspace::Workspace;
use crate::Result;

/// Execute the `ralph init` command, creating a workspace with default configuration,
/// index, and template files.
pub fn execute(args: InitArgs) -> Result<()> {
    let workspace = Workspace::init(&args.dir)?;

    let templates_dir = workspace.root.join("templates");

    // Write canonical template files
    fs::write(templates_dir.join("spec.md"), default_planner_template())?;
    fs::write(
        templates_dir.join("implementation.md"),
        default_implementer_template(),
    )?;
    fs::write(templates_dir.join("review.md"), default_reviewer_template())?;
    fs::write(
        templates_dir.join("completion.md"),
        default_completer_template(),
    )?;
    fs::write(templates_dir.join("qa.md"), default_qa_template())?;

    // Create legacy symlinks for backward compatibility.
    // If symlinking fails for a reason other than the link already existing,
    // fall back to copying the file so legacy names still resolve.
    let legacy_links: &[(&str, &str)] = &[
        ("spec.md", "planner.md"),
        ("implementation.md", "implementer.md"),
        ("review.md", "reviewer.md"),
        ("completion.md", "completer.md"),
    ];
    for (canonical, legacy) in legacy_links {
        let legacy_path = templates_dir.join(legacy);
        match unix_fs::symlink(canonical, &legacy_path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => {
                // Symlink unsupported or permission denied – copy instead
                let _ = fs::copy(templates_dir.join(canonical), &legacy_path);
            }
        }
    }

    println!("initialized workspace at {}", workspace.root.display());
    Ok(())
}
