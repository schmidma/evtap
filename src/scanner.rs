use std::future::Future;

use camino::{Utf8Path, Utf8PathBuf};
use color_eyre::{Result, eyre::Context};
use evdev::{AttributeSetRef, Device, KeyCode};
use tokio::{
    fs,
    sync::mpsc::{self, Receiver, Sender},
};
use tracing::{info, warn};

use crate::wake::WakeSignal;

const CHANNEL_CAPACITY: usize = 1;
const REQUIRED_KEYBOARD_KEYS: [KeyCode; 4] = [
    KeyCode::KEY_A,
    KeyCode::KEY_Z,
    KeyCode::KEY_ENTER,
    KeyCode::KEY_SPACE,
];

#[derive(Clone, Debug, PartialEq)]
pub struct DeviceMetadata {
    pub path: Utf8PathBuf,
    pub name: String,
    pub physical_path: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceScanIssueKind {
    PermissionDenied,
    Unavailable,
    Unknown,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeviceScanIssue {
    pub path: String,
    pub message: String,
    pub kind: DeviceScanIssueKind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScanReport {
    pub devices: Vec<DeviceMetadata>,
    pub issues: Vec<DeviceScanIssue>,
}

pub enum Event {
    ScanFinished {
        result: std::result::Result<ScanReport, String>,
    },
}

enum Command {
    Scan,
}

pub struct ScannerHandle {
    events: Receiver<Event>,
    commands: Sender<Command>,
}

impl ScannerHandle {
    pub fn try_recv_event(&mut self) -> Option<Event> {
        self.events.try_recv().ok()
    }

    pub fn start_scan(&self) -> Result<()> {
        self.commands
            .try_send(Command::Scan)
            .wrap_err("failed to request an input-device scan")
    }
}

pub fn spawn(wake_signal: WakeSignal) -> ScannerHandle {
    spawn_with(wake_signal, scan_devices)
}

fn spawn_with<F, Fut>(wake_signal: WakeSignal, scan: F) -> ScannerHandle
where
    F: FnMut() -> Fut + Send + 'static,
    Fut: Future<Output = Result<ScanReport>> + Send + 'static,
{
    let (event_sender, event_receiver) = mpsc::channel(CHANNEL_CAPACITY);
    let (command_sender, command_receiver) = mpsc::channel(CHANNEL_CAPACITY);

    tokio::spawn(async move {
        Scanner {
            event_sender,
            command_receiver,
            wake_signal,
        }
        .run(scan)
        .await;
    });

    ScannerHandle {
        events: event_receiver,
        commands: command_sender,
    }
}

struct Scanner {
    event_sender: Sender<Event>,
    command_receiver: Receiver<Command>,
    wake_signal: WakeSignal,
}

impl Scanner {
    async fn run<F, Fut>(mut self, mut scan: F)
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<ScanReport>>,
    {
        while let Some(command) = self.command_receiver.recv().await {
            match command {
                Command::Scan => {
                    let result = scan().await.map_err(|error| format!("{error:#}"));
                    if self
                        .event_sender
                        .send(Event::ScanFinished { result })
                        .await
                        .is_err()
                    {
                        return;
                    }
                    self.wake_signal.notify();
                }
            }
        }
        info!("Scanner command channel closed, shutting down");
    }
}

async fn scan_devices() -> Result<ScanReport> {
    scan_devices_with(
        Utf8Path::new("/dev/input"),
        Utf8Path::new("/sys/class/input"),
        inspect_device,
    )
    .await
}

async fn scan_devices_with<F>(
    input_directory: &Utf8Path,
    sysfs_input_root: &Utf8Path,
    mut inspect: F,
) -> Result<ScanReport>
where
    F: FnMut(&Utf8Path) -> std::io::Result<DeviceInspection>,
{
    let mut read_dir = fs::read_dir(input_directory).await.wrap_err_with(|| {
        format!("failed to read {input_directory}; verify that Linux evdev is available")
    })?;
    let mut devices = Vec::new();
    let mut issues = Vec::new();

    while let Some(entry) = read_dir
        .next_entry()
        .await
        .wrap_err("failed to read an entry from /dev/input")?
    {
        let path = match Utf8PathBuf::from_path_buf(entry.path()) {
            Ok(path) => path,
            Err(path) => {
                let path = path.to_string_lossy().into_owned();
                warn!(%path, "ignoring input device with a non-UTF-8 path");
                issues.push(DeviceScanIssue {
                    path,
                    message: "device path is not valid UTF-8".to_owned(),
                    kind: DeviceScanIssueKind::Unknown,
                });
                continue;
            }
        };

        if !path
            .file_name()
            .is_some_and(|name| name.starts_with("event"))
        {
            continue;
        }

        let candidate = keyboard_candidate_from_sysfs(&path, sysfs_input_root).await;
        let inspection = match inspect(&path) {
            Ok(inspection) => inspection,
            Err(error) => {
                if matches!(candidate, Ok(false)) {
                    continue;
                }
                warn!(%path, %error, "failed to inspect input device");
                let kind = match (&candidate, error.kind()) {
                    (Ok(true), std::io::ErrorKind::PermissionDenied) => {
                        DeviceScanIssueKind::PermissionDenied
                    }
                    (Ok(true), _) => DeviceScanIssueKind::Unavailable,
                    (Err(_), _) => DeviceScanIssueKind::Unknown,
                    (Ok(false), _) => unreachable!("non-keyboards were handled above"),
                };
                let message = match candidate {
                    Err(probe_error) => {
                        format!("{error}; keyboard capability probe failed: {probe_error:#}")
                    }
                    Ok(_) => error.to_string(),
                };
                issues.push(DeviceScanIssue {
                    path: path.into_string(),
                    message,
                    kind,
                });
                continue;
            }
        };

        let DeviceInspection::Keyboard {
            name,
            physical_path,
        } = inspection
        else {
            continue;
        };
        info!(%name, %path, "found keyboard");
        devices.push(DeviceMetadata {
            path,
            name,
            physical_path,
        });
    }

    devices.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.path.cmp(&right.path))
    });

