# Aggregate persistence specification

**Status:** Accepted  
**Target:** evtap 0.2.0  
**Last updated:** 2026-07-28

This document specifies local persistence for evtap. It is a design artifact, not a description of behavior available in 0.1.x.

## 1. Summary

evtap 0.2.0 will optionally persist aggregate session analytics in a local SQLite database. Persistence is disabled by default and requires an explicit privacy disclosure. The database will never contain raw key events, event order, event timestamps, recent text history, or pressed-key state.

The first persistence release will support:

- crash-safe checkpoints of the active session;
- resuming aggregate state after restarting evtap;
- finalized session history;
- per-session detail views using existing metric reports;
- configurable retention;
- individual deletion and complete analytics deletion;
- remembered XKB preferences in a separate settings file.

Cross-session charts, export, synchronization, accounts, telemetry, and encryption are not part of this release.

## 2. Motivation

Session-only analysis makes it impossible to inspect earlier sessions, compare observations over time, or recover from an application restart. Persistence should add those capabilities without turning evtap into a raw key logger or coupling storage to Linux evdev events.

The design must preserve these architectural properties:

1. Capture produces normalized `KeyEvent`s.
2. Metrics consume events and own their aggregate algorithms.
3. Persistence receives only versioned aggregate snapshots.
4. Rendering consumes UI-neutral metric reports.
5. Raw input never crosses into the storage subsystem.

## 3. Accepted product decisions

1. Persistence is opt-in and disabled by default.
2. Once enabled, all aggregate metrics are persisted, including character labels, bigrams, deleted-character counts, and correction pairs.
3. Raw events and transient metric context are never persisted.
4. An active session resumes across process restarts.
5. Stopping capture pauses a session; finishing archives it; discarding deletes it.
6. Completed sessions are retained for 90 days by default.
7. Storage is an unencrypted SQLite database protected by restrictive filesystem permissions.
8. The first history UI provides a session list and per-session details, not trends.
9. XKB model, layout, and variant preferences are remembered; evdev paths are not.
10. Persistence is a 0.2.0 feature and changes the documented 0.1.x privacy contract.

## 4. Terminology

- **Capture run:** one continuous interval between starting and stopping the listener.
- **Active session:** the mutable analysis session receiving zero or more capture runs.
- **Completed session:** an immutable session finalized by the user.
- **Checkpoint:** an atomic database update containing all durable state for the active session.
- **Aggregate state:** counts, exact duration totals, sample counts, and their dimensions.
- **Transient state:** context needed only to interpret adjacent events, such as pressed keys or the previous character.
- **Dirty generation:** a monotonically increasing in-memory revision indicating changes not yet acknowledged by storage.

## 5. Goals

### 5.1 Functional goals

- Preserve the current session's aggregate values across restart.
- Keep completed sessions until retention or explicit deletion removes them.
- Render restored sessions through the existing generic metric-report UI.
- Keep capture responsive while storage performs blocking I/O.
- Make storage health and unsaved state visible.
- Permit metric implementations and payload schemas to evolve independently.
- Preserve unknown metric snapshots rather than silently deleting them.

### 5.2 Privacy and safety goals

- Make persistence an informed opt-in.
- Prevent raw events from entering the storage API by architecture.
- Store the minimum context needed to reproduce aggregate reports.
- Never log metric payloads, labels, or SQL parameter values containing analytics.
- Use private directories and files.
- Offer understandable retention and deletion controls.
- Fail without corrupting an existing database or interrupting capture.

## 6. Non-goals

The first persistence release will not provide:

- raw key-event or ordered text storage;
- event replay;
- import or export;
- cross-session trends, comparisons, merging, or lifetime totals;
- cloud or local-network synchronization;
- user accounts;
- telemetry or crash-report uploads;
- encryption at rest;
- operating-system keyring integration;
- a stable public database or payload format;
- concurrent access by multiple evtap processes;
- retention based on individual metric rows;
- physical keyboard heatmap history.

## 7. Privacy model

### 7.1 Data classification

#### Class A: never persisted

