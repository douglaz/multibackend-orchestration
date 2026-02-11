# rust-bitcoind: Product Ideation Document

## Core Concept

rust-bitcoind is a production-grade, full Bitcoin node implementation written in Rust, designed to be a drop-in alternative to Bitcoin Core for mainnet operation. The project takes a pure-Rust-from-the-start approach to consensus, leveraging the `rust-bitcoin` crate for primitive types (transactions, blocks, scripts, keys, hashes) and `redb` as its storage engine, while building its own consensus validation, mempool policy, P2P networking (on Tokio), and RPC interface.

The defining commitment: **pass every test in Bitcoin Core's Python functional test suite**. This is not a research prototype or an "alternative client with different rules" — it is a bit-for-bit consensus-compatible implementation that proves Rust can deliver a safe, performant, and maintainable Bitcoin full node. By skipping the `libbitcoinkernel` C++ FFI fallback entirely, the project accepts a harder initial path but avoids the long-term maintenance burden of binding to C++ and gains a fully auditable, single-language consensus stack.

The architecture targets modern Bitcoin network participants only (protocol version 70016+, post-SegWit, post-compact-blocks), shedding legacy protocol baggage that Bitcoin Core still carries. Consensus and storage layers are designed with `no_std` compatibility as a structural goal — traits and core types avoid `std`-only dependencies where practical — so that the consensus engine can eventually be extracted for embedded or WASM targets, though this is not enforced at compile time during initial development.

## Target Users

**Node operators seeking defense-in-diversity.** The Bitcoin network's resilience depends on not having a single implementation as a monoculture. Operators running rust-bitcoind alongside or instead of Bitcoin Core reduce the blast radius of implementation-specific bugs. These are technically sophisticated users running infrastructure on Linux servers, comfortable compiling from source.

**Bitcoin protocol developers and researchers.** Rust's type system, ownership model, and tooling (Cargo, `clippy`, `miri`, fuzzing via `cargo-fuzz`) make the codebase significantly more approachable for experimentation and auditing than C++. Researchers modeling consensus changes, soft fork proposals, or mempool policy experiments benefit from a codebase where invariants are encoded in types rather than comments.

