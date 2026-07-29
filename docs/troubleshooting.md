# Troubleshooting

Run evtap from a terminal to retain diagnostic logs:

```sh
RUST_LOG=evtap=debug cargo run --release --locked
```

For a downloaded release, replace the Cargo command with the path to the `evtap` binary.

## No readable keyboards

If the UI reports that input devices could not be inspected, verify device ownership and your groups:

```sh
ls -l /dev/input/event*
id
```

After adding your account to the `input` group, log out of the entire desktop session and back in. Opening a new terminal is not enough. Confirm that `id` lists the group afterward.

For temporary diagnosis, an administrator can grant a read ACL to a specific event device:

```sh
sudo setfacl -m "u:$USER:r" /dev/input/eventN
```

Replace `eventN` with the keyboard interface. This ACL may disappear after reboot or reconnection, and choosing the wrong interface will not help. Device-specific udev rules are more durable but are distribution- and hardware-specific.

Do not solve desktop permission problems by routinely running evtap as root.

## A device is missing

Linux hardware can expose several event interfaces, including separate media-key and pointer interfaces. evtap lists only readable interfaces that report a basic keyboard key set. Inspect kernel descriptions with:

```sh
grep -E '^(N: Name|H: Handlers|B: EV|B: KEY)' /proc/bus/input/devices
```

Reconnect the keyboard and choose **Rescan**. Check the terminal log for an open error associated with its `/dev/input/event*` path.

## Characters or bigrams are wrong

Select the XKB model, layout, and variant that match the active typing layout. evtap does not currently follow desktop layout switching automatically.

If `localectl` is unavailable or returns an error, evtap uses a small fallback list. This can make uncommon layouts or variants unavailable. The terminal log will include the `localectl` failure.

The physical **Key Usage** table remains based on key identity; only text-oriented metric labels depend on XKB interpretation.

## Metrics remain empty

Confirm that the UI says **Listening**, then type on the selected physical keyboard. Input from a different keyboard device is intentionally ignored.

Some timing reports need more context:

- flight time requires a release followed by a text-producing press within two seconds;
- dwell time requires a complete text-key press and release;
- bigrams require three samples of the same pair before appearing;
- correction signals require text retained in the short recent-history buffer before Backspace.

Function and modifier keys do not produce text-oriented samples.

## The listener stops unexpectedly

A disconnect, suspend/resume transition, permission change, or kernel read failure can close the event stream. evtap displays the listener error. Reconnect the device, choose **Rescan**, select it again, and restart listening.

## A saved session does not load

Inspect the session storage status and error shown in the Session panel. evtap loads only the ID recorded as the last-selected saved session. If that session was deleted, evtap deliberately starts a new **Untitled session** rather than choosing an unexpected older session.

Expected paths are:

```text
$XDG_CONFIG_HOME/evtap/settings.json
$XDG_DATA_HOME/evtap/evtap.sqlite3
```

with `~/.config/evtap` and `~/.local/share/evtap` fallbacks. The application directories should be accessible only to your user, and settings/database files should be private:

```sh
ls -ld ~/.config/evtap ~/.local/share/evtap
ls -l ~/.config/evtap/settings.json ~/.local/share/evtap/evtap.sqlite3*
```

Do not replace a storage error by deleting files until deciding whether the existing saved sessions matter. evtap deliberately refuses to overwrite corrupt databases, databases belonging to another application, and incompatible schema versions. The unreleased experimental persistence schema is not migrated; move or delete that development database manually. Keep a private copy before investigating. Never attach a real analytics database to a public issue because aggregate labels are sensitive.

A restored session is always paused. The remembered keyboard name may preselect one unique match, but device paths are never stored. Select any intended readable keyboard before restarting capture.

## Settings cannot be loaded or changed

Malformed or unsupported-version `settings.json` files cause evtap to use safe defaults without overwriting the existing file. The UI then refuses preference changes until the file is fixed or deliberately moved aside.

If preserving it matters, copy it privately before editing. Otherwise, with evtap stopped:

```sh
mv ~/.config/evtap/settings.json ~/.config/evtap/settings.json.backup
```

Restarting uses defaults. Treat the backup as private because it records the storage-disclosure acknowledgement, autosave, last-selected session ID, and keyboard preferences.

## Saves fail or remain dirty

A storage failure never stops capture or clears in-memory metrics. Check free space, ownership, permissions, read-only filesystem state, and whether another process has replaced the database path. Use **Retry storage operation** after correcting the cause; retry serializes a fresh snapshot of the latest aggregates.

With autosave enabled, normal close waits only for a bounded final save. With autosave disabled, a dirty close offers Save, Exit without saving, or Cancel. A timeout or forced process termination can lose changes after the most recent acknowledged save. Do not assume the **Saved** label until the worker has acknowledged the current generation.

## Deletion appears incomplete

Individual session deletion is transactional and immediate even when autosave is off. **Delete all saved sessions** closes SQLite and removes the database plus `-wal`, `-shm`, and rollback-journal sidecars. There is no automatic retention policy.

The UI reports filesystem failures, but evtap cannot erase copies held by backups, filesystem snapshots, SSD remapping, or other storage layers. Stop evtap before manually inspecting or moving SQLite files, and keep the database and sidecars together.

## The window does not open

eframe/winit supports X11 and Wayland in this build. Verify that the process has access to the current graphical session and that either `DISPLAY` or `WAYLAND_DISPLAY` is set:

```sh
printf 'DISPLAY=%s\nWAYLAND_DISPLAY=%s\n' "$DISPLAY" "$WAYLAND_DISPLAY"
```

Running the application through `sudo` commonly loses both display-session access and the correct user environment, which is another reason not to run the GUI as root.

## Reporting a problem

Include:

- Linux distribution and version;
- desktop environment and X11 or Wayland session;
- evtap version or Git commit;
- keyboard model and connection type;
- selected XKB model/layout/variant;
- relevant logs with private paths or system details redacted.

Never include captured text, passwords, or other sensitive input in an issue.