- `KeyEvent` values;
- event sequence or per-event timestamp;
- evdev event paths;
- currently pressed keys;
- previous press or release timestamps;
- the correction metric's recent-text queue;
- a pending deleted-to-typed inference;
- listener channel contents;
- logs of physical keys or produced text.

#### Class B: persisted aggregate analytics

- total physical press count;
- physical key code, display label, and count;
- deleted text labels and counts;
- inferred deleted-to-typed labels and counts;
- character labels with exact duration totals and sample counts;
- character-pair labels with exact duration totals and sample counts.

Class B remains sensitive. Character frequencies, bigrams, and corrections can reveal a language, habits, or fragments of activity even though they cannot reconstruct an ordered input stream reliably.

#### Class C: persisted session metadata

- creation, update, and completion times;
- accumulated capture duration;
- evtap version;
- keyboard display name, when available;
- XKB model, layout, and variant;
- metric IDs and schema versions.

The keyboard display name may identify hardware. Device paths, serial numbers, vendor/product IDs, and stable hardware fingerprints are excluded.

#### Class D: non-analytics preferences

- persistence enabled state;
- retention policy;
- last selected XKB model, layout, and variant.

Class D is stored separately so evtap can remember that persistence was disabled while retaining an existing database.

### 7.2 Opt-in disclosure

Before creating the analytics database, the UI must explain:

- aggregate character labels, bigrams, and correction pairs will be written to disk;
- data is local and unencrypted;
- no raw event sequence is written;
- the default retention period is 90 days;
- anyone who can read the user's files may read the database;
- filesystem backups or snapshots may retain deleted data.

Enabling persistence requires an affirmative action. It must not be bundled with starting capture or accepting input permissions.

### 7.3 Filesystem protection

On Linux:

- the data and configuration directories are created with mode `0700`;
- settings and database files are created with mode `0600`;
- SQLite sidecar files remain protected by the containing `0700` directory;
- permissions are checked after opening and tightened when owned by the current user;
- unsafe ownership or an inability to establish private access produces a storage error.

These controls protect primarily against other unprivileged local users. They do not protect against root, malware running as the same user, memory inspection, backups, or a compromised machine.

### 7.4 Encryption decision

The initial implementation will not encrypt the database. Encryption would require a defensible key-storage and recovery design; embedding a key or deriving one without user authentication would provide misleading protection. Encryption can be specified separately if the threat model later requires it.

### 7.5 Deletion limits

`PRAGMA secure_delete = ON` will be enabled before analytics are written. Deletion operations will checkpoint the WAL and reclaim pages where practical. Complete deletion will close SQLite and remove the database, WAL, and shared-memory files.

The UI and documentation must describe deletion as a best effort, not guaranteed forensic erasure. SSD behavior, filesystem snapshots, swap, backups, and external copies remain outside evtap's control.

## 8. Session lifecycle

### 8.1 States

The application-level session state machine is:

```text
Empty
  └─ Start listening ─▶ ActiveListening

ActiveListening
  ├─ Stop listening ─▶ ActivePaused
  ├─ listener failure ─▶ ActivePaused + capture error
  └─ Finish requested ─▶ StoppingForFinish ─▶ Finishing ─▶ Empty

ActivePaused
  ├─ Start listening ─▶ ActiveListening
  ├─ Finish session ─▶ Finishing ─▶ Empty
  └─ Discard session ─▶ Discarding ─▶ Empty
```

Storage status (`Disabled`, `Loading`, `Saved`, `Dirty`, `Saving`, or `Failed`) is tracked separately from capture and session state.

### 8.2 Session creation

- An empty in-memory session exists without creating a database row.
- The configuration becomes fixed when the first capture run starts.
- With persistence enabled, the active database row is created at first start.
- Starting and stopping repeatedly continues the same active session.
- Only one active session may exist in a database.

### 8.3 Finishing

- Finishing while listening first performs a graceful listener stop.
- The latest complete aggregate snapshot and `completed` status are committed in one transaction.
- A session does not appear as completed until that transaction succeeds.
- After acknowledgement, the application creates a new empty in-memory session.
- A failed finish leaves the session active and dirty, with retry available.

