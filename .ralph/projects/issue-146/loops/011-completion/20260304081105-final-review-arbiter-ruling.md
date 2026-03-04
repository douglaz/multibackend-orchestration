---
artifact: final-review-arbiter-ruling
loop: 11
project: issue-146
backend: codex
role: arbiter
created_at: 2026-03-04T08:11:05Z
---

I’m verifying the backend-spec parsing and comparison paths in source before issuing the ruling, so the decision is based on actual behavior rather than assumptions.
I confirmed the exact comparison and parser behavior; next I’m checking how the backend specs are sourced and whether any normalization already happens before `validate_distinct_backends`.
I found additional call sites and now I’m checking config/CLI normalization functions to determine whether semantically equivalent specs can still arrive as different strings.
I confirmed there’s no trimming in `resolve_optional_backend_override`; I’m now checking config setters/parsers to see whether whitespace can be preserved into runtime values.
# Arbiter Ruling

## Amendment: QD-BACKEND-EQUALITY-002

### Ruling
ACCEPT

### Rationale
The current check is raw string equality, but backend resolution semantics are parser-based. `parse_backend_spec` trims input, while sources feeding `validate_distinct_backends` are not canonicalized first (`resolve_optional_backend_override` just clones strings, and CLI/config values can preserve surrounding whitespace). That makes cases like `"claude"` vs `" claude "` able to pass the distinctness gate but still resolve to the same backend. Since the rule is semantic distinctness, canonical comparison (`name` + `model`) is the correct behavior, with low implementation risk and clear conformance tests for normalization edge cases.
