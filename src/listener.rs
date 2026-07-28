use std::{fmt, io, time::SystemTime};

use camino::Utf8PathBuf;
use color_eyre::{Result, eyre::Context};
use evdev::{Device, InputEvent, KeyCode};
use tokio::{
    select,
    sync::mpsc::{
        Receiver, Sender, UnboundedReceiver, UnboundedSender, channel, unbounded_channel,
    },
};
use tracing::{error, info, warn};

use crate::wake::WakeSignal;

const EVENT_CHANNEL_CAPACITY: usize = 2_048;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyValue {
    Up,
    Down,
    Repeat,
}

#[derive(Debug)]
pub enum StopReason {
    Requested,
    OpenFailed(String),
    EventStreamFailed(String),
    ReadFailed(String),
}

impl StopReason {
    pub fn is_error(&self) -> bool {
        !matches!(self, Self::Requested)
    }
}

impl fmt::Display for StopReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Requested => formatter.write_str("stopped"),
            Self::OpenFailed(error) => write!(formatter, "failed to open keyboard: {error}"),
            Self::EventStreamFailed(error) => {
                write!(formatter, "failed to start keyboard event stream: {error}")
            }
            Self::ReadFailed(error) => write!(formatter, "keyboard read failed: {error}"),
        }
    }
}

pub enum Event {
    Connected,
    Input {
        timestamp: SystemTime,
        key_code: KeyCode,
        value: KeyValue,
    },
    Stopped {
        reason: StopReason,
    },
}

enum Command {
    Stop,
}

pub struct ListenerHandle {
    command_sender: UnboundedSender<Command>,
    event_receiver: Receiver<Event>,
}

impl ListenerHandle {
    pub fn try_recv_event(&mut self) -> Option<Event> {
        self.event_receiver.try_recv().ok()
    }

    pub fn stop(&self) -> Result<()> {
        self.command_sender
            .send(Command::Stop)
            .wrap_err("failed to request listener shutdown")
    }
}

pub fn spawn(device_path: Utf8PathBuf, wake_signal: WakeSignal) -> ListenerHandle {
    let (command_sender, command_receiver) = unbounded_channel();
    let (event_sender, event_receiver) = channel(EVENT_CHANNEL_CAPACITY);
    tokio::spawn(async move {
        Listener {
            device_path,
            command_receiver,
            event_sender,
            wake_signal,
        }
        .run()
        .await;
    });
    ListenerHandle {
        command_sender,
        event_receiver,
    }
}

struct Listener {
    device_path: Utf8PathBuf,
    command_receiver: UnboundedReceiver<Command>,
    event_sender: Sender<Event>,
    wake_signal: WakeSignal,
}

impl Listener {
    async fn run(mut self) {
        let device = match Device::open(&self.device_path) {
            Ok(device) => device,
            Err(error) => {
                error!(path = %self.device_path, %error, "failed to open keyboard");
                self.send_event(Event::Stopped {
                    reason: StopReason::OpenFailed(error.to_string()),
                })
                .await;
                return;
            }
        };

        let mut device_events = match device.into_event_stream() {
            Ok(events) => events,
            Err(error) => {
                error!(path = %self.device_path, %error, "failed to create keyboard event stream");
                self.send_event(Event::Stopped {
                    reason: StopReason::EventStreamFailed(error.to_string()),
                })
                .await;
                return;
            }
        };

        if !self.send_event(Event::Connected).await {
            return;
        }

        loop {
            select! {
                biased;
                command = self.command_receiver.recv() => {
                    match command {
                        Some(Command::Stop) => {
                            self.send_event(Event::Stopped {
                                reason: StopReason::Requested,
                            }).await;
                        }
                        None => info!("listener command channel closed, shutting down"),
                    }
                    return;
                }
                event = device_events.next_event() => {
                    if !self.handle_event(event).await {
                        return;
                    }
                }
            }
        }
    }

    async fn handle_event(&self, event: io::Result<InputEvent>) -> bool {
        let event = match event {
            Ok(event) => event,
            Err(error) => {
                error!(path = %self.device_path, %error, "failed to read keyboard event");
                self.send_event(Event::Stopped {
                    reason: StopReason::ReadFailed(error.to_string()),
                })
                .await;
                return false;
            }
        };

        let evdev::EventSummary::Key(key_event, key_code, value) = event.destructure() else {
            return true;
        };
        let value = match value {
            0 => KeyValue::Up,
            1 => KeyValue::Down,
            2 => KeyValue::Repeat,
            _ => {
                warn!(?key_code, value, "ignoring unknown key value");
                return true;
            }
        };

        self.send_event(Event::Input {
            timestamp: key_event.timestamp(),
            key_code,
            value,
        })
        .await
    }

    async fn send_event(&self, event: Event) -> bool {
        if self.event_sender.send(event).await.is_err() {
            return false;
        }
        self.wake_signal.notify();
        true
    }
}
