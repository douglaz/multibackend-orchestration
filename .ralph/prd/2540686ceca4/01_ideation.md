# rust-bitcoind: Product Ideation Document

## Core Concept

rust-bitcoind is a production-grade, full-node implementation of the Bitcoin protocol written entirely in Rust. It aims for complete consensus compatibility with Bitcoin Core, verified by passing Bitcoin Core's entire Python functional test suite. The project takes a pure-Rust-from-the-start approach to consensus logic — skipping the libbitcoinkernel FFI path — and builds on the existing rust-bitcoin ecosystem for primitive types (transactions, blocks, scripts, hashes) while implementing all validation, peer-to-peer networking, mempool policy, and storage from scratch.

The node targets a modern protocol surface (v70016+), uses **redb** as its pure-Rust ACID storage engine, runs its async networking layer on **tokio**, and is structured as a Cargo workspace of composable crates. It replicates Bitcoin Core's policy rules exactly (not just consensus), ensuring drop-in behavioral compatibility for miners, exchanges, and infrastructure operators who depend on specific relay and mempool semantics.

The architecture consciously defers non-essential subsystems — ZMQ notification, AssumeUTXO, and security disclosure processes — until post-1.0 or mainnet readiness, focusing initial effort on correctness, test parity, and a clean Rust-native codebase that can eventually run consensus-critical code in `no_std` environments.

## Target Users

**Primary:**

- **Node operators and infrastructure providers** who want a memory-safe, performant alternative to Bitcoin Core for running mainnet full nodes — particularly those already operating in Rust-heavy environments (exchanges, Lightning implementations like LDK, mining pool backends).
- **Bitcoin protocol developers and researchers** seeking a readable, well-structured reference implementation that makes consensus rules explicit in a strongly-typed language, lowering the barrier to auditing and contributing to consensus code.
- **Rust-ecosystem Bitcoin projects** (LDK, BDK, electrs, Fedimint) that currently shell out to or wrap Bitcoin Core and would benefit from a native Rust node they can link against, embed, or co-deploy without FFI boundaries.

**Secondary:**

- **Embedded and constrained-environment operators** (future): the `no_std`-friendly consensus design opens a path to running validation logic on hardware wallets, secure enclaves, or WASM runtimes.
- **Bitcoin Core test infrastructure maintainers** who gain a second independent implementation to validate against, strengthening the network's defense against consensus bugs.
- **Solo developers and hobbyists** who prefer `cargo build` over autotools/cmake and want to run a full node from source with minimal system dependencies.

## Key Problems Solved

1. **Memory safety in consensus-critical code.** Bitcoin Core's C++ codebase has historically been susceptible to memory corruption classes (buffer overflows, use-after-free). Rust's ownership model eliminates these at compile time, reducing the attack surface of the most security-sensitive software in the cryptocurrency ecosystem.

2. **Implementation monoculture risk.** The Bitcoin network currently relies on a single consensus implementation. A second production-quality implementation — validated against the same test suite — provides defense in depth. If a platform-specific or compiler-specific bug affects Core, rust-bitcoind operators remain unaffected (and vice versa), without risking chain splits because consensus rules are identical.

3. **Dependency complexity and build friction.** Bitcoin Core requires autotools or cmake, Berkeley DB or LevelDB, Boost, libevent, and platform-specific toolchains. rust-bitcoind compiles with `cargo build` on Linux x86_64 (primary) and macOS/Linux aarch64 (secondary), with all dependencies — including the storage engine (redb) and crypto (rust-secp256k1 or k256 under evaluation) — pulled from crates.io. No system libraries required.

4. **Codebase legibility and contributor onboarding.** Bitcoin Core's consensus logic is interleaved with decades of incremental changes, implicit invariants, and C++ template complexity. A clean-room Rust implementation with explicit types, workspace-separated crates, and modern tooling (clippy, miri, property testing) makes consensus rules more auditable and the project more approachable for new contributors.

5. **Ecosystem integration friction.** Rust-native Bitcoin projects currently need IPC, RPC, or FFI to interact with Core. rust-bitcoind's crate workspace allows downstream projects to depend on individual crates (e.g., `rust-bitcoind-consensus`, `rust-bitcoind-mempool`) as libraries, enabling tighter integration without process boundaries.

6. **Policy divergence ambiguity.** Alternative node implementations often differ from Core in subtle mempool and relay policy, causing operational surprises (transactions not relaying, fee estimation mismatches). rust-bitcoind replicates Core's policy behavior exactly, making it a true drop-in replacement.

## Proposed Features

