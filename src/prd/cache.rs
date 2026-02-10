//! PRD cache management.

use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use fs2::FileExt;

use crate::error::RalphError;
use crate::prd::state::{PipelineContext, PrdMeta, Stage};
use crate::util::hash::sha256_hex;
use crate::Result;

const LOCK_FILENAME: &str = ".lock";
const META_FILENAME: &str = "meta.json";
const MISSING_INFO_REPORT_FILENAME: &str = "missing_info_report.md";

#[derive(Debug, Clone)]
pub struct CacheManager {
    cache_dir: PathBuf,
    idea: String,
    idea_hash: String,
}

impl CacheManager {
    pub fn new(workspace_root: &Path, idea: &str) -> Result<Self> {
        let idea_hash = sha256_hex(idea)[..12].to_owned();
        let ralph_root = resolve_ralph_root(workspace_root);
        let cache_dir = ralph_root.join("prd").join(&idea_hash);
        fs::create_dir_all(&cache_dir)?;

        Ok(Self {
            cache_dir,
            idea: idea.to_owned(),
            idea_hash,
        })
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    pub fn idea_hash(&self) -> &str {
        &self.idea_hash
    }

    pub fn acquire_lock(&self) -> Result<PrdLock> {
        let lock_path = self.cache_dir.join(LOCK_FILENAME);
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)?;

        if file.try_lock_exclusive().is_err() {
            return Err(RalphError::PrdPipelineFailed(format!(
                "PRD cache is locked: {}",
                lock_path.display()
            )));
        }

        Ok(PrdLock {
            file,
            path: lock_path,
        })
    }

    pub fn read_stage_output(&self, stage: Stage) -> Result<Option<String>> {
        read_optional_string(&self.cache_dir.join(stage.artifact_filename()))
    }

    pub fn write_stage_output(&self, stage: Stage, content: &str) -> Result<()> {
        fs::write(self.cache_dir.join(stage.artifact_filename()), content)?;
        Ok(())
    }

    pub fn read_meta(&self) -> Result<Option<PrdMeta>> {
        let meta_path = self.cache_dir.join(META_FILENAME);
        let Some(raw) = read_optional_string(&meta_path)? else {
            return Ok(None);
        };

        let meta = serde_json::from_str::<PrdMeta>(&raw)?;
        Ok(Some(meta))
    }

    pub fn write_meta(&self, meta: &PrdMeta) -> Result<()> {
        let path = self.cache_dir.join(META_FILENAME);
        let content = serde_json::to_string_pretty(meta)?;
        fs::write(path, format!("{content}\n"))?;
        Ok(())
    }

    pub fn write_missing_info_report(&self, report: &str) -> Result<()> {
        fs::write(self.cache_dir.join(MISSING_INFO_REPORT_FILENAME), report)?;
        Ok(())
    }

    pub fn validate_resume_idea(&self) -> Result<()> {
        let Some(meta) = self.read_meta()? else {
            return Ok(());
        };

        if meta.idea != self.idea {
            return Err(RalphError::PrdCacheMismatch(format!(
                "resume idea mismatch for cache {}: expected {:?}, found {:?}",
                self.cache_dir.display(),
                self.idea,
                meta.idea
            )));
        }

        Ok(())
    }

    pub fn should_skip_stage(&self, stage: Stage, context: &PipelineContext) -> bool {
        let current_hash = self.compute_stage_input_hash(stage, context);
        context
            .stage_input_hashes
            .get(&stage)
            .is_some_and(|cached| cached == &current_hash)
    }

    pub fn compute_stage_input_hash(&self, stage: Stage, context: &PipelineContext) -> String {
        let mut input = String::new();
        input.push_str("idea:");
        input.push_str(&context.idea);
        input.push('\n');

        input.push_str("answers:\n");
        for (key, value) in &context.answers {
            input.push_str(key);
            input.push('=');
            input.push_str(value);
            input.push('\n');
        }

        input.push_str("prior_stage_outputs:\n");
        for prior_stage in Stage::all().iter().copied().take(stage.index()) {
            if let Some(output) = context.stage_outputs.get(&prior_stage) {
                input.push_str(prior_stage.artifact_filename());
                input.push('\n');
                input.push_str(output);
                input.push('\n');
            }
        }

        sha256_hex(&input)
    }
}

#[derive(Debug)]
pub struct PrdLock {
    file: File,
    path: PathBuf,
}

