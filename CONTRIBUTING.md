# Contributing to evtap

Thanks for helping improve evtap. The project currently targets Linux systems that expose keyboard events through evdev.

## Development setup

Install the system dependencies listed in the [README](README.md), then install the current stable Rust toolchain. The minimum supported Rust version is 1.92.

```sh
rustup show
cargo build --locked
```

Before submitting a change, run:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo nextest run --all-targets --locked
cargo +1.92.0 check --all-targets --locked
cargo audit
```

Install `cargo-nextest` using its documented installer or package for your platform. If it is unavailable, use `cargo test --all-targets --locked` instead.

`cargo audit` can be installed with:

```sh
cargo install cargo-audit --version 0.22.2 --locked
```

## Architecture

The main internal boundaries are:

- `listener` and `scanner`: Linux evdev discovery and capture lifecycle.
- `input`: normalized, backend-independent keyboard events.
- `metric`: the metric contract, generic report model, and metric registry.
- `metric_view`: generic egui rendering for metric reports.
- `session`: persisted-session metadata and isolated metric recovery.
- `settings` and `paths`: atomic privacy preferences and XDG locations.
- `database`: SQLite schema, migrations, validation, and transactions.
- `storage`: the background command/event protocol; it accepts aggregate `SessionSnapshot` values and must never import `KeyEvent`.
- `app`: capture/session lifecycle, dirty tracking, and UI orchestration.

A metric must not depend directly on evdev, Tokio, or egui. To add one:

1. Implement `Metric` in a module under `src/metric/`.
2. Give it a unique, stable descriptor ID and explain its sampling semantics.
3. Return UI-independent scalar or table report sections.
4. Register it in `default_metrics` in `src/metric.rs`.
5. Implement a deterministic, versioned snapshot that stores only durable aggregate state.
6. Validate the complete snapshot before mutating the metric during restore.
7. Add deterministic tests covering event kinds, timing boundaries, snapshot round trips, malformed values, transient-state exclusion, and reset behavior.

The generic report renderer means a normal metric addition should not require changes to the UI layer.

## Privacy requirements

evtap handles globally captured keyboard input. Changes must preserve these rules:

- Never persist or transmit raw keyboard events, ordered text, event timestamps, pressed-key state, or transient analysis context.
- Persistent code may receive only validated aggregate snapshots; do not add a `KeyEvent` dependency to `database`, `storage`, `settings`, or session-history code.
- Keep transient text buffers bounded and only as large as an analysis requires.
- Keep persistence opt-in and preserve non-destructive behavior for corrupt or newer settings/databases.
- Do not log captured labels, snapshot JSON, or SQL parameter values.
- Make any inference or uncertainty explicit in metric descriptions.
- Treat capture, storage, permission, migration, retention, and deletion failures as user-visible errors.

Export, telemetry, network behavior, crash reporting, or encryption requires a separate design and privacy review.

## Pull requests

Keep commits focused and include tests for behavior changes. Update user documentation whenever setup, permissions, privacy behavior, metric semantics, or supported environments change.

Unless stated otherwise, contributions are accepted under either the MIT License or the Apache License 2.0, at your option.