### 8.4 Discarding

- Discard is unavailable while the listener is stopping.
- Discarding an active persisted session deletes its database row and metric snapshots transactionally.
- In-memory aggregate and transient state are reset only after storage acknowledges deletion.
- When persistence is disabled, discard is equivalent to the old reset behavior.
- The action requires confirmation whenever the session contains samples.

### 8.5 Restart and recovery

On startup with persistence enabled:

1. open and migrate the database;
2. apply retention to completed sessions;
3. load the single active session, if present;
4. instantiate metrics from the registry;
5. restore durable aggregate state;
6. clear all transient state;
7. display the session as resumed and paused.

Capture never starts automatically. No timing or correction sample may bridge a process restart.

### 8.6 Configuration changes

- Device and XKB controls remain editable while the session is empty.
- After the first capture run, changing the device, model, layout, or variant requires finishing or discarding the session.
- The UI explains that mixing configurations would make persisted dimensions ambiguous.
- Device paths are used only to open the current listener and are never checkpointed.

### 8.7 Enabling or disabling persistence mid-session

Enabling persistence while a non-empty in-memory session exists offers:

- **Save current session:** create an active row and checkpoint current aggregates.
- **Start persistence with a new session:** discard current in-memory aggregates after confirmation.
- **Cancel.**

Disabling persistence requires capture to be stopped and offers:

- **Finish current session and keep history:** finalize the latest snapshot, then disable future persistence.
- **Delete all stored analytics and disable.**
- **Cancel.**

A persisted active row must not be abandoned when persistence is disabled.

## 9. User interface requirements

### 9.1 Session controls

When persistence is enabled, replace the ambiguous `Reset session` action with:

- **Finish session**;
- **Discard session**;
- **New session**, shown only when no active data exists or after finishing;
- **History**.

Start and Stop continue to control capture only.

When persistence is disabled, retain a clearly named **Discard current session** action and no history controls.

### 9.2 Storage status

The current-session view shows one of:

- `Persistence off`
- `Loading saved session…`
- `Saved`
- `Unsaved changes`
- `Saving…`
- `Could not save — Retry`

The UI must not display `Saved` until the worker acknowledges the corresponding dirty generation.

### 9.3 Settings

The settings UI provides:

- persistence enable/disable;
- retention: 30, 90, 365 days, or forever;
- storage location;
- delete all stored analytics;
- currently used disk space, if inexpensive to calculate;
- the privacy disclosure and a link to the local privacy documentation.

### 9.4 History list

Each completed-session row shows:

- local start date and time;
- accumulated capture duration;
- total physical presses;
- keyboard display name, if stored;
- XKB layout and variant;
- completion state;
- open and delete actions.

Sort newest first. Empty history has an explanatory empty state.

### 9.5 Session detail

Opening a completed session shows:

- immutable session metadata;
- all restorable metric reports through the generic renderer;
- a message for unknown or unsupported metric versions;
- delete action with confirmation.

Viewing history must not replace or mutate the active metric instances.

## 10. Storage locations and settings

### 10.1 State ownership

Persistence is split into three deliberately separate domains:

| Domain | Owner | Location | Contents |
| --- | --- | --- | --- |
| Window state | eframe | `app.ron` | Native window size, position, and maximized state only |
| Application settings | evtap | `settings.json` | Persistence consent, retention, and XKB preferences |
| Aggregate analytics | evtap | `evtap.sqlite3` | Active and completed session metadata and metric snapshots |

eframe's `persistence` feature will be enabled for native window restoration. Keep `NativeOptions::persist_window` enabled and override `App::persist_egui_memory` to return `false`. Arbitrary egui widget memory is not persisted in the initial release.

evtap must not use `App::save` or eframe's string key/value storage for privacy-sensitive application settings or analytics. The native eframe store rewrites a best-effort RON file, reports failures through logs, and does not provide the atomic writes, permission guarantees, migrations, transactional updates, or application-visible acknowledgements required by this specification.

