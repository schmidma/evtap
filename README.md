# evtap

> Local, session-based analysis of everyday typing mechanics.

`evtap` is a Linux desktop application that listens to one selected keyboard through the kernel's evdev interface and computes timing and correction metrics while you type normally. It is intended to reveal physical hesitation, key-hold patterns, common transitions, and correction signals that a conventional words-per-minute test cannot show.

> [!WARNING]
> evtap observes global keyboard input from the selected device while listening. Treat it like a keylogger even though it processes data locally and does not save raw input. Read the [privacy model](docs/privacy.md) before use.

evtap is pre-1.0 software. `0.1.x` is the session-only baseline; the current `0.2` development line adds explicitly opt-in local aggregate persistence. Interfaces and behavior may still change between minor versions.

## Current scope

- Linux desktop application using evdev input devices
- One explicitly selected keyboard at a time
- In-memory analytics by default, with optional local aggregate session history
- No raw-event persistence, export, telemetry, cloud account, or network requests
- Ranked key-usage table rather than a physical keyboard heatmap
- Manual XKB model, layout, and variant selection

The display server is not used for capture: keyboard events come from `/dev/input`. The GUI can run through the X11 or Wayland support provided by eframe/winit.

## Metrics

- **Total key presses:** physical presses during the session, excluding automatic repeats.
- **Key usage:** physical keys ranked by press count.
- **Correction signals:** deleted text and inferred deleted-to-typed corrections based on Backspace usage.
- **Flight time:** release-to-next-press timing for character keys.
- **Dwell time:** how long character keys remain held.
- **Bigram speed:** press-to-press timing for character pairs with enough samples.

Metric definitions, sampling thresholds, and limitations are documented in [docs/metrics.md](docs/metrics.md).

## Installation

### GitHub release

Download the Linux archive and checksum from the [GitHub Releases page](https://github.com/schmidma/evtap/releases), verify it, and extract it:

```sh
sha256sum --check evtap-0.1.0-x86_64-unknown-linux-gnu.tar.gz.sha256
tar -xzf evtap-0.1.0-x86_64-unknown-linux-gnu.tar.gz
cd evtap-0.1.0-x86_64-unknown-linux-gnu
./evtap
```

Replace `0.1.0` with the release you downloaded. GitHub releases are currently the only binary distribution channel; evtap is not published on crates.io or in distribution repositories.

### From source

Install Rust using [rustup](https://rustup.rs/), then:

```sh
git clone https://github.com/schmidma/evtap.git
cd evtap
cargo run --release --locked
```

The project develops against current stable Rust and supports Rust 1.92 or newer.

## System dependencies

The XKB development library is required when building from source:

```sh
# Ubuntu / Debian
sudo apt-get install libxkbcommon-dev

# Fedora
sudo dnf install libxkbcommon-devel

# Arch Linux
sudo pacman -S libxkbcommon
```

Prebuilt release archives still require the corresponding runtime libraries supplied by the Linux system.

## Input permissions

evtap must be able to read the selected `/dev/input/event*` device. Most Linux installations deny this to regular users by default.

Check the current permissions with:

```sh
ls -l /dev/input/event*
id
```

A common setup is to add your account to the system's `input` group:

```sh
sudo usermod -aG input "$USER"
```

Log out completely and back in before trying again. Membership in `input` is security-sensitive: it commonly grants access to all local input devices, not only one keyboard. On multi-user or higher-security systems, prefer a narrowly scoped udev rule or temporary ACL appropriate to that system. Avoid running the desktop application as root.

If evtap cannot inspect any input devices, it displays a permission message rather than silently showing an empty list. See [docs/troubleshooting.md](docs/troubleshooting.md) for alternatives and diagnostics.

## Usage

1. Start evtap and wait for keyboard scanning to finish.
2. Select a readable keyboard.
3. Select the XKB model, layout, and variant matching that keyboard. Blank model/layout values use XKB defaults.
4. Choose **Start listening**.
5. Type normally in other applications; evtap updates when global events arrive.
6. Choose **Stop listening** to pause capture while retaining the active session.
7. Choose **Discard current session** when persistence is off. With persistence enabled, use **Finish session** to archive it or **Discard session** to delete it.
8. Open **History** to inspect or delete completed sessions when persistence is enabled.

Keyboard and XKB configuration become fixed when a capture session starts. A recovered session resumes paused and never starts capture automatically.

## Optional aggregate persistence

Persistence is off by default. Enabling it requires an in-app disclosure explaining that aggregate labels can still be sensitive. When enabled, evtap stores versioned metric snapshots in a local, unencrypted SQLite database and uses a 90-day completed-session retention policy by default. Available retention choices are 30, 90, or 365 days, or forever.

Expected Linux locations are:

```text
$XDG_CONFIG_HOME/evtap/settings.json
$XDG_DATA_HOME/evtap/app.ron
$XDG_DATA_HOME/evtap/evtap.sqlite3
```

with the usual `~/.config` and `~/.local/share` fallbacks. `settings.json` records consent, retention, and XKB preferences. `app.ron` contains native window state only. `evtap.sqlite3` contains session metadata and aggregate metric snapshots. evtap creates private application directories and files with restrictive Unix permissions, but the database is not encrypted and filesystem backups or privileged processes can still read it.

Active sessions are checkpointed approximately every 30 seconds while dirty, immediately after capture stops, during finishing, and during graceful exit. A crash can lose changes after the latest committed checkpoint. See the [persistence specification](docs/persistence-spec.md) and [privacy model](docs/privacy.md) for exact fields and lifecycle behavior.

## Keyboard layout behavior

evtap receives Linux key codes and uses XKB to derive text for character-oriented metrics. It maintains modifier and lock state from press and release events, including Shift and Caps Lock. It does not currently detect or synchronize the desktop environment's active layout automatically; select the matching configuration in evtap.

Physical metrics such as key usage identify the key itself. Text-oriented metrics use the text produced by the configured XKB state. Changing layouts therefore changes text and bigram labels without changing physical key identities.

## Privacy

evtap:

- reads global events from one explicitly selected keyboard only while listening;
- computes aggregate metrics locally;
- keeps bounded transient state needed for timing and correction inference;
- never persists raw events, ordered text, event timestamps, pressed-key state, or transient correction history;
- can optionally persist sensitive aggregate labels, counts, and duration totals;
- does not export, transmit, or send telemetry;
- discards the active session on exit when persistence is off, or checkpoints it for paused recovery when persistence is enabled.

More detail is available in [docs/privacy.md](docs/privacy.md).

## Development

See [CONTRIBUTING.md](CONTRIBUTING.md) for setup, quality checks, architecture boundaries, metric-extension guidance, and privacy requirements. Persistence prereleases use the [manual validation guide](docs/persistence-validation.md). The current release plan is in [ROADMAP.md](ROADMAP.md), and notable changes are tracked in [CHANGELOG.md](CHANGELOG.md).

## License

Licensed under either of:

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT License](LICENSE-MIT)

at your option.
