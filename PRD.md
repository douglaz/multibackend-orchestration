# Product Requirements Document: csv2json

## Executive Summary

csv2json is a single-purpose, zero-configuration CLI tool written in Rust that converts CSV files to JSON with strict, predictable type inference. It ships as a single static binary under 5 MB with no runtime dependencies, no telemetry, and no network access.

The core thesis is that **correctness is the killer feature**. Every existing tool in this space makes at least one critical mistake: silently converting zip codes (`07042`) to numbers, requiring a language runtime, bundling CSV-to-JSON as an afterthought inside a Swiss-army-knife tool, or offering only all-or-nothing type inference. csv2json occupies the gap — a focused tool that gets the defaults right so pipelines don't silently corrupt data.

The market timing is favorable. xsv (10.8K GitHub stars), the most prominent Rust CSV tool, was archived in April 2025, leaving a visible gap. The data engineering market exceeds $120B with 87% of companies reporting skills gaps, meaning more generalist developers are building data pipelines and need tools that are safe by default. The Rust CLI renaissance (ripgrep, bat, fd, eza) has trained users to expect fast, correct, cross-platform binaries installable via `cargo install`.

The MVP (v0.1) delivers file/stdin input, file/stdout output, per-cell type inference with leading-zero preservation, JSON Schema generation, BOM handling, configurable delimiters, and cross-platform binaries — estimated at ~660 lines of Rust with a ~700-line test suite.

---

## Goals & Non-Goals

### Goals

1. **Correct type inference by default.** Per-cell inference with strict, documented rules: `true`/`false` (case-sensitive) become booleans, empty strings and `null`/`NULL` become JSON null, numeric strings become numbers — but values with leading zeros are preserved as strings to prevent silent data corruption. Large integers (>2^53) are preserved as strings to avoid IEEE 754 precision loss.

2. **Zero-configuration happy path.** `csv2json data.csv` produces correct output with no flags required. Sensible defaults (comma delimiter, compact output, fail-fast errors, type inference enabled) cover 90% of use cases.

3. **Pipeline composability.** Behave as a well-mannered Unix citizen: data to stdout, diagnostics to stderr, meaningful exit codes, stdin support without special flags. Compose seamlessly with `cat`, `jq`, `grep`, and shell pipelines.

4. **Cross-platform from day one.** Identical behavior on Linux (x86_64, aarch64), macOS (Intel + Apple Silicon), and Windows (x86_64). Pre-built binaries for all platforms on every release.

5. **Schema discovery.** Standalone `--schema` mode emits a JSON Schema Draft 2020-12 document describing inferred column types, enabling validation pipeline setup and dataset exploration without writing transformation code.

6. **Single static binary.** No runtime dependencies, no language interpreters, no shared libraries. Install via `cargo install`, download a binary from GitHub Releases, or install from a package manager.

7. **Performance competitive with native tools.** Convert a 1 GB CSV file in under 20 seconds on commodity hardware. Row-by-row streaming keeps memory usage proportional to row size, not file size.

### Non-Goals

- **No Excel, Parquet, XML, or YAML input.** csv2json converts CSV. Other formats have dedicated tools.
- **No embedded scripting or plugin system.** Complex transformations belong downstream in `jq`, `mlr`, or application code.
- **No configuration files.** All behavior is controlled via CLI flags. No dotfiles, no environment variable magic, no hidden defaults.
- **No telemetry, update checks, or network access.** The binary is fully offline.
- **No date/time inference.** Dates remain strings. Date formats are ambiguous (`01/02/03`) and locale-dependent; inference would violate the correctness-first principle.
- **No multi-file batch processing in v0.1.** Single file per invocation. Batch mode is deferred to v0.3+.
- **No query language or row filtering in v0.1.** Filtering belongs in `jq` or `mlr` for the MVP; a minimal `--where` expression language is considered for v0.3+.
- **No non-UTF-8 encoding support in v0.1.** UTF-8 only. Encoding detection and `--encoding` flag are deferred to v0.2.

---

## User Stories

### US-1: Data Pipeline Operator
> As a data engineer building ETL pipelines, I want to convert CSV exports from databases and vendor APIs into JSON so that downstream services can consume them without writing custom parsers. I need the tool to **preserve leading zeros** in fields like zip codes and part numbers, because silent numeric conversion has caused production incidents before. I need deterministic behavior so the same input always produces the same output, and I need nonzero exit codes on failure so my pipeline orchestrator (Airflow, Prefect, cron) can detect problems.

**Acceptance criteria:**
- `csv2json vendors.csv -o vendors.json` produces a JSON array of objects with correct types
- A zip code column containing `07042` appears as `"07042"` (string), not `7042` (number)
- Exit code 0 on success, nonzero on any error
- Identical output for identical input across runs

### US-2: Unix Pipeline Composer
> As a CLI power user, I want to pipe CSV data through csv2json and into jq for filtering: `cat users.csv | csv2json | jq '.[] | select(.active == true)'`. I need compact output by default (for piping) with a `--pretty` flag for human reading. I need stdin support without special flags — if no file argument is given, read from stdin.

**Acceptance criteria:**
- `cat data.csv | csv2json` reads from stdin and writes JSON to stdout
- Output is compact (single line per default) for piping efficiency
- `csv2json data.csv --pretty` produces indented, human-readable JSON
- Boolean values inferred from `true`/`false` literals work with jq boolean filters

### US-3: Schema-First API Developer
> As a backend developer receiving CSV files from external partners, I want to generate a JSON Schema from a sample CSV so I can set up validation in my API ingestion layer. I want `csv2json --schema partner_data.csv` to emit a Draft 2020-12 JSON Schema describing inferred column types.