`app.ron` may exist while analytics persistence is disabled. It must contain no evtap settings, metric values, captured labels, or raw input. Failure to save window state must not affect settings, analytics state, or storage status.

### 10.2 Paths and settings format

Use an XDG-aware project-directory library for evtap-owned files. Configure eframe's application ID or persistence path so its native state uses the same application data directory without sharing a file with evtap storage.

On Linux, expected paths are:

```text
$XDG_CONFIG_HOME/evtap/settings.json
$XDG_DATA_HOME/evtap/app.ron
$XDG_DATA_HOME/evtap/evtap.sqlite3
```

with fallbacks:

```text
~/.config/evtap/settings.json
~/.local/share/evtap/app.ron
~/.local/share/evtap/evtap.sqlite3
```

The settings file has an independent schema version and is written atomically using a temporary file in the same directory, flush, rename, and directory synchronization where supported.

Initial logical settings format:

```json
{
  "schema_version": 1,
  "persistence": {
    "enabled": false,
    "retention_days": 90
  },
  "keyboard": {
    "model": "",
    "layout": "",
    "variant": ""
  }
}
```

`null` retention means keep completed sessions until explicitly deleted. Unknown settings fields are tolerated. Unsupported newer schema versions produce a non-destructive warning and privacy-preserving defaults; they are not overwritten automatically.

The analytics database must not be created merely to remember settings.

## 11. SQLite design

### 11.1 Library direction

Use `rusqlite` with its bundled SQLite feature to avoid adding a system SQLite dependency and to keep release behavior consistent. Select the newest release that passes the Rust 1.92 MSRV job; do not raise the project MSRV implicitly. Use `serde` and `serde_json` for versioned metric payloads and an XDG-aware directory crate for paths.

Schema migrations are a small ordered list of embedded SQL migrations driven by `PRAGMA user_version`. A separate migration framework is not required initially. All migrations are forward-only and transactional.

### 11.2 Connection configuration

The storage worker owns one read/write connection configured with:

- read-write/create flags;
- `PRAGMA application_id` set to evtap's assigned constant;
- `PRAGMA foreign_keys = ON`;
- `PRAGMA journal_mode = WAL`;
- `PRAGMA synchronous = NORMAL`;
- `PRAGMA secure_delete = ON`;
- a bounded busy timeout;
- immediate transactions for checkpoints and lifecycle changes.

The chosen settings and SQLite version must be covered by integration tests. Opening a database with a newer `user_version` disables writes and shows an error rather than attempting a downgrade.

### 11.3 Schema version 1

```sql
CREATE TABLE sessions (
    id                    INTEGER PRIMARY KEY,
    status                TEXT NOT NULL CHECK (status IN ('active', 'completed')),
    created_at_ms         INTEGER NOT NULL,
    updated_at_ms         INTEGER NOT NULL,
    completed_at_ms       INTEGER,
    captured_duration_ns  INTEGER NOT NULL DEFAULT 0
                             CHECK (captured_duration_ns >= 0),
    application_version   TEXT NOT NULL,
    keyboard_name         TEXT,
    xkb_model             TEXT NOT NULL,
    xkb_layout            TEXT NOT NULL,
    xkb_variant           TEXT NOT NULL,
    CHECK (
        (status = 'active' AND completed_at_ms IS NULL) OR
        (status = 'completed' AND completed_at_ms IS NOT NULL)
    )
);

CREATE UNIQUE INDEX sessions_one_active
    ON sessions(status)
    WHERE status = 'active';

CREATE INDEX sessions_completed_at
    ON sessions(completed_at_ms DESC)
    WHERE status = 'completed';

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
```

All timestamps are UTC Unix-epoch milliseconds. Capture duration is exact accumulated nanoseconds represented as a checked signed 64-bit integer. Session IDs are local database identities and are not a public interchange format.

### 11.4 Atomic checkpoint

One immediate transaction performs:

1. insert or update the active session metadata;
2. upsert every known metric snapshot;
3. leave unknown existing metric rows untouched;
4. update the session's `updated_at_ms`;
5. commit;
6. acknowledge the saved dirty generation.

