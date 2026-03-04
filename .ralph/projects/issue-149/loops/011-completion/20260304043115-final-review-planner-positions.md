---
artifact: final-review-planner-positions
loop: 11
project: issue-149
backend: claude(opus)
role: planner
created_at: 2026-03-04T04:31:15Z
---

# Planner Positions

## Amendment: DAEMON-PUSH-RETRY-CLASSIFIER-001

### Position
ACCEPT

### Rationale

I verified the claims by reading the actual implementation. The function `is_retryable_push_stderr` at `src/daemon/github.rs:913-960` does indeed use raw `.contains()` substring matching against the lowercased stderr for bare numeric strings `"403"`, `"500"`, `"502"`, `"503"`, `"504"`.

**Branch-name collision is already mitigated** — `is_retryable_push_error` (line 968) strips the branch name before delegating to `is_retryable_push_stderr`, and this is the only production caller. However, the amendment's core concern is still valid because the *raw git stderr* passed to `is_retryable_push_stderr` can contain remote URLs with these numeric substrings.

**Confirmed real bugs:**

1. **URL containing `403` causes DNS errors to be non-retryable.** Git stderr for a network failure can look like: `fatal: unable to access 'https://github.com/org/error-403-handler.git/': Could not resolve host: github.com`. The non-retryable check fires on `"403"` in the URL (line 923) and returns `false` before the retryable `"could not resolve"` pattern (line 947) is ever reached. This is a misclassification — DNS failures are transient and should be retried.

2. **URL containing `500` causes auth failures to be retryable.** For stderr like: `fatal: unable to access 'https://github.com/org/error-500.git/': The requested URL returned error: 401`, no non-retryable pattern matches (`"401"` is absent from the non-retryable list), and then `"500"` in the URL hits the retryable list (line 936), returning `true`. This is a misclassification — HTTP 401 is permanent.

3. **HTTP 401 via `unable to access` is retryable.** Even without a numeric collision, stderr like `fatal: unable to access '...': The requested URL returned error: 401` has no matching non-retryable pattern (no `authentication`, `denied`, `forbidden`, or `403`), but the retryable pattern `"unable to access"` (line 948) fires, incorrectly classifying an authentication failure as retryable.

The existing tests (`is_retryable_push_stderr_ignores_branch_name_collisions` at line 2256) only test with clean strings like `"permission denied"` and `"HTTP 503 Service Unavailable"` — they never test with realistic git stderr containing URLs, so these bugs are not caught.

The proposed fix — matching HTTP codes only in context (e.g., `returned error: 503`, `http 503`) rather than as bare substrings, and explicitly classifying `401` as non-retryable — directly addresses all three confirmed issues. The suggested regression tests cover exactly the right scenarios.
