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
- [ ] Publish and test a GitHub prerelease before the final `v0.1.0` tag

## 0.2 — opt-in aggregate persistence

The [accepted persistence specification](docs/persistence-spec.md) defines local, versioned aggregate storage without raw event history. Implementation has not started.

The planned scope includes resumable active sessions, completed-session history, retention and deletion controls, privacy-sensitive settings, and separate eframe window-state persistence.

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
