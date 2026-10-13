# Snapper guide

Snapper is pre-alpha. The workspace currently provides a compiling project skeleton; capture, editing, upload, and recording features are being developed on the [roadmap](../../README.md#roadmap). This guide describes building the project and links to platform-specific development notes. It does not promise that planned features are available.

## Build from source

Install a stable Rust toolchain, clone the repository, then run:

```sh
cargo check --workspace
cargo run -p snapper-app
```

The app and capture workflows are still under development. If the app does not yet expose a capture action, that is expected at this stage.

## More information

- [Architecture](../ARCHITECTURE.md)
- [Architecture decision records](../adr/)
- [Contributing](../../CONTRIBUTING.md)
- [Platform permissions](../platforms/)