A partial metric checkpoint must never become visible. Serialization is completed before beginning the transaction so serialization errors cannot leave a transaction open.

### 11.5 Finalization

Finalization uses the same checkpoint transaction and additionally sets:

- `status = 'completed'`;
- `completed_at_ms`;
- final capture duration.

The unique active-session index then permits a new active row.

## 12. Metric snapshot contract

### 12.1 Interface direction

The metric boundary gains object-safe operations conceptually equivalent to:

```rust
fn has_data(&self) -> bool;
fn snapshot(&self) -> Result<MetricSnapshot, SnapshotError>;
fn restore(&mut self, snapshot: &MetricSnapshot) -> Result<(), RestoreError>;
```

`MetricSnapshot` contains:

```text
metric_id
metric_schema_version
payload bytes or JSON value
```

The stable descriptor ID remains the database key. Each metric owns its payload version and validates all restored values before mutating itself. Restore is all-or-nothing.

### 12.2 Durable versus transient state

| Metric | Durable | Never persisted |
| --- | --- | --- |
| Total presses | count | none |
| Key usage | physical code, label, count | cached application key objects beyond those dimensions |
| Corrections | deletion and correction counts | recent history, pending deletion |
| Flight time | label, total duration, samples | last release timestamp |
| Dwell time | label, total duration, samples | pressed-key map and press timestamps |
| Bigram speed | pair, total duration, samples | previous press text and timestamp |

### 12.3 Initial payload shapes

Version 1 payloads use deterministic arrays rather than JSON object keys for tuple or user-derived dimensions.

```json
{"count": 123}
```

```json
{
  "keys": [
    {"code": 30, "label": "A", "count": 12}
  ]
}
```

```json
{
  "deletions": [
    {"text": "x", "count": 3}
  ],
  "corrections": [
    {"deleted": "x", "typed": "y", "count": 2}
  ]
}
```

```json
{
  "entries": [
    {"text": "a", "total_ns": 240000000, "samples": 4}
  ]
}
```

```json
{
  "pairs": [
    {"first": "t", "second": "h", "total_ns": 300000000, "samples": 5}
  ]
}
```

Arrays are sorted canonically before serialization to make tests and diagnostics deterministic. A duration entry with zero samples is invalid. Rounded averages are never stored.

### 12.4 Validation limits

Restoration rejects, without partially applying:

- payloads larger than 16 MiB per metric;
- more than 100,000 dimension entries per metric;
- dimension strings larger than 256 UTF-8 bytes;
- zero-sample duration entries;
- overflowing counts or durations;
- duplicate dimensions;
- a metric ID mismatch;
- unsupported payload versions.

A rejected historical metric is shown as unavailable. The original database row remains untouched.

### 12.5 Evolution

- Database schema and metric payload versions evolve independently.
- Metric code must read every payload version it still claims to support.
- Payload conversion occurs in memory.
- Completed rows are not eagerly rewritten merely because they were viewed.
- Active state may be rewritten in the newest format on its next successful checkpoint.
- Removed or unknown metrics remain in the database and count toward retention/deletion.
- Semantic algorithm changes increment the metric payload version when restored values would otherwise be misinterpreted.

## 13. Storage worker and checkpoint protocol

### 13.1 Isolation

A dedicated worker thread owns the SQLite connection. The egui thread communicates through command and event channels following the scanner/listener pattern and uses `WakeSignal` for repaint requests.

The storage module operates on `SessionSnapshot` and storage commands. It must not import or expose `KeyEvent`.

### 13.2 Dirty tracking

- The app increments a dirty generation after processing input that may affect metrics or capture duration.
- At most one checkpoint is in flight.
- While one is in flight, additional events advance the generation but do not enqueue more snapshots.
- An acknowledgement marks only its generation saved.
- If the current generation is newer, the UI remains dirty and a later checkpoint is scheduled.

### 13.3 Schedule

Checkpoint:

- after 30 seconds of dirty active capture;
- immediately after Stop is acknowledged;
- as part of Finish;
- before disabling persistence;
- during graceful application exit.

