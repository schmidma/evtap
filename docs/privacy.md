# Privacy and threat model

## Why evtap is sensitive

evtap opens a Linux evdev keyboard device and can observe global input from that device while listening. This includes input directed at other applications and may include passwords, private messages, source code, and other secrets. Its access is fundamentally similar to that of a keylogger.

Only run binaries you trust. Prefer releases produced by this repository's GitHub Actions workflow, verify the published SHA-256 checksum, or build and inspect the source yourself.

## Data flow

1. The scanner inspects readable `/dev/input/event*` devices and lists interfaces that expose a basic keyboard key set.
2. Capture begins only after the user selects a keyboard and chooses **Start listening**.
3. Linux key events are converted into an internal event containing physical identity, event kind, timestamp, and optional XKB-produced text.
4. Metrics consume the event synchronously and retain aggregates or bounded state.
5. The raw event is dropped after processing.
6. Resetting or exiting drops all metric state.

## Retained in memory

Most metrics retain only counts and accumulated durations. Text-oriented aggregates retain labels such as characters or character pairs because those labels are the analyzed dimension.

Correction inference additionally retains up to ten recent text fragments so that Backspace can be associated with recently produced text. This buffer is bounded and exists only in process memory. It is cleared by **Reset session** and discarded on exit.

Therefore, "no raw history" does not mean that process memory contains no text: aggregate labels and the bounded correction buffer can reveal portions of typed content. Anyone able to inspect evtap's memory may be able to recover that data.

## What evtap does not do

The `0.1.x` scope has:

- no database or configuration persistence;
- no event or metric export;
- no telemetry or analytics service;
- no network requests initiated by evtap;
- no cloud account or synchronization;
- no logging of captured key codes or produced text.

The GUI and dependencies may interact with the local windowing, accessibility, and desktop services required to display the application, but captured input is not intentionally sent to them.

## Permission risk

Membership in the Linux `input` group often grants access to every keyboard and pointing device on the machine. Any process running as that user may then exercise the same access. This permission remains a system-level risk even when evtap is not running.

Prefer the narrowest workable permission mechanism for the machine. On shared or sensitive systems, use a device-specific udev rule or temporary ACL rather than broad group membership. Avoid running the GUI as root.

## Future changes

Persistence, export, telemetry, network behavior, crash reporting, or cross-process APIs are outside the current privacy contract. Any such feature requires:

1. an explicit design and threat review;
2. opt-in behavior where appropriate;
3. documented data fields and retention;
4. migration and deletion behavior;
5. updated UI disclosure and user documentation.

A change that violates the current contract must not be introduced as an incidental implementation detail.