**Acceptance criteria:**
- `csv2json --schema data.csv` outputs a valid JSON Schema Draft 2020-12 document
- Schema includes `$schema`, `type: "array"`, `items.properties` with per-column types
- Mixed-type columns produce union type arrays (e.g., `{"type": ["number", "string"]}`)
- All columns appear in the `required` array
- Schema-only CSV (headers, no data rows) produces properties typed as `{"type": "null"}`

### US-4: DevOps Engineer
> As a DevOps engineer, I maintain inventory data in CSV spreadsheets that need to be converted to JSON for Terraform and Ansible. I need the tool to handle CSVs with duplicate column headers (common after spreadsheet merges) without crashing — warn me, but don't block the pipeline. I need the tool to work identically on my macOS laptop, the Linux CI runners, and the Windows jump box.

**Acceptance criteria:**
- Duplicate column headers produce a warning on stderr but do not cause an error
- Last-value-wins semantics for duplicate headers
- Output is byte-identical across Linux, macOS, and Windows for the same input

### US-5: Cautious Data Analyst
> As a data analyst exploring an unfamiliar CSV dataset, I want to convert it to JSON to load into a notebook. I want booleans to only be `true`/`false` (not `yes`/`1`/`TRUE`), nulls to be explicit, and numbers to not lose precision. If the CSV has no headers, I want `--no-header` to auto-generate sensible keys.

**Acceptance criteria:**
- Only literal `true` and `false` (case-sensitive) become JSON booleans
- `True`, `TRUE`, `yes`, `1` remain strings
- `--no-header` generates keys `field_0`, `field_1`, `field_2`, etc.
- Large integers (>2^53) are preserved as strings, not truncated by floating-point representation

### US-6: CI/CD Integration Engineer
> As a developer integrating csv2json into CI pipelines, I want shell completions for bash, zsh, fish, and PowerShell so my team can discover flags without reading docs. I want distinct exit codes per error category so I can write conditional logic in shell scripts.

**Acceptance criteria:**
- `csv2json --completions bash` outputs a valid bash completion script
- Exit codes: `0` success, `1` I/O error, `2` CSV parse error, `3` invalid arguments, `4` schema error
- `csv2json --version` outputs `csv2json 0.1.0`

---

## Functional Requirements

### FR-1: Input Handling

| ID | Requirement | Priority |
|---|---|---|
| FR-1.1 | Accept a single file path as a positional argument: `csv2json data.csv` | Must |
| FR-1.2 | Read from stdin when no file argument is provided: `cat data.csv \| csv2json` | Must |
| FR-1.3 | When stdin is a TTY (interactive terminal) and no file argument is given, print a help hint to stderr and wait for input | Must |
| FR-1.4 | Detect and strip UTF-8 BOM (`0xEF 0xBB 0xBF`) from the beginning of input | Must |
| FR-1.5 | Reject non-UTF-8 input with exit code 2 and a clear error message identifying the byte offset of the invalid sequence | Must |
| FR-1.6 | Accept only a single file per invocation; multiple file arguments produce exit code 3 | Must |

### FR-2: Output Handling

| ID | Requirement | Priority |
|---|---|---|
| FR-2.1 | Write JSON output to stdout by default | Must |
| FR-2.2 | Support `-o` / `--output <PATH>` to write to a file; overwrite if the file exists | Must |
| FR-2.3 | Compact JSON output by default (no unnecessary whitespace) | Must |
| FR-2.4 | `--pretty` flag produces indented JSON (2-space indent) | Must |
| FR-2.5 | Output is a JSON array of objects in data mode, where each CSV row becomes one object | Must |
| FR-2.6 | Object keys appear in CSV column order (insertion-order preservation) | Must |
| FR-2.7 | Empty CSV (headers only, no data rows) produces `[]` | Must |
| FR-2.8 | Ensure binary-mode stdout on Windows (no `\n` → `\r\n` translation) | Must |

### FR-3: CSV Parsing

| ID | Requirement | Priority |
|---|---|---|
| FR-3.1 | Parse CSV per RFC 4180 baseline via the `csv` crate | Must |
| FR-3.2 | Default delimiter is comma; configurable via `--delimiter <CHAR>` | Must |
| FR-3.3 | `--delimiter` accepts literal characters and the `\t` escape sequence for tab | Must |
| FR-3.4 | First row is treated as the header row by default | Must |
| FR-3.5 | `--no-header` flag: treat all rows as data, auto-generate keys `field_0`, `field_1`, ... | Must |
| FR-3.6 | Trim leading and trailing whitespace from all cell values before inference | Must |
| FR-3.7 | Trim leading and trailing whitespace from header values | Must |

### FR-4: Duplicate Header Handling

| ID | Requirement | Priority |
|---|---|---|
| FR-4.1 | When duplicate column headers are detected, emit a warning to stderr identifying the duplicated header name(s) | Must |
| FR-4.2 | For duplicate headers, last-value-wins: the rightmost column with a given name determines the value in the output object | Must |

### FR-5: Ragged Row Handling

| ID | Requirement | Priority |
|---|---|---|
| FR-5.1 | By default, rows with fewer fields than headers are null-padded (missing fields become `null`) | Must |
| FR-5.2 | `--strict` flag: rows with fewer or more fields than the header row cause an error (exit code 2) | Must |
| FR-5.3 | Without `--strict`, rows with more fields than headers silently drop extra fields | Should |

### FR-6: Type Inference Engine

