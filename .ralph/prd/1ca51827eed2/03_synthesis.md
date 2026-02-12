I'll synthesize the ideation and research into a coherent product strategy document.

## Product Vision

csv2json is the **ripgrep of CSV-to-JSON conversion**: a single-purpose, zero-configuration CLI tool that converts CSV files to JSON with strict, predictable type inference. Written in Rust, it ships as a single static binary under 5 MB with no runtime dependencies, no telemetry, and no hidden behavior.

The core thesis is that **correctness is the killer feature**. Every existing tool in this space makes at least one of these mistakes: silently converting zip codes (`07042`) to numbers, requiring a language runtime, bundling CSV-to-JSON as an afterthought inside a Swiss-army-knife tool, or offering only all-or-nothing type inference. csv2json occupies the gap: a focused tool that gets the defaults right so pipelines don't silently corrupt data.

The market timing is favorable. xsv (10.8K stars) was archived in April 2025, leaving a visible gap in the Rust CSV tooling ecosystem. The data engineering market exceeds $120B with 87% of companies reporting skills gaps — meaning more generalist developers are building data pipelines and need tools that are safe by default. The Rust CLI renaissance (ripgrep, bat, fd, eza) has trained users to expect fast, correct, cross-platform binaries installable via `cargo install`.

csv2json targets the permanent need to bridge CSV (the lingua franca of tabular data) and JSON (the lingua franca of APIs and configuration). It does one thing, does it correctly, and composes with everything else.

## User Stories

### Data Pipeline Operator
> As a data engineer building ETL pipelines, I want to convert CSV exports from databases and vendor APIs into JSON so that downstream services can consume them without writing custom parsers. I need the tool to **preserve leading zeros** in fields like zip codes and part numbers, because silent numeric conversion has caused production incidents before. I need deterministic behavior so the same input always produces the same output, and I need nonzero exit codes on failure so my pipeline orchestrator (Airflow, Prefect, cron) can detect problems.

### Unix Pipeline Composer
> As a CLI power user, I want to pipe CSV data through csv2json and into jq for filtering and transformation: `cat users.csv | csv2json | jq '.[] | select(.active == true)'`. I need compact output by default (for piping) with a `--pretty` flag for human reading. I need stdin support without special flags — if no file argument is given, read from stdin. I expect the tool to behave like a well-mannered Unix citizen: data to stdout, diagnostics to stderr, meaningful exit codes.

### Schema-First API Developer
> As a backend developer receiving CSV files from external partners, I want to generate a JSON Schema from a sample CSV so I can set up validation in my API ingestion layer before writing any transformation code. I want `csv2json --schema partner_data.csv` to emit a Draft 2020-12 JSON Schema that describes the inferred column types, so I can review the schema, adjust it, and use it with a JSON Schema validator in my service.

### DevOps Engineer
> As a DevOps engineer, I maintain inventory and configuration data in CSV spreadsheets that need to be converted to JSON for Terraform and Ansible. I need the tool to handle CSVs with duplicate column headers (common after spreadsheet merges) without crashing — warn me, but don't block the pipeline. I need the tool to work identically on my macOS laptop, the Linux CI runners, and the Windows jump box.

### Cautious Data Analyst
> As a data analyst exploring an unfamiliar CSV dataset, I want to quickly convert it to JSON to load into a notebook or pass to a visualization tool. I don't want to worry about type inference surprises — I want booleans to only be `true`/`false` (not `yes`/`1`/`TRUE`), nulls to be explicit, and numbers to not lose precision. If the CSV has no headers, I want `--no-header` to auto-generate sensible keys (`field_0`, `field_1`, ...) rather than treating the first data row as headers.

### CI/CD Integration Engineer
> As a developer integrating csv2json into CI pipelines, I want shell completions for bash, zsh, fish, and PowerShell so my team can discover flags without reading docs. I want distinct exit codes per error category (I/O error vs. parse error vs. argument error) so I can write conditional logic in shell scripts. I want the `--delimiter` flag to handle tab-separated files from database exports without needing a separate tool.

