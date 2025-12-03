use std::collections::HashMap;

use eframe::egui;
use evdev::KeyCode;
use tracing::info;

use crate::{
    listener::{self, ListenerHandle},
    scanner::{self, DeviceMetadata, ScannerHandle},
};

#[derive(Default)]
struct KeyStats {
    total_presses: u64,
    counts: HashMap<KeyCode, u64>,
}

pub struct App {
    devices: Option<Vec<DeviceMetadata>>,
    selected_device: Option<usize>,
    scanner: ScannerHandle,
    listener: Option<ListenerHandle>,
    stats: KeyStats,
}

impl App {
    pub fn new(creation_context: &eframe::CreationContext<'_>) -> Self {
        if let Some(_storage) = creation_context.storage {
            // Load previous app state here if needed
        }
        let scanner = scanner::spawn();
        scanner.start_scan().unwrap();

        Self {
            devices: None,
            selected_device: None,
            scanner,
            listener: None,
            stats: KeyStats::default(),
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Some(event) = self.scanner.try_recv_event() {
            match event {
                scanner::Event::Scan { metadata } => {
                    self.devices = Some(metadata);
                    self.selected_device = None;
                }
            }
        }
        while let Some(event) = self.listener.as_mut().and_then(|l| l.try_recv_event()) {
            match event {
                listener::Event::Connected => {
                    info!("Listener connected to device");
                }
                listener::Event::Stopped => {
                    self.listener = None;
                    info!("Listener stopped");
                }
                listener::Event::Input { key_code, value } => {
                    if let listener::KeyValue::Down = value {
                        self.stats.total_presses += 1;
                        *self.stats.counts.entry(key_code).or_insert(0) += 1;
                    }
                }
            }
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(devices) = &self.devices {
                let text = self
                    .selected_device
                    .map_or("Select a device", |index| &devices[index].name);
                egui::ComboBox::from_label("Device")
                    .selected_text(text)
                    .show_ui(ui, |ui| {
                        for (i, device) in devices.iter().enumerate() {
                            ui.selectable_value(&mut self.selected_device, Some(i), &device.name)
                                .on_hover_ui(|ui| {
                                    ui.label(format!("{} ({})", device.physical_path, device.path));
                                });
                        }
                    });
            }
            if let (Some(devices), Some(index)) = (&self.devices, self.selected_device)
                && ui
                    .button("Start Listening")
                    .on_hover_text("Start listening for keyboard events from the selected device.")
                    .clicked()
            {
                let device_path = devices[index].path.clone();
                self.listener = Some(listener::spawn(device_path));
            }
            ui.separator();
            ui.label(format!("Total Key Presses: {}", self.stats.total_presses));
            egui::ScrollArea::vertical().show(ui, |ui| {
                let mut counts: Vec<_> = self.stats.counts.iter().collect();
                counts.sort_by_key(|(_code, count)| *count);
                for (key_code, count) in counts {
                    ui.label(format!("Keycode {:?}: {} presses", key_code, count));
                }
            });
        });
    }
}