| ID | Requirement | Priority |
|---|---|---|
| FR-6.1 | Type inference is per-cell: each value is independently inferred regardless of other values in the same column | Must |
| FR-6.2 | Inference is enabled by default; `--no-inference` emits all values as JSON strings (except empty cells, which are still `null`) | Must |
| FR-6.3 | **Null inference:** empty string `""` → `null`; literal `null`/`NULL`/`Null` (case-insensitive) → `null` | Must |
| FR-6.4 | **Null inference for whitespace-only cells:** trim first, then if empty → `null` | Must |
| FR-6.5 | **Boolean inference:** only case-sensitive `true` → `true`, `false` → `false`; all other variants (`True`, `TRUE`, `yes`, `1`, `on`) remain strings | Must |
| FR-6.6 | **Numeric inference:** valid numeric strings become JSON numbers; integers, decimals, negative numbers, and numbers with a leading `+` sign are accepted | Must |
| FR-6.7 | **Leading-zero preservation:** any numeric string with a leading zero (except plain `0`) is preserved as a string (`007` → `"007"`, `0.5` → `0.5`, `0` → `0`) | Must |
| FR-6.8 | **Scientific notation:** strings matching `[+-]?\d+(\.\d+)?[eE][+-]?\d+` are parsed as JSON numbers | Must |
| FR-6.9 | **Large number precision:** integers with absolute value > 2^53 are preserved as strings to avoid IEEE 754 precision loss | Must |
| FR-6.10 | **NaN/Infinity:** `NaN`, `Infinity`, `-Infinity`, `+Infinity` (any casing) remain strings (not valid JSON numbers) | Must |
| FR-6.11 | **Negative zero:** `-0` is treated as numeric `0` (JSON number `0`) | Must |
| FR-6.12 | **Quoted field inference:** CSV quoting is structural, not semantic; quoted values are inferred the same as unquoted values | Must |
| FR-6.13 | **Embedded JSON:** cell values that look like JSON objects or arrays are kept as strings (not parsed) | Must |
| FR-6.14 | **Empty cells with `--no-inference`:** empty cells produce `null` regardless of the `--no-inference` flag | Must |

### FR-7: Schema Mode

| ID | Requirement | Priority |
|---|---|---|
| FR-7.1 | `--schema` flag: output a JSON Schema Draft 2020-12 document instead of data | Must |
| FR-7.2 | Schema includes `"$schema": "https://json-schema.org/draft/2020-12/schema"` | Must |
| FR-7.3 | Root schema type is `"array"` with `items` describing the row object structure | Must |
| FR-7.4 | Each CSV column becomes a property in `items.properties` with inferred type(s) | Must |
| FR-7.5 | Mixed-type columns use union type arrays: `{"type": ["number", "string"]}` | Must |
| FR-7.6 | All columns appear in the `required` array | Must |
| FR-7.7 | Headers-only CSV (no data rows) emits schema with all properties typed as `{"type": "null"}` | Must |
| FR-7.8 | Schema mode respects `--pretty` for indented output | Must |
| FR-7.9 | Schema mode exit code for schema generation errors is `4` | Must |

### FR-8: Error Handling

| ID | Requirement | Priority |
|---|---|---|
| FR-8.1 | Default error mode is fail-fast: exit on the first error encountered | Must |
| FR-8.2 | Error messages are human-readable and written to stderr | Must |
| FR-8.3 | Error messages include context where applicable: line number, column index, problematic value | Must |
| FR-8.4 | Exit code `0`: success | Must |
| FR-8.5 | Exit code `1`: I/O error (file not found, permission denied, write failure) | Must |
| FR-8.6 | Exit code `2`: CSV parse error (malformed row, encoding error, ragged row in strict mode) | Must |
| FR-8.7 | Exit code `3`: invalid arguments (unknown flag, bad delimiter value, conflicting flags) | Must |
| FR-8.8 | Exit code `4`: schema generation error | Must |
| FR-8.9 | The `--on-error` flag is omitted from v0.1; fail-fast is the only behavior | Must |

### FR-9: CLI Interface

| ID | Requirement | Priority |
|---|---|---|
| FR-9.1 | `csv2json --help` displays usage, all flags, and a brief description | Must |
| FR-9.2 | `csv2json --version` outputs `csv2json 0.1.0` (simple format, no extra metadata) | Must |
| FR-9.3 | `csv2json --completions <SHELL>` generates shell completion scripts for bash, zsh, fish, and PowerShell | Must |
| FR-9.4 | Unknown flags produce exit code 3 with a helpful error message | Must |

---

## Non-Functional Requirements

### NFR-1: Performance

| ID | Requirement | Target |
|---|---|---|
| NFR-1.1 | End-to-end throughput for CSV-to-JSON conversion | ≥60 MB/s on commodity hardware |
| NFR-1.2 | 1 GB CSV file conversion time | <20 seconds on a GitHub Actions runner |
| NFR-1.3 | Memory usage during conversion | O(row_size), not O(file_size); streaming row-by-row |
| NFR-1.4 | Startup time (empty input) | <50ms cold start |

### NFR-2: Binary Distribution

| ID | Requirement | Target |
|---|---|---|
| NFR-2.1 | Release binary size (all platforms) | <5 MB statically linked |
| NFR-2.2 | Linux binary | Fully static via musl; zero runtime dependencies |
| NFR-2.3 | macOS binary | Universal binary (x86_64 + aarch64) |
| NFR-2.4 | Windows binary | Static CRT linking |
| NFR-2.5 | No runtime dependencies | No language interpreters, shared libraries, or system packages required |

### NFR-3: Compatibility

| ID | Requirement |
|---|---|
| NFR-3.1 | Minimum supported Rust version (MSRV): current stable minus 2 (N-2 policy) |
| NFR-3.2 | CI tests pass on Linux x86_64, Linux aarch64, macOS x86_64, macOS aarch64, Windows x86_64 |
| NFR-3.3 | Identical output for identical input across all supported platforms |
| NFR-3.4 | License: MIT OR Apache-2.0 (dual-licensed, Rust ecosystem convention) |

