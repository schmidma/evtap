# Resumable session persistence reference

**Applies to:** evtap 0.2.x

This document defines the session lifecycle, durable data, privacy boundaries, and recovery behavior of evtap's optional local persistence.

## 1. Summary

evtap always operates on one loaded, mutable working session. A session accumulates aggregate keyboard metrics across any number of capture runs and may be resumed indefinitely. Users may save sessions, switch between saved sessions, rename them, reset them, or delete them.

Disk storage is not a separate session mode. Without saving, the same session workflow operates entirely in memory and disappears when the process exits. A manual save or autosave writes the working session to a local, unencrypted SQLite database.

There are no completed sessions, finalization, history, or retention policies. Session lifetime is controlled exclusively by explicit user deletion.

## 2. Product model

### 2.1 Session

A session is a mutable statistics bucket with:

- an internal identity after its first save;
- an optional user-facing name;
- creation and update times;
- accumulated active-capture duration;
- remembered keyboard display name and XKB configuration;
- versioned durable snapshots for every metric.

A name is optional. The UI renders a missing name as **Untitled session** and uses metadata to distinguish multiple untitled saved sessions. Nonempty names must be unique as a usability safeguard; internal IDs remain authoritative.

A session is not tied to a workplace, device, or layout by enforcement. Remembered keyboard and XKB values are defaults only. Users may intentionally accumulate events from any readable keyboard or configuration into any session.

### 2.2 Working session

Exactly one session is loaded into memory at a time. It may be:

- a new unsaved session with no database identity;
- a clean copy of a saved session; or
- a saved session with unsaved changes.

There is never an unselected application state. If no requested saved session can be loaded, evtap creates a new untitled in-memory session.

Other sessions in the selector are saved database records, not additional mutable in-memory metric registries.

### 2.3 Capture

Start and Stop control only the listener. Repeated capture runs add to the same working session. Capture never starts automatically after startup or a session switch.

The keyboard display name and XKB model, layout, and variant used most recently are remembered with the session. They are suggestions and do not prevent the user from selecting another input setup.

## 3. Storage controls

### 3.1 Manual save

**Save now** writes the complete current working session. A first save assigns an internal database ID. An untitled session may be saved without forcing a name.

The first operation that can write analytics displays a privacy disclosure. Confirming it records a non-analytics `storage_disclosure_acknowledged` preference so the warning is not repeatedly shown.

Manual save remains available whether autosave is on or off.

### 3.2 Autosave

Autosave is an editor-like preference, not an application or session state. When enabled, evtap saves dirty state:

- approximately 30 seconds after metric changes during capture;
- after Stop is acknowledged;
- before switching sessions;
- during normal application close.

The autosave interval is fixed at 30 seconds. Autosave never writes per key event.

Enabling autosave requires the same disclosure as a first manual save. The `autosave_enabled` preference is stored in `settings.json`.

### 3.3 Saved and dirty state

The working session is dirty when its durable representation differs from the last acknowledged database save. Causes include:

- metric aggregate changes;
- accumulated capture-duration changes;
- rename;
- reset;
- remembered keyboard or XKB changes.

Storage status is visible as:

- `Unsaved session`;
- `Saved`;
- `Unsaved changes`;
- `Saving…`;
- `Could not save — Retry`.

Only acknowledgement of the matching dirty generation may display `Saved`.

## 4. Startup

On startup:

1. load settings;
2. scan keyboards;
3. if a recognized database exists, open it without creating a new one;
4. read the `last_session_id` preference;
5. load exactly that session if it still exists;
6. otherwise create a new untitled in-memory session;
7. restore durable metric state;
8. clear all in-flight context;
9. remain paused.

Do not fall back to an older saved session when the last-selected ID is missing. Loading an unexpected older session is less clear than starting an untitled session.

Saved sessions are loadable whether autosave is enabled or disabled. Autosave controls automatic writes, not reads. Selecting a saved session may immediately update `last_session_id` in `settings.json`; this preference write is not an analytics checkpoint.

The analytics database is not created merely by starting evtap, scanning devices, changing preferences, or using an unsaved session.

## 5. Switching and creating sessions

### 5.1 Boundary behavior

Switching includes selecting another saved session or choosing **New session**. If capture is running, evtap first stops the listener and clears in-flight context.

For a dirty session with autosave disabled, show:

- **Save and switch**;
- **Discard changes and switch**;
- **Cancel**.

For an unsaved session, the discard label is **Discard session and switch**. Choosing save invokes the disclosure if necessary. Untitled sessions may remain untitled when saved.

With autosave enabled, save automatically before switching. A failed save blocks the switch and preserves the working state in memory.

A clean session switches without a prompt.

### 5.2 New session

Creating a new session passes through the same dirty-state boundary. After that boundary succeeds, evtap creates an untitled in-memory session with zero aggregates and no database ID. It does not write a database row until saved.

