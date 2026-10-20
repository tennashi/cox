# xtask

Development tasks for the workspace, run as `cargo xtask <task>`. Run
`cargo xtask help` for the list.

xtask is a member of the workspace but not of `default-members`, so plain
`cargo build` and `cargo test` skip it. Checks that must cover it, such as
those in `cargo xtask ci`, pass `--workspace`.
