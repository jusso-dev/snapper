# Contributing to Snapper

Thanks for considering a contribution. Snapper is pre-alpha, so read the [architecture](docs/ARCHITECTURE.md) and relevant [architecture decisions](docs/adr/) before changing platform or workspace structure.

## Get set up

Install a stable Rust toolchain with rustup. From the repository root:

```sh
cargo check --workspace
cargo test --workspace
```

Before opening a pull request, run:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

If a platform dependency prevents a check, include the exact command, platform, and error in your pull request. Do not report a check as passing unless it completed successfully.

## Make a change

1. Find an open issue that matches the change. If an issue asks contributors to claim it, comment there and wait for confirmation before starting.
2. Keep a pull request focused on one issue. Avoid changing platform behavior or public interfaces beyond the issue's scope.
3. Add or update tests for behavior changes. For rendered-output changes, update golden images only when the expected output intentionally changed; explain the reason in the pull request.
4. Use Australian English in user-facing documentation and comments.
5. Run the checks above and review the final diff.

## Open a pull request

Use a short, descriptive title. In the description, link the issue, explain the behavior change, list the commands you ran and their results, and call out platform-specific limitations. Do not include screenshots of features that are not implemented yet.

## Platform notes

Read the [platform permissions guide](docs/platforms/) before working on capture backends. Snapper is pre-alpha; those notes describe current development constraints and must be kept aligned with the code as platform support is implemented.
