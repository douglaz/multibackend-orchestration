//! PRD answer persistence support.

use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::util::hash::sha256_hex;
use crate::Result;

#[derive(Debug, Clone)]
pub struct AnswerStore {
    path: PathBuf,
    answers: BTreeMap<String, String>,
}

impl AnswerStore {
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            answers: BTreeMap::new(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn answers(&self) -> &BTreeMap<String, String> {
        &self.answers
    }

    pub fn load(&mut self) -> Result<BTreeMap<String, String>> {
        let raw = match fs::read_to_string(&self.path) {
            Ok(content) => content,
            Err(err) if err.kind() == ErrorKind::NotFound => {
                self.answers.clear();
                return Ok(self.answers.clone());
            }
            Err(err) => return Err(err.into()),
        };

        if raw.trim().is_empty() {
            self.answers.clear();
            return Ok(self.answers.clone());
        }

        let parsed = serde_yaml::from_str::<BTreeMap<String, String>>(&raw)?;
        self.answers = parsed;
        Ok(self.answers.clone())
    }

    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.path, self.serialize_answers()?)?;
        Ok(())
    }

    pub fn merge(&mut self, new_answers: BTreeMap<String, String>) {
        for (key, value) in new_answers {
            self.answers.insert(key, value);
        }
    }

    pub fn hash(&self) -> Result<String> {
        Ok(sha256_hex(&self.serialize_answers()?))
    }

    fn serialize_answers(&self) -> Result<String> {
        Ok(serde_yaml::to_string(&self.answers)?)
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn load_returns_empty_map_when_file_missing() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("answers.yaml");
        let mut store = AnswerStore::new(&path);

        let answers = store.load().expect("load answers");
        assert!(answers.is_empty());
        assert!(store.answers().is_empty());
    }

    #[test]
    fn yaml_roundtrip() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("answers.yaml");

        let mut store = AnswerStore::new(&path);
        let mut initial = BTreeMap::new();
        initial.insert("target_user".to_owned(), "engineering managers".to_owned());
        initial.insert("deployment".to_owned(), "cloud".to_owned());

        store.merge(initial.clone());
        store.save().expect("save answers");

        let mut loaded_store = AnswerStore::new(&path);
        let loaded = loaded_store.load().expect("load answers");
        assert_eq!(loaded, initial);
    }

    #[test]
    fn merge_overwrites_existing_and_adds_new_keys() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("answers.yaml");
        let mut store = AnswerStore::new(&path);

        let mut first = BTreeMap::new();
        first.insert("platform".to_owned(), "web".to_owned());
        first.insert("users".to_owned(), "developers".to_owned());
        store.merge(first);

        let mut second = BTreeMap::new();
        second.insert("platform".to_owned(), "mobile".to_owned());
        second.insert("region".to_owned(), "us".to_owned());
        store.merge(second);

        assert_eq!(store.answers().len(), 3);
        assert_eq!(store.answers().get("platform"), Some(&"mobile".to_owned()));
        assert_eq!(store.answers().get("users"), Some(&"developers".to_owned()));
        assert_eq!(store.answers().get("region"), Some(&"us".to_owned()));
    }

    #[test]
    fn hash_is_stable_for_same_content() {
        let temp = TempDir::new().expect("temp dir");
        let path1 = temp.path().join("answers-a.yaml");
        let path2 = temp.path().join("answers-b.yaml");

        let mut store1 = AnswerStore::new(&path1);
        let mut map1 = BTreeMap::new();
        map1.insert("a".to_owned(), "1".to_owned());
        map1.insert("b".to_owned(), "2".to_owned());
        store1.merge(map1);

        let mut store2 = AnswerStore::new(&path2);
        let mut map2 = BTreeMap::new();
        map2.insert("b".to_owned(), "2".to_owned());
        map2.insert("a".to_owned(), "1".to_owned());
        store2.merge(map2);

        let hash1 = store1.hash().expect("hash1");
        let hash2 = store2.hash().expect("hash2");
        assert_eq!(hash1, hash2);

        let mut changed = BTreeMap::new();
        changed.insert("a".to_owned(), "updated".to_owned());
        store2.merge(changed);
        let hash3 = store2.hash().expect("hash3");
        assert_ne!(hash1, hash3);
    }
}
