# ralph

`ralph` is a multi-backend orchestration tool for structured AI development loops.

It coordinates planner, implementer, QA, reviewer, and completer phases across AI backends (for example `claude` and `codex`), while persisting project state and artifacts in a workspace-local `.ralph/` directory.

## Highlights

- Structured loop lifecycle: planning, implementation, QA, review, commit, completion.
- Multi-backend support with role-specific backend overrides.
- Prompt review gate before first loop (configurable and skippable).
- First-class project state, history, artifact tailing, and rollback.
- Built-in conformance validation suite (`ralph validate`).

## Install

Run directly from GitHub without cloning:

```bash
nix run github:douglaz/multibackend-orchestration -- --help
```

## Build

```bash
nix develop -c cargo check
nix develop -c cargo test
nix build -L
```

Built binary:

```bash
./result/bin/ralph --help
```

## Quickstart

1. Initialize a workspace.

```bash
./result/bin/ralph init
```

2. Create a project from a prompt file.

```bash
./result/bin/ralph project new --id demo --name "Demo Project" --prompt ./PROMPT.md --backend claude
./result/bin/ralph project use demo
```

3. Run one loop (or run until completion).

```bash
./result/bin/ralph run --loops 1
# or
./result/bin/ralph run --until-complete
```

4. Inspect progress.

```bash
./result/bin/ralph status
./result/bin/ralph history --verbose
./result/bin/ralph tail -F
```

## Core Commands

- `ralph init`
- `ralph project new|list|use|show`
- `ralph run`
- `ralph status`
- `ralph history`
- `ralph tail`
- `ralph rollback`
- `ralph config show|get|set|edit`
- `ralph prd`
- `ralph quick-prd`
- `ralph auto`
- `ralph validate`

## Backend Specs

Backend values use either:

- `backend_name`
- `backend_name(model)`

Examples:

- `claude`
- `codex`
- `openrouter`
- `claude(opus)`
- `codex(gpt-5.3-codex-xhigh)`
- `openrouter(openai/gpt-5.3-codex)`

The `openrouter` backend routes model requests through the [OpenRouter](https://openrouter.ai/) API (a model-routing API) and uses [Goose](https://github.com/block/goose) as its CLI runner. It provides access to models from multiple providers (for example OpenAI, Anthropic, and Google) through one backend. This backend is disabled by default (`enabled = false`) and requires an OpenRouter API key.

Role-specific overrides are available on `run` and in config:

- `workflow.planner_backend`
- `workflow.implementer_backend`
- `workflow.reviewer_backend`
- `workflow.qa_backend`
- `workflow.completer_backend`

## Config

Global config is stored at `.ralph/ralph.toml`.
Project overrides are stored at `.ralph/projects/<id>/config.toml`.
Project values override global values.

Examples:

```bash
./result/bin/ralph config show
./result/bin/ralph config get workflow.qa_enabled
./result/bin/ralph config set workflow.qa_enabled false
./result/bin/ralph config set workflow.planner_backend codex(gpt-5.3-codex-high) --project demo
```

## Validate Conformance Tests

Run the conformance suite against a built binary:

```bash
./result/bin/ralph validate --bin ./result/bin/ralph
./result/bin/ralph validate --bin ./result/bin/ralph --list
```

Every new feature or CLI command should add coverage in `src/validate/`.

## Workspace Layout

```text
.ralph/
  ralph.toml
  index.json
  templates/
  projects/<id>/
    state.json
    config.toml
    prompt.md
    loops/<NNN-slug>/
```
