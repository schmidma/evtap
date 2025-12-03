use std::{io, time::SystemTime};

use camino::Utf8PathBuf;
use color_eyre::{Result, eyre::Context};
use evdev::{Device, InputEvent, KeyCode};
use tokio::{
    select,
    sync::mpsc::{Receiver, Sender, channel},
};
use tracing::{error, info};

pub enum KeyValue {
    Up,
    Down,
    Repeat,
}

pub enum Event {
    Connected,
    Input {
        timestamp: SystemTime,
        key_code: KeyCode,
        value: KeyValue,
    },
    Stopped,
}

pub enum Command {
    Stop,
}

pub struct ListenerHandle {
    sender: Sender<Command>,
    receiver: Receiver<Event>,
}

impl ListenerHandle {
    pub fn try_recv_event(&mut self) -> Option<Event> {
        self.receiver.try_recv().ok()
    }

    pub fn stop(&self) -> Result<()> {
        self.sender
            .try_send(Command::Stop)
            .wrap_err("failed to send stop command")
    }
}

pub fn spawn(device_path: Utf8PathBuf) -> ListenerHandle {
    let (command_sender, command_receiver) = channel(1337);
    let (event_sender, event_receiver) = channel(1337);
    tokio::spawn(async move {
        let listener = Listener {
            device_path,
            command_receiver,
            event_sender,
        };
        listener.run().await
    });
    ListenerHandle {
        sender: command_sender,
        receiver: event_receiver,
    }
}

struct Listener {
    device_path: Utf8PathBuf,
    command_receiver: Receiver<Command>,
    event_sender: Sender<Event>,
}

impl Listener {
    pub async fn run(mut self) {
        let device = match Device::open(&self.device_path) {
            Ok(d) => d,
            Err(e) => {
                error!("failed to open device {}: {e:#?}", self.device_path);
                let _ = self.event_sender.send(Event::Stopped).await;
                return;
            }
        };
        let _ = self.event_sender.send(Event::Connected).await;

        let mut device_events = match device.into_event_stream() {
            Ok(d) => d,
            Err(err) => {
                error!(
                    "failed to create event stream for device {}: {err:#?}",
                    self.device_path
                );
                let _ = self.event_sender.send(Event::Stopped).await;
                return;
            }
        };

        loop {
            select! {
                command = self.command_receiver.recv() => {
                    match command {
                        Some(Command::Stop) => {
                            let _ = self.event_sender.send(Event::Stopped).await;
                            return;
                        }
                        None => {
                            info!("Listener command channel closed, shutting down.");
                            let _ = self.event_sender.send(Event::Stopped).await;
                            return;
                        }
                    }
                }
                maybe_event = device_events.next_event() => {
                    self.handle_event(maybe_event).await;
                }
            }
        }
    }

    async fn handle_event(&self, maybe_event: io::Result<InputEvent>) {
        match maybe_event {
            Ok(event) => {
                if let evdev::EventSummary::Key(key_event, key_code, value) = event.destructure() {
                    let value = match value {
                        0 => KeyValue::Up,
                        1 => KeyValue::Down,
                        2 => KeyValue::Repeat,
                        _ => {
                            error!("unknown key value {value} for key code {key_code:?}");
                            return;
                        }
                    };
                    let _ = self
                        .event_sender
                        .send(Event::Input {
                            timestamp: key_event.timestamp(),
                            key_code,
                            value,
                        })
                        .await;
                };
            }
            Err(err) => {
                error!(
                    "error reading event from device {}: {err:#?}",
                    self.device_path
                );
                let _ = self.event_sender.send(Event::Stopped).await;
            }
        }
    }
}
