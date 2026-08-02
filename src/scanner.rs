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
    let (event_sender, event_receiver) = mpsc::channel(CHANNEL_CAPACITY);
    let (command_sender, command_receiver) = mpsc::channel(CHANNEL_CAPACITY);

    tokio::spawn(async move {
        Scanner {
            event_sender,
            command_receiver,
            wake_signal,
        }
        .run()
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
    async fn run(mut self) {
        while let Some(command) = self.command_receiver.recv().await {
            match command {
                Command::Scan => {
                    let result = scan_devices().await.map_err(|error| format!("{error:#}"));
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
    let mut read_dir = fs::read_dir("/dev/input")
        .await
        .wrap_err("failed to read /dev/input; verify that Linux evdev is available")?;
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

        let candidate = keyboard_candidate_from_sysfs(&path).await;
        let device = match Device::open(&path) {
            Ok(device) => device,
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

        if !is_keyboard(&device) {
            continue;
        }

        let name = device.name().unwrap_or("Unknown keyboard").to_owned();
        let physical_path = device.physical_path().unwrap_or("Unknown").to_owned();
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

async fn keyboard_candidate_from_sysfs(device_path: &Utf8Path) -> Result<bool> {
    let event_name = device_path
        .file_name()
        .ok_or_else(|| color_eyre::eyre::eyre!("input-device path has no event name"))?;
    let capabilities_path = Utf8PathBuf::from("/sys/class/input")
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
    use evdev::AttributeSet;

    use super::{
        REQUIRED_KEYBOARD_KEYS, has_required_keyboard_keys,
        keyboard_capabilities_include_required_keys,
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
        let mut low_word = 0_u64;
        for key in REQUIRED_KEYBOARD_KEYS {
            low_word |= 1_u64 << key.code();
        }
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
}
