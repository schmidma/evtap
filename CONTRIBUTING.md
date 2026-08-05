# Contributing to evtap

Thanks for helping improve evtap. The project currently targets Linux systems that expose keyboard events through evdev.

## Development setup

Install the system dependencies listed in the [README](README.md), then install the current stable Rust toolchain. The minimum supported Rust version is 1.92.

```sh
rustup show
cargo build --locked
```

Before submitting a change, run the same locked commands used by CI:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
cargo +1.92.0 check --all-targets --locked
cargo audit
```

`cargo nextest run --all-targets --locked` is an optional faster local equivalent to the CI test command. If evtap gains a library target with doctests, run them separately with `cargo test --doc --locked`; `--all-targets` does not include doctests.

`cargo audit` can be installed with:

```sh
cargo install cargo-audit --version 0.22.2 --locked
```

Persistence changes also use the [focused manual validation runbook](docs/persistence-validation.md) for native evdev, crash, filesystem-fault, and privacy checks that automated tests do not cover.

## Architecture

The main internal boundaries are:

- `listener` and `scanner`: Linux evdev discovery and capture lifecycle.
- `input`: normalized, backend-independent keyboard events.
- `metric`: the built-in metric contract, concrete `SessionMetrics` aggregate, metric-owned egui presentation, and durable snapshot validation.
- `session`: mutable saved-session metadata and isolated metric recovery.
- `settings` and `paths`: atomic privacy preferences and XDG locations.
- `private_fs`: shared Unix permission enforcement with normal operating-system symlink resolution.
- `database`: SQLite identity/schema validation and transactional mutable-session storage.
- `storage`: dirty-generation tracking and the background command/event protocol; it accepts aggregate `SessionSnapshot` values and must never import `KeyEvent`.
- `app`: the single egui-thread owner, split into capture, persistence, session-lifecycle, and keyboard/settings concerns.
- `app/working_session`: the loaded mutable session, metrics, transient context, and the durable snapshot boundary.
- `app/view`: egui presentation and modal interaction; it does not own durable state.
- `app/tests`: headless lifecycle fixtures and end-to-end UI scenarios.

A metric must not depend directly on evdev or Tokio. Built-in metrics intentionally own their summary and analysis presentation through the `Metric` egui methods; do not reintroduce a generic report or view-model layer. To add one:

1. Implement `Metric` in a module under `src/metric/`.
2. Give it a unique, stable ID and explain its sampling semantics.
3. Implement its summary and analysis UI using the focused primitives under `app/view/components`.
4. Add a concrete field, explicit lifecycle dispatch, snapshot restoration branch, and accessor to `SessionMetrics` in `src/metric.rs`.
5. Place the metric explicitly in the appropriate page under `app/view/shell/analytics.rs`.
6. Implement a deterministic, versioned snapshot that stores only durable aggregate state.
7. Validate the complete snapshot before mutating the metric during restore.
8. Add deterministic tests covering event kinds, timing boundaries, presentation, snapshot round trips, malformed values, transient-state exclusion, and reset behavior.

## Privacy requirements

evtap handles globally captured keyboard input. Changes must preserve these rules:

- Never persist or transmit raw keyboard events, ordered text, event timestamps, pressed-key state, or transient analysis context.
- Storage code may receive only validated aggregate snapshots; do not add a `KeyEvent` dependency to `database`, `storage`, `settings`, or saved-session code.
- Keep transient text buffers bounded and only as large as an analysis requires.
- Require the disclosure before the first analytics write and preserve non-destructive behavior for corrupt or incompatible settings/databases.
- Do not log captured labels, snapshot JSON, or SQL parameter values.
- Make any inference or uncertainty explicit in metric descriptions.
- Treat capture, storage, permission, schema, switch, close, and deletion failures as user-visible errors.

Export, telemetry, network behavior, crash reporting, or encryption requires a separate design and privacy review.

## Pull requests

Keep commits focused and include tests for behavior changes. Update user documentation whenever setup, permissions, privacy behavior, metric semantics, or supported environments change.

Treat documentation as product content for human readers. Keep durable user and contributor guidance in the repository; keep implementation progress, temporary investigation notes, raw validation output, and duplicated test plans in issues, pull requests, tests, or git history instead.

Unless stated otherwise, contributions are accepted under either the MIT License or the Apache License 2.0, at your option.