impl PrdLock {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for PrdLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

fn read_optional_string(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

fn resolve_ralph_root(workspace_root: &Path) -> PathBuf {
    if workspace_root
        .file_name()
        .is_some_and(|name| name == ".ralph")
    {
        workspace_root.to_path_buf()
    } else {
        workspace_root.join(".ralph")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use tempfile::TempDir;

    use super::*;

    fn make_meta(idea: &str, idea_hash: &str) -> PrdMeta {
        PrdMeta {
            idea: idea.to_owned(),
            idea_hash: idea_hash.to_owned(),
            backend: "codex(gpt-5)".to_owned(),
            started_at: "2026-02-10T20:00:00Z".to_owned(),
            completed_at: None,
            stage_timings: BTreeMap::new(),
            question_rounds: 0,
            rerun_stages: Vec::new(),
        }
    }

    fn make_context(idea: &str) -> PipelineContext {
        PipelineContext {
            idea: idea.to_owned(),
            answers: BTreeMap::new(),
            stage_outputs: BTreeMap::new(),
            stage_input_hashes: BTreeMap::new(),
            answers_hash: sha256_hex(""),
            question_rounds: 0,
        }
    }

    #[test]
    fn creates_cache_dir_with_expected_hash() {
        let temp = TempDir::new().expect("temp dir");
        let manager = CacheManager::new(temp.path(), "smart onboarding").expect("cache manager");

        assert_eq!(manager.idea_hash(), &sha256_hex("smart onboarding")[..12]);
        assert!(manager.cache_dir().is_dir());
        assert!(manager
            .cache_dir()
            .to_string_lossy()
            .contains(".ralph/prd/"));
    }

    #[test]
    fn stage_output_roundtrip() {
        let temp = TempDir::new().expect("temp dir");
        let manager = CacheManager::new(temp.path(), "test idea").expect("cache manager");

        manager
            .write_stage_output(Stage::Ideation, "## Core Concept\nA thing")
            .expect("write stage output");
        let output = manager
            .read_stage_output(Stage::Ideation)
            .expect("read stage output");

        assert_eq!(output, Some("## Core Concept\nA thing".to_owned()));
        assert_eq!(
            manager
                .read_stage_output(Stage::Research)
                .expect("read missing output"),
            None
        );
    }

    #[test]
    fn meta_roundtrip() {
        let temp = TempDir::new().expect("temp dir");
        let manager = CacheManager::new(temp.path(), "test idea").expect("cache manager");
        let meta = make_meta("test idea", manager.idea_hash());

        manager.write_meta(&meta).expect("write meta");
        let loaded = manager.read_meta().expect("read meta");

        assert_eq!(loaded, Some(meta));
    }

    #[test]
    fn writes_missing_info_report() {
        let temp = TempDir::new().expect("temp dir");
        let manager = CacheManager::new(temp.path(), "test idea").expect("cache manager");

        manager
            .write_missing_info_report("# Missing Info\nPlease provide target users.")
            .expect("write report");

        let report_path = manager.cache_dir().join(MISSING_INFO_REPORT_FILENAME);
        let report = fs::read_to_string(report_path).expect("read report");
        assert_eq!(report, "# Missing Info\nPlease provide target users.");
    }

    #[test]
    fn should_skip_stage_when_input_hash_matches() {
        let temp = TempDir::new().expect("temp dir");
        let manager = CacheManager::new(temp.path(), "test idea").expect("cache manager");
        let mut context = make_context("test idea");

        context
            .answers
            .insert("target_user".to_owned(), "engineering managers".to_owned());
        context
            .stage_outputs
            .insert(Stage::Ideation, "ideation output".to_owned());

        let matching = manager.compute_stage_input_hash(Stage::Research, &context);
        context
            .stage_input_hashes
            .insert(Stage::Research, matching.to_owned());

        assert!(manager.should_skip_stage(Stage::Research, &context));

        context.answers.insert(
            "target_user".to_owned(),
            "independent developers".to_owned(),
        );
        assert!(!manager.should_skip_stage(Stage::Research, &context));
    }

    #[test]
    fn resume_validation_detects_idea_mismatch() {
        let temp = TempDir::new().expect("temp dir");
        let manager = CacheManager::new(temp.path(), "expected idea").expect("cache manager");

        // Simulate stale or manually edited metadata in the same cache directory.
        let mismatched = make_meta("different idea", manager.idea_hash());
        manager
            .write_meta(&mismatched)
            .expect("write mismatched meta");

        let err = manager
            .validate_resume_idea()
            .expect_err("expected mismatch");
        assert!(matches!(err, RalphError::PrdCacheMismatch(_)));
    }

    #[test]
    fn prd_lock_acquisition_succeeds() {
        let temp = TempDir::new().expect("temp dir");
        let manager = CacheManager::new(temp.path(), "lock idea").expect("cache manager");

        let lock = manager.acquire_lock().expect("acquire lock");
        assert_eq!(
            lock.path().file_name().and_then(|n| n.to_str()),
            Some(".lock")
        );
    }

    #[test]
    fn prd_lock_releases_on_drop() {
        let temp = TempDir::new().expect("temp dir");
        let manager = CacheManager::new(temp.path(), "lock idea").expect("cache manager");

        let first_lock = manager.acquire_lock().expect("acquire first lock");
        let second_attempt = manager.acquire_lock();
        assert!(second_attempt.is_err());

        drop(first_lock);

        let third_attempt = manager.acquire_lock();
        assert!(third_attempt.is_ok());
    }
}
