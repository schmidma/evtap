# Changelog

All notable changes to evtap are documented here. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project follows [Semantic Versioning](https://semver.org/) with Cargo's pre-1.0 compatibility rules.

## [Unreleased]

### Added

- System, light, and dark appearance choices that persist across restarts.

### Changed

- Redesigned the dashboard with focused navigation, a persistent top bar, and dedicated Overview, Key Usage, Timing, Corrections, and Settings views.
- Organized settings into clear categories with searchable keyboard selectors, and improved the session switcher and **Manage sessions** workflow.
- Expanded analytics with sortable, expandable tables and clearer summaries. The UI now uses **Correction Signals** while the persisted `corrections` metric ID remains compatible with saved sessions.
- Made storage, device, listener, and session-list feedback specific to the failed operation, with focused dialogs, clearer recovery actions, and improved accessible labels and focus behavior.

## [0.2.0] - 2026-07-30

### Added

- Mutable saved sessions that can be selected and resumed across application restarts.
- Manual **Save** and optional autosave at periodic, Stop, switch, and normal-close boundaries.
- Untitled sessions, optional unique names, rename, reset, new-session, individual deletion, complete saved-session deletion, and disk-usage controls.
- Save, Discard, and Cancel prompts before dirty session switches or close when autosave is off.
- A one-time disclosure before the first analytics write; unsaved sessions continue to work entirely in memory.
- Private XDG preferences, native window-state restoration, and a local unencrypted SQLite analytics database.

### Changed

- **Stop** pauses the working session and triggers autosave only when configured.
- Keyboard and XKB values are remembered per session as suggestions rather than fixed configuration.
- Restored and selected sessions remain paused until the user chooses **Start**.

### Security

- Saved metric formats are versioned and validated. Corrupt, foreign, and incompatible settings or databases are rejected without replacing existing files.
- Raw key events, ordered text, event timestamps, pressed-key state, correction context, and unfinished timing observations are excluded from saved snapshots.
- Capture and session boundaries clear in-flight context without clearing durable aggregates.
- Settings and analytics files use restrictive Unix permissions; saved aggregates remain readable to the same user, privileged processes, and backups.

## [0.1.0] - 2026-07-28

### Added

- Session-only Linux evdev keyboard capture with explicit keyboard selection.
- Total press, ranked key usage, correction signal, flight time, dwell time, and bigram metrics.
- Modifier-aware XKB model, layout, and variant handling.
- User-visible device permission, scan, listener, and XKB errors.
- Session reset and clear empty states for metric tables.
- Dual MIT or Apache-2.0 licensing.

### Changed

- Renamed the previous heatmap presentation to the more accurate ranked **Key Usage** table.
- Labeled Backspace-derived values as correction signals rather than a true error rate or confusion matrix.

### Fixed

- Proportional UI text displays symbols such as the bigram arrow correctly.
- Keyboard modifiers and locks update XKB state across press and release events.
- Background capture and scanning update the GUI when data arrives.
- Listener read failures stop capture cleanly.

[Unreleased]: https://github.com/schmidma/evtap/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/schmidma/evtap/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/schmidma/evtap/releases/tag/v0.1.0
