# Changelog

All notable changes to evtap are documented here. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project follows [Semantic Versioning](https://semver.org/) with Cargo's pre-1.0 compatibility rules.

## [Unreleased]

## [0.2.0] - 2026-07-30

### Added

- Mutable saved sessions that can be selected and resumed indefinitely across application restarts.
- Manual **Save now** and an editor-like autosave preference covering periodic, Stop, switch, and normal-close boundaries.
- Untitled sessions, optional unique names, rename, reset, New session, individual deletion, complete saved-session deletion, and disk-usage display.
- Familiar Save, Discard, and Cancel prompts before dirty session switches or close when autosave is off.
- A one-time disclosure before the first analytics write; session management itself always works in memory.
- Versioned, deterministic snapshots for every bundled metric, with strict size and value validation.
- Private XDG settings, native window-state restoration, a bundled SQLite analytics database, retryable saves, and bounded shutdown.
- Generic non-destructive rejection of incompatible, future, corrupt, unidentified, or foreign databases.

### Changed

- **Stop listening** now pauses the working session and triggers autosave only when configured.
- Keyboard and XKB values are remembered per session as suggestions rather than locked configuration.
- Restored and selected sessions remain paused until the user starts listening.

### Security

- Raw key events, ordered text, event timestamps, pressed-key state, recent correction context, and unfinished correction/timing observations are excluded from saved snapshots.
- Stop, switch, listener failure, exit, and restart clear metric in-flight context without clearing durable aggregates.
- Settings and analytics use restrictive Unix permissions, atomic settings replacement, SQLite application identity, foreign keys, secure deletion, and WAL-backed transactions.
- Storage follows normal operating-system symlink resolution while enforcing private directory and file modes.

## [0.1.0] - 2026-07-28

### Added

- Session-only Linux evdev keyboard capture with explicit keyboard selection.
- Total press, ranked key usage, correction signal, flight time, dwell time, and bigram metrics.
- Modifier-aware XKB model, layout, and variant handling.
- User-visible device permission, scan, listener, and XKB errors.
- Generic normalized input, metric, report, registry, and egui rendering interfaces.
- Session reset control and clear empty states for metric tables.
- Deterministic unit tests for bundled metrics and keyboard capability detection.
- Rust 1.92 minimum-version check, strict Clippy, formatting, tests, and dependency auditing in CI.
- Reproducible GitHub release archives with SHA-256 checksums.
- Dual MIT or Apache-2.0 licensing and contributor documentation.

### Changed

- Updated the dependency baseline, including eframe 0.35 and Tokio 1.53.
- Renamed the previous heatmap presentation to the more accurate ranked **Key Usage** table.
- Labeled Backspace-derived values as correction signals rather than a true error rate or confusion matrix.

### Fixed

- Proportional UI text now falls back to egui's bundled Hack font for symbols such as the bigram arrow.
- Keyboard modifiers and locks now update XKB state across press and release events.
- Background capture and scanning now wake the GUI when data arrives.
- Listener read failures terminate cleanly instead of repeatedly emitting stop events.
- Normal startup and scan paths no longer rely on panic-prone unwraps or expects.

[Unreleased]: https://github.com/schmidma/evtap/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/schmidma/evtap/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/schmidma/evtap/releases/tag/v0.1.0
