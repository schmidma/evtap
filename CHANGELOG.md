# Changelog

All notable changes to evtap are documented here. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project follows [Semantic Versioning](https://semver.org/) with Cargo's pre-1.0 compatibility rules.

## [Unreleased]

### Added

- Explicitly opt-in local aggregate persistence with a privacy disclosure and 90-day default retention.
- Versioned, deterministic snapshots for every bundled metric, with strict size and value validation.
- Private XDG settings, native window-state restoration, and a bundled SQLite analytics database.
- Crash-recoverable active sessions, periodic transactional checkpoints, dirty-generation acknowledgements, and bounded graceful shutdown.
- Finish/discard session lifecycle, configuration locking, storage status, retries, and enable/disable resolution flows.
- Paginated completed-session history, isolated metric detail restoration, retention controls, individual deletion, complete analytics deletion, and disk-usage display.
- Fault tests for malformed snapshots/settings, symlink paths, transaction rollback, migration failure, corrupt/newer databases, worker recovery, retention, and deletion.

### Changed

- **Stop listening** now pauses the active session instead of implying that the analytics session ended.
- Session configuration becomes fixed at first capture and recovered sessions resume paused.
- Privacy, metrics, troubleshooting, roadmap, and contributor documentation now describe the optional aggregate-storage boundary.

### Security

- Raw key events, ordered text, event timestamps, pressed-key state, and transient correction/timing context are excluded from persistent snapshots.
- Settings and analytics use restrictive Unix permissions, atomic settings replacement, SQLite application identity, foreign keys, secure deletion, and WAL-backed transactions.
- Unsupported settings or database versions and corrupt/unidentified databases are handled non-destructively rather than reset automatically.

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

[Unreleased]: https://github.com/schmidma/evtap/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/schmidma/evtap/releases/tag/v0.1.0
