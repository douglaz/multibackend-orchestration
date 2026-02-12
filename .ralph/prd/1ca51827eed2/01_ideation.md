## Core Concept

csv2json is a fast, correct, and zero-configuration CLI tool written in Rust that converts CSV files to JSON. It prioritizes deterministic behavior, strict type inference, and predictable error handling. The tool reads CSV input (from files or stdin), infers column types with well-defined rules, and emits JSON output (to files or stdout). It ships as a single static binary with no runtime dependencies, targeting Linux, macOS, and Windows from day one. There is no telemetry, no network access, and no hidden behavior — what you see is what you get.

## Target Users

- **Data engineers** who need a reliable, scriptable CSV-to-JSON conversion step in ETL pipelines and shell scripts.
- **Backend developers** ingesting CSV exports from databases, spreadsheets, or third-party APIs into JSON-consuming services.
- **DevOps/SRE teams** converting configuration or inventory CSVs into JSON for consumption by infrastructure-as-code tools (Terraform, Ansible).
- **Data analysts and scientists** who work in polyglot environments and need quick, trustworthy format conversion without spinning up Python or Node.
- **CLI power users** who chain tools together with pipes (`cat data.csv | csv2json | jq '.[] | select(.active)'`) and expect Unix-philosophy composability.

## Key Problems Solved

1. **Type ambiguity.** Most naive converters emit everything as strings. csv2json applies strict, documented inference rules: `true`/`false` become JSON booleans, empty strings and literal `null`/`NULL` become JSON `null`, numeric values become numbers — but values with leading zeros (like zip codes `07042` or part numbers `001234`) are preserved as strings to prevent silent data corruption.

2. **Duplicate key handling.** Real-world CSVs frequently contain duplicate column headers (e.g., after a bad merge). csv2json warns to stderr and uses last-value-wins semantics, so the user is informed but the pipeline doesn't break unexpectedly.

3. **Error discipline.** The tool fails fast on the first error by default (malformed rows, encoding issues, I/O failures) with a clear error message and nonzero exit code. No silent data loss, no partial output mistaken for success.

4. **Schema discovery.** In standalone schema mode, csv2json outputs a JSON Schema describing the inferred structure of the CSV rather than the data itself — useful for documentation, validation pipeline setup, or understanding an unfamiliar dataset before processing it.

5. **Cross-platform inconsistency.** Many CSV tools behave differently on Windows vs. Unix (line endings, encoding, path handling). csv2json normalizes behavior across all three platforms from the start.

## Proposed Features

### MVP (v0.1)

- **File and stdin input.** `csv2json data.csv` or `cat data.csv | csv2json`.
- **File and stdout output.** `csv2json data.csv -o data.json` or pipe to next tool.
- **Array-of-objects output.** Default output is a JSON array where each row becomes an object keyed by header names.
- **Strict type inference.**
  - Booleans: only literal `true` / `false` (case-sensitive).
  - Nulls: empty string `""` and literal `null` / `NULL` map to JSON `null`.
  - Numbers: standard numeric strings become JSON numbers; values with leading zeros (e.g., `007`) are preserved as strings.
- **Duplicate column headers.** Warn to stderr, last value wins.
- **Fail-fast error handling.** Exit on first error with nonzero status and a human-readable message to stderr.
- **Delimiter flag.** `--delimiter ','` (default comma), supporting tab (`--delimiter '\t'`), pipe, semicolon, etc.
- **Pretty-print flag.** `--pretty` for indented output; compact by default.
- **Schema output mode.** `csv2json --schema data.csv` emits a JSON Schema document describing inferred column types instead of data.

### v0.2

- **NDJSON / JSON Lines output.** `--format ndjson` emits one JSON object per line for streaming consumers.
- **Column selection.** `--columns name,email,age` or `--exclude id,internal_notes`.
- **Row filtering.** `--where 'age > 30'` with a minimal expression language.
- **Custom null values.** `--null-values 'NA,N/A,-'` to extend null inference.
- **Encoding detection.** Auto-detect UTF-8/UTF-16/Latin-1 with `--encoding` override.
- **Header override.** `--no-header --columns a,b,c` for headerless CSVs.
- **Streaming mode.** Process arbitrarily large files without loading them entirely into memory.

### v0.3+

- **Batch conversion.** `csv2json *.csv --outdir ./json/` converting multiple files in parallel.
- **Type override map.** `--types 'zip:string,count:int'` to force specific column types.
- **Configurable error handling.** `--on-error skip` to skip malformed rows (logging to stderr) instead of failing.
- **Shell completions.** Auto-generated completions for bash, zsh, fish, and PowerShell.
- **`--explain` mode.** Dry-run that shows what types would be inferred for each column without producing output.

## Success Metrics

- **Correctness.** 100% of test cases pass for type inference edge cases (leading zeros, mixed-type columns, Unicode, null variants, boolean casing). A comprehensive test suite with property-based tests covers these.
- **Performance.** Converts a 1 GB CSV file in under 10 seconds on commodity hardware (competitive with `xsv` and `mlr`). Benchmarked in CI on every release.
- **Binary size.** Statically linked release binary under 5 MB on all platforms.
- **Adoption signals.** 500+ GitHub stars and 50+ crates.io downloads/week within 6 months of launch. Positive mentions in data engineering and Rust communities.
- **Cross-platform CI.** All tests pass on Linux (x86_64, aarch64), macOS (Apple Silicon + Intel), and Windows (x86_64) in CI before every release.
- **Zero open correctness bugs.** Any reported data corruption or incorrect inference is treated as a P0 and patched within 48 hours.

## Constraints & Assumptions

- **Language:** Rust, using the `csv` and `serde_json` crates as foundational dependencies.
- **License:** Dual-licensed under MIT OR Apache-2.0, following Rust ecosystem convention.
- **No telemetry.** No analytics, crash reporting, update checks, or network access of any kind. The binary is fully offline.
- **No configuration files.** All behavior is controlled via CLI flags. No dotfiles, no environment variable magic, no hidden defaults.
- **CSV dialect:** RFC 4180 as the baseline, with pragmatic extensions (configurable delimiter, flexible quoting). Non-UTF-8 input is an error by default (explicit `--encoding` opt-in later).
- **Boolean inference is strict.** Only literal `true` and `false` (case-sensitive) become JSON booleans. `True`, `TRUE`, `yes`, `1` remain strings. This is intentional — users can override with `--types` in a future version.
- **Leading zeros are always strings.** `007`, `00123`, `0` with leading zeros → string. This prevents zip code and identifier corruption. Plain `0` is numeric.
- **No embedded scripting or plugin system.** The tool does one thing well. Complex transformations belong in `jq`, `mlr`, or application code downstream.
- **Minimum supported Rust version (MSRV):** Stable Rust, latest minus two releases. No nightly-only features.
- **Assumption:** Input files fit in available disk space for output. Streaming mode (v0.2) will relax the memory constraint but disk space is the user's responsibility.