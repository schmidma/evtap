# Persistence manual validation runbook

Use this runbook for persistence behavior that requires native evdev capture, process termination, filesystem faults, or direct privacy inspection. Run the canonical automated checks in [CONTRIBUTING.md](../CONTRIBUTING.md) separately.

## Safety and prerequisites

- Use only nonsensitive synthetic typing. Do not enter passwords, tokens, private messages, or production source code anywhere while evtap is listening.
- Use a dedicated test keyboard or carefully isolated virtual evdev device. Confirm which device evtap selected before every capture.
- Set `EVTAP_BIN` to a locally built evtap executable.
- Use disposable XDG directories and keep logs, databases, and backups private.
- Use a disposable VM, loopback filesystem, or quota for destructive storage faults. Permission changes alone may not affect an already-open database connection.
- Keep native windows away from the active desktop during automation. Run evtap and the automation process on the same Xvfb display. Xvfb isolates the display only; it does not generate evdev input, so use a dedicated physical or virtual evdev source for capture checks.

Start each clean run in an isolated environment:

```sh
validation_root="$(mktemp -d)"
export XDG_CONFIG_HOME="$validation_root/config"
export XDG_DATA_HOME="$validation_root/data"
RUST_LOG=evtap=debug "$EVTAP_BIN"
```

## Fresh in-memory behavior and disclosure

1. Start evtap with empty XDG directories.
2. Confirm the active session is **Untitled session** and no `evtap.sqlite3` exists.
3. Confirm the data directory is mode `0700`. `app.ron` may appear after a normal close.
4. Select the test keyboard, choose **Start**, enter a short synthetic sequence, then choose **Stop**. With autosave off, confirm analytics remain in memory and the status is **Unsaved changes**.
5. Choose **Save**. Confirm the disclosure identifies sensitive aggregate labels, local unencrypted storage, and the exclusion of raw events and in-flight context.
6. Cancel and confirm no database was created.
7. Choose **Save** again, accept, and confirm the status advances through **Saving…** to **Saved**.
8. Inspect permissions:

   ```sh
   find "$XDG_CONFIG_HOME/evtap" "$XDG_DATA_HOME/evtap" \
     -maxdepth 1 -printf '%m %p\n'
   ```

9. Confirm settings and database files are mode `0600`, application directories are `0700`, and `settings.json` contains preferences but no analytics.

## Manual save, close, and restart

1. With autosave off, add more synthetic input and close the window.
2. Confirm the close dialog offers save, discard, and cancel choices.
3. Cancel and verify the in-memory state remains.
4. Close again and save before exit.
5. Restart with the same XDG directories and confirm:
   - exactly the last-selected saved session loads;
   - capture remains stopped;
   - durable counts and totals match the acknowledged save;
   - a uniquely matching keyboard may be suggested, but capture does not start;
   - no device path was stored;
   - the next timing, bigram, or correction observation does not bridge the restart.
6. Select the intended test keyboard, choose **Start**, and confirm new samples add to the restored aggregates.

## Session switching and autosave

1. Rename the current session to `Home` and save it.
2. Use the session switcher to create an untitled session, capture synthetic input, and save it without assigning a name.
3. Rename it to `Work`; confirm a duplicate nonempty name is rejected.
4. With autosave off, modify `Work` and select `Home`. Exercise each save, discard, and cancel choice and confirm that it preserves or changes the active session as described.
5. Confirm switching stops an active listener and the loaded session remains paused.
6. Delete the last-selected saved session, restart, and confirm evtap creates a new untitled session rather than loading an older saved session.
7. Enable **Autosave sessions**. During capture, make a metric change and wait at least 30 seconds; confirm **Unsaved changes** advances through **Saving…** to **Saved**.
8. Confirm autosave writes after **Stop**, before a dirty switch, and during normal close. Disable autosave and confirm **Stop** leaves dirty state in memory until **Save** or a switch/close decision.

## Session management and deletion

1. Open **Manage sessions** and confirm saved sessions can be distinguished, renamed, and deleted.
2. Reset statistics and confirm the name and remembered setup remain while metrics and capture duration become zero. Confirm reset follows normal save behavior.
3. Delete the current saved session and confirm the warning covers its saved copy and unsaved changes. Confirm a new untitled session replaces it and the deleted session does not return after restart.
4. Save an active session, then choose **Delete all saved sessions**. Confirm all saved records and SQLite sidecars are removed and the active saved session becomes untitled.
5. Create an unsaved in-memory draft while another saved session exists. Choose **Delete all saved sessions** and confirm the saved records are removed while the active unsaved draft remains unchanged.
6. Confirm `settings.json` remains after deleting all saved sessions.

## Crash recovery

1. Produce an acknowledged saved session.
2. Enable autosave, add synthetic input until the status is **Unsaved changes**, and terminate the disposable process with `kill -KILL` before the next save.
3. Restart and confirm the last committed aggregates are intact and only later changes are absent.
4. Repeat while a save is likely in progress. Confirm the restored database contains either the complete earlier or complete later multi-metric snapshot, never a partial mix.
5. Confirm recovery does not create timing, adjacency, dwell, or correction observations across restart.

## Fault handling and recovery

Exercise these cases only in a disposable environment:

- no-space or quota exhaustion during save;
- read-only or write-denied settings and analytics paths;
- a busy database that exceeds the bounded timeout;
- malformed or unsupported `settings.json`;
- corrupt, unidentified, foreign, or incompatible-schema SQLite files;
- listener failure while storage is dirty;
- failed autosave before switch or close;
- graceful-shutdown timeout.

For save failures, confirm capture continues, in-memory aggregates remain, the status stays dirty, and the UI shows an operation-specific error without captured payloads. Correct the fault and choose **Retry save**; confirm the retry saves the latest in-memory state. A failed automatic save must block the requested switch or close.

For list, load, rename, and deletion failures, confirm the focused error and recovery action match the failed operation. Use **Manage sessions** to verify that a failed deletion leaves the saved session visible.

Confirm incompatible files are not replaced, truncated, renamed, or silently reset. If recovery requires moving a file aside, stop evtap first, keep the database and SQLite sidecars together, and retain any copy privately until it is no longer needed.

## Privacy inspection

After synthetic use, inspect `settings.json`, the SQLite schema and rows, `app.ron`, and logs. Confirm:

- SQLite contains only documented session metadata and aggregate metric snapshots;
- no raw key-event sequence, ordered text, per-event timestamp, device path, pressed-key map, recent correction buffer, or timing context appears;
- `settings.json` contains only documented preferences, including appearance, and an internal last-session ID;
- `app.ron` contains window state but no evtap settings or analytics;
- logs contain no captured labels or serialized metric payloads;
- evtap initiates no telemetry or synchronization traffic.

## Cleanup

Stop evtap before cleanup. Remove the isolated validation directory and any private logs or filesystem images created for fault injection:

```sh
rm -rf -- "$validation_root"
```

If cleanup fails, check for a running evtap process or mounted test filesystem before retrying. Do not copy a real or synthetic analytics database into a public issue.
