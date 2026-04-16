# Contributing

## Prerequisites

- **Rust** — install via [rustup](https://rustup.rs/). The project uses Rust
  edition 2024, so a recent stable toolchain is required.
- **Xcode Command Line Tools** — required to compile whisper.cpp, which is built
  from source as part of the dependency chain.

  ```sh
  xcode-select --install
  ```

- **macOS** — this project uses macOS-only APIs (Accessibility, CoreAudio,
  LaunchAgents) and cannot be built on other platforms.

## Clone the Repo

```sh
git clone https://github.com/yuzokamoto/jabberwok.git
cd jabberwok
```

> The first build will take a while — whisper.cpp is compiled from source.

## Build & Verify

### Build

```sh
cargo build
```

### Run tests

```sh
cargo test
```

### Lint

```sh
cargo clippy -- -D warnings
```

### Format

```sh
cargo fmt --check
```

Run `cargo fmt` (without `--check`) to auto-format.

## Model Setup for Local Dev

Most subcommands require a model to be present. After building, download the
default model:

```sh
cargo run -- download-model parakeet-v3
```

Then confirm everything is configured correctly:

```sh
cargo run -- doctor
```

## Run in Development Mode

Run any subcommand directly from source (no install required):

```sh
cargo run -- <subcommand> [flags]
```

Examples:

```sh
cargo run -- list-devices                         # no model required
cargo run -- record --duration 10 --output clip.wav
cargo run -- transcribe --duration 5              # requires model
cargo run -- transcribe --file clip.wav           # requires model
cargo run -- daemon                               # requires model + permissions
```

Set `RUST_LOG` to control log verbosity during development:

```sh
RUST_LOG=jabberwok=debug cargo run -- daemon
```

### Debug: clear permission state

In debug builds only, you can reset local permission state for testing:

```sh
cargo run -- permissions microphone --remove
cargo run -- permissions accessibility --remove
cargo run -- permissions all --remove
```

`--remove` may still require a reboot or a manual re-grant in System Settings.

## Building and Packaging

### Package the app bundle

The default macOS packaging flow is CI-safe and deterministic. It performs the
full build: release binary, staged `Jabberwok.app`, and
`target/xtask/macos/Jabberwok.dmg`.

```sh
cargo xtask package macos
```

For CI-safe or staged packaging, use the explicit macOS commands:

```sh
cargo xtask package-macos build-binary
cargo xtask package-macos stage-app
cargo xtask package-macos package-dmg
cargo xtask package-macos all
```

### Install as a background service (local dev)

Copies the app bundle to `~/Applications/Jabberwok.app` and registers a
LaunchAgent so Jabberwok launches automatically at login.

```sh
cargo xtask install-service macos
```

### Uninstall the dev service

Removes the LaunchAgent and deletes `~/Applications/Jabberwok.app`.

```sh
cargo xtask uninstall-service macos
```

## Releasing

Prepare a tagged release from a clean working tree with one command:

```sh
cargo xtask release 0.1.1
# or a prerelease
cargo xtask release 0.1.1-rc.1
# or prepare and push in one step
cargo xtask release 0.1.1 --push
```

That command:

- updates the root package version in `Cargo.toml`,
- regenerates `Cargo.lock`,
- creates a `Release v0.1.1` commit,
- creates a matching `v0.1.1` git tag.

Push the release after it succeeds:

```sh
git push origin main
git push origin v0.1.1
```

Or let `xtask` push the current branch and matching tag for you:

```sh
cargo xtask release 0.1.1 --push
```