### 5.3 Transient boundary

Stop, switch, listener failure, process exit, and restart clear all in-flight context. No timing, adjacency, dwell, or correction observation may bridge one of these boundaries.

## 6. Close behavior

On normal close:

- stop capture and account for the final capture segment;
- clear in-flight context;
- if clean, close normally;
- if dirty with autosave enabled, save before closing;
- if dirty with autosave disabled, offer **Save and exit**, **Exit without saving**, and **Cancel**.

A failed automatic or requested save cancels close and leaves the application open with retryable in-memory state. A graceful save/shutdown has a three-second storage timeout. Forced termination or power loss can still lose changes after the last acknowledged save.

Window-state persistence remains independent and may write `app.ron` during a normal close.

## 7. Session management

The initial session-management UI provides:

- session selector;
- **New session**;
- **Save now**;
- **Rename**;
- **Reset statistics**;
- **Delete session**;
- **Delete all saved sessions**;
- **Autosave sessions**.

Saved sessions are ordered by most recently selected/opened. Rows show enough metadata to distinguish duplicate-looking untitled entries, including update time and keyboard name where available.

### 7.1 Rename

Names are optional Unicode strings of at most 80 UTF-8 bytes after trimming. An empty name becomes unnamed. Nonempty names must not duplicate another saved session name. Rename changes the in-memory working session and marks it dirty; it does not bypass normal save behavior.

### 7.2 Reset

Reset requires confirmation, clears all metric aggregates, accumulated capture duration, and in-flight context, and leaves the session identity, optional name, and remembered setup intact. It is an ordinary dirty change. Autosave may subsequently persist it; otherwise it waits for manual save or a save boundary.

### 7.3 Delete

Deletion is explicit and immediate even when autosave is disabled. It requires confirmation and transactionally removes session metadata and metric snapshots.

Deleting the current saved session warns that both its saved copy and any unsaved changes will be lost. After acknowledgement, evtap creates a new untitled in-memory session rather than loading an older session.

Deleting all saved sessions removes every database session. The working session becomes a new untitled in-memory session. SQLite sidecars are checkpointed and storage pages reclaimed where practical. The UI describes filesystem deletion as best effort rather than guaranteed forensic erasure.

There is no retention cleanup. Saved sessions remain until explicit deletion.

## 8. Privacy model

### 8.1 Durable metric state

Durable state is the minimum cumulative state needed for a metric to continue producing the same aggregate report:

| Metric | Durable state |
| --- | --- |
| Total presses | count |
| Key usage | physical code, display label, count |
| Corrections | deletion-label counts and correction-pair counts |
| Flight time | label, total duration, sample count |
| Dwell time | label, total duration, sample count |
| Bigram speed | pair labels, total duration, sample count |

Sensitive aggregate labels remain local and unencrypted.

### 8.2 In-flight context

In-flight context interprets unfinished or adjacent events and is never written to disk:

- raw `KeyEvent` values;
- event order and per-event timestamps;
- evdev paths;
- currently pressed keys;
- unfinished press/release observations;
- previous-event timing context;
- recent ordered correction context;
- pending correction inference;
- listener channel contents.

In-flight context belongs only to the loaded working session while capture is active. It is cleared at every capture/session/process boundary. A restored metric continues its aggregate totals, but its first new event starts fresh context.

The storage subsystem accepts only validated session metadata and versioned metric snapshots. It has no raw-input command or event API.

### 8.3 Disclosure

Before the first analytics write, explain that:

- evtap reads global keyboard input while listening;
- character, bigram, correction, key-usage, count, and timing aggregates can be sensitive;
- the database is local and unencrypted;
- raw event sequences and in-flight context are not stored;
- files readable by the user, privileged processes, backups, or snapshots can expose saved data;
- evtap performs no telemetry, synchronization, or network transfer.

### 8.4 Filesystem policy

Expected Linux modes remain:

- application configuration and data directories: `0700`;
- `settings.json` and `evtap.sqlite3`: `0600`.

The application does not impose special symlink rejection. Normal operating-system path resolution and I/O errors apply. These permissions protect mainly against other unprivileged local users; they do not protect against root or code running as the same user.

## 9. Storage domains

| Domain | Owner | Location | Contents |
| --- | --- | --- | --- |
| Window state | eframe | `$XDG_DATA_HOME/evtap/app.ron` | Window size, position, and maximized state only |
| Preferences | evtap | `$XDG_CONFIG_HOME/evtap/settings.json` | Disclosure acknowledgement, autosave, last session ID, fallback XKB preferences |
| Session analytics | evtap | `$XDG_DATA_HOME/evtap/evtap.sqlite3` | Saved session metadata and aggregate metric snapshots |

Use the usual `~/.config` and `~/.local/share` fallbacks. `app.ron` must not contain widget memory, evtap preferences, or analytics.