### NFR-4: Reliability

| ID | Requirement |
|---|---|
| NFR-4.1 | Deterministic output: same input always produces byte-identical output |
| NFR-4.2 | No partial output on error: either the entire output is written or nothing is (for file output mode) |
| NFR-4.3 | No silent data loss or corruption under any input |
| NFR-4.4 | Any reported data corruption or incorrect inference is treated as P0 |

### NFR-5: Privacy & Telemetry

| ID | Requirement |
|---|---|
| NFR-5.1 | No telemetry, analytics, or crash reporting of any kind |
| NFR-5.2 | No network access; the binary is fully offline |
| NFR-5.3 | No update checks or phone-home behavior |

---

## Technical Architecture

### System Pipeline

```
┌─────────────────────────────────────────────────────────────────┐
│                         CLI Layer (clap)                        │
│  Argument parsing, validation, help generation, completions     │
└──────────────────────────────┬──────────────────────────────────┘
                               │ Config struct
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│                        Input Layer                              │
│  File reader / stdin reader → BOM detection → byte stream       │
└──────────────────────────────┬──────────────────────────────────┘
                               │ BOM-stripped byte stream
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│                     CSV Parser (csv crate)                      │
│  Configurable delimiter, RFC 4180, flexible quoting             │
│  Emits: headers (StringRecord) + row iterator (StringRecord)    │
└──────────┬───────────────────────────────────────┬──────────────┘
           │ headers                               │ rows (streaming)
           ▼                                       ▼
┌─────────────────────┐              ┌────────────────────────────┐
│  Header Processor   │              │    Type Inference Engine   │
│  Duplicate detect   │              │  Per-cell decision tree:   │
│  Warn to stderr     │              │  null → bool → number →    │
│  --no-header: gen   │              │  string (fallback)         │
│  field_0, field_1   │              │  Respects --no-inference   │
└─────────┬───────────┘              └─────────────┬──────────────┘
          │ Vec<String>                            │ serde_json::Value
          ▼                                        ▼
┌─────────────────────────────────────────────────────────────────┐
│                     Output Serializer                           │
│  Mode: data (array-of-objects) │ schema (JSON Schema 2020-12)  │
│  Format: compact (default) │ pretty (--pretty)                  │
│  Target: stdout (default) │ file (-o path)                      │
└──────────────────────────────┬──────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│                        Error Handler                            │
│  Structured errors → stderr with context                        │
│  Exit codes: 0 success, 1 I/O, 2 parse, 3 argument, 4 schema   │
└─────────────────────────────────────────────────────────────────┘
```

### Module Structure

```
csv2json/
├── Cargo.toml                 # Workspace manifest, dependencies, metadata
├── src/
│   ├── main.rs                # Entry point, error-to-exit-code mapping
│   ├── cli.rs                 # clap derive structs, argument validation
│   ├── input.rs               # File/stdin reader, BOM detection and stripping
│   ├── inference.rs           # Per-cell type inference engine (pure function)
│   ├── headers.rs             # Header processing, duplicate detection, auto-generation
│   ├── output.rs              # JSON array-of-objects serialization (data mode)
│   ├── schema.rs              # JSON Schema 2020-12 generation (schema mode)
│   └── error.rs               # Error enum, Display impl, exit code mapping
├── tests/
│   ├── inference_tests.rs     # Unit tests for every inference rule
│   ├── integration_tests.rs   # End-to-end CLI tests (file in → JSON out)
│   ├── schema_tests.rs        # JSON Schema output validation
│   └── fixtures/              # Test CSV files (various delimiters, edge cases)
├── .github/
│   └── workflows/
│       ├── ci.yml             # Test on Linux/macOS/Windows, clippy, fmt
│       └── release.yml        # Build binaries, create GitHub Release
├── LICENSE-MIT
├── LICENSE-APACHE
└── README.md
```

### Dependency Graph (MVP)

| Crate | Version | Purpose | Downloads |
|---|---|---|---|
| `csv` | 1.x | CSV parsing (RFC 4180, configurable delimiter) | 129M+ |
| `serde` | 1.x | Serialization framework | 400M+ |
| `serde_json` | 1.x | JSON serialization, `Value` type, pretty-printing | 400M+ |
| `clap` | 4.x (derive + string features) | CLI parsing, help, completions | 200M+ |
| `unicode-bom` | 2.x | BOM detection and classification | 1M+ |
| `indexmap` | 2.x | Insertion-order-preserving map for key ordering | 200M+ |

Total direct dependencies: **6**. Estimated transitive: ~35. All MIT/Apache-2.0 compatible.

### Key Design Decisions

1. **Per-cell inference, not per-column.** Each cell is independently typed via a stateless pure function `fn infer(value: &str) -> serde_json::Value`. This avoids two-pass architectures, buffering, and ambiguity when columns contain mixed data. JSON arrays natively support mixed types.

2. **Streaming row-by-row processing.** Write `[` at the start, serialize each row object separated by commas, write `]` at the end. Memory usage is O(row_size). Schema mode requires a full pass but only stores type metadata per column, not data.

3. **`IndexMap` for key ordering.** Each row object is built using `indexmap::IndexMap<String, serde_json::Value>` to preserve CSV column order in JSON output. `serde_json::to_writer` serializes `IndexMap` in insertion order.

4. **Atomic file output.** When `-o` is specified, write to a temporary file in the same directory, then rename on success. This prevents partial output files on error. (On error, the temporary file is deleted.)

