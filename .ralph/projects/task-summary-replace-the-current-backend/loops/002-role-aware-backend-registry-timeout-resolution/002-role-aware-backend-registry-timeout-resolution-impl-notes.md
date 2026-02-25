# Implementation Notes

## Decisions Made
- **Cache key uses timeout value, not role name**: `role_backend_cache_key` produces keys like `claude@timeout=300` rather than `claude@role=planner`. This means two different roles that happen to resolve to the same timeout will share a cached backend instance, which is correct since they would be functionally identical.
- **`backend_from_config_with_timeout` wraps existing constructor**: Both `claude.rs` and `codex.rs` retain their original `backend_from_config` function unchanged (delegates to the new `_with_timeout` variant with `None`), ensuring all existing callsites are unaffected.
- **Default timeout is 7200 in all fallback paths**: When `explicit_timeout_secs` is `None` (non-role path), the constructors default to 7200. When a role is unknown, `resolve_role_timeout` falls back to 7200 via `RoleTimeouts::for_role` returning `None`.
- **Deprecated `timeout_seconds` is never read**: Neither `backend_from_config_with_timeout` in `claude.rs` nor `codex.rs` references `backend.timeout_seconds`. The non-role path defaults to 7200, and the role path reads from `timeouts.<role>`.

## Spec Deviations
- None. All acceptance criteria are implemented as specified.

## Testing
- **8 new tests** added to `tests/backend.rs`:
  - `get_or_create_for_role_uses_role_timeout_from_config` — verifies role-aware backend creation succeeds with configured timeouts
  - `get_or_create_for_role_caches_by_spec_and_timeout` — same (spec, role) returns cached instance; different roles with different timeouts don't collide
  - `get_or_create_for_role_same_timeout_shares_cache` — different roles with identical timeouts share the cached instance
  - `get_or_create_for_role_unknown_role_falls_back_to_7200` — unknown roles default to 7200 and share that cache entry
  - `get_or_create_for_role_does_not_collide_with_get_or_create_for_spec` — role cache keys don't collide with non-role spec keys
  - `get_or_create_for_role_with_model_spec` — model specs like `claude(opus)` work with role acquisition
  - `get_or_create_for_role_deprecated_timeout_seconds_not_used` — deprecated `timeout_seconds` is not consulted in role path
  - `get_or_create_for_spec_unchanged_behavior` — non-role `get_or_create_for_spec` caching continues to work
- All 620 tests pass (43 in `tests/backend.rs`, 410 in unit tests, plus integration tests)
- Verification: `nix develop -c cargo check` and `nix develop -c cargo test` both succeed with zero failures
