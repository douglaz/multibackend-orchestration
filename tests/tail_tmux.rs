//! Tests for `ralph tail --tmux` command behavior without requiring real tmux.
#![allow(clippy::await_holding_lock)]

use std::fs;
use std::sync::{Mutex, MutexGuard};

use clap::Parser;

use ralph::cli::tail::tmux_attach;
use ralph::cli::{Cli, Commands};
use ralph::error::RalphError;

// --- CLI parsing tests ---

#[test]
fn parses_tail_with_tmux_flag() {
    let cli = Cli::parse_from(["ralph", "tail", "--tmux"]);
    let Commands::Tail(args) = cli.command else {
        panic!("expected tail command");
    };
    assert!(args.tmux);
    assert!(!args.follow);
    assert!(!args.json);
}

#[test]
fn parses_tail_without_tmux_flag() {
    let cli = Cli::parse_from(["ralph", "tail"]);
    let Commands::Tail(args) = cli.command else {
        panic!("expected tail command");
    };
    assert!(!args.tmux);
}

#[test]
fn parses_tail_with_tmux_and_other_flags() {
    // --tmux should work alongside other tail flags (they are just ignored
    // when --tmux is used, but parsing should succeed)
    let cli = Cli::parse_from(["ralph", "tail", "--tmux", "--json"]);
    let Commands::Tail(args) = cli.command else {
        panic!("expected tail command");
    };
    assert!(args.tmux);
    assert!(args.json);
}

#[test]
fn parses_tail_follow_without_tmux() {
    let cli = Cli::parse_from(["ralph", "tail", "-F"]);
    let Commands::Tail(args) = cli.command else {
        panic!("expected tail command");
    };
    assert!(!args.tmux);
    assert!(args.follow);
}

// --- tmux_attach behavior tests using explicit tmux binary override ---
//
// These tests use fake tmux scripts to verify the behavior of `tmux_attach`
// without requiring real tmux. We serialize environment override writes to
// avoid races between parallel tests.

static TMUX_BIN_LOCK: Mutex<()> = Mutex::new(());

struct TmuxBinGuard {
    original: Option<String>,
}

impl TmuxBinGuard {
    fn set(path: &str) -> Self {
        let original = std::env::var("RALPH_TMUX_BIN").ok();
        unsafe { std::env::set_var("RALPH_TMUX_BIN", path) };
        Self { original }
    }
}

impl Drop for TmuxBinGuard {
    fn drop(&mut self) {
        if let Some(value) = self.original.as_ref() {
            unsafe { std::env::set_var("RALPH_TMUX_BIN", value) };
        } else {
            unsafe { std::env::remove_var("RALPH_TMUX_BIN") };
        }
    }
}

fn lock_tmux_bin() -> MutexGuard<'static, ()> {
    TMUX_BIN_LOCK.lock().expect("tmux-bin lock poisoned")
}

fn write_executable(path: &std::path::Path, body: &str) {
    fs::write(path, body).expect("write script");
    let mut perms = fs::metadata(path).expect("stat script").permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o755);
    }
    fs::set_permissions(path, perms).expect("chmod script");
}

#[tokio::test]
async fn tmux_attach_fails_when_tmux_unavailable() {
    let _lock = lock_tmux_bin();
    let temp = tempfile::tempdir().expect("temp dir");
    let missing_tmux = temp.path().join("tmux");
    let _guard = TmuxBinGuard::set(&missing_tmux.to_string_lossy());

    let result = tmux_attach("ralph").await;

    match result {
        Err(RalphError::Validation(msg)) => {
            assert!(
                msg.contains("not installed") || msg.contains("not on PATH"),
                "expected tmux unavailable message: {msg}"
            );
        }
        other => panic!("expected Validation error for missing tmux, got: {other:?}"),
    }
}

#[tokio::test]
async fn tmux_attach_fails_when_session_does_not_exist() {
    let _lock = lock_tmux_bin();
    let temp = tempfile::tempdir().expect("temp dir");
    let tmux_script = temp.path().join("tmux");

    // has-session returns non-zero (session doesn't exist)
    write_executable(
        &tmux_script,
        r#"#!/usr/bin/env bash
if [[ "$1" == "has-session" ]]; then
    exit 1
fi
exit 1
"#,
    );

    let _guard = TmuxBinGuard::set(&tmux_script.to_string_lossy());

    let result = tmux_attach("nonexistent-session").await;

    match result {
        Err(RalphError::Validation(msg)) => {
            assert!(
                msg.contains("does not exist"),
                "expected session-not-found message: {msg}"
            );
            assert!(
                msg.contains("nonexistent-session"),
                "should mention session name: {msg}"
            );
        }
        other => panic!("expected Validation error for missing session, got: {other:?}"),
    }
}

#[tokio::test]
async fn tmux_attach_fails_when_attach_returns_nonzero() {
    let _lock = lock_tmux_bin();
    let temp = tempfile::tempdir().expect("temp dir");
    let tmux_script = temp.path().join("tmux");

    // has-session succeeds, but attach fails
    write_executable(
        &tmux_script,
        r#"#!/usr/bin/env bash
if [[ "$1" == "has-session" ]]; then
    exit 0
fi
if [[ "$1" == "attach" ]]; then
    exit 1
fi
exit 1
"#,
    );

    let _guard = TmuxBinGuard::set(&tmux_script.to_string_lossy());

    let result = tmux_attach("ralph").await;

    match result {
        Err(RalphError::Validation(msg)) => {
            assert!(
                msg.contains("non-zero status"),
                "expected attach failure message: {msg}"
            );
        }
        other => panic!("expected Validation error for attach failure, got: {other:?}"),
    }
}

#[tokio::test]
async fn tmux_attach_succeeds_when_session_exists() {
    let _lock = lock_tmux_bin();
    let temp = tempfile::tempdir().expect("temp dir");
    let tmux_script = temp.path().join("tmux");

    // has-session succeeds, attach succeeds
    write_executable(
        &tmux_script,
        r#"#!/usr/bin/env bash
if [[ "$1" == "has-session" ]]; then
    exit 0
fi
if [[ "$1" == "attach" ]]; then
    exit 0
fi
exit 1
"#,
    );

    let _guard = TmuxBinGuard::set(&tmux_script.to_string_lossy());

    let result = tmux_attach("ralph").await;
    assert!(result.is_ok(), "attach should succeed: {result:?}");
}
