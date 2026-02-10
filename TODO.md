# TODO

- **auto_branch race condition**: When `ralph run` creates a new branch via `auto_branch`, it branches from the current HEAD. If `ralph project new` was committed separately after the prompt commit, the branch point may be *before* the project files commit, causing "No such file or directory" on the new branch. Fix: either `auto_branch` should ensure it includes all commits up to and including the project state files, or `ralph project new` + prompt commit should be atomic, or `ralph run` should detect the missing state and merge the latest master commit before proceeding.
