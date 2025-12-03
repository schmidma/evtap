use color_eyre::{Result, eyre::ContextCompat};
use eframe::egui;
use tracing::{error, info};
use xkbcommon::xkb::{self, Context, Keymap};

use crate::{
    listener::{self, ListenerHandle},
    metric::{KeyContext, Metric, heatmap::HeatMap, total_presses::TotalPresses},
    scanner::{self, DeviceMetadata, ScannerHandle},
};

pub struct App {
    devices: Option<Vec<DeviceMetadata>>,
    selected_device: Option<usize>,
    scanner: ScannerHandle,
    listener: Option<ListenerHandle>,
    metrics: Vec<Box<dyn Metric>>,

    /// Keyboard model (e.g., "pc105")
    model: Option<String>,
    /// Keyboard layout (e.g., "us", "de")
    layout: Option<String>,
    /// Keyboard variant (e.g., "dvorak")
    variant: Option<String>,

    xkb_state: xkb::State,
}

impl App {
    pub fn new(creation_context: &eframe::CreationContext<'_>) -> Self {
        if let Some(_storage) = creation_context.storage {
            // Load previous app state here if needed
        }
        let scanner = scanner::spawn();
        scanner.start_scan().unwrap();

        let metrics: Vec<Box<dyn Metric>> = vec![
            Box::new(TotalPresses::default()),
            Box::new(HeatMap::default()),
        ];

        let model = None;
        let layout = None;
        let variant = None;
        let xkb_state = init_keyboard_state(&model, &layout, &variant)
            .expect("failed to create initial keymap");

        Self {
            devices: None,
            selected_device: None,
            scanner,
            listener: None,
            metrics,
            model,
            layout,
            variant,
            xkb_state,
        }
    }
}

fn init_keyboard_state(
    model: &Option<String>,
    layout: &Option<String>,
    variant: &Option<String>,
) -> Result<xkb::State> {
    let context = Context::new(xkb::CONTEXT_NO_FLAGS);
    let keymap = Keymap::new_from_names(
        &context,
        "",
        model.as_deref().unwrap_or(""),
        layout.as_deref().unwrap_or(""),
        variant.as_deref().unwrap_or(""),
        None,
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
    .wrap_err("failed to create XKB keymap")?;
    Ok(xkb::State::new(&keymap))
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
                listener::Event::Input {
                    timestamp,
                    key_code,
                    value,
                } => {
                    let xkb_code = key_code.code() + 8;
                    let utf8 = self.xkb_state.key_get_utf8(xkb_code.into());
                    let utf8 = if utf8.is_empty() { None } else { Some(utf8) };
                    let key_context = KeyContext {
                        key_code,
                        utf8,
                        timestamp,
                        value,
                    };
                    for metric in &mut self.metrics {
                        metric.process(&key_context);
                    }
                }
            }
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(devices) = &self.devices {
                    let text = self
                        .selected_device
                        .map_or("Select a device", |index| &devices[index].name);
                    egui::ComboBox::from_label("Device")
                        .selected_text(text)
                        .show_ui(ui, |ui| {
                            for (i, device) in devices.iter().enumerate() {
                                ui.selectable_value(
                                    &mut self.selected_device,
                                    Some(i),
                                    &device.name,
                                )
                                .on_hover_ui(|ui| {
                                    ui.label(format!("{} ({})", device.physical_path, device.path));
                                });
                            }
                        });
                }

                ui.label("Model:");
                let model = ui.text_edit_singleline(self.model.get_or_insert_default());
                ui.label("Layout:");
                let layout = ui.text_edit_singleline(self.layout.get_or_insert_default());
                ui.label("Variant:");
                let variant = ui.text_edit_singleline(self.variant.get_or_insert_default());

                if model.lost_focus() || layout.lost_focus() || variant.lost_focus() {
                    {
                        match init_keyboard_state(&self.model, &self.layout, &self.variant) {
                            Ok(state) => {
                                self.xkb_state = state;
                            }
                            Err(err) => {
                                error!("Error initializing keyboard state: {err:?}");
                            }
                        }
                    }
                }

                if let (Some(devices), Some(index)) = (&self.devices, self.selected_device)
                    && ui
                        .button("Listen")
                        .on_hover_text(
                            "Start listening for keyboard events from the selected device.",
                        )
                        .clicked()
                {
                    let device_path = devices[index].path.clone();
                    self.listener = Some(listener::spawn(device_path));
                }
            });
            for metric in &self.metrics {
                ui.separator();
                metric.ui(ui);
            }
        });
    }
}
