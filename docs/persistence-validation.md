# Resumable-session prerelease validation

Use only nonsensitive synthetic typing. Do not paste passwords, tokens, private messages, or production source code into any application while evtap is listening.

Run validation with isolated XDG directories:

```sh
validation_root="$(mktemp -d)"
export XDG_CONFIG_HOME="$validation_root/config"
export XDG_DATA_HOME="$validation_root/data"
RUST_LOG=evtap=debug cargo run --release --locked
```

Keep terminal logs private and remove the temporary directory afterward.

## Automated headless coverage

The regular Rust suite includes `egui_kittest` lifecycle tests for disclosure cancellation and acceptance, manual save and restart, dirty close cancellation and save, autosaved switches and closes, untitled-session saves, unique-name enforcement, switch Save/Discard/Cancel choices, reset, deletion, exact last-session behavior, delete-all cleanup, and busy-database save failure followed by a latest-state retry. They create no native window and can be run with both display protocols explicitly unavailable:

```sh
env -u DISPLAY -u WAYLAND_DISPLAY cargo nextest run headless_
```

These tests do not replace readable-evdev capture checks, native backend smoke tests, crash timing, or constrained-filesystem fault injection below. For native UI automation without showing windows in the current desktop session, run evtap and the automation process together under `xvfb-run`.

## Fresh in-memory behavior and disclosure

1. Start evtap with empty XDG directories.
2. Confirm the current session is **Untitled session** and no `evtap.sqlite3` exists.
3. Confirm the data directory is mode `0700`. `app.ron` may exist after normal close.
4. Start and stop a short synthetic capture with autosave off. Confirm analytics remain in memory and the status says **Unsaved changes**.
5. Choose **Save now**. Confirm the disclosure names sensitive character, bigram, correction, key-usage, count, and timing aggregates; local unencrypted storage; and excluded raw/in-flight data.
6. Cancel and confirm no database was created.
7. Choose **Save now** again, accept, and confirm the status advances through **Saving…** to **Saved**.
8. Inspect:

   ```sh
   find "$XDG_CONFIG_HOME/evtap" "$XDG_DATA_HOME/evtap" \
     -maxdepth 1 -printf '%m %p\n'
   ```

9. Confirm settings/database files are mode `0600`, application directories are `0700`, and `settings.json` contains no metric data.

## Manual save, close, and restart

1. With autosave off, add more synthetic input and close the window.
2. Confirm the close prompt offers **Save and exit**, **Exit without saving**, and **Cancel**.
3. Cancel and verify in-memory state remains.
4. Close again with **Save and exit**.
5. Restart with the same XDG directories and confirm:
   - exactly the last-selected saved session loads;
   - it remains paused;
   - durable counts and totals match the acknowledged save;
   - a uniquely matching keyboard may be suggested, but capture does not start;
   - no device path was stored;
   - the next flight, dwell, bigram, or correction observation does not bridge restart.
6. Select any intended readable keyboard, resume capture, and confirm new samples add to the restored aggregates.

## Session switching

1. Rename the current session to `Home` and save it.
2. Choose **New session**, leave it untitled, capture synthetic input, and save it. Confirm untitled sessions can be saved without a naming barrier.
3. Rename it to `Work`; confirm duplicate nonempty names are rejected while IDs remain internal.
4. With autosave off, modify `Work` and select `Home`.
5. Confirm the switch prompt offers Save, Discard, and Cancel:
   - Cancel leaves `Work` and its changes visible;
   - Discard loads the last saved `Home` state;
   - Save persists `Work` before loading `Home`.
6. Confirm switching stops a running listener first and the newly loaded session remains paused.
7. Delete the last-selected saved session, restart, and confirm evtap creates a new untitled session rather than loading an older saved session.
8. Confirm changing keyboard or XKB configuration is allowed while paused and updates the session's remembered suggestion after save.

## Autosave

1. Enable **Autosave sessions** and confirm the disclosure appears only if it was not previously accepted.
2. During capture, make a metric change and wait at least 30 seconds. Confirm **Unsaved changes** → **Saving…** → **Saved**.
3. Verify Stop triggers a save.
4. Verify switching a dirty session saves automatically without a Save/Discard prompt.
5. Verify normal close saves automatically.
6. Disable autosave and confirm Stop leaves dirty state in memory without writing until **Save now** or a switch/close decision.

## Rename, reset, and deletion

1. Rename a saved session and confirm the rename remains dirty until manually or automatically saved.
2. Reset statistics, confirm name/setup remain while metrics and capture duration become zero, and verify reset follows normal save behavior.
3. Delete the current saved session and confirm one warning covers both its saved copy and unsaved changes.
4. Confirm deletion is immediate even with autosave off and a new untitled session replaces it.
5. Restart and confirm the deleted session does not reappear.
6. Exercise **Delete all saved sessions** and verify `evtap.sqlite3`, `-wal`, `-shm`, and rollback-journal files are absent while `settings.json` remains.

