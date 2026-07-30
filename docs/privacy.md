# Privacy and threat model

## Why evtap is sensitive

evtap opens a Linux evdev keyboard device and can observe global input from that device while listening. This may include passwords, private messages, source code, and other secrets. Its capture privilege is fundamentally similar to that of a keylogger.

Only run binaries you trust. Prefer releases produced by this repository's GitHub Actions workflow, verify the published SHA-256 checksum, or build and inspect the source yourself.

## In-memory sessions and disk saves

evtap always processes one mutable working session in memory. Session creation, capture, pause, rename, reset, and switching are ordinary application behavior rather than a separate persistence mode.

A session is written to disk only by **Save now** or autosave. Before the first analytics write, evtap explains that aggregate labels remain sensitive and the database is local and unencrypted. The acknowledgement is remembered so the warning is not shown repeatedly.

Autosave is off by default. When off, dirty session switches and normal close offer Save, Discard, or Cancel. When on, evtap saves periodically and at Stop, switch, and close boundaries. Existing saved sessions can be loaded regardless of the autosave setting.

## Capture and processing flow

1. The scanner inspects readable `/dev/input/event*` devices and lists interfaces that expose a basic keyboard key set.
2. Capture begins only after the user selects a keyboard and chooses **Start listening**. Loading a session never starts capture automatically.
3. Linux key events are converted into an internal event containing physical identity, event kind, timestamp, and optional XKB-produced text.
4. Metrics consume the event synchronously and retain aggregates or bounded in-flight context.
5. The normalized event is dropped after processing. The storage module cannot receive it.
6. A manual or automatic save serializes complete durable aggregate snapshots and sends them to a dedicated SQLite worker.

## Retained in memory

Most metrics retain counts and accumulated durations. Text-oriented aggregates retain labels such as characters or character pairs because those labels are the analyzed dimension.

Correction inference additionally retains up to ten recent text fragments so Backspace can be associated with recently produced text. Timing metrics retain short-lived press or release context. This in-flight context is cleared on Stop, session switch, listener failure, and process exit or restart.

Therefore, "no raw history" does not mean process memory contains no text. Anyone able to inspect evtap's memory may be able to recover aggregate labels and transient analysis state.

## Saved aggregate analytics

`evtap.sqlite3` may contain:

- local session IDs and optional names;
- UTC creation, update, and most-recently-opened times;
- accumulated capture duration;
- evtap version;
- the most recently used keyboard display name and XKB model, layout, and variant;
- total physical press count;
- physical key code, display label, and count;
- deletion labels/counts and inferred deleted-to-typed pair/count aggregates;
- flight and dwell labels with accumulated durations and sample counts;
- bigram labels with accumulated durations and sample counts;
- unknown versioned metric rows retained for forward compatibility.

These aggregates are sensitive. Character and pair dimensions may reveal frequently typed fragments, correction habits, or keyboard-use patterns. Session timestamps and durations reveal when and for how long capture occurred.

Saved sessions remain mutable and resumable until explicitly deleted. There is no completed-session history or automatic retention.

## Never saved

evtap's storage boundary never accepts or stores:

- raw evdev or normalized key events;
- an ordered keystroke or text stream;
- per-event timestamps;
- device paths;
- pressed-key state or unfinished press timestamps;
- recent correction history or a pending deletion;
- previous press/release timing context;
- arbitrary window-widget memory containing analytics;
- captured labels or payloads in logs.

No timing, adjacency, dwell, or correction observation can bridge Stop, a session switch, listener failure, or process restart. Restored metrics continue cumulative aggregate totals but begin fresh in-flight context.

## Files and permissions

Expected Linux files are:

```text
$XDG_CONFIG_HOME/evtap/settings.json
$XDG_DATA_HOME/evtap/app.ron
$XDG_DATA_HOME/evtap/evtap.sqlite3
```

with `~/.config/evtap` and `~/.local/share/evtap` fallbacks.

- `settings.json` stores the settings schema version, disclosure acknowledgement, autosave preference, last-selected session ID, and fallback XKB preferences. It is written atomically and contains no analytics.
- `app.ron` is owned by eframe and contains native window state only. Arbitrary egui widget-memory persistence is disabled.
- `evtap.sqlite3` and its WAL/shared-memory sidecars store saved session metadata and aggregate snapshots.

evtap creates or tightens its settings/data directories to mode `0700` and settings/database files to mode `0600`. It uses normal operating-system path and symbolic-link resolution; it does not promise a special no-symlink policy. Permissions reduce access by other unprivileged local users but do not protect against the same user, root, compromised processes, filesystem snapshots, backups, or copied files.

The analytics database is not encrypted. Encryption without a defensible key-management design would provide misleading protection.

## Saves, crashes, and deletion

**Save now** creates an explicit durability boundary. With autosave enabled, dirty state is also saved approximately every 30 seconds during continuous capture and after Stop, before switching sessions, and during graceful close. A crash or power loss can discard changes after the latest acknowledged transaction.

Deleting a saved session transactionally removes its metric snapshots through SQLite foreign-key cascading. **Delete all saved sessions** closes SQLite and removes the database, WAL, shared-memory, and rollback-journal files. SQLite secure deletion and best-effort page reclamation are enabled, but deletion cannot erase copies in backups, snapshots, SSD remapping, or forensic storage layers.

Corrupt, unidentified, or incompatible-schema databases are not automatically replaced, downgraded, renamed, or truncated. Unsupported database and settings schemas have no automatic migration path; move or delete the incompatible file manually to start fresh.

## Network and logging

evtap initiates no telemetry, synchronization, cloud, crash-reporting, or metric-export requests. It has no account system. The GUI and dependencies may interact with local windowing, timezone, and desktop services required to display the application.

Logs may contain operation names, safe filesystem/device paths, and payload-free errors. They must not contain captured key codes, produced text, aggregate labels, or serialized metric payloads.

## Permission risk

Membership in the Linux `input` group often grants access to every keyboard and pointing device on the machine. Any process running as that user may then exercise the same access. This permission remains a system-level risk even when evtap is not running.

Prefer the narrowest workable permission mechanism. On shared or sensitive systems, use a device-specific udev rule or temporary ACL rather than broad group membership. Avoid running the GUI as root.

## Future changes

Export, telemetry, network behavior, crash reporting, cross-process APIs, or encryption require a separate design and threat review. They must not be introduced as incidental implementation details.
