use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::cli::InitArgs;
use crate::config::GlobalConfig;
use crate::error::RalphError;
use crate::prompts::templates::{
    default_arbiter_template, default_completer_template, default_final_reviewer_template,
    default_implementer_template, default_planner_position_template, default_planner_template,
    default_prompt_review_validator_template, default_prompt_reviewer_template,
    default_qa_template, default_reviewer_template, default_vote_template,
};
use crate::workspace::Workspace;
use crate::Result;

type TemplateContentFn = fn() -> &'static str;

pub(crate) const TEMPLATE_FILES: &[(&str, TemplateContentFn)] = &[
    ("spec.md", default_planner_template),
    ("implementation.md", default_implementer_template),
    ("review.md", default_reviewer_template),
    ("prompt_reviewer.md", default_prompt_reviewer_template),
    (
        "prompt_review_validator.md",
        default_prompt_review_validator_template,
    ),
    ("completion.md", default_completer_template),
    ("qa.md", default_qa_template),
    ("final_reviewer.md", default_final_reviewer_template),
    ("planner_position.md", default_planner_position_template),
    ("vote.md", default_vote_template),
    ("arbiter.md", default_arbiter_template),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InitAction {
    CreateDir {
        path: PathBuf,
    },
    WriteMinimalConfig {
        path: PathBuf,
    },
    WriteConfig {
        path: PathBuf,
    },
    MergeConfig {
        path: PathBuf,
        config: GlobalConfig,
    },
    WriteTemplate {
        path: PathBuf,
        content: &'static str,
    },
    SkipExisting {
        path: PathBuf,
    },
}

impl InitAction {
    fn describe(&self) -> String {
        match self {
            Self::CreateDir { path } => format!("create-dir {}", path.display()),
            Self::WriteMinimalConfig { path } => format!("write-config {}", path.display()),
            Self::WriteConfig { path } => format!("write-config {}", path.display()),
            Self::MergeConfig { path, .. } => format!("merge-config {}", path.display()),
            Self::WriteTemplate { path, .. } => format!("write-template {}", path.display()),
            Self::SkipExisting { path } => format!("skip-existing {}", path.display()),
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

pub(crate) fn validate_target(root: &Path, copy_files: bool) -> Result<()> {
    match fs::metadata(root) {
        Ok(metadata) => {
            if !metadata.is_dir() {
                return Err(invalid_target(root, "target exists but is not a directory"));
            }

            let mut entries = fs::read_dir(root).map_err(|err| {
                invalid_target(root, format!("cannot read target directory: {err}"))
            })?;
            if entries.next().transpose()?.is_some()
                && (!copy_files || !root.join("ralph.toml").is_file())
            {
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

fn minimal_config_toml() -> String {
    let version = GlobalConfig::default().workspace.version;
    format!("[workspace]\nversion = {version:?}\n")
}

pub(crate) fn plan_actions_minimal(root: &Path) -> Vec<InitAction> {
    vec![
        InitAction::CreateDir {
            path: root.join("projects"),
        },
        InitAction::WriteMinimalConfig {
            path: root.join("ralph.toml"),
        },
    ]
}

pub(crate) fn plan_actions_full(root: &Path) -> Result<Vec<InitAction>> {
    let templates_dir = root.join("templates");
    let mut actions = vec![
        InitAction::CreateDir {
            path: root.join("projects"),
        },
        InitAction::CreateDir {
            path: templates_dir.clone(),
        },
    ];
    let config_path = root.join("ralph.toml");
    if config_path.exists() {
        actions.push(InitAction::MergeConfig {
            path: config_path.clone(),
            config: GlobalConfig::load(&config_path)?,
        });
    } else {
        actions.push(InitAction::WriteConfig { path: config_path });
    }

    for (name, content_fn) in TEMPLATE_FILES {
        let template_path = templates_dir.join(name);
        if template_path.exists() {
            actions.push(InitAction::SkipExisting {
                path: template_path,
            });
            continue;
        }
        actions.push(InitAction::WriteTemplate {
            path: template_path,
            content: content_fn(),
        });
    }

    Ok(actions)
}

pub(crate) fn plan_actions(root: &Path, copy_files: bool) -> Result<Vec<InitAction>> {
    if copy_files {
        plan_actions_full(root)
    } else {
        Ok(plan_actions_minimal(root))
    }
}

pub(crate) fn execute_actions(actions: &[InitAction]) -> Result<()> {
    for action in actions {
        match action {
            InitAction::CreateDir { path } => fs::create_dir_all(path)?,
            InitAction::WriteMinimalConfig { path } => fs::write(path, minimal_config_toml())?,
            InitAction::WriteConfig { path } => {
                GlobalConfig::default().save(path)?;
            }
            InitAction::MergeConfig { path, config } => {
                config.save(path)?;
            }
            InitAction::WriteTemplate { path, content } => {
                fs::write(path, content)?;
            }
            InitAction::SkipExisting { .. } => {}
        }
    }

    Ok(())
}

fn create_workspace_from_actions(root: &Path, actions: &[InitAction]) -> Result<Workspace> {
    execute_actions(actions)?;
    Workspace::load(root.to_path_buf())
}

pub(crate) fn create_workspace(root: &Path, copy_files: bool) -> Result<Workspace> {
    validate_target(root, copy_files)?;
    let actions = plan_actions(root, copy_files)?;
    create_workspace_from_actions(root, &actions)
}

pub(crate) fn print_actions(actions: &[InitAction]) {
    for action in actions {
        println!("dry-run: {}", action.describe());
    }
}

/// Execute the `ralph init` command, creating a workspace.
pub fn execute(args: InitArgs) -> Result<()> {
    validate_target(&args.dir, args.copy_files)?;
    let actions = plan_actions(&args.dir, args.copy_files)?;

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
        create_workspace, plan_actions_full, plan_actions_minimal, validate_target, InitAction,
        TEMPLATE_FILES,
    };
    use crate::error::RalphError;
    use crate::prompts::templates::{
        default_arbiter_template, default_completer_template, default_final_reviewer_template,
        default_implementer_template, default_planner_position_template, default_planner_template,
        default_prompt_review_validator_template, default_prompt_reviewer_template,
        default_qa_template, default_reviewer_template, default_vote_template,
    };
    use crate::{config::GlobalConfig, workspace::Workspace};

    #[test]
    fn create_workspace_minimal_writes_only_projects_and_minimal_config() {
        let temp = tempdir().expect("temp dir");
        let workspace_root = temp.path().join(".ralph");
        let workspace =
            create_workspace(&workspace_root, false).expect("workspace should be created");

        assert_eq!(workspace.root, workspace_root);
        assert!(workspace_root.join("projects").is_dir());
        assert!(!workspace_root.join("templates").exists());
        assert_eq!(workspace.config, GlobalConfig::default());

        let loaded = Workspace::load(workspace_root).expect("workspace should load");
        assert_eq!(loaded.config, GlobalConfig::default());
    }

    #[test]
    fn create_workspace_with_copy_files_writes_all_templates() {
        let temp = tempdir().expect("temp dir");
        let workspace_root = temp.path().join(".ralph");
        let workspace =
            create_workspace(&workspace_root, true).expect("workspace should be created");

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
            std::fs::read_to_string(templates_dir.join("prompt_review_validator.md"))
                .expect("read prompt review validator template"),
            default_prompt_review_validator_template()
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
        assert_eq!(
            std::fs::read_to_string(templates_dir.join("final_reviewer.md"))
                .expect("read final reviewer template"),
            default_final_reviewer_template()
        );
        assert_eq!(
            std::fs::read_to_string(templates_dir.join("planner_position.md"))
                .expect("read planner position template"),
            default_planner_position_template()
        );
        assert_eq!(
            std::fs::read_to_string(templates_dir.join("vote.md")).expect("read vote template"),
            default_vote_template()
        );
        assert_eq!(
            std::fs::read_to_string(templates_dir.join("arbiter.md"))
                .expect("read arbiter template"),
            default_arbiter_template()
        );
    }

    #[test]
    fn minimal_config_parses_to_default_config() {
        let temp = tempdir().expect("temp dir");
        let workspace_root = temp.path().join(".ralph");
        create_workspace(&workspace_root, false).expect("workspace should be created");

        let raw = std::fs::read_to_string(workspace_root.join("ralph.toml")).expect("read config");
        let parsed: GlobalConfig = toml::from_str(&raw).expect("parse minimal config");
        assert_eq!(parsed, GlobalConfig::default());
    }

    #[test]
    fn plan_actions_minimal_has_only_two_actions() {
        let actions = plan_actions_minimal(Path::new(".ralph"));
        assert_eq!(actions.len(), 2);
        assert!(matches!(
            actions.first(),
            Some(InitAction::CreateDir { .. })
        ));
        assert!(matches!(
            actions.get(1),
            Some(InitAction::WriteMinimalConfig { .. })
        ));
    }

    #[test]
    fn plan_actions_full_uses_shared_constants_in_stable_order() {
        let temp = tempdir().expect("temp dir");
        let root = temp.path().join(".ralph");
        let actions = plan_actions_full(&root).expect("plan full actions");

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

        assert_eq!(template_actions, TEMPLATE_FILES.len());
    }

    #[test]
    fn plan_actions_full_marks_overlay_steps_for_existing_workspace() {
        let temp = tempdir().expect("temp dir");
        let workspace_root = temp.path().join(".ralph");
        std::fs::create_dir_all(workspace_root.join("templates")).expect("create templates dir");
        std::fs::write(
            workspace_root.join("ralph.toml"),
            r#"[workspace]
default_backend = "codex"
"#,
        )
        .expect("write config");
        std::fs::write(workspace_root.join("templates").join("spec.md"), "custom")
            .expect("write existing template");

        let actions = plan_actions_full(&workspace_root).expect("plan full actions");
        assert!(
            actions
                .iter()
                .any(|action| matches!(action, InitAction::MergeConfig { .. })),
            "expected merge-config action for overlay mode"
        );
        assert!(
            actions
                .iter()
                .any(|action| matches!(action, InitAction::SkipExisting { path } if path.ends_with("spec.md"))),
            "expected skip-existing for existing template"
        );
    }

    #[test]
    fn validate_target_rejects_nonempty_directory_without_workspace_marker() {
        let temp = tempdir().expect("temp dir");
        let workspace_root = temp.path().join(".ralph");
        std::fs::create_dir_all(&workspace_root).expect("create dir");
        std::fs::write(workspace_root.join("existing.txt"), "x").expect("write existing file");

        let err =
            validate_target(&workspace_root, false).expect_err("non-empty target should fail");
        assert!(matches!(err, RalphError::Validation(_)));

        let err = validate_target(&workspace_root, true).expect_err("non-empty target should fail");
        assert!(matches!(err, RalphError::Validation(_)));
    }

    #[test]
    fn validate_target_allows_copy_files_overlay_when_ralph_toml_exists() {
        let temp = tempdir().expect("temp dir");
        let workspace_root = temp.path().join(".ralph");
        std::fs::create_dir_all(&workspace_root).expect("create dir");
        std::fs::write(workspace_root.join("ralph.toml"), "[workspace]\n").expect("write config");
        std::fs::write(workspace_root.join("note.txt"), "keep").expect("write note");

        validate_target(&workspace_root, true).expect("copy-files overlay should validate");
        let err = validate_target(&workspace_root, false).expect_err("minimal mode should reject");
        assert!(matches!(err, RalphError::Validation(_)));
    }

    #[test]
    fn validate_target_rejects_file_target() {
        let temp = tempdir().expect("temp dir");
        let workspace_file = temp.path().join(".ralph");
        std::fs::write(&workspace_file, "not-a-dir").expect("write file");

        let err = validate_target(&workspace_file, false).expect_err("file target should fail");
        assert!(matches!(err, RalphError::InitTargetInvalid { .. }));

        let err = validate_target(&workspace_file, true).expect_err("file target should fail");
        assert!(matches!(err, RalphError::InitTargetInvalid { .. }));
    }

    #[test]
    fn copy_files_overlay_fails_for_invalid_existing_config_without_partial_writes() {
        let temp = tempdir().expect("temp dir");
        let workspace_root = temp.path().join(".ralph");
        std::fs::create_dir_all(&workspace_root).expect("create workspace root");
        std::fs::write(workspace_root.join("ralph.toml"), "not = [valid").expect("write config");

        let err = create_workspace(&workspace_root, true).expect_err("invalid config should fail");
        assert!(matches!(err, RalphError::TomlDecode(_)));
        assert!(
            !workspace_root.join("templates").exists(),
            "copy-files overlay should not create templates on invalid config"
        );
    }

    #[test]
    fn minimal_and_copy_files_action_descriptions_match_dry_run_contract() {
        let minimal = plan_actions_minimal(Path::new(".ralph"))
            .into_iter()
            .map(|action| action.describe())
            .collect::<Vec<_>>();
        assert_eq!(
            minimal,
            vec![
                "create-dir .ralph/projects".to_owned(),
                "write-config .ralph/ralph.toml".to_owned(),
            ]
        );

        let temp = tempdir().expect("temp dir");
        let root = temp.path().join(".ralph");
        let full = plan_actions_full(&root)
            .expect("plan full actions")
            .into_iter()
            .take(3)
            .map(|action| action.describe())
            .collect::<Vec<_>>();
        assert_eq!(
            full[0],
            format!("create-dir {}", root.join("projects").display())
        );
        assert_eq!(
            full[1],
            format!("create-dir {}", root.join("templates").display())
        );
        assert_eq!(
            full[2],
            format!("write-config {}", root.join("ralph.toml").display())
        );
    }
}