5. **Binary-mode stdout on Windows.** Use `std::io::stdout().lock()` wrapped in `BufWriter`, and avoid `println!` macro (which performs newline translation on Windows). Explicitly write `\n` bytes.

---

## Data Model

### Input Model: CSV

```
┌─────────────────────────────────────────────────┐
│ CSV File                                         │
│                                                  │
│  [Optional BOM: 0xEF 0xBB 0xBF]                │
│  header_1,header_2,...,header_n\n                │
│  value_1_1,value_1_2,...,value_1_n\n            │
│  value_2_1,value_2_2,...,value_2_n\n            │
│  ...                                             │
│  value_m_1,value_m_2,...,value_m_n\n            │
└─────────────────────────────────────────────────┘
```

- Delimiter: configurable (default `,`)
- Quoting: double-quote per RFC 4180
- Line endings: CRLF, LF, or CR (handled by `csv` crate)
- Encoding: UTF-8 only (v0.1)

### Output Model: JSON Data Mode

```json
[
  {
    "header_1": <inferred_value>,
    "header_2": <inferred_value>,
    ...
    "header_n": <inferred_value>
  },
  ...
]
```

Each `<inferred_value>` is one of:
- `null` — from empty cell, whitespace-only cell, or `null`/`NULL`/`Null` literal
- `true` / `false` — from case-sensitive `true` / `false` literals
- JSON number — from valid numeric strings without leading zeros, within 2^53 precision
- JSON string — everything else (including leading-zero numbers, large integers, NaN, Infinity, embedded JSON)

### Output Model: JSON Schema Mode

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "array",
  "items": {
    "type": "object",
    "properties": {
      "header_1": { "type": "string" },
      "header_2": { "type": ["number", "null"] },
      "header_3": { "type": ["string", "number", "boolean", "null"] }
    },
    "required": ["header_1", "header_2", "header_3"]
  }
}
```

- Single-type columns: `{"type": "string"}`
- Mixed-type columns: `{"type": ["string", "number"]}` (union type array, ordered alphabetically)
- Columns with null values: `null` appears in the type array alongside other observed types
- Headers-only CSV: all properties typed as `{"type": "null"}`

### Type Inference Decision Tree

```
fn infer(value: &str) -> Value:
    trimmed = value.trim()

    if trimmed.is_empty():
        return Null

    if trimmed.eq_ignore_ascii_case("null"):
        return Null

    if trimmed == "true":
        return Bool(true)

    if trimmed == "false":
        return Bool(false)

    if is_numeric(trimmed):
        if has_leading_zeros(trimmed):     // "007", "00", "01" → string
            return String(trimmed)
        if is_nan_or_infinity(trimmed):    // NaN, Infinity, -Infinity → string
            return String(trimmed)
        if integer_exceeds_2_53(trimmed):  // > 9007199254740991 → string
            return String(trimmed)
        return Number(parse(trimmed))      // handles int, float, scientific, +/-

    return String(trimmed)
```

### Exit Code Model

| Code | Meaning | Trigger Examples |
|---|---|---|
| `0` | Success | Normal conversion completed |
| `1` | I/O error | File not found, permission denied, disk full, broken pipe |
| `2` | CSV parse error | Malformed CSV, invalid UTF-8, ragged row (strict mode) |
| `3` | Invalid arguments | Unknown flag, invalid delimiter, conflicting flags |
| `4` | Schema error | Schema generation failure |

---

## API Design

csv2json is a CLI tool. Its "API" is its command-line interface — flag names, positional arguments, output format, and exit codes form a stable contract.

### Command Synopsis

```
csv2json [OPTIONS] [FILE]

Arguments:
  [FILE]  Input CSV file (reads from stdin if omitted)

Options:
  -o, --output <PATH>       Write output to file instead of stdout
      --delimiter <CHAR>    Field delimiter [default: ,]
      --no-header           Treat first row as data; generate field_0, field_1, ... keys
      --no-inference        Disable type inference; emit all values as strings
      --pretty              Pretty-print JSON output with 2-space indentation
      --strict              Error on ragged rows (rows with different field counts than header)
      --schema              Output JSON Schema instead of data
      --completions <SHELL> Generate shell completions [possible values: bash, zsh, fish, powershell]
  -h, --help                Print help
  -V, --version             Print version
