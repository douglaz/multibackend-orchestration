---
artifact: final-review-proposals
loop: 4
project: summary-enable-the-daemon-to-process-iss
backend: gemini
role: final_reviewer
created_at: 2026-02-26T00:06:28Z
---

# Final Review: AMENDMENTS

## Amendment: PRD-FALLBACK-1

### Problem
In `src/daemon/runtime.rs`, when a `ralph:prd-done` issue fails to have its approved spec extracted, the fallback logic incorrectly sets the `raw_idea` to an empty string. The expected behavior is to fall back to using the issue's title and body, similar to how non-PRD issues are handled. This bug prevents the daemon from processing `prd-done` issues when spec extraction fails.

The incorrect code is in `src/daemon/runtime.rs`:
```rust
let raw_idea = if issue.has_label("ralph:prd-done") {
    eprintln!("info: {}: issue has prd-done label, attempting to extract approved spec", issue.key());
    // ... (extraction logic) ...
    match approved_spec {
        Some(spec) => {
            // ...
            spec
        }
        None => {
            eprintln!(
                "warn: {}: approved spec not found, falling back",
                issue.key()
            );
            // THIS IS THE BUG: should be compose_raw_idea(...)
            String::new() 
        }
    }
} else {
    compose_raw_idea(&issue.title, issue.body.as_deref())
};
```

### Proposed Change
Modify the fallback path inside `poll_and_claim` to call `compose_raw_idea` instead of returning an empty string. This will ensure that if spec extraction from comments fails for a `prd-done` issue, the system correctly falls back to using the issue's title and body as the task input.

### Affected Files
- `src/daemon/runtime.rs`
