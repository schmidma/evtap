# Changelog

All notable changes to evtap are documented here. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project follows [Semantic Versioning](https://semver.org/) with Cargo's pre-1.0 compatibility rules.

## [Unreleased]

## [0.1.0-rc.1] - 2026-07-28

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

[Unreleased]: https://github.com/schmidma/evtap/compare/v0.1.0-rc.1...HEAD
[0.1.0-rc.1]: https://github.com/schmidma/evtap/releases/tag/v0.1.0-rc.1