```

### Flag Semantics

| Flag | Type | Default | Behavior |
|---|---|---|---|
| `FILE` | positional, optional | stdin | Path to input CSV file |
| `-o, --output` | string | stdout | Output file path; overwrites existing |
| `--delimiter` | char | `,` | Single character or `\t` for tab |
| `--no-header` | bool | `false` | Auto-generate `field_0`, `field_1`, ... |
| `--no-inference` | bool | `false` | All values as strings (empty cells still `null`) |
| `--pretty` | bool | `false` | 2-space indented JSON |
| `--strict` | bool | `false` | Error on ragged rows |
| `--schema` | bool | `false` | Emit JSON Schema instead of data |
| `--completions` | enum | n/a | Emit shell completion script and exit |

### Output Contract

**Data mode** (default):
- Content-type equivalent: `application/json`
- Root element: JSON array `[...]`
- Each element: JSON object with string keys matching CSV headers
- Key order: CSV column order (left to right)
- Encoding: UTF-8, no BOM in output
- Line ending: `\n` (LF) on all platforms

**Schema mode** (`--schema`):
- Content-type equivalent: `application/schema+json`
- Root element: JSON Schema object
- Conforms to JSON Schema Draft 2020-12

**Stderr output:**
- Warnings (duplicate headers, etc.): `csv2json: warning: <message>`
- Errors: `csv2json: error: <message>`
- Help hint (TTY stdin): `csv2json: reading from stdin; pass a file argument or pipe data. Use --help for usage.`

### Versioning Strategy

The CLI interface follows semantic versioning:
- **Patch** (0.1.x): bug fixes, performance improvements, no CLI changes
- **Minor** (0.x.0): new flags (always additive), new output formats
- **Major** (x.0.0): breaking changes to existing flag behavior or output format

The v0.x series reserves the right to make breaking changes per semver convention, but the intent is to stabilize the core interface (flag names, exit codes, inference rules) from v0.1 onward.

---

## Security Considerations

### Threat Model

csv2json processes untrusted input (user-provided CSV files) and produces output. It performs no network access, no code execution, and no privileged system operations. The attack surface is narrow.

### Input Validation

| Threat | Mitigation |
|---|---|
| **Malformed CSV causing crashes** | The `csv` crate is memory-safe (Rust) and has been fuzz-tested. csv2json adds no `unsafe` code. All parsing errors are caught and reported via exit code 2. |
| **Path traversal via `-o` flag** | The output path is passed directly to `std::fs::File::create`. No path manipulation or directory creation is performed. The OS enforces filesystem permissions. |
| **Denial of service via large input** | Streaming architecture processes one row at a time. Memory usage is bounded by the largest single row, not the file size. CPU usage is linear in input size — no algorithmic amplification. |
| **Billion-row CSV** | No row count limit. Processing is linear. The tool will run until complete or until I/O fails. |
| **Malicious filenames in error messages** | Error messages include the filename as provided by the user. No shell escaping is needed because error messages go to stderr (not executed). Terminal escape sequences in filenames could affect terminal display; this is a known, accepted risk for CLI tools. |
| **Zip-bomb-style compressed input** | csv2json does not decompress input. Compressed files are invalid UTF-8 and rejected immediately. |

### Supply Chain

| Concern | Mitigation |
|---|---|
| **Dependency count** | 6 direct dependencies, ~35 transitive. All are high-download, actively maintained crates in the Rust ecosystem. |
| **Dependency auditing** | Run `cargo audit` in CI on every build. Pin dependencies via `Cargo.lock` in the repository. |
| **Binary provenance** | GitHub Actions builds release binaries. Consider SLSA provenance attestations for release artifacts. |
| **No telemetry** | No network access of any kind. Verified by the absence of any networking crate in the dependency tree. |

### File System Safety

- Output file writing uses atomic rename (write to temp file, rename on success) to prevent partial output
- No directory creation; `-o` only writes to the specified path
- No file deletion
- No reading of files other than the specified input file
- No environment variable access beyond what clap provides for `--help` width detection

---

## Testing Strategy

### Test Pyramid

```
                    ┌───────────────┐
                    │  Performance  │   Benchmarks (1 GB CSV, CI)
                    │   Benchmarks  │
                    ├───────────────┤
                  ┌─┤  Integration  ├─┐   End-to-end CLI tests
                  │ │    Tests      │ │   (file → binary → output)
                  │ ├───────────────┤ │
                ┌─┤ │    Unit       │ ├─┐   Type inference, headers,
                │ │ │    Tests      │ │ │   schema, error mapping
                │ │ ├───────────────┤ │ │
              ┌─┤ │ │   Property    │ │ ├─┐   Fuzz type inference
              │ │ │ │    Tests      │ │ │ │   with random strings
              │ │ │ └───────────────┘ │ │ │
              └─┴─┴───────────────────┴─┴─┘