## Feature Prioritization

Features are organized into three tiers using a **must-have / should-have / nice-to-have** framework, informed by the gap analysis against existing tools and the risk assessment.

### Tier 1 — Must-Have (MVP v0.1)

These features define the minimum viable product. Without any one of them, the tool either fails to differentiate from alternatives or fails to function in real pipelines.

| Feature | Rationale | Effort |
|---|---|---|
| File input + stdin input | Table-stakes for any Unix CLI tool | Low |
| File output (`-o`) + stdout output | Required for pipeline composition | Low |
| Array-of-objects JSON output | The expected default format; competitors all support this | Low |
| Per-cell type inference engine | The core differentiator; strict booleans, null inference, leading-zero preservation, large-number-as-string | Medium |
| `--no-inference` flag | Escape hatch to emit everything as strings; matches user requirement for disable_type_inference | Low |
| `--delimiter` flag | Custom delimiters (tab, pipe, semicolon) are essential for real-world CSVs | Low |
| `--pretty` flag | Compact default for piping, pretty for humans | Low |
| `--no-header` with auto-generated keys | Headerless CSVs are common; `field_0`, `field_1` naming is safe | Low |
| `--schema` mode | JSON Schema 2020-12 output; a differentiator no focused tool offers | Medium |
| Duplicate header handling | Warn to stderr, last-value-wins; prevents pipeline breakage | Low |
| BOM detection and stripping | Windows-origin CSVs frequently have BOMs; the csv crate doesn't handle this | Low |
| Ragged CSV handling | Null-pad short rows by default, `--strict` to error; matches user requirement | Low |
| Whitespace trimming | Trim by default; matches user requirement | Low |
| Distinct exit codes | Required for script integration; user requirement specifies per-category codes | Low |
| Fail-fast error mode (`--on-error fail`) | Default behavior; clear error to stderr on first problem | Low |
| Cross-platform CI (Linux, macOS, Windows) | Day-one requirement from user; well-trodden path in Rust ecosystem | Medium |
| Shell completions (bash, zsh, fish, PowerShell) | User requirement; trivially generated by clap's derive API | Low |

### Tier 2 — Should-Have (v0.2)

These features address the next layer of real-world usage and are informed by competitive gaps.

| Feature | Rationale | Effort |
|---|---|---|
| NDJSON / JSON Lines output (`--format ndjson`) | Required for streaming consumers (Elasticsearch, BigQuery, etc.); qsv only does JSONL, not arrays — csv2json should do both | Low |
| Streaming architecture | Process files larger than RAM; the csv crate supports this natively | Medium |
| Column selection (`--columns`, `--exclude`) | Common need; avoids piping through jq for simple column filtering | Medium |
| Custom null values (`--null-values 'NA,N/A,-'`) | Real CSVs use many null representations beyond empty/null | Low |
| Encoding detection (UTF-8/UTF-16/Latin-1) | User requirement deferred to post-v0.1; needed for international datasets | Medium |
| `--key-by <column>` | Output as object-of-objects keyed by a column value; common request | Low |

### Tier 3 — Nice-to-Have (v0.3+)

These features extend the tool's reach but risk scope creep if prioritized too early.

| Feature | Rationale | Effort |
|---|---|---|
| Type override map (`--types 'zip:string,count:int'`) | Per-column type forcing; escape valve for when inference isn't enough | Medium |
| Row filtering (`--where 'age > 30'`) | Useful but overlaps with jq/mlr; risk of building a query language | High |
| Batch conversion (`csv2json *.csv --outdir ./json/`) | Parallel multi-file conversion; user requirement says single-file-only for v0.1 | Medium |
| Configurable error mode (`--on-error skip`) | Skip bad rows instead of failing; useful for dirty data | Low |
| `--explain` dry-run mode | Show inferred types without producing output; useful for debugging | Low |
| Header override (`--header 'a,b,c'`) | Rename headers without editing the file | Low |

