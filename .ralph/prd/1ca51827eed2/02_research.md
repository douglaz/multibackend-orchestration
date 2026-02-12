The research is complete. Let me compile the final report.

## Market Context

The data engineering market exceeds **$120 billion in 2026** (14-18% CAGR), with the ETL segment alone valued at **$8.85 billion** and projected to reach $18.60 billion by 2030. Over 60% of companies now implement real-time data pipelines, and 90% of AI/ML projects depend directly on data engineering pipelines. The addressable user base is substantial:

| Segment | Estimated Global Size | Primary Use Case |
|---|---|---|
| Data Engineers | ~500K (87% of companies report skills gaps) | ETL pipelines, data lake ingestion, format normalization |
| Backend Developers | ~15M+ | API data import, config transformation, migration scripts |
| DevOps/SRE | ~2M+ | Log processing, infrastructure-as-data, CI/CD data transforms |
| Data Analysts | ~3M+ | Ad-hoc exploration, spreadsheet-to-API workflows |

The "Rewrite It in Rust" movement has produced a generation of CLI tools that achieved mainstream adoption: ripgrep (~59K stars), bat (~57K stars), fd (~41K stars), eza (~20K stars). These tools share traits that csv2json would replicate: single static binary, sensible defaults, cross-platform support, and dramatic performance improvements. Cargo (Rust's toolchain) is rated the **most admired infrastructure tool** at 71% in the 2025 Stack Overflow survey.

A critical market signal: **xsv**, the most-starred Rust CSV tool (10.8K stars), was **archived in April 2025**. Its README now directs users to qsv or xan, neither of which focuses on CSV-to-JSON conversion. This creates a visible gap in the ecosystem — there is no "ripgrep of CSV-to-JSON" that has achieved breakout success in that specific niche. CSV remains the lingua franca of tabular data exchange (databases, spreadsheets, APIs, government data portals), while JSON dominates as the interchange format for APIs, configuration, and web services. A fast, correct bridge between these two formats is a permanent need.

## Technical Landscape

**Rust ecosystem maturity is high.** The foundational crates are battle-tested:

- **`csv` crate** (v1.4.0, 129M downloads, 17M recent): Table-based DFA parser achieving ~241 MB/s raw parsing, ~122 MB/s for string records, ~83 MB/s with Serde deserialization. RFC 4180 compliant with configurable delimiters, quote characters, flexible record terminators (CRLF/LF/CR), and ragged CSV support. The `csv-core` sub-crate is no-std compatible. In benchmarks with a 3.6GB CSV file, Rust csv completed in 23 seconds vs. Go at 187 seconds (8x slower) and Python significantly behind.

- **`serde_json`** (400M+ downloads): Mature JSON serialization with pretty-printing, streaming serialization, and dynamic `Value` manipulation — all needed for this tool.

- **`clap`** (v4.5.58): Feature-rich CLI parser with derive API, auto-generated help/completions, argument validation, and environment variable integration. Dual-licensed Apache 2.0 / MIT.

- **`schemars`** (v1.1.0): Generates JSON Schema with **Draft 2020-12 as the default** output, matching the project requirement exactly. While primarily derive-based, it provides patterns for programmatic schema construction.

**Cross-platform compilation is well-established:**

| Platform | Target | Approach |
|---|---|---|
| Linux x86_64 | `x86_64-unknown-linux-musl` | Fully static binary, zero runtime deps |
| Linux aarch64 | `aarch64-unknown-linux-musl` | Via `cross-rs` or `cargo-zigbuild` |
| macOS Universal | `x86_64-apple-darwin` + `aarch64-apple-darwin` | Combined via `lipo` |
| Windows | `x86_64-pc-windows-msvc` | Static CRT linking via `target-feature=+crt-static` |

GitHub Actions with `cross-rs/cross` or `cargo-zigbuild` can build all targets in a single CI workflow — this is a well-trodden path used by ripgrep, bat, fd, and dozens of other Rust CLI tools.

**BOM handling** requires manual implementation — the `csv` crate does not strip UTF-8 BOM (documented in issue #81). The `unicode-bom` crate detects and classifies BOMs. Implementation is straightforward: read the first 3 bytes, check for the BOM prefix `0xEF 0xBB 0xBF`, and create the CSV reader from the remaining bytes.

**Known technical constraints:**
- JSON numbers use IEEE 754 double-precision (53-bit mantissa), so integers beyond 2^53 lose precision. The design decision to preserve large numbers as strings is correct.
- Windows console encoding defaults to legacy codepages; UTF-8 output may display incorrectly without `chcp 65001`. The tool should detect console capabilities or document this.
- `stdout` on Windows may translate `\n` to `\r\n` unless binary mode is explicitly set.

## Comparable Solutions

### Detailed Analysis

**csvkit (`csvjson`)** — Python, ~6.3K stars, v2.2.0 (Dec 2025)
Suite of 15+ CSV tools. Type inference is all-or-nothing (`--no-inference` flag). Leading zeros in "01234" are silently converted to 1234 unless inference is entirely disabled. Requires Python + pip with frequent dependency conflicts. Performance is the documented weakness — csvkit's own docs acknowledge it may not handle large files well. No JSON Schema generation.

**Miller (`mlr`)** — Go, ~9.7K stars, v6.16.0 (Jan 2026)
Swiss-army knife for structured data with a full DSL. Powerful but complex — `mlr --csv --json cat input.csv` is the conversion syntax, which is non-obvious. Type inference handles numbers but **does not infer booleans** (by design). Built-in BOM stripping since v5.2.0. Go binaries are 15-30MB, larger than Rust equivalents. No JSON Schema generation. Documented issues with ragged CSV handling during JSONL conversion.

**qsv** — Rust, ~3.2K stars, v16.0.0 (Feb 2026)
Fork of the archived xsv with 50+ commands. The `tojsonl` command does smart type inference (scans CSV, computes stats, infers types) but **only outputs JSONL, not JSON arrays**. The `schema` command generates JSON Schema Draft 2020-12. However, leading zeros are still converted to numbers unless the column is pre-classified as string — no simple CLI flag exists. The tool suffers from feature bloat and documentation sprawl; CSV-to-JSON conversion is buried among dozens of commands.

**dasel** — Go, ~7.7K stars, v3.1.4 (Dec 2025)
Universal data selector across JSON, YAML, TOML, XML, CSV. CSV support is basic with no type inference — all fields remain strings. Query-oriented rather than batch-conversion oriented. Not competitive for the CSV-to-JSON use case.

**csvtojson** — Node.js, ~2K stars, 825K weekly npm downloads
Has `checkType` for inference and `colParser` for per-column overrides. Requires Node.js runtime. Stagnating development (v2.0.10 unchanged for years). Single-threaded JavaScript performance. No schema generation.

**jq** — C, ~31K stars
No native CSV parser. Requires fragile manual field splitting via `--slurp --raw-input` that breaks on any non-trivial CSV (quoted fields, embedded commas, multiline values). Not a realistic competitor for this use case.

### Gap Analysis

| Capability | csvkit | Miller | qsv | dasel | csvtojson | **csv2json** |
|---|---|---|---|---|---|---|
| Single static binary | No | Yes | Yes | Yes | No | **Yes** |
| Type inference | All-or-nothing | Numbers only | Smart (stats-based) | None | Per-column | **Per-cell, strict** |
| Leading-zero preservation | No | No | No | N/A | Manual | **Yes (default)** |
| Boolean inference | Broad | No | Heuristic | No | checkType | **Strict: true/false** |
| JSON Schema 2020-12 | No | No | Yes | No | No | **Yes** |
| JSON array output | Yes | Yes | No (JSONL only) | Yes | Yes | **Yes** |
| JSONL output | Yes | Yes | Yes | No | Yes | **Yes (v0.2)** |
| BOM handling | No | Yes | Yes | No | No | **Yes** |
| Focused UX | Moderate | Complex | Complex | Generic | Simple | **Minimal** |
| Performance | Slow | Fast | Fast | Moderate | Moderate | **Fast** |

**The key gap**: No existing tool combines single-binary simplicity, strict type inference with leading-zero preservation as a default, JSON Schema generation, and both output formats in a focused, single-purpose package.

## Technical Feasibility

**Verdict: Highly feasible.** Every component has mature Rust crate support, and the architecture is straightforward.

**Core pipeline architecture:**
```
Input (file/stdin) → BOM detection → CSV Reader → Per-cell type inference → JSON serialization → Output (file/stdout)
```

**Type inference engine** — The most novel component, but algorithmically simple. Per-cell inference with the specified rules:

1. Check for empty string or `null`/`NULL` → JSON `null`
2. Check for `true`/`false` (case-sensitive) → JSON boolean
3. Check for numeric: reject if leading zeros (except plain `0`), accept if valid float/integer, preserve as string if precision would be lost (>53-bit integers)
4. Scientific notation (`1.5e10`) → JSON number
5. Default → JSON string

This is a deterministic finite decision tree with no ambiguity — each cell maps to exactly one JSON type. Implementation is ~50-100 lines of Rust.

**JSON Schema generation** — Programmatic construction using `serde_json::Value`. Scan all cells in each column, collect the set of observed types, and emit a schema with `type` arrays for mixed-type columns. Output conforms to Draft 2020-12 with `$schema`, `type: "array"`, `items.properties`, and per-property type constraints. Estimated at ~200 lines of Rust.

**Performance projections:**
- The `csv` crate at ~122 MB/s for string records, combined with `serde_json` serialization, should yield end-to-end throughput of **60-100 MB/s** for typical CSV files — comfortably converting a 1GB file in 10-17 seconds on commodity hardware.
- Streaming architecture (read row → infer → serialize → write) keeps memory usage proportional to a single row, not the entire file.
- The pretty-print flag adds minimal overhead (indentation is cheap).

**Binary size projection:** Based on comparable Rust CLI tools (ripgrep: ~5.5MB, fd: ~3.5MB, bat: ~5.8MB), a csv2json binary with `csv` + `serde_json` + `clap` should land at **2-4 MB** statically linked, well under the 5MB target.

**Dependency count is minimal:**
- `csv` (+ `csv-core`) — CSV parsing
- `serde` + `serde_json` — serialization
- `clap` — CLI parsing
- `unicode-bom` — BOM detection
- Total transitive dependencies: estimated ~30-40, all well-maintained

**Development effort estimate for MVP (v0.1):**
- CLI argument parsing and I/O: ~200 lines
- BOM detection wrapper: ~30 lines
- Type inference engine: ~100 lines
- JSON array serialization: ~50 lines
- JSON Schema generation: ~200 lines
- Duplicate header detection: ~30 lines
- Error handling and exit codes: ~50 lines
- **Total: ~660 lines of Rust** (excluding tests)
- Test suite: ~500-800 lines for comprehensive coverage of edge cases

## Risk Assessment

### 1. Type Inference Correctness — **HIGH RISK**

This is the single most important technical risk. The design decisions are sound (conservative defaults, strict boolean inference, leading-zero preservation), but edge cases are numerous:

| Edge Case | Risk Level | csv2json Handling |
|---|---|---|
| Leading zeros (`007`, `01234`) | Critical | Preserved as strings — **correct** |
| Plain `0` vs. `00` | High | `0` → number, `00` → string — must test boundary |
| Scientific notation (`1E10`, `1e-3`) | Moderate | Parsed as number — correct but could surprise if field is an ID |
| Large integers (`9999999999999999`) | High | Preserved as string to avoid IEEE 754 precision loss — **correct** |
| Phone numbers (`+1-555-1234`) | Low | Not numeric, stays string — correct |
| Negative numbers (`-42`) | Low | Parsed as number — correct |
| Whitespace-padded values (` 42 `) | Moderate | Trimmed then inferred — matches spec |
| Mixed-type columns (`42` then `foo`) | Moderate | Per-cell inference means row 1 is number, row 2 is string — JSON array permits mixed types |
| `NaN`, `Infinity` | Moderate | Not valid JSON numbers; should remain strings |
| Dates (`2024-01-15`) | Low | Not inferred as dates in v0.1; stays string — correct |

**Mitigation:** Comprehensive test suite with property-based tests (using `proptest` or `quickcheck` crates). Document all inference rules explicitly in `--help` and README. The per-cell (not per-column) inference model is simpler and less error-prone than statistical approaches.

### 2. Market Positioning — **LOW-MODERATE RISK**

The market is fragmented rather than saturated. No dominant single-purpose tool exists. The risk is not competition but **discoverability** — users need to find csv2json among the noise.

**Mitigation:** Clear positioning as "the ripgrep of CSV-to-JSON." Leading-zero preservation as the differentiating feature in marketing. Target the Rust community, Hacker News, and r/dataengineering for initial traction. Submit to package managers early (Homebrew, cargo install, apt/dnf, winget/scoop/chocolatey).

### 3. Feature Creep — **MODERATE RISK**

Users will inevitably request: Excel support, Parquet output, XML input, embedded scripting, SQL queries, date parsing, custom formatters. Each addition dilutes the single-purpose value proposition.

**Mitigation:** Explicit project scope in README and CONTRIBUTING.md. Follow the Unix philosophy: csv2json does one thing. Complex transformations belong downstream (`jq`, `mlr`, application code). The `--types` flag (v0.3) provides an escape valve for type control without general-purpose scripting.

### 4. Cross-Platform Edge Cases — **LOW RISK**

The Rust ecosystem handles cross-platform concerns well. The `csv` crate transparently handles CRLF/LF/CR line endings. Rust's `Path`/`OsString` handles Unicode paths on all platforms. The main risks are:

- **Windows console encoding**: UTF-8 output may render incorrectly in legacy `cmd.exe`. Mitigation: detect console capabilities, document the `chcp 65001` workaround.
- **stdout binary mode on Windows**: Rust's `stdout()` on Windows may translate `\n` to `\r\n`. Mitigation: use `BufWriter` with explicit binary mode for JSON output.

### 5. Maintenance Burden — **LOW RISK**

A focused tool with a minimal dependency tree (3-4 direct dependencies, all actively maintained) has inherently low maintenance burden. The CLI interface is a stable contract — flag names and output format rarely need breaking changes. Rust's type system catches many bugs at compile time. The primary ongoing costs are:

- Responding to edge case bug reports in type inference
- Keeping dependencies up to date (Dependabot/Renovate)
- Reviewing and triaging feature requests (discipline to say no)

### 6. Duplicate Header Policy — **LOW RISK**

The "warn to stderr, last value wins" policy is pragmatic and matches user expectations. The main risk is users who expect different behavior (error-out, suffix with `_1`/`_2`, or array aggregation). Document the behavior clearly; consider a `--duplicate-headers` flag in a future version.

### Risk Summary Matrix

| Risk | Severity | Likelihood | Overall | Primary Mitigation |
|---|---|---|---|---|
| Type inference bugs | High | Moderate | **HIGH** | Comprehensive test suite, conservative defaults |
| Market discoverability | Moderate | Moderate | **MODERATE** | Strong positioning, early package manager presence |
| Feature creep | Moderate | High | **MODERATE** | Explicit scope, Unix philosophy discipline |
| Cross-platform issues | Low | Low | **LOW** | Mature Rust ecosystem, CI on all platforms |
| Maintenance burden | Low | Low | **LOW** | Minimal dependencies, stable CLI interface |
| Duplicate header edge cases | Low | Low | **LOW** | Clear documentation, future flag extensibility |