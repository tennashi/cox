# Contributing

## Commands

- `cargo xtask setup` installs the pinned development tools.
- `cargo xtask ci` runs every check. A change is ready only when it passes.

## Pull requests

Pull requests are merged by rebasing, so every commit lands on `main` as
written and must pass `cargo xtask ci` on its own.

## Commit messages

Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/).
