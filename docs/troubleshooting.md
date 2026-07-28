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
