# TODO

- When a new run is executed with `--project`, we should either set active project = None or override active project with the specified one
- **auto_branch race condition**: When `ralph run` creates a new branch via `auto_branch`, it branches from the current HEAD. If `ralph project new` was committed separately after the prompt commit, the branch point may be *before* the project files commit, causing "No such file or directory" on the new branch. Fix: either `auto_branch` should ensure it includes all commits up to and including the project state files, or `ralph project new` + prompt commit should be atomic, or `ralph run` should detect the missing state and merge the latest master commit before proceeding.
- `ralph tail` without `--project` shows stale output from last active project instead of the currently running project. Consider auto-detecting which project has an active run, or at least warn when showing a completed project.
