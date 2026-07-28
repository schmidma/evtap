# Privacy and threat model

## Why evtap is sensitive

evtap opens a Linux evdev keyboard device and can observe global input from that device while listening. This may include passwords, private messages, source code, and other secrets. Its capture privilege is fundamentally similar to that of a keylogger.

Only run binaries you trust. Prefer releases produced by this repository's GitHub Actions workflow, verify the published SHA-256 checksum, or build and inspect the source yourself.

## Privacy modes

Aggregate persistence is **off by default**. In this mode, session state exists only in process memory and is discarded when the process exits.

Persistence can be enabled only through an explicit in-app disclosure. It is local and unencrypted. Opting in changes the privacy boundary because labels such as characters, character pairs, correction pairs, and physical-key usage may reveal information about what was typed even though they are unordered aggregates.

Disabling persistence requires resolving any active persisted session: either finish it and keep its completed history, or delete all stored analytics. A persisted active row is not silently abandoned.

## Capture and processing flow

1. The scanner inspects readable `/dev/input/event*` devices and lists interfaces that expose a basic keyboard key set.
2. Capture begins only after the user selects a keyboard and chooses **Start listening**. Recovery never starts capture automatically.
3. Linux key events are converted into an internal event containing physical identity, event kind, timestamp, and optional XKB-produced text.
4. Metrics consume the event synchronously and retain aggregates or bounded transient state.
5. The normalized event is dropped after processing. The storage module cannot receive it.
6. If persistence is enabled and the session is dirty, the UI serializes complete aggregate snapshots and sends those snapshots to a dedicated SQLite worker.

## Retained in memory

Most metrics retain counts and accumulated durations. Text-oriented aggregates retain labels such as characters or character pairs because those labels are the analyzed dimension.

Correction inference additionally retains up to ten recent text fragments so that Backspace can be associated with recently produced text. Timing metrics retain short-lived press or release context. These buffers exist only in process memory and are cleared on discard, finish, or restart.

Therefore, "no raw history" does not mean process memory contains no text. Anyone able to inspect evtap's memory may be able to recover aggregate labels and transient analysis state.

## Persisted aggregate analytics

When persistence is enabled, `evtap.sqlite3` may contain:

- local session IDs and active/completed state;
- UTC creation, update, and completion times for sessions;
- accumulated capture duration;
- evtap version;
- keyboard display name and selected XKB model, layout, and variant;
- total physical press count;
- physical key code, display label, and count;
- deletion labels/counts and inferred deleted-to-typed pair/count aggregates;
- flight and dwell labels with accumulated durations and sample counts;
- bigram labels with accumulated durations and sample counts;
- unknown versioned metric rows retained for forward compatibility.

These aggregates are sensitive. Character and pair dimensions may reveal frequently typed fragments, correction habits, or keyboard use patterns. Session timestamps and durations reveal when and for how long capture occurred.

## Never persisted

evtap's persistence boundary never accepts or stores:

- raw evdev or normalized key events;
- an ordered keystroke or text stream;
- per-event timestamps;
- device paths;
- pressed-key state or press timestamps;
- recent correction history or a pending deletion;
- previous press/release timing context;
- window-widget memory containing analytics;
- captured labels or payloads in logs.

No timing or correction sample is allowed to bridge a restart.

## Files and permissions

On Linux, the expected files are:

```text
$XDG_CONFIG_HOME/evtap/settings.json
$XDG_DATA_HOME/evtap/app.ron
$XDG_DATA_HOME/evtap/evtap.sqlite3
```

with `~/.config/evtap` and `~/.local/share/evtap` fallbacks.

- `settings.json` stores the settings schema version, persistence consent, retention choice, and XKB preferences. It is written atomically.
- `app.ron` is owned by eframe and contains native window state only. Arbitrary egui widget memory is disabled.
- `evtap.sqlite3` and its temporary WAL/shared-memory sidecars store aggregate analytics.

evtap creates or tightens its settings/data directories to mode `0700` and settings/database files to mode `0600`. It rejects symbolic links at critical settings and database paths. These controls reduce accidental access but do not protect against the same user, root, compromised processes, filesystem snapshots, backups, or copied files.

The analytics database is not encrypted. Encryption without a defensible key-management design would provide misleading protection.

## Retention, checkpoints, and deletion

Completed sessions default to 90-day retention. The available choices are 30, 90, or 365 days, or forever. Retention uses completion time and never deletes an active session.

Dirty active sessions are checkpointed approximately every 30 seconds during continuous capture, immediately after Stop, as part of Finish, and during graceful shutdown. A crash or power loss can discard changes after the latest committed transaction.

Deleting a session transactionally removes its metric snapshots through SQLite foreign-key cascading. **Delete all stored analytics** closes SQLite and removes the database, WAL, shared-memory, and rollback-journal files before optionally creating a new empty database. SQLite secure deletion and best-effort page reclamation are enabled, but deletion cannot erase copies in backups, snapshots, SSD remapping, or forensic storage layers.

Corrupt, unidentified, or newer-schema databases are not automatically replaced, downgraded, renamed, or truncated. Settings with an unsupported schema use persistence-off defaults in memory and are not overwritten automatically.

## Network and logging

evtap initiates no telemetry, synchronization, cloud, crash-reporting, or metric-export requests. It has no account system. The GUI and dependencies may interact with local windowing, accessibility, timezone, and desktop services required to display the application.

Logs may contain operation names, safe filesystem/device paths, and payload-free errors. They must not contain captured key codes, produced text, aggregate labels, or serialized metric payloads.

## Permission risk

Membership in the Linux `input` group often grants access to every keyboard and pointing device on the machine. Any process running as that user may then exercise the same access. This permission remains a system-level risk even when evtap is not running.

Prefer the narrowest workable permission mechanism. On shared or sensitive systems, use a device-specific udev rule or temporary ACL rather than broad group membership. Avoid running the GUI as root.

## Future changes

Export, telemetry, network behavior, crash reporting, cross-process APIs, or encryption require a separate design and threat review. They must not be introduced as incidental implementation details.