Do not write per key event. A process or power failure may lose changes after the most recently committed checkpoint, bounded during continuous capture to approximately 30 seconds.

### 13.4 Shutdown

On graceful exit:

1. stop capture;
2. serialize the newest aggregate state;
3. request final checkpoint and storage shutdown;
4. wait for acknowledgement with a finite timeout;
5. log only success or a payload-free error.

A timed-out exit may lose unsaved changes but must not hang indefinitely. The exact timeout is an implementation constant covered by tests.

### 13.5 Errors

- Storage failure never stops capture or clears in-memory metrics.
- Failed writes leave the generation dirty.
- Retry uses a fresh snapshot of the latest generation.
- Error messages include operation and path where safe, never SQL values or serialized payloads.
- Corrupt databases are not replaced, truncated, or automatically renamed.
- A migration failure rolls back and opens no writable storage session.

## 14. Retention and deletion

### 14.1 Retention

Available policies:

- 30 days;
- 90 days, default;
- 365 days;
- forever.

Retention uses `completed_at_ms`, never `created_at_ms`, and never deletes an active session. It runs:

- after opening and migrating the database;
- after finalizing a session;
- immediately after shortening the retention setting.

The history UI refreshes only after deletion acknowledgement.

### 14.2 Individual deletion

Deleting a completed session removes the session and cascades its metric snapshots in one transaction. A WAL checkpoint is requested after deletion. Failure leaves the history item visible and retryable.

### 14.3 Delete all analytics

The worker:

1. stops accepting writes;
2. closes the SQLite connection;
3. removes the database, `-wal`, and `-shm` files;
4. reports any path it could not remove;
5. creates a fresh empty database only if persistence remains enabled.

The settings file is retained unless the user separately resets preferences.

## 15. History loading

- History-list queries return metadata and total presses without loading every payload.
- Opening a detail view loads all snapshots for that session.
- The app creates a separate default metric registry and restores into those instances.
- The active registry is never replaced or mutated.
- Unknown metric rows produce an `Unsupported metric: <id>` entry.
- Known metrics with unsupported versions produce a version-specific message.
- A malformed snapshot affects only that metric's detail section.

The total press count may be denormalized into a future schema if list performance requires it. Version 1 can read the small total-presses payload for each displayed page.

History is paginated; the initial page size is 50 sessions.

## 16. Configuration and time handling

- Database timestamps are UTC Unix milliseconds.
- UI timestamps are converted to the local timezone only for display.
- Clock changes can make wall-clock session timestamps non-monotonic; capture duration is accumulated from monotonic process time and persisted separately.
- An active capture segment's elapsed duration is included in each checkpoint.
- No timing metric bridges process restart, even if the session resumes.
- Time-formatting dependencies must pass Rust 1.92 before adoption.

## 17. Dependency constraints

Anticipated direct dependencies and feature changes:

- eframe's `persistence` feature for native window state only;
- `rusqlite` with bundled SQLite;
- `serde` with derive;
- `serde_json`;
- an XDG/project-directory crate;
- a timezone-aware display-time crate if the standard library is insufficient.

Before adoption, each dependency must pass:

- Rust 1.92 build;
- RustSec audit;
- license compatibility review;
- default-feature review;
- Linux release build;
- transitive dependency inspection.

No dependency choice in this specification authorizes an MSRV increase.

## 18. Testing requirements

### 18.1 Metric snapshot tests

For every metric:

- deterministic version 1 serialization;
- aggregate round trip;
- reset after restore;
- rejection of wrong ID/version;
- rejection of malformed and overflowing values;
- confirmation that transient state is absent;
- resumed processing adds to restored aggregates correctly;
- no cross-restart timing or correction inference.

### 18.2 Database tests

Using private temporary directories:

- create schema from an empty path;
- apply every migration from each earlier version;
- reject a newer schema version;
- reject wrong `application_id` without modifying it;
- enforce one active session;
- atomic multi-metric checkpoint;
- rollback on injected failure;
- finalize and create a new active session;
- cascade individual deletion;
- complete database-file deletion;
- retention boundaries at exactly 30, 90, and 365 days;
- preserve active sessions during retention;
- preserve unknown metric rows during checkpoints;
- WAL reopen after simulated unclean shutdown;
- file and directory permissions on Linux.

