# Missing Information Report

## Stage
`Ideation`

## Pipeline Stopped
Maximum question rounds reached (3/3).

## Questions
- **rpc_json_equivalence_definition** (`Prd`): When you say 'byte-identical JSON responses', do you mean literal byte-for-byte identical HTTP response bodies (including JSON key ordering and whitespace), or semantically equivalent JSON with deterministic key ordering? Core uses UniValue with insertion-order keys — will you replicate that ordering?
  Type: Choice [Byte-identical (replicate Core's exact serialization including key order and whitespace), Semantic equivalence with deterministic key ordering (same keys/values, may differ in whitespace), Semantic equivalence only (same keys/values, ordering may differ)]
  Suggested default: Semantic equivalence with deterministic key ordering (same keys/values, may differ in whitespace)
- **default_port_resolution** (`Prd`): Your answers contain a conflict: 'different default ports' vs 'same ports with configurable offset'. Should rust-bitcoind default to the same ports as Core (8333 mainnet P2P, 8332 RPC) and offer a CLI flag to offset them, or should it default to different ports (e.g., 18333/18332 or 38333/38332)?
  Type: Choice [Same default ports as Core, with optional offset flag, Different default ports (specify in follow-up), with option to use Core's ports, Same ports, no offset mechanism (only one node per host)]
  Suggested default: Same default ports as Core, with optional offset flag
- **cli_argument_compatibility** (`Prd`): Should rust-bitcoind accept the same CLI arguments as bitcoind (e.g., -datadir=, -regtest, -rpcport=, -debug=) for drop-in compatibility, or use a Rust-idiomatic CLI (e.g., --data-dir, --network regtest) with the TOML config as the primary interface?
  Type: Choice [Mirror bitcoind's CLI flags exactly (dash-prefixed, no equals for booleans), Rust-idiomatic CLI (clap-style --long-flags) with a compatibility alias layer, Rust-idiomatic CLI only, no compatibility with bitcoind flags]
- **mempool_policy_knob_scope** (`Research`): Core exposes dozens of mempool policy knobs (-maxmempool, -limitancestorcount, -limitdescendantcount, -bytespersigop, -datacarrier, -datacarriersize, -maxorphantx, -minrelaytxfee, etc.). Should rust-bitcoind implement all of these with the same defaults, only those exercised by the functional test suite, or a curated subset?
  Type: Choice [All Core mempool policy flags with identical defaults, Only those exercised by the functional test suite, Curated subset matching Core defaults, add more as tests require]
  Suggested default: All Core mempool policy flags with identical defaults
- **corruption_recovery_strategy** (`Research`): With reindex excluded, what is the recovery strategy if the redb database becomes corrupted? Options include: full resync from genesis, export/import tools, or redb-level repair utilities. This affects whether you need any data export tooling pre-1.0.
  Type: Choice [Full resync from genesis is acceptable (delete datadir and restart), Implement a basic export/import mechanism for UTXO snapshots, Investigate redb's built-in repair capabilities and document the recovery path]
  Suggested default: Full resync from genesis is acceptable (delete datadir and restart)
- **block_relay_only_and_anchors** (`Research`): Should rust-bitcoind implement Core's block-relay-only connections (2 outbound connections that do not relay transactions or addresses, for partition resistance) and anchor connections (persisted across restarts)? These are important anti-eclipse-attack measures.
  Type: YesNo
  Suggested default: Yes
- **user_agent_string** (`Research`): What user-agent string should rust-bitcoind advertise on the P2P network? Core uses '/Satoshi:30.0.0/'. Common choices are '/rust-bitcoind:0.1.0/' or '/Satoshi:30.0.0/' (masquerading as Core). The choice affects peer acceptance and network monitoring.
  Type: Choice [/rust-bitcoind:x.y.z/ (honest identification), /Satoshi:30.0.0/ (masquerade as Core for compatibility), Configurable, defaulting to /rust-bitcoind:x.y.z/]
  Suggested default: Configurable, defaulting to /rust-bitcoind:x.y.z/
- **historical_consensus_bugs_verification** (`Research`): You confirmed implementing historical consensus bugs. Have you identified the full list? Key ones include: BIP-30 duplicate txids, value overflow (CVE-2010-5139), March 2013 fork (CVE-2013-3220/BDB lock limits), SIGHASH_SINGLE out-of-range returning hash of 1, FindAndDelete in script verification, 64-byte transaction Merkle tree vulnerability, and time-warp attack edge cases. Should the ideation enumerate these explicitly, or is 'all historical consensus bugs as exercised by Core's test suite' sufficient scoping?
  Type: Choice [Enumerate all known bugs explicitly in the PRD, Scope to 'all bugs exercised by Core's test suite' — if a test catches it, we implement it, Both — enumerate known bugs AND use the test suite as the completeness check]
  Suggested default: Both — enumerate known bugs AND use the test suite as the completeness check
- **signal_handling_and_graceful_shutdown** (`Research`): How should rust-bitcoind handle Unix signals? Core handles SIGTERM/SIGINT for graceful shutdown and SIGHUP is sometimes used for log rotation. Should rust-bitcoind replicate Core's signal handling behavior exactly, or implement a simpler model?
  Type: Choice [Replicate Core's signal handling (SIGTERM/SIGINT for shutdown, SIGHUP for log reopen), SIGTERM/SIGINT for graceful shutdown only, no SIGHUP handling, Configurable via TOML, with Core-compatible defaults]
  Suggested default: SIGTERM/SIGINT for graceful shutdown only, no SIGHUP handling
- **test_suite_python_version_and_deps** (`Research`): Core's functional test suite requires specific Python dependencies and test framework setup. Are you planning to vendor the test suite into the rust-bitcoind repo, maintain it as a git submodule pointing to Core v30.0, or use a separate repo? This affects CI setup and how test patches are managed.
  Type: Choice [Vendor (copy) into rust-bitcoind repo with patches applied, Git submodule pointing to a fork with minimal patches, Separate companion repo with the adapted test suite]
  Suggested default: Vendor (copy) into rust-bitcoind repo with patches applied
- **addr_relay_and_addrv2** (`Research`): The ideation mentions 'addr v2 support' in passing under P2P. Should rust-bitcoind implement both addr (BIP-31) and addrv2 (BIP-155) message types, or only addrv2 given the v70016+ protocol minimum? Core supports both for backward compatibility.
  Type: Choice [Both addr and addrv2 (full backward compatibility), addrv2 only (since minimum protocol version is v70016+ which supports it), Both, but only initiate addrv2 (accept legacy addr from peers)]
  Suggested default: Both addr and addrv2 (full backward compatibility)
- **max_connections_defaults** (`Research`): What should the default maximum connection limits be? Core defaults to 125 total (8 outbound full-relay, 2 block-relay-only, up to 115 inbound). Should rust-bitcoind use the same defaults?
  Type: YesNo
  Suggested default: Yes

## Missing Fields
- **historical_consensus_bugs_inventory**: The ideation mentions implementing 'all historical consensus bugs' but does not enumerate them. A concrete list (BIP-30 duplicates, CVE-2010-5139 value overflow, CVE-2013-3220 March 2013 fork behavior, CVE-2018-17144 inflation bug, the 500-byte transaction size policy, SIGHASH_SINGLE out-of-range bug, FindAndDelete semantics, etc.) is needed to scope the work and verify completeness.
- **rpc_versioning_behavior**: No specification of how rust-bitcoind handles the 'getnetworkinfo' version field, user-agent string, or protocol version advertisement. Clients and monitoring tools often branch on these values.
- **signal_handling_behavior**: No specification of how the node handles SIGTERM, SIGINT, SIGHUP, or other Unix signals — particularly whether SIGHUP triggers config reload or log rotation, as Core supports debug.log rotation.
- **debug_log_behavior**: Core writes to debug.log with rotation and category-based filtering (-debug=net, -debug=rpc, etc.). No specification of whether rust-bitcoind replicates this interface or only provides structured tracing.
- **max_connections_and_limits**: No specification of default max inbound/outbound connections, max upload target, or bandwidth throttling behavior.
- **block_relay_only_connections**: Core maintains block-relay-only connections (no addr/tx relay) for partition resistance. Not mentioned whether rust-bitcoind implements this.
- **anchor_connections**: Core persists anchor connections across restarts (anchors.dat). Not mentioned.
- **mempool_size_limit**: No specification of default mempool size limit or eviction policy when the limit is reached.
- **rpc_thread_model**: No specification of how RPC requests are served — dedicated thread pool, tokio tasks, max concurrent requests, etc.
- **binary_name_and_cli_interface**: The binary is presumably 'rust-bitcoind' but no specification of CLI argument parsing, help output format, or whether it mirrors bitcoind's CLI flags exactly.

## Ambiguities
- **rpc_byte_identical_responses**: The success metric says 'byte-identical JSON responses' but JSON serialization order, whitespace, and numeric formatting (scientific notation, trailing zeros) vary across implementations. Need to clarify whether this means semantic equivalence with deterministic key ordering, or literal byte-for-byte matching of the raw HTTP response body.
- **configurable_port_offset_vs_same_ports**: The user context contains contradictory answers: 'default_port_conflict' says 'different default ports to allow co-existence' but 'network_magic_and_port_defaults' says 'same ports with configurable offset'. These need reconciliation — are the defaults the same as Core's (8333/8332) with an offset option, or are the defaults already offset?
- **k256_one_time_decision_vs_fallback**: The user says k256 is a 'one-time project decision' but the ideation describes a fallback to C libsecp256k1 as a contingency. Clarify: is the fallback path actually planned/designed for, or is it purely hypothetical? If k256 fails, does the project pivot to libsecp256k1-rs or is it a project-ending blocker?
- **target_core_version_contradiction**: User context has both 'core_version_pin: v30.0' and 'target_core_version: Track latest master'. The ideation resolves this as 'pin to v30.0 for 1.0' but the user's intent for pre-1.0 development is ambiguous — should the test suite track v30.0 from day one, or track master during development and pin at 1.0?
- **mempool_functionally_equivalent**: The user defines mempool equivalence as 'validated by test suite' — but Core's mempool behavior involves numerous policy knobs (maxmempool, mempoolminfee, limitancestorcount, etc.). Unclear whether all these knobs must be implemented or only the subset exercised by the test suite.
- **fee_estimation_replicate_exactly**: Core's fee estimation uses a complex CBlockPolicyEstimator with bucketed historical data. 'Replicate exactly' could mean identical algorithm producing identical estimates given identical block history, or merely a reasonable fee estimation. The former is extremely difficult to verify.
- **no_reindex_but_corruption_recovery**: Reindex is explicitly excluded, but no alternative is specified for database corruption recovery. If redb becomes corrupted, is the only recourse a full resync from genesis?
- **headers_pre_sync_scope**: The user says 'implement headers pre-sync (matches modern Core)' — Core's headers pre-sync (added in v25.0) involves downloading headers from a single peer before doing parallel header download. Need to clarify whether this refers to that specific mechanism or just general header-first sync.
- **compact_blocks_version**: Compact blocks (BIP-152) has version 1 (non-segwit) and version 2 (segwit). Since SegWit is in scope, presumably version 2, but this should be explicit.
- **crate_boundary_definitions**: The ideation lists crate names (consensus, p2p, rpc, storage, mempool, cli) but doesn't define what lives where. For example, does 'consensus' include script interpretation, or is that a separate crate? Does 'p2p' include the mempool acceptance logic or just wire protocol?

## Suggested Defaults
- **rpc_json_equivalence_definition** = `Semantic equivalence with deterministic key ordering matching Core's insertion order` (Byte-identical matching is fragile across JSON libraries and adds no functional value. Matching key ordering covers the realistic compatibility surface (scripts parsing JSON with key-order assumptions).)
- **default_port_resolution** = `Same default ports as Core with --port-offset flag` (Using the same defaults minimizes configuration for single-node deployments and test suite compatibility. An offset flag handles the co-existence case cleanly.)
- **corruption_recovery_strategy** = `Full resync from genesis (delete and restart)` (Pre-1.0, this is the simplest correct approach. Reindex is deferred, and building export/import tooling is scope creep. Document it as a known limitation.)
- **max_connections_defaults** = `Match Core's defaults (125 total, 8 full-relay outbound, 2 block-relay-only)` (Matching Core's defaults ensures consistent network behavior and avoids surprising operators or the functional test suite.)
- **user_agent_string** = `/rust-bitcoind:x.y.z/ (configurable)` (Honest identification is important for network health monitoring. Masquerading as Core would undermine the defense-in-depth value proposition. Configurability allows operators to choose.)
- **test_suite_management** = `Vendor into repo with patches` (Vendoring avoids submodule complexity, makes patches self-contained and reviewable, and ensures CI reproducibility without external dependencies.)
