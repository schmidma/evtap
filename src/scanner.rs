use camino::Utf8PathBuf;
use color_eyre::{Result, eyre::Context};
use evdev::Device;
use tokio::{
    fs,
    sync::mpsc::{self, Receiver, Sender},
};
use tokio_stream::{StreamExt, wrappers::ReadDirStream};
use tracing::{info, warn};

#[derive(Clone, Debug, PartialEq)]
pub struct DeviceMetadata {
    pub path: Utf8PathBuf,
    pub name: String,
    pub physical_path: String,
}

pub enum Event {
    Scan { metadata: Vec<DeviceMetadata> },
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
            .wrap_err("failed to send scan command")
    }
}

pub fn spawn() -> ScannerHandle {
    let (event_sender, event_receiver) = mpsc::channel(1337);
    let (command_sender, command_receiver) = mpsc::channel(1337);

    tokio::spawn(async move {
        let scanner = Scanner {
            sender: event_sender,
            receiver: command_receiver,
        };
        scanner.run().await;
    });

    ScannerHandle {
        events: event_receiver,
        commands: command_sender,
    }
}

struct Scanner {
    sender: Sender<Event>,
    receiver: Receiver<Command>,
}

impl Scanner {
    async fn run(mut self) {
        loop {
            match self.receiver.recv().await {
                Some(command) => match command {
                    Command::Scan => {
                        let metadata = scan_devices().await.unwrap_or_default();
                        let _ = self.sender.send(Event::Scan { metadata }).await;
                    }
                },
                None => {
                    info!("Scanner command channel closed, shutting down.");
                    break;
                }
            };
        }
    }
}

async fn scan_devices() -> Result<Vec<DeviceMetadata>> {
    let read_dir = fs::read_dir("/dev/input")
        .await
        .wrap_err("failed to read /dev/input directory")?;
    ReadDirStream::new(read_dir)
        .filter_map(|path| {
            let path = Utf8PathBuf::try_from(path.expect("failed to read /dev/input entry").path())
                .expect("path to be valid UTF-8");
            if !path.as_str().contains("event") {
                return None;
            }
            let Ok(device) = Device::open(&path) else {
                warn!("failed to open device at {path}");
                return None;
            };
            let name = device.name().unwrap_or("Unknown").to_string();
            info!("Found: {name} ({path})");
            let physical_path = device.physical_path().unwrap_or("Unknown").to_string();
            let metadata = DeviceMetadata {
                path,
                name,
                physical_path,
            };
            Some(Ok(metadata))
        })
        .collect()
        .await
}
