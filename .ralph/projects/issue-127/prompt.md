Update `README.md` to clearly document the `openrouter` backend in the **Backend Specs** section.

### Goal
Make `openrouter` appear as a first-class backend option alongside `claude`, `codex`, and `gemini`, with a concise explanation of what it is, how it works, and any usage prerequisites.

### Scope
- **In scope**: `README.md` only (Backend Specs section)
- **Out of scope**: Rust code, config files, new docs pages, setup tutorials, or behavior changes

### Required Changes

1. **Backend examples list update (in Backend Specs)**
   - In the existing backend examples list (the one that currently includes entries like `claude`, `codex(...)`, `gemini(...)`), ensure the following entries are present:
     - `openrouter`
     - `openrouter(openai/gpt-5.3-codex)`
   - Place them naturally with the other backend examples (before the role-specific overrides paragraph).
   - Avoid duplicates if either entry already exists.

2. **Visible explanatory paragraph (not HTML comment)**
   - Immediately after the backend examples list (and before role-specific overrides), add a concise paragraph that explicitly states:
     - `openrouter` routes model requests through the [OpenRouter](https://openrouter.ai/) API (a model-routing API).
     - The backend uses [Goose](https://github.com/block/goose) as its CLI runner.
     - It enables access to models from multiple providers (e.g., OpenAI, Anthropic, Google) via one backend.
     - It is disabled by default (`enabled = false`) and requires an OpenRouter API key.

### Acceptance Criteria
- [ ] Only `README.md` is modified.
- [ ] Backend Specs examples include both:
  - [ ] `openrouter`
  - [ ] `openrouter(openai/gpt-5.3-codex)`
- [ ] A **visible paragraph** in Backend Specs explains:
  - [ ] what OpenRouter is (model-routing API),
  - [ ] Goose is the underlying CLI runner,
  - [ ] multi-provider access through a single backend,
  - [ ] disabled-by-default status (`enabled = false`) and API key requirement.
- [ ] New text is placed before the role-specific overrides paragraph and reads naturally with existing section flow.
- [ ] No duplicate backend example entries are introduced.

### Implementation Notes
- Preserve existing README style and formatting conventions.
- Keep wording concise (roughly 3–5 sentences for the explanatory paragraph).
- Prefer adding links for both OpenRouter and Goose in the explanatory paragraph.

### Validation Steps
1. Inspect `README.md` Backend Specs section and confirm placement/content against acceptance checklist.
2. Confirm diff touches only `README.md`.
3. Sanity-check the disabled-by-default statement against existing project behavior/docs before finalizing wording.