## Crash recovery

1. Produce an acknowledged saved session.
2. Enable autosave, add input until the UI says **Unsaved changes**, and terminate the disposable process with `kill -KILL` before the next save.
3. Restart and confirm the last committed aggregates are intact and only later changes are absent.
4. Repeat while a save is likely in progress. Confirm either the complete old or complete new multi-metric snapshot is visible, never a partial mix.
5. Confirm WAL recovery does not create timing or correction observations across restart.

## Failure handling

Use a disposable VM, constrained filesystem, loopback mount, or quota. Permission changes alone may not affect an already-open database connection.

Validate:

- no-space or quota exhaustion during save;
- read-only or write-denied settings and analytics paths;
- a busy database exceeding the bounded timeout;
- malformed and unsupported `settings.json`;
- corrupt, unidentified, foreign, and incompatible-schema SQLite files;
- listener failure while storage is dirty;
- failed autosave before switch or close;
- graceful-shutdown timeout behavior.

For save failures, confirm capture continues, in-memory aggregates remain, the generation stays dirty, and the UI shows a payload-free error. **Retry storage operation** must serialize a fresh latest snapshot. A failed automatic save must block the requested switch or close.

Confirm incompatible files are not replaced, truncated, renamed, or silently reset. The application should emit one generic incompatible-schema error with the database path; it must not contain special migration logic for an old unreleased experiment.

## Privacy inspection

After synthetic use, inspect `settings.json`, the SQLite schema/rows, `app.ron`, and logs. Confirm:

- SQLite contains only documented session metadata and aggregate snapshot JSON;
- there are no active/completed, completion-time, event, history, or retention columns/tables;
- no raw key-event sequence, ordered text, per-event timestamp, device path, pressed-key map, recent correction buffer, or timing context appears;
- `settings.json` contains only preferences and an internal last-session ID;
- `app.ron` contains window state but no evtap settings or analytics;
- logs contain no captured labels or serialized payloads;
- the application initiates no telemetry or synchronization traffic.

Record the evtap commit, Rust version, Linux distribution, desktop environment, display protocol, filesystem type, SQLite fault setup, and result for each prerelease candidate.

## Validation record: 2026-07-30

Code under test: `ddfe165f3e8c0301efddbc13838ec2fa580ee618` (`0.2.0-dev`).

Environment:

- Arch Linux, Linux `7.1.5-zen1-1-zen`, x86-64.
- Rust `1.97.1`; the same code also passed the Rust `1.92` MSRV job.
- niri on Wayland. Native interaction ran on an isolated Xvfb X11 display.
- Disposable XDG roots were on `/tmp` (`tmpfs`).
- A synthetic uinput keyboard was marked `LIBINPUT_IGNORE_DEVICE=1`, so niri never opened it; evtap read it directly through evdev.
- SQLite `ENOSPC` and settings `EROFS` failures were injected at the system-call boundary in disposable processes. The regular suite separately covers bounded busy-database failure, incompatible identities and schemas, rollback, malformed settings, and symlink handling.

Results:

- **Passed:** disclosure cancellation and acceptance, private file modes, manual save, exact restart restoration, advisory keyboard restoration, and paused restart.
- **Passed:** real evdev Start/Stop capture with key usage, counts, dwell, flight, bigram, and correction aggregates.
- **Passed:** in-flight state did not bridge restart, Stop, session switch, or listener `ENODEV` failure. A held key at Stop produced no dwell sample after its later release.
- **Passed:** the 30-second autosave checkpoint, Stop autosave, switch autosave while listening, listener-failure autosave, and normal-close autosave.
- **Passed:** `kill -KILL` before autosave restored the previous complete snapshot. A kill immediately after requesting Stop/autosave recovered one complete new generation; SQLite integrity remained `ok`, and all metric rows had one generation timestamp.
- **Passed:** injected `ENOSPC` kept the previous committed generation, left the latest state dirty, showed a payload-free failure, blocked automatic switch and close, and successfully retried the latest snapshot after recovery.
- **Passed:** injected settings `EROFS` left `settings.json` byte-for-byte unchanged, reverted the attempted preference change, displayed a payload-free error, and saved successfully after recovery.
- **Passed:** final inspection found only `sessions` and `metric_snapshots` in SQLite, documented preferences in `settings.json`, and window geometry in `app.ron`. No device path, raw-event structure, pressed-key state, correction buffer, or in-flight context appeared in those durable stores. Diagnostic logs named the selected device path and listener error but contained no captured key label or aggregate payload. No evtap TCP connection was present.
- **Passed:** the complete automated suite, strict Clippy, formatting, Rust `1.92`, RustSec audit, release build, repeated display-free headless tests, native Xvfb smoke test, and GitHub CI run `30525439959`.

Only nonsensitive generated input was used. Disposable databases, screenshots, logs, fault helpers, and the synthetic device were removed after validation.
