use camino::Utf8PathBuf;
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

#[derive(Clone, Debug, PartialEq)]
pub struct DeviceScanIssue {
    pub path: String,
    pub message: String,
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

        let device = match Device::open(&path) {
            Ok(device) => device,
            Err(error) => {
                warn!(%path, %error, "failed to inspect input device");
                issues.push(DeviceScanIssue {
                    path: path.into_string(),
                    message: error.to_string(),
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

    use super::{REQUIRED_KEYBOARD_KEYS, has_required_keyboard_keys};

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
}
