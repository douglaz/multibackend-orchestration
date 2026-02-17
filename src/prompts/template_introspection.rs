use std::fs;
use std::path::Path;

pub fn template_uses_var(template_source: &str, var_name: &str) -> bool {
    let needle = format!("{{{{{var_name}}}}}");
    template_source.contains(&needle)
}

pub fn load_template_source(path: &Path, fallback: &str) -> String {
    if path.exists() {
        fs::read_to_string(path).unwrap_or_else(|_| fallback.to_owned())
    } else {
        fallback.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{load_template_source, template_uses_var};

    #[test]
    fn template_uses_var_detects_present_placeholder() {
        let source = "before {{master_prompt}} after";
        assert!(template_uses_var(source, "master_prompt"));
    }

    #[test]
    fn template_uses_var_detects_absent_placeholder() {
        let source = "no placeholders here";
        assert!(!template_uses_var(source, "master_prompt"));
    }

    #[test]
    fn template_uses_var_requires_exact_match() {
        let source = "{{variable}}";
        assert!(!template_uses_var(source, "var"));
    }

    #[test]
    fn template_uses_var_returns_true_for_repeated_placeholders() {
        let source = "{{master_prompt}}\n{{master_prompt}}";
        assert!(template_uses_var(source, "master_prompt"));
    }

    #[test]
    fn load_template_source_reads_file_when_present() {
        let temp = tempdir().expect("temp dir");
        let template_path = temp.path().join("planner.md");
        fs::write(&template_path, "from file").expect("write template");

        let loaded = load_template_source(&template_path, "fallback");
        assert_eq!(loaded, "from file");
    }

    #[test]
    fn load_template_source_uses_fallback_when_missing() {
        let temp = tempdir().expect("temp dir");
        let template_path = temp.path().join("missing.md");

        let loaded = load_template_source(&template_path, "fallback");
        assert_eq!(loaded, "fallback");
    }
}