### Consensus Engine (Pure Rust)
- Full script interpreter supporting all historical consensus rules, soft forks, and flag transitions (P2SH, SegWit, Taproot, OP_CSV, OP_CLTV) — built on rust-bitcoin primitives
- Exact replication of historical consensus bugs (e.g., `FindAndDelete`, `OP_CHECKMULTISIG` off-by-one, BIP-30 duplicate txid handling) to ensure block-for-block IBD compatibility
- AssumeValid support for fast initial block download, skipping script validation below a hardcoded checkpoint
- Block and transaction validation pipeline with parallel script verification using rayon or tokio tasks
- `no_std`-compatible design for the consensus crate (not enforced initially, but no `std`-only dependencies in the critical path)

### Storage Layer
- **redb** as the single storage backend: pure Rust, ACID-compliant, single-file database with crash recovery
- UTXO set, block index, and chain state stored in typed redb tables
- `txindex` and `blockfilterindex` (BIP 157/158) as opt-in index modules
- Pruning support for disk-constrained nodes
- Atomic chain-tip updates with redb's transaction model (no custom write-ahead log needed)

### Peer-to-Peer Networking
- Modern protocol only: v70016+ (post-SegWit, compact blocks, `wtxid` relay)
- Tokio-based async connection management with configurable inbound/outbound peer limits
- Compact block relay (BIP 152) for low-latency block propagation
- `addr` / `addrv2` (BIP 155) peer discovery with Tor/I2P address support
- Transaction relay with fee-rate-based filtering, `wtxid`-based inventory
- Headers-first synchronization with parallel block download

### Mempool & Policy
- Exact replication of Bitcoin Core's mempool acceptance rules (standardness, dust threshold, signature operation limits, witness size limits)
- Replace-by-Fee (BIP 125) with Core-compatible conflict resolution semantics
- CPFP-aware descendant/ancestor package tracking
- Fee estimation algorithm compatible with Core's `estimatesmartfee`
- Memory-limited eviction using Core's mining-score-based approach

