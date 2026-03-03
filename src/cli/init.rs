use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use toml_edit::DocumentMut;

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

pub(crate) const MINIMAL_TOML: &str = r#"
# ralph workspace configuration
# The defaults are loaded from built-in values in the application and shown here
# for convenience so this file remains intentionally minimal.
[workspace]
"#;

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
    WriteTemplate {
        path: PathBuf,
        content: &'static str,
    },
    OverlayConfig {
        path: PathBuf,
    },
}

impl InitAction {
    fn describe(&self) -> String {
        match self {
            Self::CreateDir { path } => format!("create-dir {}", path.display()),
            Self::WriteMinimalConfig { path } => format!("write-config {}", path.display()),
            Self::WriteConfig { path } => format!("write-config {}", path.display()),
            Self::WriteTemplate { path, .. } => format!("write-template {}", path.display()),
            Self::OverlayConfig { path } => format!("overlay-config {}", path.display()),
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

/// Result of validating a `--copy-files` target directory.
#[derive(Debug)]
enum CopyFilesTarget {
    /// Directory is new or empty — full scaffold needed.
    NewOrEmpty,
    /// Existing workspace with valid ralph.toml — overlay mode.
    ExistingWorkspace,
}

/// Validate the target for `--copy-files` mode. Returns:
/// - `Ok(NewOrEmpty)` for non-existent or empty directories.
/// - `Ok(ExistingWorkspace)` when `ralph.toml` exists and parses.
/// - `Err` with exit code 2 for non-workspace non-empty dirs.
/// - `Err` with exit code 1 for malformed ralph.toml.
fn validate_copy_files_target(root: &Path) -> Result<CopyFilesTarget> {
    match fs::metadata(root) {
        Ok(metadata) => {
            if !metadata.is_dir() {
                return Err(invalid_target(root, "target exists but is not a directory"));
            }

            let mut entries = fs::read_dir(root).map_err(|err| {
                invalid_target(root, format!("cannot read target directory: {err}"))
            })?;
            let is_empty = entries.next().transpose()?.is_none();
            if is_empty {
                return Ok(CopyFilesTarget::NewOrEmpty);
            }

            // Non-empty directory — check for ralph.toml
            let toml_path = root.join("ralph.toml");
            match fs::metadata(&toml_path) {
                Ok(m) if m.is_file() => {}
                Ok(_) | Err(_) => {
                    return Err(RalphError::Validation(
                        "directory exists but is not a ralph workspace (no ralph.toml found)"
                            .to_owned(),
                    ));
                }
            }

            // ralph.toml exists — try to parse it
            let raw = fs::read_to_string(&toml_path)?;
            let _config: GlobalConfig = toml::from_str(&raw).map_err(|e| {
                RalphError::Orchestration(format!("failed to parse ralph.toml: {e}"))
            })?;

            Ok(CopyFilesTarget::ExistingWorkspace)
        }
        Err(err) if err.kind() == ErrorKind::NotFound => {
            validate_parent_chain(root)?;
            Ok(CopyFilesTarget::NewOrEmpty)
        }
        Err(err) => Err(invalid_target(root, format!("cannot access target: {err}"))),
    }
}

pub(crate) fn plan_full_actions(root: &Path) -> Vec<InitAction> {
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

    actions
}

pub(crate) fn plan_minimal_actions(root: &Path) -> Vec<InitAction> {
    vec![
        InitAction::CreateDir {
            path: root.join("projects"),
        },
        InitAction::WriteMinimalConfig {
            path: root.join("ralph.toml"),
        },
    ]
}

/// Plan overlay actions for an existing workspace. Only creates missing
/// directories and template files; merges config without overwriting.
fn plan_overlay_actions(root: &Path) -> Vec<InitAction> {
    let templates_dir = root.join("templates");

    let mut actions = Vec::new();

    // Ensure projects/ and templates/ directories exist.
    if !root.join("projects").is_dir() {
        actions.push(InitAction::CreateDir {
            path: root.join("projects"),
        });
    }
    if !templates_dir.is_dir() {
        actions.push(InitAction::CreateDir {
            path: templates_dir.clone(),
        });
    }

    // Overlay config merge.
    actions.push(InitAction::OverlayConfig {
        path: root.join("ralph.toml"),
    });

    // Only create missing template files.
    for (name, content_fn) in TEMPLATE_FILES {
        let path = templates_dir.join(name);
        if !path.exists() {
            actions.push(InitAction::WriteTemplate {
                path,
                content: content_fn(),
            });
        }
    }

    actions
}

/// Merge missing default keys into an existing TOML document.
/// Preserves existing values, comments, and formatting. Only adds
/// keys present in the default reference but absent from the existing doc.
fn merge_overlay_config(existing_raw: &str) -> Result<String> {
    let mut existing_doc: DocumentMut = existing_raw
        .parse()
        .map_err(|e| RalphError::Orchestration(format!("failed to parse ralph.toml: {e}")))?;

    let default_config = GlobalConfig::default();
    let default_raw = toml::to_string_pretty(&default_config)?;
    let default_doc: DocumentMut = default_raw
        .parse()
        .map_err(|e| RalphError::Orchestration(format!("failed to parse default config: {e}")))?;

    merge_tables(existing_doc.as_table_mut(), default_doc.as_table());

    Ok(existing_doc.to_string())
}

/// Recursively merge missing keys from `default` into `existing`.
/// Never overwrites values already present in `existing`.
/// Handles inline tables by converting them to regular tables when
/// nested defaults need to be merged in.
fn merge_tables(existing: &mut toml_edit::Table, default: &toml_edit::Table) {
    for (key, default_item) in default.iter() {
        if existing.contains_key(key) {
            // Determine if the default side is a table we should recurse into.
            let default_table = match default_item {
                toml_edit::Item::Table(t) => Some(t),
                toml_edit::Item::Value(toml_edit::Value::InlineTable(_)) => {
                    // Handled in the else-if branch below.
                    None
                }
                _ => None,
            };

            if let Some(dt) = default_table {
                // Default is a regular table — recurse if existing is also table-like.
                let existing_item = existing.get_mut(key).expect("key exists");
                if let Some(existing_table) = existing_item.as_table_mut() {
                    merge_tables(existing_table, dt);
                } else if existing_item.as_inline_table().is_some() {
                    // Convert existing inline table to regular table for merging.
                    let inline = existing_item.as_inline_table().unwrap().clone();
                    let mut regular = inline.into_table();
                    regular.set_implicit(true);
                    merge_tables(&mut regular, dt);
                    *existing_item = toml_edit::Item::Table(regular);
                }
                // Otherwise existing is a scalar — user value takes precedence.
            } else if default_item
                .as_value()
                .and_then(|v| v.as_inline_table())
                .is_some()
            {
                // Default is an inline table — convert to regular table for merge.
                let default_inline = default_item
                    .as_value()
                    .unwrap()
                    .as_inline_table()
                    .unwrap();
                let default_as_table = default_inline.clone().into_table();
                let existing_item = existing.get_mut(key).expect("key exists");
                if let Some(existing_table) = existing_item.as_table_mut() {
                    merge_tables(existing_table, &default_as_table);
                } else if existing_item.as_inline_table().is_some() {
                    let inline = existing_item.as_inline_table().unwrap().clone();
                    let mut regular = inline.into_table();
                    regular.set_implicit(true);
                    merge_tables(&mut regular, &default_as_table);
                    *existing_item = toml_edit::Item::Table(regular);
                }
            }
            // Otherwise, the existing value takes precedence — do nothing.
        } else {
            // Key missing from existing: insert the default.
            existing.insert(key, default_item.clone());
        }
    }
}

pub(crate) fn execute_actions(actions: &[InitAction]) -> Result<()> {
    for action in actions {
        match action {
            InitAction::CreateDir { path } => fs::create_dir_all(path)?,
            InitAction::WriteMinimalConfig { path } => {
                fs::write(path, MINIMAL_TOML)?;
            }
            InitAction::WriteConfig { path } => {
                GlobalConfig::default().save(path)?;
            }
            InitAction::WriteTemplate { path, content } => {
                fs::write(path, content)?;
            }
            InitAction::OverlayConfig { path } => {
                let existing_raw = fs::read_to_string(path)?;
                let merged = merge_overlay_config(&existing_raw)?;
                fs::write(path, merged)?;
            }
        }
    }

    Ok(())
}

fn create_workspace_from_actions(root: &Path, actions: &[InitAction]) -> Result<Workspace> {
    execute_actions(actions)?;
    Workspace::load(root.to_path_buf())
}

pub(crate) fn create_workspace(root: &Path) -> Result<Workspace> {
    validate_target(root)?;
    let actions = plan_minimal_actions(root);
    create_workspace_from_actions(root, &actions)
}

pub(crate) fn print_actions(actions: &[InitAction]) {
    for action in actions {
        println!("dry-run: {}", action.describe());
    }
}

/// Execute the `ralph init` command, creating a workspace with the requested
/// initialization scaffold.
pub fn execute(args: InitArgs) -> Result<()> {
    if args.copy_files {
        let target = validate_copy_files_target(&args.dir)?;
        let actions = match target {
            CopyFilesTarget::NewOrEmpty => plan_full_actions(&args.dir),
            CopyFilesTarget::ExistingWorkspace => plan_overlay_actions(&args.dir),
        };

        if args.dry_run {
            print_actions(&actions);
            return Ok(());
        }

        let workspace = create_workspace_from_actions(&args.dir, &actions)?;
        println!("initialized workspace at {}", workspace.root.display());
        Ok(())
    } else {
        validate_target(&args.dir)?;
        let actions = plan_minimal_actions(&args.dir);

        if args.dry_run {
            print_actions(&actions);
            return Ok(());
        }

        let workspace = create_workspace_from_actions(&args.dir, &actions)?;
        println!("initialized workspace at {}", workspace.root.display());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use tempfile::tempdir;

    use super::{
        create_workspace, create_workspace_from_actions, merge_overlay_config, plan_full_actions,
        plan_minimal_actions, plan_overlay_actions, validate_copy_files_target, validate_target,
        CopyFilesTarget, InitAction, TEMPLATE_FILES,
    };
    use crate::config::GlobalConfig;
    use crate::error::RalphError;
    use crate::prompts::templates::{
        default_arbiter_template, default_completer_template, default_final_reviewer_template,
        default_implementer_template, default_planner_position_template, default_planner_template,
        default_prompt_review_validator_template, default_prompt_reviewer_template,
        default_qa_template, default_reviewer_template, default_vote_template,
    };

    #[test]
    fn create_workspace_full_plan_writes_all_templates() {
        let temp = tempdir().expect("temp dir");
        let workspace_root = temp.path().join(".ralph");
        let actions = plan_full_actions(&workspace_root);
        let workspace = create_workspace_from_actions(&workspace_root, &actions)
            .expect("workspace should be created");

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
    fn create_workspace_minimal_plan_writes_no_templates() {
        let temp = tempdir().expect("temp dir");
        let workspace_root = temp.path().join(".ralph");
        let workspace = create_workspace(&workspace_root).expect("workspace should be created");

        assert_eq!(workspace.root, workspace_root);
        assert!(!workspace_root.join("templates").exists());
        let toml = std::fs::read_to_string(workspace_root.join("ralph.toml"))
            .expect("read minimal toml");
        assert!(toml.contains("[workspace]"));
        assert!(!toml.contains("workspace.version"));
        assert!(!toml.contains("backends."));
    }

    #[test]
    fn plan_full_actions_uses_shared_constants_in_stable_order() {
        let actions = plan_full_actions(Path::new(".ralph"));

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
    fn plan_minimal_actions_keeps_only_minimal_workspace_steps() {
        let actions = plan_minimal_actions(Path::new(".ralph"));

        assert!(matches!(
            actions.first(),
            Some(InitAction::CreateDir { .. })
        ));
        assert!(matches!(actions.get(1), Some(InitAction::WriteMinimalConfig { .. })));

        let template_actions = actions
            .iter()
            .filter(|action| matches!(action, InitAction::WriteTemplate { .. }))
            .count();
        let dir_actions = actions
            .iter()
            .filter(|action| matches!(action, InitAction::CreateDir { .. }))
            .count();

        assert_eq!(template_actions, 0);
        assert_eq!(dir_actions, 1);
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

    #[test]
    fn validate_copy_files_target_new_dir() {
        let temp = tempdir().expect("temp dir");
        let workspace_root = temp.path().join(".ralph");
        let result =
            validate_copy_files_target(&workspace_root).expect("new dir should be NewOrEmpty");
        assert!(matches!(result, CopyFilesTarget::NewOrEmpty));
    }

    #[test]
    fn validate_copy_files_target_empty_dir() {
        let temp = tempdir().expect("temp dir");
        let workspace_root = temp.path().join(".ralph");
        std::fs::create_dir_all(&workspace_root).expect("create dir");
        let result =
            validate_copy_files_target(&workspace_root).expect("empty dir should be NewOrEmpty");
        assert!(matches!(result, CopyFilesTarget::NewOrEmpty));
    }

    #[test]
    fn validate_copy_files_target_existing_workspace() {
        let temp = tempdir().expect("temp dir");
        let workspace_root = temp.path().join(".ralph");
        std::fs::create_dir_all(&workspace_root).expect("create dir");
        std::fs::write(
            workspace_root.join("ralph.toml"),
            super::MINIMAL_TOML,
        )
        .expect("write ralph.toml");
        let result = validate_copy_files_target(&workspace_root)
            .expect("existing workspace should be ExistingWorkspace");
        assert!(matches!(result, CopyFilesTarget::ExistingWorkspace));
    }

    #[test]
    fn validate_copy_files_target_nonempty_no_toml() {
        let temp = tempdir().expect("temp dir");
        let workspace_root = temp.path().join(".ralph");
        std::fs::create_dir_all(&workspace_root).expect("create dir");
        std::fs::write(workspace_root.join("some_file.txt"), "stuff").expect("write file");

        let err = validate_copy_files_target(&workspace_root)
            .expect_err("non-workspace should error");
        assert!(matches!(err, RalphError::Validation(ref msg) if msg.contains("not a ralph workspace")));
    }

    #[test]
    fn validate_copy_files_target_malformed_toml() {
        let temp = tempdir().expect("temp dir");
        let workspace_root = temp.path().join(".ralph");
        std::fs::create_dir_all(&workspace_root).expect("create dir");
        std::fs::write(
            workspace_root.join("ralph.toml"),
            "[workspace]\nversion = [1, 2, 3]\n",
        )
        .expect("write bad ralph.toml");

        let err = validate_copy_files_target(&workspace_root)
            .expect_err("malformed toml should error");
        let msg = err.to_string();
        assert!(
            msg.contains("failed to parse ralph.toml"),
            "expected 'failed to parse ralph.toml', got: {msg}"
        );
    }

    #[test]
    fn merge_overlay_config_fills_missing_keys() {
        let existing = "[workspace]\nversion = \"2.0\"\n";
        let merged = merge_overlay_config(existing).expect("merge should succeed");

        let parsed: GlobalConfig =
            toml::from_str(&merged).expect("merged config should parse");
        assert_eq!(parsed.workspace.version, "2.0");
        // Default-filled keys should be present.
        assert_eq!(parsed.workspace.default_backend, GlobalConfig::default().workspace.default_backend);
    }

    #[test]
    fn merge_overlay_config_preserves_user_values() {
        let existing = "[workspace]\ndefault_backend = \"codex\"\n";
        let merged = merge_overlay_config(existing).expect("merge should succeed");

        let parsed: GlobalConfig =
            toml::from_str(&merged).expect("merged config should parse");
        assert_eq!(parsed.workspace.default_backend, "codex");
    }

    #[test]
    fn merge_overlay_config_preserves_comments() {
        let existing = "# my workspace\n[workspace]\nversion = \"1.0\"\n";
        let merged = merge_overlay_config(existing).expect("merge should succeed");
        assert!(
            merged.contains("# my workspace"),
            "comment should be preserved in merged output"
        );
    }

    #[test]
    fn plan_overlay_actions_skips_existing_templates() {
        let temp = tempdir().expect("temp dir");
        let workspace_root = temp.path().join(".ralph");
        std::fs::create_dir_all(workspace_root.join("templates")).expect("create templates dir");
        std::fs::write(workspace_root.join("ralph.toml"), super::MINIMAL_TOML)
            .expect("write ralph.toml");
        // Create one template file.
        std::fs::write(workspace_root.join("templates/spec.md"), "custom").expect("write spec.md");

        let actions = plan_overlay_actions(&workspace_root);

        // spec.md should NOT appear in actions since it already exists.
        let template_names: Vec<_> = actions
            .iter()
            .filter_map(|a| {
                if let InitAction::WriteTemplate { path, .. } = a {
                    path.file_name().map(|n| n.to_string_lossy().into_owned())
                } else {
                    None
                }
            })
            .collect();
        assert!(!template_names.contains(&"spec.md".to_owned()));
        assert_eq!(template_names.len(), TEMPLATE_FILES.len() - 1);
    }

    #[test]
    fn plan_overlay_actions_includes_overlay_config() {
        let temp = tempdir().expect("temp dir");
        let workspace_root = temp.path().join(".ralph");
        std::fs::create_dir_all(&workspace_root).expect("create dir");
        std::fs::write(workspace_root.join("ralph.toml"), super::MINIMAL_TOML)
            .expect("write ralph.toml");

        let actions = plan_overlay_actions(&workspace_root);
        let has_overlay = actions
            .iter()
            .any(|a| matches!(a, InitAction::OverlayConfig { .. }));
        assert!(has_overlay, "overlay actions should include OverlayConfig");
    }

    #[test]
    fn merge_overlay_config_handles_inline_tables() {
        // Existing config uses inline table syntax.
        let existing = "workspace = { default_backend = \"codex\" }\n";
        let merged = merge_overlay_config(existing).expect("merge should succeed");

        let parsed: GlobalConfig =
            toml::from_str(&merged).expect("merged config should parse");
        // User value preserved.
        assert_eq!(parsed.workspace.default_backend, "codex");
        // Missing default keys should be filled in.
        assert_eq!(
            parsed.workspace.version,
            GlobalConfig::default().workspace.version,
            "version should be filled from defaults"
        );

        // Verify the key is physically present in the merged output.
        let doc: toml_edit::DocumentMut = merged.parse().expect("parse as doc");
        let ws = doc.get("workspace").expect("workspace key");
        assert!(
            ws.as_table().is_some() || ws.as_inline_table().is_some(),
            "workspace should be a table type"
        );
    }

    #[test]
    fn merge_overlay_config_fills_missing_keys_verifiable_in_file() {
        // Minimal config — only [workspace] header.
        let existing = "[workspace]\n";
        let merged = merge_overlay_config(existing).expect("merge should succeed");

        // Check that default keys are physically present (not just deserialization defaults).
        let doc: toml_edit::DocumentMut = merged.parse().expect("parse as doc");
        let ws = doc
            .get("workspace")
            .expect("workspace")
            .as_table()
            .expect("workspace is a table");
        assert!(
            ws.contains_key("version"),
            "version should be physically present in merged TOML"
        );
        assert!(
            ws.contains_key("default_backend"),
            "default_backend should be physically present in merged TOML"
        );

        // backends section should be present too.
        assert!(
            doc.get("backends").is_some(),
            "backends section should be filled from defaults"
        );
    }
}
