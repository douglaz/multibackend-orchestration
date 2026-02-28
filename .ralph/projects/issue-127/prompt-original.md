## Summary

Add a visible explanation of the `openrouter` backend to the **Backend Specs** section of `README.md`, describing what it is, how it works (uses [Goose](https://github.com/block/goose) as its CLI runner routed through the OpenRouter API), and why it exists alongside the other backends (`claude`, `codex`, `gemini`). Also add `openrouter` examples to the existing backend examples list so it appears naturally alongside the other backends.

## Acceptance Criteria

- [ ] `README.md` contains a visible paragraph (not an HTML comment) in the **Backend Specs** section describing the `openrouter` backend
- [ ] The explanation covers: what OpenRouter is (a model-routing API), that it uses `goose` as the underlying CLI, and that it enables access to models from multiple providers (e.g. OpenAI, Anthropic, Google) through a single backend
- [ ] The existing backend examples list includes `openrouter` and `openrouter(openai/gpt-5.3-codex)` entries alongside the existing `claude`, `codex`, `gemini` examples
- [ ] The paragraph notes that the `openrouter` backend is disabled by default (`enabled = false` in config) and requires an OpenRouter API key to use
- [ ] The text fits naturally into the existing Backend Specs section structure
- [ ] No other files are modified

## Technical Approach

1. **Add `openrouter` entries to the existing backend examples list** — Locate the examples list in the **Backend Specs** section (the list that contains entries like `claude`, `codex(gpt-5.3-codex-xhigh)`, etc.) and append `openrouter` and `openrouter(openai/gpt-5.3-codex)` entries to it. Use structural anchors for placement: insert after the last existing backend example entry and before the role-specific overrides paragraph.

2. **Insert an explanatory paragraph** — After the updated examples list and before the role-specific overrides paragraph, add a concise paragraph covering:
   - `openrouter` is a backend that routes requests through the [OpenRouter](https://openrouter.ai/) API, giving access to models from multiple providers (OpenAI, Anthropic, Google, etc.) via a unified endpoint.
   - Under the hood it uses `goose` (from Block) as its CLI runner.
   - It is disabled by default (`enabled = false` in config) and requires an OpenRouter API key.

3. **No new files or abstractions** — This is a documentation-only change to an existing file.

## Files & Modules

| File | Change |
|------|--------|
| `README.md` (Backend Specs section — examples list) | Add `openrouter` and `openrouter(openai/gpt-5.3-codex)` to the backend examples list, after the last existing backend example and before the role-specific overrides paragraph |
| `README.md` (Backend Specs section — after examples list) | Add a visible paragraph explaining the `openrouter` backend, its use of `goose`, OpenRouter API routing, and that it is disabled by default (`enabled = false`) |

## Testing Strategy

- **Manual review**: Read the updated README to verify clarity, accuracy, and placement within the Backend Specs section.
- **No automated tests needed**: This is a documentation-only change with no code impact.
- **Verify no regressions**: Confirm `cargo check` and `cargo test` still pass as a sanity check (the change is Markdown-only).
- **Config accuracy check**: Verify that the `enabled = false` claim matches the actual `BackendEnabled::Disabled` serialization in the codebase before finalizing the text.

## Out of Scope

- Adding documentation for the `gemini` backend or other backends beyond `openrouter`
- Modifying `goose` configuration or OpenRouter setup guides
- Adding a dedicated "Backends" documentation page outside of README
- Changing any Rust source code or configuration files