### RPC Interface
- JSON-RPC 1.0/2.0 server with exact field-for-field compatibility on supported endpoints
- Priority endpoints: `getblockchaininfo`, `getblock`, `getblockhash`, `getrawtransaction`, `sendrawtransaction`, `getmempoolinfo`, `getpeerinfo`, `estimatesmartfee`, `gettxout`
- Authentication via `.cookie` file and `rpcuser`/`rpcpassword` (matching Core's auth model)
- Batch request support

### Testing & Validation
- **Bitcoin Core Python test suite**: forked and minimally adapted to run against rust-bitcoind's RPC (same interface, same assertions)
- Regtest mode for local development, integration testing, and test suite execution
- Unit tests, integration tests, and property-based tests (proptest/quickcheck) for consensus-critical code
- Differential fuzzing against Bitcoin Core on script evaluation and transaction validation
- CI pipeline: fast checks (clippy, fmt, unit tests) on GitHub Actions; full functional test suite on self-hosted infrastructure

### CLI & Configuration
- `rust-bitcoind` binary with Core-compatible CLI flags (`-datadir`, `-regtest`, `-testnet`, `-port`, `-rpcport`, `-txindex`, etc.)
- TOML or Core-compatible `bitcoin.conf` configuration file support
- Signal handling (SIGTERM/SIGINT graceful shutdown)
- Logging via `tracing` with structured output and configurable verbosity

### Workspace Crate Architecture
| Crate | Purpose |
|---|---|
| `rust-bitcoind-consensus` | Script validation, block/tx validation rules, soft fork activation |
| `rust-bitcoind-storage` | redb-backed UTXO set, block store, indexes |
| `rust-bitcoind-mempool` | Policy enforcement, fee estimation, package relay |
| `rust-bitcoind-net` | P2P protocol, peer management, message serialization |
| `rust-bitcoind-rpc` | JSON-RPC server, authentication, request routing |
| `rust-bitcoind-node` | Orchestration crate tying all components together |
| `rust-bitcoind` | CLI binary |

### Deferred (Post-1.0)
- AssumeUTXO snapshot loading
- ZMQ notification interface
- Wallet functionality
- GUI
- Security disclosure process (deferred until mainnet-ready)
- Additional indexes beyond txindex and blockfilterindex

## Success Metrics

### Correctness (Gate to any release)
- **100% pass rate on Bitcoin Core's Python functional test suite** (forked, minimally adapted) on regtest — this is the single most important metric
- Zero consensus divergence on full mainnet IBD from genesis to tip, verified by comparing UTXO set hash at every 10,000th block against a Core reference node
- Zero script evaluation differences when differential-fuzzed against Core for 10M+ random and historical transactions

### Initial Block Download (IBD) Performance
- Complete mainnet IBD (genesis to tip) in under 12 hours on reference hardware (8-core x86_64, NVMe, 32 GB RAM) — competitive with Core
- Peak memory usage during IBD under 8 GB (UTXO set + mempool + block processing buffers)
- Storage footprint within 20% of Core for equivalent configuration (unpruned, txindex enabled)

### Steady-State Operation
- Block validation latency (tip block, fully cached UTXO set): < 500ms p99
- Mempool acceptance latency for standard transactions: < 10ms p99
- Stable operation for 30+ consecutive days on mainnet without crash, memory leak, or consensus stall
- Maintains 8+ outbound and 50+ inbound peer connections continuously

### Build & Developer Experience
- `cargo build --release` from clean checkout in under 5 minutes on reference hardware
- Zero `unsafe` blocks in consensus-critical crates (or fully audited and documented if unavoidable)
- CI green on every merge to main: clippy (zero warnings), rustfmt, all unit/integration tests, miri on consensus crate

### Ecosystem Adoption (Longer-term)
- At least one downstream Rust-Bitcoin project (LDK, BDK, electrs) integrating a rust-bitcoind crate as a library dependency
- At least 5 independent mainnet nodes running rust-bitcoind for 90+ consecutive days
- Contributions from developers outside the founding team

## Constraints & Assumptions

### Hard Constraints
- **MIT license** — maximally permissive, matching rust-bitcoin ecosystem norms
- **Pure Rust consensus from day one** — no FFI to libbitcoinkernel or Bitcoin Core C++; all consensus logic implemented natively in Rust
- **Exact policy replication** — not just consensus-valid but relay-compatible; transactions accepted/rejected by rust-bitcoind must match Core's behavior for the same mempool state
- **Linux x86_64 as primary platform** — all CI, benchmarks, and performance targets reference this platform; macOS and Linux aarch64 are secondary (must compile and pass tests, not performance-optimized)
- **Self-funded until PoC** — scope decisions must respect solo/small-team capacity; no dependency on grants or external funding for initial milestones
- **SemVer versioning, independent of Core** — rust-bitcoind 1.0 does not correspond to Bitcoin Core 28.0; version numbers track this project's maturity

### Technical Assumptions
- **rust-bitcoin primitives are correct and sufficient** — transaction, block, script, and hash types from rust-bitcoin (latest stable) are used as the foundation; if gaps are found, upstream contributions are preferred over local forks
- **redb is production-ready for this workload** — ACID guarantees, crash recovery, and performance are assumed sufficient for UTXO set management at mainnet scale (~170M UTXOs); if redb proves inadequate, migration to another pure-Rust engine is possible due to storage abstraction
- **secp256k1 binding or k256 provides equivalent security** — rust-secp256k1 (C FFI) is the baseline; k256 (pure Rust) will be evaluated for correctness and performance, but the C binding remains the fallback if k256 cannot match Core's edge-case behavior
- **tokio is stable for long-running network services** — the async runtime is assumed reliable for a process that runs for months without restart
- **Bitcoin Core's Python tests are the canonical correctness oracle** — if the tests pass, consensus compatibility is assumed; divergences not covered by tests are bugs in the test suite, not acceptable deviations
- **Historical consensus bugs must be replicated exactly** — this is not a "clean" reimplementation that fixes old mistakes; every quirk that affects block validation on the historical chain must be reproduced

### Scope Boundaries
- **No wallet functionality** — rust-bitcoind is a headless full node; wallet features are out of scope entirely (BDK exists for this)
- **No GUI** — CLI and RPC only
- **No ZMQ** — notification via ZMQ is deferred indefinitely; WebSocket or similar may be considered later
- **No AssumeUTXO until post-1.0** — snapshot-based UTXO loading is complex and not required for initial mainnet viability
- **Modern P2P only** — no support for protocol versions below 70016; pre-SegWit peers cannot connect
- **No timeline commitments** — the project ships when it's correct, not when a deadline arrives

### Risk Acknowledgments
- **Consensus divergence is an existential risk** — a single validation difference on mainnet could cause a chain split; extensive differential testing and conservative deployment (long parallel-running period before any operator relies on it) are required
- **redb at scale is unproven for this specific workload** — Bitcoin's UTXO set is one of the largest key-value workloads in any cryptocurrency; redb has not been tested at this scale in production
- **Solo/small-team sustainability** — a project of this scope requires years of focused work; burnout and funding exhaustion are primary non-technical risks
- **Upstream rust-bitcoin changes may break assumptions** — tracking latest stable means absorbing breaking changes; a version pinning and migration strategy is needed
- **k256 may not replicate all secp256k1 edge cases** — subtle differences in point validation, error handling, or nonce generation could cause consensus failures; exhaustive comparison testing is mandatory before any mainnet use