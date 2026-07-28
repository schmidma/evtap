# evtap roadmap

This roadmap describes intent rather than a compatibility promise. evtap follows Cargo's pre-1.0 semantic-versioning rules: incompatible changes may ship in a new minor release (`0.1` to `0.2`), while patch releases should remain compatible within their release line.

## 0.1 — usable session analyzer

The `0.1.x` goal is a trustworthy Linux-only beta for personal use.

### Product scope

- Capture one explicitly selected, readable evdev keyboard.
- Keep all analytics in memory for one process session.
- Do not persist, export, transmit, or synchronize data.
- Support manual XKB model, layout, and variant configuration.
- Present physical key usage as a ranked table.
- Explain correction inference and timing sample limitations.

### Quality gate

- [x] Current dependency baseline and Rust 1.92 MSRV
- [x] Keyboard-capability filtering and user-visible permission errors
- [x] Reliable listener start, stop, disconnect, and background UI wake-up
- [x] Modifier-aware XKB state handling
- [x] Backend- and UI-independent metric event API
- [x] Generic metric report and renderer interfaces
- [x] Deterministic tests for all bundled metrics
- [x] Format, strict Clippy, test, MSRV, and RustSec CI checks
- [x] Reproducible GitHub release archive and checksum workflow
- [x] Open-source licensing and contributor guidance
- [x] User documentation for setup, privacy, metrics, and troubleshooting
- [x] Manual hardware validation with the maintainer's keyboard and layout
- [x] Validate at least one permission-denied and reconnect workflow interactively
- [x] Publish and test a GitHub prerelease before the final `v0.1.0` tag

## 0.2 — opt-in aggregate persistence

The [accepted persistence specification](docs/persistence-spec.md) defines local, versioned aggregate storage without raw event history. The implementation is now on `main` for prerelease hardening.

### Product scope

- Persistence remains off until the user accepts an explicit sensitivity disclosure.
- Versioned aggregate snapshots never cross the storage boundary as raw events.
- Active sessions recover paused with transient metric state cleared.
- Completed history is paginated and rendered through separate metric instances.
- Retention and per-session/complete deletion are user controlled.
- Privacy settings, window state, and analytics use separate files.

### Quality gate

- [x] Versioned, deterministic, size-limited snapshots for every bundled metric
- [x] Durable/transient state separation and restart-safe restoration tests
- [x] Atomic private settings with unsupported-version protection
- [x] SQLite identity, schema migration, transactions, retention, and cascading deletion
- [x] Dedicated worker, dirty generations, 30-second schedule, retries, and bounded shutdown
- [x] Capture/session separation, configuration locking, finish/discard, and enable/disable flows
- [x] Paginated history, isolated detail restoration, retention, and deletion UI
- [x] Rust 1.92, strict Clippy, unit/integration, RustSec, and privacy fault checks
- [ ] Interactive permission, restart, crash-recovery, disk-failure, retention, and deletion validation
- [ ] Publish and test a `0.2.0` prerelease before the final tag

## After 0.2

Potential directions, ordered only loosely:

- Richer tables, trends, distributions, and confidence indicators
- Physical keyboard geometry and layout-aware heatmap visualization
- Better automatic desktop XKB configuration detection
- Device hotplug monitoring and smoother reconnect behavior
- More robust correction and editing analysis with explicit uncertainty
- Optional aggregate export designed with a stable data schema
- A deliberately public library or plugin API if external consumers emerge
- Additional operating systems only when capture and permission models can be supported responsibly

Raw keystroke persistence, cloud synchronization, accounts, and telemetry are not planned. They require a separate threat model and explicit product decision if ever reconsidered.