    Ok(ScanReport { devices, issues })
}

enum DeviceInspection {
    Keyboard { name: String, physical_path: String },
    NotKeyboard,
}

fn inspect_device(path: &Utf8Path) -> std::io::Result<DeviceInspection> {
    let device = Device::open(path)?;
    if !is_keyboard(&device) {
        return Ok(DeviceInspection::NotKeyboard);
    }
    Ok(DeviceInspection::Keyboard {
        name: device.name().unwrap_or("Unknown keyboard").to_owned(),
        physical_path: device.physical_path().unwrap_or("Unknown").to_owned(),
    })
}

async fn keyboard_candidate_from_sysfs(
    device_path: &Utf8Path,
    sysfs_input_root: &Utf8Path,
) -> Result<bool> {
    let event_name = device_path
        .file_name()
        .ok_or_else(|| color_eyre::eyre::eyre!("input-device path has no event name"))?;
    let capabilities_path = sysfs_input_root
        .join(event_name)
        .join("device/capabilities/key");
    let capabilities = fs::read_to_string(&capabilities_path)
        .await
        .wrap_err_with(|| format!("failed to read {capabilities_path}"))?;
    keyboard_capabilities_include_required_keys(&capabilities)
}

fn keyboard_capabilities_include_required_keys(capabilities: &str) -> Result<bool> {
    let words = capabilities
        .split_whitespace()
        .rev()
        .map(|word| {
            u64::from_str_radix(word, 16)
                .wrap_err_with(|| format!("invalid hexadecimal capability word {word:?}"))
        })
        .collect::<Result<Vec<_>>>()?;
    let bits_per_word = usize::BITS as usize;
    Ok(REQUIRED_KEYBOARD_KEYS.iter().all(|key| {
        let code = usize::from(key.code());
        words
            .get(code / bits_per_word)
            .is_some_and(|word| word & (1_u64 << (code % bits_per_word)) != 0)
    }))
}

fn is_keyboard(device: &Device) -> bool {
    device
        .supported_keys()
        .is_some_and(has_required_keyboard_keys)
}