```

### Unit Tests (~30 tests)

**Type inference engine** (`inference.rs`):

| Test Case | Input | Expected Output |
|---|---|---|
| Empty string | `""` | `null` |
| Whitespace only | `"   "` | `null` |
| Literal null (lowercase) | `"null"` | `null` |
| Literal NULL (uppercase) | `"NULL"` | `null` |
| Literal Null (mixed) | `"Null"` | `null` |
| Literal nUlL (mixed) | `"nUlL"` | `null` |
| Boolean true | `"true"` | `true` |
| Boolean false | `"false"` | `false` |
| Boolean True (rejected) | `"True"` | `"True"` |
| Boolean TRUE (rejected) | `"TRUE"` | `"TRUE"` |
| Boolean yes (rejected) | `"yes"` | `"yes"` |
| Boolean 1 (rejected) | `"1"` | `1` (number) |
| Integer | `"42"` | `42` |
| Negative integer | `"-42"` | `-42` |
| Plus-sign integer | `"+42"` | `42` |
| Float | `"3.14"` | `3.14` |
| Scientific notation | `"1.5e10"` | `15000000000` |
| Scientific notation (E) | `"3E-4"` | `0.0003` |
| Leading zero | `"007"` | `"007"` |
| Leading zero (double) | `"00"` | `"00"` |
| Plain zero | `"0"` | `0` |
| Float with leading zero integer part | `"0.5"` | `0.5` |
| Large integer (>2^53) | `"9007199254740993"` | `"9007199254740993"` |
| Large integer (exactly 2^53) | `"9007199254740992"` | `9007199254740992` |
| NaN | `"NaN"` | `"NaN"` |
| Infinity | `"Infinity"` | `"Infinity"` |
| Negative infinity | `"-Infinity"` | `"-Infinity"` |
| Negative zero | `"-0"` | `0` |
| Whitespace-padded number | `"  42  "` | `42` |
| Regular string | `"hello"` | `"hello"` |
| Embedded JSON | `'{"key": "val"}'` | `'{"key": "val"}'` |

**Header processing** (`headers.rs`):
- No duplicates → no warning
- One duplicate → warning with header name
- All same → warning listing the name
- `--no-header` → `field_0`, `field_1`, `field_2`
- Whitespace in headers → trimmed

### Integration Tests (~20 tests)

End-to-end tests that invoke the compiled binary via `std::process::Command`:

| Test | Command | Expected |
|---|---|---|
| Basic conversion | `csv2json test.csv` | Correct JSON array to stdout |
| Stdin input | `echo "a,b\n1,2" \| csv2json` | `[{"a":1,"b":2}]` |
| File output | `csv2json test.csv -o out.json` | Correct JSON in `out.json` |
| Pretty print | `csv2json test.csv --pretty` | Indented JSON |
| Tab delimiter | `csv2json test.tsv --delimiter '\t'` | Correct parsing |
| No header | `csv2json noheader.csv --no-header` | Keys are `field_0`, `field_1` |
| No inference | `csv2json test.csv --no-inference` | All values as strings (empty → null) |
| Schema mode | `csv2json test.csv --schema` | Valid JSON Schema 2020-12 |
| Schema + pretty | `csv2json test.csv --schema --pretty` | Indented schema |
| Empty CSV (headers only) | `csv2json empty.csv` | `[]` |
| BOM handling | `csv2json bom.csv` | Correct output (BOM stripped) |
| Duplicate headers | `csv2json dup.csv` | Warning on stderr, last-value-wins |
| Ragged rows (default) | `csv2json ragged.csv` | Null-padded |
| Ragged rows (strict) | `csv2json ragged.csv --strict` | Exit code 2 |
| Missing file | `csv2json nonexistent.csv` | Exit code 1 |
| Invalid UTF-8 | `csv2json binary.csv` | Exit code 2 |
| Version | `csv2json --version` | `csv2json 0.1.0` |
| Completions | `csv2json --completions bash` | Non-empty bash completion script |
| Key ordering | `csv2json test.csv` | Keys in CSV column order |
| Overwrite output | `csv2json test.csv -o existing.json` | File overwritten |

### Property-Based Tests (~5 test generators)

Using the `proptest` crate:

1. **Roundtrip safety:** For any valid CSV (generated), csv2json produces valid JSON that parses without error.
2. **Determinism:** Running csv2json twice on the same input produces byte-identical output.
3. **No inference is safe:** With `--no-inference`, all values in output are strings or null — never numbers or booleans.
4. **Leading-zero preservation:** Any string matching `0\d+` in input appears as a string (not number) in output.
5. **Column count consistency:** Every object in the output array has the same set of keys (matching headers).

### Performance Benchmarks

- **1 GB synthetic CSV** (100 columns, 2M rows, mixed types): benchmark end-to-end conversion time in CI. Target: <20 seconds on GitHub Actions runner.
- **Benchmark suite** using `criterion` crate for micro-benchmarks of the inference engine (ns per cell).
- Run on every tagged release; results tracked over time.

### CI Matrix

| Platform | Rust Version | Tests | Clippy | Fmt |
|---|---|---|---|---|
| Linux x86_64 | stable | Yes | Yes | Yes |
| Linux x86_64 | MSRV (N-2) | Yes | No | No |
| Linux aarch64 | stable | Yes | No | No |
| macOS x86_64 | stable | Yes | No | No |
| macOS aarch64 | stable | Yes | No | No |
| Windows x86_64 | stable | Yes | No | No |

---

## Rollout Plan

### Phase 1: Development (Weeks 1-3)

**Week 1: Core pipeline**
- Set up Cargo project, CI workflow, and test infrastructure
- Implement `cli.rs` (clap derive), `input.rs` (file/stdin + BOM), `error.rs` (error enum + exit codes)
- Implement `inference.rs` with full test suite (30+ unit tests)
- Implement `headers.rs` (duplicate detection, auto-generation)

**Week 2: Output and integration**
- Implement `output.rs` (streaming JSON array serialization with key ordering)
- Implement `schema.rs` (JSON Schema 2020-12 generation)
- Write integration test suite (20+ tests)
- Handle ragged rows, whitespace trimming, `--strict` mode

**Week 3: Polish and cross-platform**
- Windows binary-mode stdout, BOM edge cases
- Shell completion generation and testing
- Property-based tests
- Performance benchmarking (1 GB CSV)
- README with installation, usage, inference rules, exit codes

### Phase 2: Pre-Release (Week 4)

- Build release binaries for all 5 targets via GitHub Actions
- Manual testing on physical Linux, macOS, and Windows machines
- Verify `cargo install csv2json` works from crates.io (dry-run or staging)
- Write CHANGELOG.md for v0.1.0
- Create GitHub repository with topics, description, and social preview

### Phase 3: Launch (Week 5)

- Publish v0.1.0 to crates.io
- Create GitHub Release with prebuilt binaries and checksums
- Submit to Homebrew (via tap initially, core formula later)
- Submit to package managers: AUR, Nixpkgs, winget, scoop, chocolatey
- Announce on: Hacker News, r/rust, r/dataengineering, r/commandline, Rust users forum, Twitter/Mastodon

### Phase 4: Post-Launch (Weeks 6-8)

- Monitor GitHub Issues for bug reports, especially type inference edge cases
- Triage feature requests against the v0.2 roadmap
- Fix any P0 correctness bugs within 48 hours
- Release v0.1.1 patch if needed

### Phase 5: v0.2 Development (Weeks 9-16)

- NDJSON output (`--format ndjson`)
- Column selection (`--columns`, `--exclude`)
- Custom null values (`--null-values`)
- Encoding detection (UTF-8/UTF-16/Latin-1)
- `--key-by` object output
- Streaming architecture optimization for files >RAM

---

## Success Metrics

### Correctness Metrics (Primary — these are non-negotiable)

| Metric | Target | Measurement |
|---|---|---|
| Type inference test coverage | 100% of documented rules have passing tests | CI test suite |
| Cross-platform parity | Byte-identical output for same input on all platforms | CI matrix comparison |
| Zero open correctness bugs | No reported data corruption or incorrect inference unresolved >48h | GitHub Issues triage |
| Property test failures | 0 failures across 10,000 generated inputs per test | CI `proptest` runs |

### Performance Metrics

| Metric | Target | Measurement |
|---|---|---|
| 1 GB CSV conversion | <20 seconds (CI), <10 seconds (local) | `criterion` benchmark in CI |
| Inference throughput | >10M cells/second | Micro-benchmark |
| Memory usage (1 GB file) | <50 MB RSS | Manual profiling |
| Binary size | <5 MB (all platforms) | CI artifact size check |

### Adoption Metrics (6-month targets)

| Metric | Target | Measurement |
|---|---|---|
| GitHub stars | 500+ | GitHub API |
| crates.io weekly downloads | 50+ | crates.io stats |
| GitHub Issues (total) | 20+ (indicates real usage) | GitHub Issues |
| Package manager availability | Available in Homebrew, AUR, Nixpkgs, winget/scoop | Package registry checks |
| Community mentions | 5+ blog posts, tutorials, or tool recommendations | Web search, social monitoring |

### Quality Metrics

| Metric | Target | Measurement |
|---|---|---|
| CI pass rate | >99% on all platforms | GitHub Actions history |
| Dependency audit | Zero known vulnerabilities | `cargo audit` in CI |
| Clippy warnings | Zero warnings on stable | CI clippy job |
| Test count | >50 total (unit + integration + property) | `cargo test` output |

---

## Open Questions

### OQ-1: `IndexMap` vs. `serde_json::Map` for key ordering
**Question:** Should row objects use `indexmap::IndexMap<String, Value>` (explicit insertion-order guarantee) or `serde_json::Map<String, Value>` (which preserves insertion order when the `preserve_order` feature is enabled on `serde_json`)?

**Trade-offs:** Using `serde_json`'s `preserve_order` feature avoids an additional dependency but couples key ordering to a Cargo feature flag. `IndexMap` is explicit and guaranteed. The `serde_json` `preserve_order` feature uses `IndexMap` internally anyway.

**Recommendation:** Enable `serde_json/preserve_order` feature. This avoids adding `indexmap` as a direct dependency while still getting insertion-order preservation. The feature flag is well-documented and widely used.

### OQ-2: Atomic file output implementation
**Question:** Should file output (`-o`) use atomic write (temp file + rename) or direct write?

**Trade-offs:** Atomic write prevents partial output on error but adds complexity (temp file naming, same-filesystem requirement for rename, cleanup on error). Direct write is simpler but can leave a corrupt partial file on error.

**Recommendation:** Atomic write. The implementation is ~20 lines of code and prevents a class of pipeline bugs where a downstream tool reads a partial JSON file from a previous failed run.

### OQ-3: `--strict` scope expansion
**Question:** Should `--strict` in future versions expand to cover other strictness behaviors (e.g., reject duplicate headers, reject non-UTF-8), or should separate flags be introduced?

**Trade-offs:** A single `--strict` flag is simpler for users but may enable unwanted strictness. Separate flags (`--strict-headers`, `--strict-encoding`) are more granular but clutter the CLI.

**Recommendation:** Keep `--strict` narrowly scoped to ragged rows in v0.1 (matching user requirement `strict_flag_scope: Ragged rows only`). If additional strictness dimensions are needed later, introduce them as separate flags and consider `--strict-all` as a convenience alias.

### OQ-4: Schema mode and `--no-inference` interaction
**Question:** What should `csv2json --schema --no-inference` produce? If inference is disabled, all values are strings — should the schema reflect `{"type": "string"}` for all columns, or should schema mode ignore `--no-inference` and always run inference?

**Trade-offs:** Honoring `--no-inference` in schema mode produces an uninformative schema (all strings). Ignoring it is surprising behavior.

**Recommendation:** Honor `--no-inference`: schema mode with `--no-inference` produces `{"type": "string"}` for all non-empty columns and `{"type": "null"}` for always-empty columns. This is consistent and predictable, even if less informative. Users who want inferred types in the schema simply omit `--no-inference`.

### OQ-5: crates.io name availability
**Question:** Is `csv2json` available as a crate name on crates.io?

**Action required:** Check crates.io before any public announcement. If taken, preferred alternatives in order: `csv2json-cli`, `csv-to-json`, `csv2json-rs`.

### OQ-6: Negative zero representation
**Question:** The spec says `-0` should normalize to `0`. However, `serde_json` serializes `-0.0_f64` as `-0.0`. Should csv2json explicitly check for negative zero and replace with `0`, or rely on the JSON consumer to normalize?

**Recommendation:** Explicitly normalize. Parse `-0` as `0i64` (integer zero) rather than `-0.0f64` (float negative zero). This matches the spec and avoids consumer-dependent behavior.

### OQ-7: Maximum line length / cell size
**Question:** Should csv2json impose any limit on individual cell size or line length, or should it accept arbitrarily large cells?

**Trade-offs:** No limit is simpler and handles all valid CSV. However, a malicious CSV with a single multi-gigabyte unquoted cell could exhaust memory since the `csv` crate buffers individual records.

**Recommendation:** No limit in v0.1. The `csv` crate handles this efficiently in practice, and real-world CSVs rarely have cells exceeding a few KB. Document that memory usage is proportional to the largest single row. Consider an optional `--max-field-size` flag in a future version if this becomes a reported issue.