### What We Will NOT Build

These are explicit anti-goals to guard against feature creep:

- **No Excel/Parquet/XML/YAML input.** csv2json converts CSV. Other formats have their own tools.
- **No embedded scripting or plugin system.** Complex transforms belong in jq, mlr, or application code.
- **No configuration files.** All behavior is controlled via CLI flags. No dotfiles, no env-var magic.
- **No telemetry, update checks, or network access.** The binary is fully offline.
- **No date/time inference.** Dates remain strings. Date formats are ambiguous (`01/02/03`) and locale-dependent; inference would violate the correctness-first principle.

## Architecture Overview

### System Architecture

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
├── Cargo.toml
├── src/
│   ├── main.rs              # Entry point, error handling, exit codes
│   ├── cli.rs               # clap derive structs, argument validation
│   ├── input.rs             # File/stdin reader, BOM detection
│   ├── inference.rs         # Per-cell type inference engine
│   ├── headers.rs           # Header processing, duplicate detection
│   ├── output.rs            # JSON serialization (data mode)
│   ├── schema.rs            # JSON Schema generation (schema mode)
│   └── error.rs             # Error types, exit code mapping
├── tests/
│   ├── inference_tests.rs   # Unit tests for type inference edge cases
│   ├── integration_tests.rs # End-to-end CLI tests
│   ├── schema_tests.rs      # JSON Schema output validation
│   └── fixtures/            # Test CSV files
├── .github/
│   └── workflows/
│       ├── ci.yml           # Test on Linux/macOS/Windows
│       └── release.yml      # Build + publish binaries
└── README.md
```

### Key Design Decisions

**1. Per-cell inference, not per-column.** Each cell is independently typed. This is simpler to implement, simpler to reason about, and produces valid JSON (arrays permit mixed types). The alternative — scanning all values in a column to determine a single type — requires either a two-pass architecture or buffering the entire file, and introduces ambiguity when columns contain mixed data.

**2. Streaming row-by-row processing.** The CSV crate's iterator interface enables processing one row at a time. For data mode, the tool writes `[` at the start, serializes each row object separated by commas, and writes `]` at the end. Memory usage is O(row_size), not O(file_size). Schema mode requires a full pass to collect type sets per column, but this only stores type metadata, not data.

**3. Type inference as a pure function.** `fn infer(value: &str) -> serde_json::Value` is a stateless, deterministic function with no side effects. This makes it trivially testable with table-driven tests and property-based tests. The decision tree:

```
infer("") → Value::Null
infer("null") → Value::Null    (case-insensitive)
infer("true") → Value::Bool(true)   (case-sensitive)
infer("false") → Value::Bool(false)  (case-sensitive)
infer("42") → Value::Number(42)
infer("3.14") → Value::Number(3.14)
infer("1.5e10") → Value::Number(1.5e10)
infer("007") → Value::String("007")   (leading zero)
infer("9999999999999999") → Value::String("9999999999999999")  (>53-bit)
infer("NaN") → Value::String("NaN")
infer("hello") → Value::String("hello")
```

**4. clap derive API for CLI.** Automatic `--help`, `--version`, error messages, and shell completion generation. The derive API produces a `Config` struct that the rest of the application consumes — clean separation between CLI parsing and business logic.

**5. Structured error types with mapped exit codes.** A Rust enum `Error { Io(...), Parse(...), Argument(...), Schema(...) }` maps to distinct exit codes. All errors print a human-readable message to stderr. The `main` function catches `Result<(), Error>` and calls `std::process::exit()` with the appropriate code.

### Dependency Graph (MVP)

| Crate | Purpose | Downloads | Maintenance |
|---|---|---|---|
| `csv` 1.x | CSV parsing | 129M+ | Active, BurntSushi |
| `serde` 1.x | Serialization framework | 400M+ | Active |
| `serde_json` 1.x | JSON serialization | 400M+ | Active |
| `clap` 4.x (derive) | CLI parsing + completions | 200M+ | Active |
| `unicode-bom` | BOM detection | 1M+ | Stable |

Total direct dependencies: **5**. Estimated transitive: **30-40**. All dual-licensed MIT/Apache-2.0.

## MVP Scope

### What Ships in v0.1

The MVP is a single binary that handles the complete happy path and the most important edge cases. Estimated at **~660 lines of Rust** (excluding tests) with a **~700-line test suite**.

#### Input/Output
- Read from a file argument (`csv2json data.csv`) or stdin (`cat data.csv | csv2json`)
- Write to stdout (default) or file (`-o output.json` / `--output output.json`)
- UTF-8 only; non-UTF-8 input produces an error with a clear message
- BOM detection and stripping for UTF-8 BOM (`0xEF 0xBB 0xBF`)

#### CSV Parsing
- RFC 4180 baseline via the `csv` crate
- Configurable delimiter: `--delimiter ','` (default), `--delimiter '\t'`, `--delimiter '|'`, etc.
- Header row by default; `--no-header` generates `field_0`, `field_1`, ...
- Duplicate headers: warn to stderr, last value wins
- Ragged rows: null-pad short rows by default; `--strict` errors on ragged input
- Whitespace trimming on all cell values

#### Type Inference
- Per-cell inference enabled by default
- `--no-inference` flag to emit all values as strings
- Inference rules:
  - Empty string `""` → `null`
  - `null`, `NULL`, `Null` (case-insensitive) → `null`
  - `true` → `true`, `false` → `false` (case-sensitive only)
  - Numeric strings → JSON number, **except**: leading zeros → string, >2^53 integers → string, `NaN`/`Infinity`/`-Infinity` → string
  - Scientific notation (`1.5e10`, `3E-4`) → JSON number
  - Everything else → string

#### Output Formats
- **Data mode** (default): JSON array of objects, compact
- **Pretty mode** (`--pretty`): indented JSON array of objects
- **Schema mode** (`--schema`): JSON Schema Draft 2020-12 document describing inferred column types

#### Error Handling
- Fail on first error by default (`--on-error fail`)
- Human-readable error messages to stderr with context (line number, column, value)
- Exit codes: `0` success, `1` I/O error, `2` CSV parse error, `3` invalid arguments

#### Distribution
- `cargo install csv2json`
- GitHub Releases with prebuilt binaries for:
  - Linux x86_64 (musl, static)
  - Linux aarch64 (musl, static)
  - macOS x86_64 + aarch64 (universal binary)
  - Windows x86_64 (static CRT)
- Shell completions for bash, zsh, fish, PowerShell (generated at build time via clap)
- MIT OR Apache-2.0 dual license

#### Test Coverage (MVP)

| Category | Test Count (est.) | Coverage Target |
|---|---|---|
| Type inference edge cases | 30+ | Every rule in the decision tree, boundary values |
| BOM handling | 3 | UTF-8 BOM present, absent, non-UTF-8 BOM |
| Duplicate headers | 3 | None, some, all duplicates |
| Ragged rows | 4 | Short rows (pad vs. strict), long rows, empty rows |
| Delimiter variants | 5 | Comma, tab, pipe, semicolon, custom |
| No-header mode | 3 | With/without --no-header, header override |
| Schema output | 5 | Single-type columns, mixed-type, all-null, empty CSV |
| Empty CSV | 2 | Headers only, no data → `[]` |
| CLI flag combinations | 10+ | --pretty + --schema, --no-inference + --schema, etc. |
| Error cases | 5 | Missing file, invalid UTF-8, permission denied, bad delimiter arg |
| End-to-end integration | 10+ | Full pipeline: file → csv2json → expected JSON output |

### What Does NOT Ship in v0.1

- NDJSON/JSON Lines output (v0.2)
- Column selection/exclusion (v0.2)
- Row filtering (v0.3+)
- Custom null values (v0.2)
- Non-UTF-8 encoding support (v0.2)
- Batch/multi-file conversion (v0.3+)
- Type override map (v0.3+)
- `--on-error skip` (v0.3+)
- `--explain` mode (v0.3+)
- `--key-by` object output (v0.2)

### MVP Exit Criteria

The MVP is ready to ship when:

1. All tests pass on Linux x86_64, Linux aarch64, macOS (Intel + Apple Silicon), and Windows x86_64 in CI.
2. The type inference test suite covers every rule in the decision tree with boundary cases (leading zeros, precision limits, null variants, boolean casing).
3. A 1 GB synthetic CSV converts in under 20 seconds on a GitHub Actions runner (relaxed from the aspirational 10s target to account for CI variability; local performance should be faster).
4. The release binary is under 5 MB on all platforms.
5. `csv2json --help`, `csv2json --version`, and shell completions work correctly.
6. README documents all flags, inference rules, exit codes, and installation methods.

## Open Questions

### 1. Mixed-type columns in schema mode
When a column contains both numbers and strings (e.g., a "code" column with `42`, `AB12`, `007`), the schema output must represent this. Should the schema emit `{"type": ["number", "string"]}` (union type), or should it collapse to `{"type": "string"}` (most permissive)? The union type is more informative but may confuse downstream validators that expect uniform types. **Recommendation:** union types, since JSON Schema 2020-12 supports them natively and they accurately describe the data.

### 2. Null in schema mode representation
When a column contains null values alongside other types, should the schema use `{"type": ["string", "null"]}` or use the `nullable` pattern? JSON Schema 2020-12 does not have a `nullable` keyword (that's OpenAPI). The correct approach is the type array, but this needs confirmation against the user's downstream schema consumers.

### 3. `--on-error fail` as a flag vs. default behavior
The user specified `on_error_default: fail`. Should `--on-error` be exposed as a flag in v0.1 (with only `fail` as the accepted value, as forward-compatible API surface), or should it be omitted entirely from v0.1 and introduced in v0.3 when `skip` is implemented? Exposing it early establishes the CLI contract; omitting it keeps the MVP minimal. **Recommendation:** omit from v0.1, add in v0.3 with both `fail` and `skip`.

### 4. Key ordering in JSON output
The user specified `key_ordering: Yes`. JSON objects are technically unordered, but many consumers depend on stable key order. Should keys appear in CSV column order (matching the header row), or alphabetically? Column order is more intuitive and preserves the CSV's structure. **Recommendation:** column order, matching the CSV header sequence — this requires using an ordered map (`IndexMap` or `serde_json::Map` which preserves insertion order) rather than `HashMap`.

### 5. Package manager strategy and naming
`csv2json` is a common tool name. Is the name available on crates.io, Homebrew, npm (for npx distribution), and system package managers? If `csv2json` is taken on crates.io, alternatives include `csv2json-cli`, `csv-to-json`, or a distinctive name. Early verification is needed before any public announcement. **Recommendation:** check crates.io availability immediately; if taken, prefer `csv2json-rs` to signal the Rust implementation.

### 6. Embedded JSON in CSV cells
The user specified `embedded_json_handling: Keep as string (safe default)`. This is correct for v0.1, but should a future `--parse-embedded-json` flag be part of the public roadmap? Parsing embedded JSON would make csv2json useful for log processing (where CSV cells contain JSON payloads), but it significantly complicates the type inference engine and error handling. **Recommendation:** keep as string in all versions; this is out of scope for a CSV-to-JSON converter and belongs in downstream tools like jq.

### 7. Windows stdout binary mode
Rust's `println!` on Windows translates `\n` to `\r\n`, which would corrupt JSON output when piped to a file. The implementation must use `BufWriter<Stdout>` with explicit writes (not `println!`) or set stdout to binary mode. This is a confirmed technical requirement, not an open question per se, but it needs explicit handling in the implementation plan to avoid a subtle cross-platform bug.