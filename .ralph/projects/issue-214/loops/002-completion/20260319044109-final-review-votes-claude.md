---
artifact: final-review-votes
loop: 2
project: issue-214
backend: claude
role: final_reviewer
created_at: 2026-03-19T04:41:09Z
---

# Vote Results

## Amendment: ORACLE-REV-001

### Vote
ACCEPT

### Rationale
The code confirms two distinct brittleness issues. First, `ORACLE_SYSTEM_WRAPPER` (line 17) monkey-patches commander's `Command.prototype.option` to silently inject a `--system` option — this depends on commander's internal prototype shape, which upstream is free to change at any time. Second, `resolve_oracle_cli_entrypoint()` (line 341) gates the wrapper path on the resolved binary canonicalizing to exactly `.../dist/bin/oracle-cli.js`, coupling the daemon to oracle's internal package layout. Both paths then unconditionally pass `--system` (line 408), which is not in oracle's documented CLI surface.

The validate suite never exercises the wrapper branch: the mock harness writes a plain `oracle` script, so `resolve_oracle_cli_entrypoint()` always returns `None`. This means the production-only launcher path has zero test coverage. A normal oracle package update — renaming the entrypoint, upgrading commander, or removing the undocumented `--system` passthrough — would silently break production without any test catching it. The amendment's proposed fix (use only documented flags, fold system instructions into the prompt text) is the correct approach and would eliminate an entire class of upstream coupling.

## Amendment: ORACLE-REV-002

### Vote
ACCEPT

### Rationale
Line 105 shows the temp path is derived deterministically as `path.with_extension("json.tmp")`, producing a single fixed file like `state.json.tmp`. Two concurrent daemon instances targeting the same workspace would race: one writer's `fs::write` (line 106) can overwrite the other's temp contents between write and rename, or the first `fs::rename` (line 112) can remove the temp file before the second rename runs, causing a spurious `ENOENT`. The `save()` method is called at two points during normal operation (lines 200 and 271), making this a live race window, not a theoretical one.

The fix is minimal and well-understood: use `tempfile::NamedTempFile::new_in(parent)` or append a PID/random suffix to the temp name. This is a standard atomic-write pattern and the cost of the change is trivially low relative to the robustness it provides.

## Amendment: ORACLE-REV-003

### Vote
ACCEPT

### Rationale
The mock's `spawn` branch (line 1287-1289) is a process that successfully starts, prints `"oracle spawn: mock spawn failure"` to stderr, and exits 7. This is fundamentally different from an actual spawn failure, where `Command::spawn()` returns `Err` at `process.rs:24` and produces the message `"failed to spawn command: ..."` without ever creating a child process.

The test passes only because `classify_oracle_error` (line 454) treats both `"failed to spawn command"` and `"oracle spawn:"` as the same error class. So the test proves that the classifier handles the mock's synthetic stderr text, not that the daemon correctly handles a real `ENOENT`/`EACCES` spawn failure. The test name `oracle_spawn_failure_isolated` overstates what it covers. Either the test should force a real spawn failure (e.g., by pointing at a nonexistent binary for the first invocation) or it should be renamed to reflect that it tests exit-code-based error classification and isolation, not spawn failures.
