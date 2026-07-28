# evtap

> Local, session-based analysis of everyday typing mechanics.

`evtap` is a Linux desktop application that listens to one selected keyboard through the kernel's evdev interface and computes timing and correction metrics while you type normally. It is intended to reveal physical hesitation, key-hold patterns, common transitions, and correction signals that a conventional words-per-minute test cannot show.

> [!WARNING]
> evtap observes global keyboard input from the selected device while listening. Treat it like a keylogger even though it processes data locally and does not save raw input. Read the [privacy model](docs/privacy.md) before use.

evtap is currently pre-1.0 software. The first supported release line is `0.1.x`; interfaces and behavior may still change between minor versions.

## Current scope

- Linux desktop application using evdev input devices
- One explicitly selected keyboard at a time
- Session-only, in-memory analytics
- No export, persistence, telemetry, or network behavior
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
6. Choose **Stop listening** to retain and inspect the current session results.
7. Choose **Reset session** to clear every metric.

Keyboard selection and XKB configuration are locked while listening so that a session does not change interpretation midway through a pressed-key sequence.

## Keyboard layout behavior

evtap receives Linux key codes and uses XKB to derive text for character-oriented metrics. It maintains modifier and lock state from press and release events, including Shift and Caps Lock. It does not currently detect or synchronize the desktop environment's active layout automatically; select the matching configuration in evtap.

Physical metrics such as key usage identify the key itself. Text-oriented metrics use the text produced by the configured XKB state. Changing layouts therefore changes text and bigram labels without changing physical key identities.

## Privacy

While listening, evtap:

- reads global events from one keyboard;
- computes aggregate metrics locally;
- keeps only bounded transient text needed for correction inference;
- does not persist raw events or typed text;
- does not export, transmit, or send telemetry;
- discards the entire session when reset or when the process exits.

More detail is available in [docs/privacy.md](docs/privacy.md).

## Development

See [CONTRIBUTING.md](CONTRIBUTING.md) for setup, quality checks, architecture boundaries, metric-extension guidance, and privacy requirements. The current release plan is in [ROADMAP.md](ROADMAP.md), and notable changes are tracked in [CHANGELOG.md](CHANGELOG.md).

## License

Licensed under either of:

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT License](LICENSE-MIT)

at your option.