Settings writes remain schema-versioned, validated, size-limited, synchronized, private, and atomic through a same-directory temporary file, flush, rename, and directory synchronization where supported.

Logical settings schema:

```json
{
  "schema_version": 2,
  "storage": {
    "disclosure_acknowledged": false,
    "autosave_enabled": false,
    "last_session_id": null
  },
  "keyboard": {
    "model": "",
    "layout": "",
    "variant": ""
  }
}
```

Unknown fields are tolerated. Unsupported schema versions produce a non-destructive generic error and are not overwritten automatically.

## 10. SQLite design

evtap uses bundled SQLite through `rusqlite` and configures:

- read-write/create flags only when a write is requested;
- evtap `application_id`;
- foreign keys;
- WAL journal mode;
- `synchronous = NORMAL`;
- `secure_delete = ON`;
- bounded busy timeout;
- immediate transactions for saves and deletion.

### 10.1 Schema

evtap 0.2 uses `user_version = 2`:

```sql
CREATE TABLE sessions (
    id                    INTEGER PRIMARY KEY,
    name                  TEXT,
    created_at_ms         INTEGER NOT NULL,
    updated_at_ms         INTEGER NOT NULL,
    last_opened_at_ms     INTEGER NOT NULL,
    captured_duration_ns  INTEGER NOT NULL DEFAULT 0
                             CHECK (captured_duration_ns >= 0),
    application_version   TEXT NOT NULL,
    keyboard_name         TEXT,
    xkb_model             TEXT NOT NULL,
    xkb_layout            TEXT NOT NULL,
    xkb_variant           TEXT NOT NULL,
    CHECK (name IS NULL OR length(name) > 0)
);

CREATE UNIQUE INDEX sessions_unique_name
    ON sessions(name)
    WHERE name IS NOT NULL;

CREATE INDEX sessions_recently_opened
    ON sessions(last_opened_at_ms DESC, id DESC);

CREATE TABLE metric_snapshots (
    session_id             INTEGER NOT NULL
                               REFERENCES sessions(id) ON DELETE CASCADE,
    metric_id              TEXT NOT NULL,
    metric_schema_version  INTEGER NOT NULL
                               CHECK (metric_schema_version > 0),
    payload_json           TEXT NOT NULL,
    updated_at_ms          INTEGER NOT NULL,
    PRIMARY KEY (session_id, metric_id)
) WITHOUT ROWID;

PRAGMA user_version = 2;
```

All timestamps are UTC Unix-epoch milliseconds. Capture duration uses checked signed nanoseconds. Session IDs are local implementation details.

### 10.2 Save transaction

One immediate transaction:

1. validates and serializes all known metric snapshots;
2. inserts a new session or updates the matching session;
3. upserts every known metric snapshot;
4. preserves unknown existing metric rows;
5. updates metadata and `updated_at_ms`;
6. commits;
7. acknowledges the exact dirty generation and assigned ID.

No partial multi-metric state may become visible. Serialization finishes before opening the transaction. At most one save is in flight. A save acknowledgement applies only to the matching dirty generation, and retrying after failure serializes the latest in-memory state.

### 10.3 Loading and listing

Listing reads lightweight session metadata ordered by `last_opened_at_ms`. Loading one session reads all its metric snapshots into a fresh default metric registry. Unknown, malformed, duplicated, or unsupported metric snapshots are isolated; valid metrics still restore.

Selecting a session updates its `last_opened_at_ms` as an explicit metadata operation and updates `last_session_id` in settings.

### 10.4 Incompatible databases

Empty databases initialize directly to schema version 2. A nonempty database with the wrong application identity or any unsupported schema version is left unchanged and produces a generic error containing the database path, for example:

```text
The evtap database at … uses an incompatible schema. Move or delete it to start fresh.
```

Unsupported schema versions have no automatic migration path. Future schema changes require an explicit migration design and compatibility tests.

## 11. Metric snapshot contract

Each metric uses a stable metric ID and an independently versioned snapshot format. Payloads are deterministic and fully validated before they can mutate metric state.

Snapshot limits are:

- at most 16 MiB per metric payload;
- at most 100,000 dimensions per metric;
- at most 256 UTF-8 bytes per dimension string;
- no duplicate dimensions;
- no zero-sample duration entries;
- checked counts and duration totals;
- exact metric ID and supported payload version.

Arrays are canonically ordered. Rounded averages are never stored. Unknown existing metric rows survive a save so temporarily removed metrics are not silently erased.

## 12. Error behavior

- Settings and database errors preserve existing files.
- Corrupt, unidentified, and incompatible databases are not replaced, truncated, renamed, or silently reset.
- Failed saves leave working state dirty and retryable without stopping capture or clearing metrics.
- Failed delete operations leave the session visible.
- Failed automatic save blocks the requested switch or close.
- A process kill may lose unsaved state but a committed SQLite transaction is all-or-nothing.
- Logs never contain captured labels, metric payloads, or raw input.
