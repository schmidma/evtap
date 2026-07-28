# Persistence prerelease validation

Use only nonsensitive synthetic typing during these checks. Do not paste passwords, tokens, private messages, or production source code into any application while evtap is listening.

Run validation with isolated XDG directories so it cannot affect normal settings or history:

```sh
validation_root="$(mktemp -d)"
export XDG_CONFIG_HOME="$validation_root/config"
export XDG_DATA_HOME="$validation_root/data"
RUST_LOG=evtap=debug cargo run --release --locked
```

Keep the terminal log private. Remove the temporary directory when validation is complete.

## Default and opt-in boundary

1. Start evtap with empty XDG directories.
2. Confirm persistence says **Off** and no `evtap.sqlite3` exists.
3. Confirm the data directory is mode `0700`. `app.ron` may exist after a normal window close.
4. Choose **Enable persistence…** and verify the disclosure names sensitive character, bigram, correction, key-usage, count, and timing aggregates; local unencrypted storage; excluded raw/transient data; and default retention.
5. Cancel and confirm no database was created.
6. Enable persistence and inspect:

   ```sh
   find "$XDG_CONFIG_HOME/evtap" "$XDG_DATA_HOME/evtap" \
     -maxdepth 1 -printf '%m %p\n'
   ```

7. Confirm settings/database files are mode `0600`, application directories are `0700`, and `settings.json` contains no metric data.

## Capture, pause, and recovery

1. Select a keyboard and XKB configuration, then start capture.
2. Type a known synthetic sequence and verify metrics update.
3. Confirm keyboard/XKB controls become fixed for the active session.
4. Wait at least 30 seconds after a metric change and confirm the status advances through **Unsaved changes**, **Saving…**, and **Saved**.
5. Stop capture and confirm it checkpoints immediately and remains the same active session.
6. Close evtap normally, restart with the same XDG directories, and confirm:
   - the active session is restored paused;
   - capture does not start automatically;
   - durable counts/totals match the last acknowledged checkpoint;
   - the device path was not restored;
   - the next flight, dwell, bigram, or correction sample does not bridge the restart.
7. Select the matching keyboard and resume capture. Confirm new samples add to restored aggregates.

## Finish, history, and deletion

1. Finish while listening and confirm capture stops before the session appears in history.
2. Open history and verify local start time, capture duration, total presses, keyboard, layout, and variant.
3. Open details and compare every metric with the final active view.
4. Start another session and confirm viewing history does not change its active metrics.
5. Delete one completed session, confirm it disappears only after acknowledgement, and restart to verify it remains deleted.
6. Exercise previous/next pagination with more than 50 synthetic completed sessions if practical.
7. Choose **Delete all stored analytics…**, confirm the active and completed sessions clear only after acknowledgement, and verify the database is fresh and empty.
8. Exercise **Delete all analytics and disable** and confirm `evtap.sqlite3`, `-wal`, `-shm`, and rollback-journal files are absent while `settings.json` remains.

## Retention

Validate 30-, 90-, and 365-day policies and forever using a disposable database with controlled fixture timestamps or an integration build. Confirm retention:

- uses `completed_at_ms`, not creation time;
- removes only sessions strictly older than the cutoff;
- never removes the active session;
- refreshes history only after worker acknowledgement.

Do not change the host clock on a normal workstation merely to test retention.

## Crash recovery

1. Produce a saved active checkpoint.
2. Add more synthetic input until the UI says **Unsaved changes**.
3. Terminate the disposable process with `kill -KILL`.
4. Restart and confirm the database opens through WAL recovery, the last committed aggregates are intact, and only post-checkpoint changes may be absent.
5. Repeat while a checkpoint is likely in progress. Confirm either the complete old or complete new multi-metric snapshot is visible, never a partial mix.

## Failure handling

Use a disposable VM, constrained filesystem, loopback mount, or quota. Permission-bit changes alone may not affect a database connection that is already open.

Validate:

- no-space/quota exhaustion during checkpoint;
- read-only or write-denied settings and analytics paths;
- a busy database exceeding the bounded timeout;
- malformed and unsupported-newer `settings.json`;
- corrupt, unidentified, and newer-schema SQLite files;
- listener failure while storage is dirty;
- graceful-exit timeout behavior.

For each storage failure, confirm capture continues, in-memory aggregates remain, the generation stays dirty, the UI shows a payload-free error, and **Retry storage operation** serializes a fresh latest snapshot. Confirm corrupt/newer files are not replaced, truncated, renamed, or silently reset.

## Privacy inspection

After synthetic use, inspect `settings.json`, SQLite schema/rows, `app.ron`, and logs. Confirm:

- SQLite contains only documented session metadata and aggregate snapshot JSON;
- no raw key-event sequence, ordered text, per-event timestamp, device path, pressed-key map, recent correction buffer, or timing context appears;
- `app.ron` contains window state but no evtap settings or analytics;
- logs contain no captured labels or serialized payloads;
- the application initiates no telemetry or synchronization traffic.

Record the evtap commit, Rust version, Linux distribution, desktop environment, display protocol, filesystem type, SQLite fault setup, and result for each prerelease candidate.
