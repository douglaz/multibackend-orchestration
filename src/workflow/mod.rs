pub mod orchestrator;
pub mod parser;
pub mod pre_commit_checks;
pub mod quick_dev_orchestrator;

/// Resolve the effective max-backend-retries from an optional configured value.
///
/// - `None` or `Some(0)` → default of 3
/// - `Some(v)` → clamped to at most 10
pub fn max_backend_retries(configured: Option<u8>) -> u8 {
    const DEFAULT_RETRIES: u8 = 3;
    const MAX_RETRIES: u8 = 10;

    match configured {
        Some(0) | None => DEFAULT_RETRIES,
        Some(v) => v.min(MAX_RETRIES),
    }
}
