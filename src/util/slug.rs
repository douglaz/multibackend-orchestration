pub fn slugify_feature_name(input: &str) -> String {
    let mut raw = String::with_capacity(input.len());

    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            raw.push(ch.to_ascii_lowercase());
        } else if ch.is_ascii_whitespace() || ch == '_' || ch == '-' {
            raw.push('-');
        }
    }

    let mut collapsed = String::with_capacity(raw.len());
    let mut last_dash = false;
    for ch in raw.chars() {
        if ch == '-' {
            if !last_dash {
                collapsed.push('-');
                last_dash = true;
            }
        } else {
            collapsed.push(ch);
            last_dash = false;
        }
    }

    let mut slug = collapsed.trim_matches('-').to_owned();
    if slug.is_empty() {
        slug = "feature".to_owned();
    }

    if slug.len() > 50 {
        let mut cut = 50;
        if let Some(idx) = slug[..50].rfind('-') {
            if idx >= 20 {
                cut = idx;
            }
        }
        slug.truncate(cut);
        slug = slug.trim_matches('-').to_owned();
        if slug.is_empty() {
            slug = "feature".to_owned();
        }
    }

    slug
}

#[cfg(test)]
mod tests {
    use super::slugify_feature_name;

    #[test]
    fn slugifies_per_convention() {
        assert_eq!(
            slugify_feature_name("User Authentication"),
            "user-authentication"
        );
        assert_eq!(
            slugify_feature_name("REST API Endpoints (v2)"),
            "rest-api-endpoints-v2"
        );
        assert_eq!(slugify_feature_name("___"), "feature");
    }

    #[test]
    fn truncates_slug() {
        let slug = slugify_feature_name(
            "this is a very long feature name that should be trimmed to fifty chars",
        );
        assert!(slug.len() <= 50);
    }
}
