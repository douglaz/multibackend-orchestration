---
artifact: quick-dev-plan-implement
loop: 15
project: issue-201
backend: claude
role: implementer
created_at: 2026-03-10T11:45:14Z
---

All 8 occurrences fixed. Zero `"p2"` remaining, 8 `"P2"` present.

# Implementation: Fix PRIORITY-CASE-01 Amendment

## Change Made

Fixed `"priority": "p2"` → `"priority": "P2"` in all 8 occurrences in `src/validate/tests_pr_review.rs` (lines 189, 389, 479, 613, 757, 900, 1026, 1123).

## Root Cause

The `AmendmentPriority` enum in `src/project/amendments.rs:49-56` uses default serde serialization (no `#[serde(rename_all)]`), so `P2` serializes as `"P2"` (uppercase). The conformance tests were writing `"p2"` (lowercase), which would fail deserialization during `drain_amendment_queue` → `read_and_parse_inflight`, causing amendments to be quarantined as malformed instead of processed.

## Why This Was Needed

While the conformance tests themselves pass (they assert staging/dispatch behavior, not orchestrator-level amendment processing), the staged amendment JSON files would be invalid when the orchestrator actually tries to parse them. This defeats the purpose of testing the end-to-end amendment flow.

## Verification

- 0 remaining lowercase `"p2"` occurrences
- 8 correct uppercase `"P2"` occurrences
- No other files affected