### 18.3 Worker tests

- dirty-generation acknowledgement;
- events arriving during an in-flight checkpoint;
- retry after write failure;
- finish waits for latest generation;
- discard waits for storage acknowledgement;
- bounded graceful shutdown;
- repaint wake on storage events;
- no capture interruption on storage failure.

### 18.4 UI tests

- disclosure before first enable;
- enable with existing in-memory samples;
- disable keep/delete choices;
- finish while listening;
- configuration lock after session starts;
- saved/dirty/saving/error status transitions;
- history empty/loading/error states;
- unsupported metric display;
- delete confirmations;
- retention setting behavior.

### 18.5 Privacy regression tests

- storage APIs have no raw-event command;
- active transient fields are not serialized;
- logs from success and failure paths contain no payloads;
- database schema has no event table or ordering column;
- settings contain no keyboard path or analytics;
- eframe widget-memory persistence is disabled;
- `app.ron` contains no evtap settings or analytics;
- synthetic event timestamps do not occur in snapshot payloads.

## 19. Acceptance criteria

The persistence feature is complete only when:

1. A fresh install creates no analytics database before opt-in.
2. Enabling persistence displays and records informed consent.
3. Aggregate results survive restart with identical reports.
4. Pressed-key, adjacency, and correction context do not survive restart.
5. Continuous capture checkpoints at the documented interval without UI stalls.
6. Stop, Finish, Discard, and restart obey the state model.
7. Completed sessions render independently from the active session.
8. Retention and deletion behave transactionally.
9. Storage errors remain visible and never silently discard in-memory state.
10. Unknown and malformed metrics do not make the whole history inaccessible.
11. Database/configuration permissions are private on Linux.
12. Privacy, metric, troubleshooting, README, roadmap, and changelog documentation are updated.
13. Format, strict Clippy, all tests, doctests, Rust 1.92, RustSec, LSP diagnostics, release build, and GUI smoke tests pass.
14. Manual testing covers restart, crash recovery, disk-full/write-denied behavior, retention, and deletion.
15. eframe restores native window state without persisting arbitrary egui widget memory.
16. A 0.2.0 prerelease is tested before the final tag.

## 20. Delivery milestones

### Milestone A — snapshot foundation

- Add owned, versioned snapshot types.
- Separate each metric's durable and transient state explicitly.
- Implement and test snapshot/restore for every metric.
- Add session aggregate and metadata types.

### Milestone B — settings and storage core

- Add XDG path resolution and atomic settings.
- Add SQLite connection hardening and schema migrations.
- Add transactional repository operations and retention.
- Verify MSRV, licenses, and RustSec.

### Milestone C — worker and recovery

- Add dedicated storage worker and event protocol.
- Add dirty generations and periodic checkpointing.
- Restore active sessions on startup.
- Add graceful flush and failure recovery.

### Milestone D — lifecycle UI

- Separate capture controls from session controls.
- Add enable/disable disclosure flows.
- Lock configuration after a session begins.
- Add storage status and retry behavior.

### Milestone E — history and deletion

- Add paginated history list and detail view.
- Add individual deletion, delete-all, and retention controls.
- Render unsupported metric states safely.

### Milestone F — release hardening

- Complete privacy and fault-injection tests.
- Update all user and contributor documentation.
- Perform manual lifecycle and recovery validation.
- Build and test a 0.2.0 prerelease.

Each milestone must pass the full repository checks before being committed. Storage must not be enabled in a release until milestones A through F are complete.

## 21. Deferred questions for later versions

The following require separate specifications:

- encryption and key management;
- aggregate export/import format;
- trend and comparison semantics;
- merging sessions or devices;
- lifetime aggregate caches;
- database backup and restore;
- multi-process access;
- opt-in persistence subsets by metric;
- physical keyboard geometry history;
- public library or plugin persistence contracts.