fn has_required_keyboard_keys(keys: &AttributeSetRef<KeyCode>) -> bool {
    REQUIRED_KEYBOARD_KEYS.iter().all(|key| keys.contains(*key))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        fs, io,
        sync::{Arc, Mutex, mpsc},
        time::Duration,
    };

    use camino::Utf8Path;
    use evdev::AttributeSet;
    use tempfile::tempdir;
    use tokio::sync::oneshot;

    use crate::wake::WakeSignal;

    use super::{
        DeviceInspection, DeviceMetadata, DeviceScanIssueKind, Event, REQUIRED_KEYBOARD_KEYS,
        ScanReport, ScannerHandle, has_required_keyboard_keys,
        keyboard_capabilities_include_required_keys, scan_devices_with, spawn_with,
    };

    #[test]
    fn recognizes_required_keyboard_keys() {
        let keys: AttributeSet<_> = REQUIRED_KEYBOARD_KEYS.into_iter().collect();

        assert!(has_required_keyboard_keys(&keys));
    }

    #[test]
    fn rejects_incomplete_keyboard_keys() {
        let mut keys: AttributeSet<_> = REQUIRED_KEYBOARD_KEYS.into_iter().collect();
        keys.remove(REQUIRED_KEYBOARD_KEYS[0]);

        assert!(!has_required_keyboard_keys(&keys));
    }

    #[test]
    fn parses_sysfs_keyboard_capability_words() {
        let mut low_word = required_capabilities_word();
        assert!(
            keyboard_capabilities_include_required_keys(&format!("0 {low_word:x}"))
                .expect("valid sysfs capabilities")
        );
        low_word &= !(1_u64 << REQUIRED_KEYBOARD_KEYS[0].code());
        assert!(
            !keyboard_capabilities_include_required_keys(&format!("0 {low_word:x}"))
                .expect("valid sysfs capabilities")
        );
        assert!(keyboard_capabilities_include_required_keys("not-hex").is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn scan_filters_classifies_and_orders_injected_devices() {
        let temporary = tempdir().expect("temporary scanner fixture");
        let root = Utf8Path::from_path(temporary.path()).expect("UTF-8 temporary path");
        let input = root.join("input");
        let sysfs = root.join("sysfs");
        fs::create_dir_all(&input).expect("input fixture directory");

        for name in [
            "mouse0",
            "event-zeta",
            "event-alpha",
            "event-readable-non-keyboard",
            "event-not-candidate",
            "event-denied",
            "event-unavailable",
            "event-unknown",
            "event-malformed",
        ] {
            fs::write(input.join(name), []).expect("input fixture entry");
        }
        let keyboard_capabilities = format!("{:x}", required_capabilities_word());
        for name in ["event-not-candidate", "event-denied", "event-unavailable"] {
            write_capabilities(
                &sysfs,
                name,
                if name == "event-not-candidate" {
                    "0"
                } else {
                    &keyboard_capabilities
                },
            );
        }
        write_capabilities(&sysfs, "event-malformed", "not-hex");

        let mut inspected = Vec::new();
        let report = scan_devices_with(&input, &sysfs, |path| {
            let name = path.file_name().expect("fixture event name");
            inspected.push(name.to_owned());
            match name {
                "event-alpha" => Ok(DeviceInspection::Keyboard {
                    name: "Alpha".to_owned(),
                    physical_path: "fixture/alpha".to_owned(),
                }),
                "event-zeta" => Ok(DeviceInspection::Keyboard {
                    name: "alpha".to_owned(),
                    physical_path: "fixture/zeta".to_owned(),
                }),
                "event-readable-non-keyboard" => Ok(DeviceInspection::NotKeyboard),
                "event-denied" | "event-not-candidate" => {
                    Err(io::Error::from(io::ErrorKind::PermissionDenied))
                }
                "event-unavailable" => Err(io::Error::from(io::ErrorKind::NotFound)),
                "event-unknown" | "event-malformed" => Err(io::Error::from(io::ErrorKind::Other)),
                unexpected => panic!("unexpected inspected path: {unexpected}"),
            }
        })
        .await
        .expect("injected scan succeeds");

        assert!(!inspected.iter().any(|name| name == "mouse0"));
        assert_eq!(
            report
                .devices
                .iter()
                .map(|device| device.path.file_name().expect("device event name"))
                .collect::<Vec<_>>(),
            ["event-alpha", "event-zeta"]
        );
        assert_eq!(report.issues.len(), 4);
        assert_issue(
            &report,
            "event-denied",
            DeviceScanIssueKind::PermissionDenied,
        );
        assert_issue(
            &report,
            "event-unavailable",
            DeviceScanIssueKind::Unavailable,
        );
        assert_issue(&report, "event-unknown", DeviceScanIssueKind::Unknown);
        assert_issue(&report, "event-malformed", DeviceScanIssueKind::Unknown);
        assert!(
            !report
                .issues
                .iter()
                .any(|issue| issue.path.ends_with("event-not-candidate"))
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn scanner_serializes_requests_delivers_reports_and_wakes_once_per_event() {
        let first = ScanReport {
            devices: vec![device("First", "/dev/input/event-first")],
            issues: Vec::new(),
        };
        let second = ScanReport {
            devices: vec![device("Second", "/dev/input/event-second")],
            issues: Vec::new(),
        };
        let (first_release, first_wait) = oneshot::channel();
        let (second_release, second_wait) = oneshot::channel();
        let waits = Arc::new(Mutex::new(VecDeque::from([first_wait, second_wait])));
        let (started_tx, started_rx) = mpsc::channel();
        let (wake_tx, wake_rx) = mpsc::channel();
        let wake = WakeSignal::new(move || {
            let _ = wake_tx.send(());
        });
        let mut scanner = spawn_with(wake, move || {
            started_tx.send(()).expect("observe scan start");
            let wait = waits
                .lock()
                .expect("scan waits lock")
                .pop_front()
                .expect("queued scan wait");
            async move { Ok(wait.await.expect("release scan")) }
        });

        scanner.start_scan().expect("start first scan");
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("first scan starts");
        scanner.start_scan().expect("queue second scan");
        assert!(scanner.start_scan().is_err(), "only one scan may queue");
        assert!(
            matches!(started_rx.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "the queued scan must not overlap the active scan"
        );

        first_release
            .send(first.clone())
            .expect("release first scan");
        assert_scan_event(&mut scanner, &wake_rx, &first);
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("queued scan starts after delivery");

        second_release
            .send(second.clone())
            .expect("release second scan");
        assert_scan_event(&mut scanner, &wake_rx, &second);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn scanner_stops_cleanly_when_either_channel_closes() {
        let ScannerHandle {
            mut events,
            commands,
        } = spawn_with(WakeSignal::new(|| {}), || async {
            panic!("a scan should not run after the command channel closes")
        });
        drop(commands);
        assert!(events.recv().await.is_none());

        let (release, wait) = oneshot::channel();
        let wait = Arc::new(Mutex::new(Some(wait)));
        let (started_tx, started_rx) = mpsc::channel();
        let (wake_tx, wake_rx) = mpsc::channel();
        let ScannerHandle { events, commands } = spawn_with(
            WakeSignal::new(move || {
                let _ = wake_tx.send(());
            }),
            move || {
                started_tx.send(()).expect("observe scan start");
                let wait = wait
                    .lock()
                    .expect("scan wait lock")
                    .take()
                    .expect("one scan wait");
                async move {
                    wait.await.expect("release scan");
                    Ok(ScanReport {
                        devices: Vec::new(),
                        issues: Vec::new(),
                    })
                }
            },
        );
        commands
            .try_send(super::Command::Scan)
            .expect("start scan before closing events");
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("scan starts");
        drop(events);
        release.send(()).expect("release scan");
        commands.closed().await;
        assert!(matches!(
            wake_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected)
        ));
    }

    fn required_capabilities_word() -> u64 {
        REQUIRED_KEYBOARD_KEYS
            .iter()
            .fold(0_u64, |word, key| word | (1_u64 << key.code()))
    }

    fn write_capabilities(sysfs: &Utf8Path, event_name: &str, contents: &str) {
        let directory = sysfs.join(event_name).join("device/capabilities");
        fs::create_dir_all(&directory).expect("sysfs fixture directory");
        fs::write(directory.join("key"), contents).expect("sysfs capability fixture");
    }

    fn assert_issue(report: &ScanReport, event_name: &str, kind: DeviceScanIssueKind) {
        assert!(
            report
                .issues
                .iter()
                .any(|issue| { issue.path.ends_with(event_name) && issue.kind == kind })
        );
    }

    fn device(name: &str, path: &str) -> DeviceMetadata {
        DeviceMetadata {
            path: path.into(),
            name: name.to_owned(),
            physical_path: "fixture".to_owned(),
        }
    }

    fn assert_scan_event(
        scanner: &mut ScannerHandle,
        wakes: &mpsc::Receiver<()>,
        expected: &ScanReport,
    ) {
        wakes
            .recv_timeout(Duration::from_secs(1))
            .expect("scanner wake");
        match scanner.try_recv_event() {
            Some(Event::ScanFinished { result }) => {
                assert_eq!(result.expect("successful injected scan"), *expected);
            }
            None => panic!("scanner wake must follow event delivery"),
        }
        assert!(matches!(wakes.try_recv(), Err(mpsc::TryRecvError::Empty)));
    }
}
