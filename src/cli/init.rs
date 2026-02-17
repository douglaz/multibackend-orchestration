use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs as unix_fs;
use std::path::{Path, PathBuf};

use crate::cli::InitArgs;
use crate::config::GlobalConfig;
use crate::error::RalphError;
use crate::prompts::templates::{
    default_completer_template, default_implementer_template, default_planner_template,
    default_prompt_reviewer_template, default_qa_template, default_reviewer_template,
};
use crate::workspace::Workspace;
use crate::Result;

type TemplateContentFn = fn() -> &'static str;

pub(crate) const TEMPLATE_FILES: &[(&str, TemplateContentFn)] = &[
    ("spec.md", default_planner_template),
    ("implementation.md", default_implementer_template),
    ("review.md", default_reviewer_template),
    ("prompt_reviewer.md", default_prompt_reviewer_template),
    ("completion.md", default_completer_template),
    ("qa.md", default_qa_template),
];

pub(crate) const LEGACY_LINKS: &[(&str, &str)] = &[
    ("spec.md", "planner.md"),
    ("implementation.md", "implementer.md"),
    ("review.md", "reviewer.md"),
    ("completion.md", "completer.md"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InitAction {
    CreateDir {
        path: PathBuf,
    },
    WriteConfig {
        path: PathBuf,
    },
    WriteTemplate {
        path: PathBuf,
        content: &'static str,
    },
    CreateLegacyLink {
        canonical_path: PathBuf,
        canonical_name: &'static str,
        legacy_path: PathBuf,
    },
}

impl InitAction {
    fn describe(&self) -> String {
        match self {
            Self::CreateDir { path } => format!("create-dir {}", path.display()),
            Self::WriteConfig { path } => format!("write-config {}", path.display()),
            Self::WriteTemplate { path, .. } => format!("write-template {}", path.display()),
            Self::CreateLegacyLink {
                canonical_name,
                legacy_path,
                ..
            } => format!(
                "create-legacy-link {} -> {}",
                legacy_path.display(),
                canonical_name
            ),
        }
    }
}

fn invalid_target(path: &Path, reason: impl Into<String>) -> RalphError {
    RalphError::InitTargetInvalid {
        path: path.to_path_buf(),
        reason: reason.into(),
    }
}

fn validate_parent_chain(root: &Path) -> Result<()> {
    let mut current = root.parent();
    while let Some(parent) = current {
        match fs::metadata(parent) {
            Ok(metadata) => {
                if !metadata.is_dir() {
                    return Err(invalid_target(
                        root,
                        format!("parent '{}' is not a directory", parent.display()),
                    ));
                }
                fs::read_dir(parent).map_err(|err| {
                    invalid_target(
                        root,
                        format!("cannot access parent '{}': {err}", parent.display()),
                    )
                })?;
                return Ok(());
            }
            Err(err) if err.kind() == ErrorKind::NotFound => {
                current = parent.parent();
            }
            Err(err) => {
                return Err(invalid_target(
                    root,
                    format!("cannot access parent '{}': {err}", parent.display()),
                ));
            }
        }
    }

    Ok(())
}

pub(crate) fn validate_target(root: &Path) -> Result<()> {
    match fs::metadata(root) {
        Ok(metadata) => {
            if !metadata.is_dir() {
                return Err(invalid_target(root, "target exists but is not a directory"));
            }

            let mut entries = fs::read_dir(root).map_err(|err| {
                invalid_target(root, format!("cannot read target directory: {err}"))
            })?;
            if entries.next().transpose()?.is_some() {
                return Err(RalphError::Validation(format!(
                    "workspace directory '{}' already exists and is not empty",
                    root.display()
                )));
            }
            Ok(())
        }
        Err(err) if err.kind() == ErrorKind::NotFound => validate_parent_chain(root),
        Err(err) => Err(invalid_target(root, format!("cannot access target: {err}"))),
    }
}

pub(crate) fn plan_actions(root: &Path) -> Vec<InitAction> {
    let templates_dir = root.join("templates");

    let mut actions = vec![
        InitAction::CreateDir {
            path: root.join("projects"),
        },
        InitAction::CreateDir {
            path: templates_dir.clone(),
        },
        InitAction::WriteConfig {
            path: root.join("ralph.toml"),
        },
    ];

    for (name, content_fn) in TEMPLATE_FILES {
        actions.push(InitAction::WriteTemplate {
            path: templates_dir.join(name),
            content: content_fn(),
        });
    }

    for (canonical, legacy) in LEGACY_LINKS {
        actions.push(InitAction::CreateLegacyLink {
            canonical_path: templates_dir.join(canonical),
            canonical_name: canonical,
            legacy_path: templates_dir.join(legacy),
        });
    }

    actions
}

pub(crate) fn execute_actions(actions: &[InitAction]) -> Result<()> {
    for action in actions {
        match action {
            InitAction::CreateDir { path } => fs::create_dir_all(path)?,
            InitAction::WriteConfig { path } => {
                GlobalConfig::default().save(path)?;
            }
            InitAction::WriteTemplate { path, content } => {
                fs::write(path, content)?;
            }
            InitAction::CreateLegacyLink {
                canonical_path,
                canonical_name,
                legacy_path,
            } => match unix_fs::symlink(canonical_name, legacy_path) {
                Ok(()) => {}
                Err(err) if err.kind() == ErrorKind::AlreadyExists => {}
                Err(_) => {
                    let _ = fs::copy(canonical_path, legacy_path);
                }
            },
        }
    }

    Ok(())
}

fn create_workspace_from_actions(root: &Path, actions: &[InitAction]) -> Result<Workspace> {
    execute_actions(actions)?;
    Workspace::load(root.to_path_buf())
}

pub(crate) fn create_workspace(root: &Path) -> Result<Workspace> {
    let actions = plan_actions(root);
    create_workspace_from_actions(root, &actions)
}

pub(crate) fn print_actions(actions: &[InitAction]) {
    for action in actions {
        println!("dry-run: {}", action.describe());
    }
}

/// Execute the `ralph init` command, creating a workspace with default configuration,
/// index, and template files.
pub fn execute(args: InitArgs) -> Result<()> {
    validate_target(&args.dir)?;
    let actions = plan_actions(&args.dir);

    if args.dry_run {
        print_actions(&actions);
        return Ok(());
    }

    let workspace = create_workspace_from_actions(&args.dir, &actions)?;
    println!("initialized workspace at {}", workspace.root.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use tempfile::tempdir;

    use super::{
        create_workspace, plan_actions, validate_target, InitAction, LEGACY_LINKS, TEMPLATE_FILES,
    };
    use crate::error::RalphError;
    use crate::prompts::templates::{
        default_completer_template, default_implementer_template, default_planner_template,
        default_prompt_reviewer_template, default_qa_template, default_reviewer_template,
    };

    #[test]
    fn create_workspace_writes_all_templates() {
        let temp = tempdir().expect("temp dir");
        let workspace_root = temp.path().join(".ralph");
        let workspace = create_workspace(&workspace_root).expect("workspace should be created");

        assert_eq!(workspace.root, workspace_root);
        let templates_dir = workspace.root.join("templates");

        assert_eq!(
            std::fs::read_to_string(templates_dir.join("spec.md")).expect("read spec template"),
            default_planner_template()
        );
        assert_eq!(
            std::fs::read_to_string(templates_dir.join("implementation.md"))
                .expect("read implementation template"),
            default_implementer_template()
        );
        assert_eq!(
            std::fs::read_to_string(templates_dir.join("review.md")).expect("read review template"),
            default_reviewer_template()
        );
        assert_eq!(
            std::fs::read_to_string(templates_dir.join("prompt_reviewer.md"))
                .expect("read prompt reviewer template"),
            default_prompt_reviewer_template()
        );
        assert_eq!(
            std::fs::read_to_string(templates_dir.join("completion.md"))
                .expect("read completion template"),
            default_completer_template()
        );
        assert_eq!(
            std::fs::read_to_string(templates_dir.join("qa.md")).expect("read qa template"),
            default_qa_template()
        );

        for legacy in &[
            "planner.md",
            "implementer.md",
            "reviewer.md",
            "completer.md",
        ] {
            assert!(
                templates_dir.join(legacy).exists(),
                "legacy template path should exist: {legacy}"
            );
        }
    }

    #[test]
    fn plan_actions_uses_shared_constants_in_stable_order() {
        let actions = plan_actions(Path::new(".ralph"));

        assert!(matches!(
            actions.first(),
            Some(InitAction::CreateDir { .. })
        ));
        assert!(matches!(actions.get(1), Some(InitAction::CreateDir { .. })));
        assert!(matches!(
            actions.get(2),
            Some(InitAction::WriteConfig { .. })
        ));

        let template_actions = actions
            .iter()
            .filter(|action| matches!(action, InitAction::WriteTemplate { .. }))
            .count();
        let legacy_actions = actions
            .iter()
            .filter(|action| matches!(action, InitAction::CreateLegacyLink { .. }))
            .count();

        assert_eq!(template_actions, TEMPLATE_FILES.len());
        assert_eq!(legacy_actions, LEGACY_LINKS.len());
    }

    #[test]
    fn validate_target_rejects_nonempty_directory() {
        let temp = tempdir().expect("temp dir");
        let workspace_root = temp.path().join(".ralph");
        std::fs::create_dir_all(&workspace_root).expect("create dir");
        std::fs::write(workspace_root.join("existing.txt"), "x").expect("write existing file");

        let err = validate_target(&workspace_root).expect_err("non-empty target should fail");
        assert!(matches!(err, RalphError::Validation(_)));
    }

    #[test]
    fn validate_target_rejects_file_target() {
        let temp = tempdir().expect("temp dir");
        let workspace_file = temp.path().join(".ralph");
        std::fs::write(&workspace_file, "not-a-dir").expect("write file");

        let err = validate_target(&workspace_file).expect_err("file target should fail");
        assert!(matches!(err, RalphError::InitTargetInvalid { .. }));
    }
}