**Mining pool operators and enterprises** requiring high-reliability node software where memory safety vulnerabilities (buffer overflows, use-after-free, data races) are categorically eliminated by the language. These users need mainnet-grade software with deterministic resource usage and clean crash-recovery semantics (provided by redb's ACID guarantees).

**Rust ecosystem contributors** in the Bitcoin space — developers already working with `rust-bitcoin`, `rust-miniscript`, `LDK`, or BDK — who want a full-node backend that speaks their language and composes naturally with their tooling without C++ FFI boundaries.

**Self-sovereign individuals** running personal nodes who value a smaller, easier-to-audit codebase with fewer historical layers of accretion than Bitcoin Core's 15+ year C++ codebase.

## Key Problems Solved

**Implementation monoculture risk.** Bitcoin Core is effectively the only production consensus implementation. A consensus-compatible Rust implementation provides genuine network-level redundancy. If a CVE affects Core's C++ code (memory corruption, integer overflow in consensus-critical paths), rust-bitcoind nodes remain unaffected and vice versa — the failure modes are uncorrelated.

**Memory safety in consensus-critical software.** Bitcoin Core has had multiple memory-safety-adjacent vulnerabilities over its history. Rust eliminates entire vulnerability classes (buffer overflows, use-after-free, data races) at compile time. For software that secures hundreds of billions of dollars in value, this is not a nice-to-have — it is a categorical improvement in the security envelope.

**Codebase comprehensibility and auditability.** Bitcoin Core's codebase has accumulated significant historical layering — the transition from Satoshi's original code through multiple architectural overhauls creates cognitive overhead. A greenfield Rust implementation, structured with modern idioms (strong typing, explicit error handling via `Result`, algebraic data types for protocol states), is substantially easier to audit end-to-end.

**Storage engine reliability.** Bitcoin Core uses LevelDB, a library that was never designed for consensus-critical financial data storage and has known corruption edge cases. redb provides RUST-native ACID transactions, crash-safe writes, and a single-file database format with zero external dependencies — a meaningfully better fit for a node's UTXO set and block index.

**Developer velocity for protocol work.** Cargo's dependency management, integrated testing, benchmarking (`criterion`), fuzzing, and documentation tooling dramatically reduce the friction of contributing to and experimenting with the codebase compared to Bitcoin Core's autotools/CMake + manual dependency management.

**Legacy protocol debt.** By targeting only protocol version 70016+, rust-bitcoind avoids implementing deprecated message types, pre-SegWit relay logic, and obsolete handshake flows, resulting in a cleaner and more secure networking layer.

## Proposed Features

### Consensus Engine (Pure Rust)
- Full script interpreter supporting all opcodes through Tapscript, built on `rust-bitcoin` script primitives
- UTXO set management with redb-backed commitment tracking
- Complete block validation: header chain, merkle roots, witness commitments, coinbase maturity, BIP30/34 height-in-coinbase, all soft fork activation logic (BIP9/BIP8 deployments)
- Signature verification using `secp256k1` Rust bindings (the same `libsecp256k1` C library Core uses, via the `secp256k1` crate — this is the one place where C FFI is acceptable, since the cryptographic primitive must be identical)
- `no_std`-compatible trait boundaries on consensus types so the engine can be extracted as a standalone crate

### P2P Networking (Tokio-based)
- Async peer management: connection pooling, peer discovery (DNS seeds, addr/addrv2 relay), eviction logic
- Compact block relay (BIP 152) for efficient block propagation
- Transaction relay with full policy enforcement (see below)
- Block-relay-only connections and anchor connections for eclipse attack resistance
- Tor/I2P/CJDNS transport support via configurable SOCKS5 proxy
- Peer banning and misbehavior scoring

### Mempool & Policy
- Exact replication of Bitcoin Core's mempool acceptance policy: fee-rate floor, ancestor/descendant limits, standardness rules, dust thresholds, OP_RETURN limits, RBF (BIP 125) signaling and rules, CPFP, package relay (as Core implements it)
- This is critical for mining compatibility — a mining operator must be able to swap in rust-bitcoind and produce identical block templates

### Storage Layer (redb)
- UTXO set stored in redb with ACID transaction semantics
- Block index and chain state metadata
- Crash recovery without external repair tools (no equivalent of Core's `-reindex` being required after unclean shutdown)
- Pruning support: configurable retention of recent blocks with full UTXO set preservation

### RPC Interface
- JSON-RPC 2.0 compatible server implementing Core's RPC methods (prioritizing: `getblockchaininfo`, `getblock`, `getblockheader`, `gettxout`, `sendrawtransaction`, `getmempoolinfo`, `getpeerinfo`, `getnetworkinfo`, `estimatesmartfee`, and the wallet-less subset)
- REST interface for block/tx/UTXO queries
- ZMQ notification support (`hashblock`, `hashtx`, `rawblock`, `rawtx`) for downstream services

### Testing & Validation
- Fork of Bitcoin Core's Python functional test suite, minimally adapted (shims for RPC differences, startup flags) to run against rust-bitcoind
- Native Rust unit and integration tests for all modules
- Continuous fuzzing of consensus-critical paths (script interpreter, deserialization, P2P message parsing)
- Deterministic chain replay: ability to sync from genesis to tip on mainnet, signet, and testnet with identical UTXO set hash at every block height compared to Core

### Operational Features
- Structured logging (via `tracing`) with configurable verbosity
- Prometheus-compatible metrics endpoint for monitoring
- Graceful shutdown with state persistence
- Configuration via TOML file and CLI flags

## Success Metrics

| Milestone | Criteria | Validation Method |
|---|---|---|
| **Consensus parity** | Pass 100% of Bitcoin Core's Python functional tests (consensus subset) | CI runs forked test suite against rust-bitcoind on every commit |
| **Full mainnet sync** | Sync from genesis block to chain tip on mainnet without error | Automated nightly sync run; compare UTXO set hash at tip against Core |
| **Policy parity** | Identical mempool acceptance decisions as Core for identical transaction streams | Replay recorded mainnet transaction streams through both implementations; diff acceptance/rejection |
| **P2P health** | Maintain stable connections to 8+ outbound and accept inbound peers on mainnet for 30+ continuous days | Long-running mainnet node with uptime and peer-count monitoring |
| **Performance baseline** | Initial block download (IBD) completes within 2x of Bitcoin Core's time on equivalent hardware | Benchmarked IBD on standardized hardware (e.g., 4-core, 16GB RAM, NVMe SSD) |
| **Safety** | Zero `unsafe` blocks outside of the `secp256k1` FFI boundary; clean `cargo clippy`, `miri` on consensus paths | CI enforcement; periodic `cargo-audit` runs |
| **External adoption signal** | At least 3 independent operators running rust-bitcoind on mainnet for 90+ days | Voluntary operator reports or identifiable nodes on network crawlers |
| **Test suite coverage** | >90% line coverage on consensus and policy modules | `cargo-tarpaulin` or `llvm-cov` in CI |

## Constraints & Assumptions

**Consensus is non-negotiable.** Any deviation from Bitcoin Core's consensus behavior — even in edge cases involving historically invalid blocks, overflow behavior, or SCRIPT_VERIFY flag combinations — is a critical bug. The Python functional tests are the minimum bar, not the ceiling. Supplementary testing (chain replay, differential fuzzing against Core) is assumed necessary.

**Self-funded, solo/small-team, no timeline pressure.** The project operates without external funding obligations or delivery deadlines. This is a strategic advantage: correctness is never traded for velocity. Features land when they are correct, tested, and reviewed — not when a milestone demands it. The funding model constrains scope (no full-time team of 10), but the absence of deadline pressure means the right architectural decisions can be made without compromise.

**secp256k1 C FFI is the one acceptable C dependency.** The `secp256k1` crate wraps the same C library Bitcoin Core uses. This is intentional — cryptographic primitives must produce identical results, and the C `libsecp256k1` is the most reviewed and tested implementation. A future pure-Rust secp256k1 implementation could replace it, but that is out of scope.

**Tracking latest Core master, not a pinned release.** The target is a moving one. As Bitcoin Core evolves (new policy rules, soft fork activations, P2P protocol changes), rust-bitcoind must track these changes. This means the forked test suite must be periodically rebased against Core's upstream tests. This is an ongoing maintenance cost, not a one-time effort.

**redb is a bet.** redb is newer and less battle-tested than LevelDB or RocksDB. The assumption is that its ACID guarantees, pure-Rust implementation, and crash-safety properties outweigh its relative immaturity. If redb proves inadequate under mainnet UTXO set scale (~5GB+), the storage layer is behind a trait boundary and can be swapped.

**No wallet.** rust-bitcoind is a headless, wallet-less node. Wallet functionality is explicitly out of scope — users should pair it with external wallet software (BDK, Sparrow, etc.) that connects via RPC. This dramatically reduces attack surface and scope.

**No GUI.** The interface is CLI, RPC, and metrics endpoints. No Qt, no Electron, no web dashboard. Monitoring is handled via standard infrastructure tooling (Prometheus/Grafana, log aggregation).

**`no_std` consensus is a design aspiration, not a gate.** Core types and traits avoid unnecessary `std` dependencies, and the consensus crate's public API is designed to be `no_std`-feasible. But enforcement (actually compiling with `#![no_std]`) is deferred until the consensus engine is stable. Premature `no_std` enforcement would slow iteration on the primary goal (passing Core's tests on mainnet).

**Modern network only.** Pre-SegWit nodes, nodes running protocol versions below 70016, and deprecated P2P message types are not supported. This simplifies the networking layer but means rust-bitcoind cannot serve as a bridge to very old network participants. This is an acceptable tradeoff — the overwhelming majority of reachable nodes run modern protocol versions.

**Exact policy replication is hard.** Bitcoin Core's mempool policy is not formally specified — it is defined by its C++ implementation. Replicating it means reading Core's code line-by-line and matching behavior, including edge cases around fee calculation rounding, ancestor package tracking, and RBF rules. This is among the most labor-intensive parts of the project and the most likely source of subtle divergence.