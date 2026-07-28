use std::collections::HashMap;

use color_eyre::{Result, eyre::ContextCompat};
use eframe::egui::{self, ScrollArea};
use evdev::KeyCode;
use tracing::{error, info};
use xkbcommon::xkb::{self, Context, Keymap};

use crate::{
    input::{KeyEvent, KeyEventKind, KeyRole, PhysicalKey},
    listener::{self, ListenerHandle},
    metric::{Metric, default_metrics},
    metric_view::render_metric,
    scanner::{self, DeviceMetadata, ScannerHandle},
    wake::WakeSignal,
    xkb_helper,
};

pub struct App {
    devices: Option<Vec<DeviceMetadata>>,
    selected_device: Option<usize>,
    scan_warning: Option<String>,
    scan_error: Option<String>,
    scanner: ScannerHandle,
    listener: Option<ListenerHandle>,
    listener_state: ListenerState,
    wake_signal: WakeSignal,
    metrics: Vec<Box<dyn Metric>>,
    physical_keys: HashMap<u16, PhysicalKey>,

    // Keyboard configuration
    model: String,
    layout: String,
    variant: String,
    keyboard_error: Option<String>,

    // Available options
    available_models: Vec<String>,
    available_layouts: Vec<String>,
    available_variants: Vec<String>,

    xkb_state: xkb::State,
}

enum ListenerState {
    Idle,
    Connecting,
    Listening,
    Stopping,
    Failed(String),
}

impl App {
    pub fn new(creation_context: &eframe::CreationContext<'_>) -> Result<Self> {
        let repaint_context = creation_context.egui_ctx.clone();
        let wake_signal = WakeSignal::new(move || repaint_context.request_repaint());
        let scanner = scanner::spawn(wake_signal.clone());
        scanner.start_scan()?;

        let metrics = default_metrics();

        let model = String::new();
        let layout = String::new();
        let variant = String::new();

        let available_models = xkb_helper::get_models();
        let available_layouts = xkb_helper::get_layouts();
        let available_variants = xkb_helper::get_variants(&layout);
        let xkb_state = init_keyboard_state(&model, &layout, &variant)?;

        Ok(Self {
            devices: None,
            selected_device: None,
            scan_warning: None,
            scan_error: None,
            scanner,
            listener: None,
            listener_state: ListenerState::Idle,
            wake_signal,
            metrics,
            physical_keys: HashMap::new(),
            model,
            layout,
            variant,
            keyboard_error: None,
            available_models,
            available_layouts,
            available_variants,
            xkb_state,
        })
    }

    fn request_scan(&mut self) {
        self.devices = None;
        self.selected_device = None;
        self.scan_warning = None;
        self.scan_error = None;
        if let Err(error) = self.scanner.start_scan() {
            self.devices = Some(Vec::new());
            self.scan_error = Some(format!("Could not start device scan: {error:#}"));
        }
    }

    fn drain_scanner_events(&mut self) {
        while let Some(event) = self.scanner.try_recv_event() {
            match event {
                scanner::Event::ScanFinished { result } => match result {
                    Ok(report) => {
                        let issue_count = report.issues.len();
                        self.scan_warning = if issue_count == 0 {
                            None
                        } else if report.devices.is_empty() {
                            Some(format!(
                                "No readable keyboard was found. Could not inspect {issue_count} input device(s); check your /dev/input permissions."
                            ))
                        } else {
                            Some(format!(
                                "Could not inspect {issue_count} input device(s); the keyboard list may be incomplete."
                            ))
                        };
                        self.scan_error = None;
                        self.devices = Some(report.devices);
                        self.selected_device = None;
                    }
                    Err(error) => {
                        self.devices = Some(Vec::new());
                        self.selected_device = None;
                        self.scan_warning = None;
                        self.scan_error = Some(format!("Device scan failed: {error}"));
                    }
                },
            }
        }
    }

    fn drain_listener_events(&mut self) {
        while let Some(event) = self
            .listener
            .as_mut()
            .and_then(ListenerHandle::try_recv_event)
        {
            match event {
                listener::Event::Connected => {
                    self.listener_state = ListenerState::Listening;
                    info!("listener connected to keyboard");
                }
                listener::Event::Stopped { reason } => {
                    let is_error = reason.is_error();
                    let message = reason.to_string();
                    self.listener = None;
                    self.listener_state = if is_error {
                        ListenerState::Failed(message.clone())
                    } else {
                        ListenerState::Idle
                    };
                    info!(%message, "listener stopped");
                }
                listener::Event::Input {
                    timestamp,
                    key_code,
                    kind,
                } => {
                    let code = key_code.code();
                    let xkb_code = (code + 8).into();
                    let text = self.xkb_state.key_get_utf8(xkb_code);
                    let text = (!text.is_empty()).then_some(text);

                    // Decode with the state produced by preceding events, then apply this
                    // event so modifiers and locks affect subsequent key events.
                    match kind {
                        KeyEventKind::Press => {
                            self.xkb_state.update_key(xkb_code, xkb::KeyDirection::Down);
                        }
                        KeyEventKind::Release => {
                            self.xkb_state.update_key(xkb_code, xkb::KeyDirection::Up);
                        }
                        KeyEventKind::Repeat => {}
                    }

                    let key = self
                        .physical_keys
                        .entry(code)
                        .or_insert_with(|| {
                            let debug_name = format!("{key_code:?}");
                            let label = debug_name
                                .strip_prefix("KEY_")
                                .unwrap_or(&debug_name)
                                .to_owned();
                            PhysicalKey::new(code, label)
                        })
                        .clone();
                    let role = if key_code == KeyCode::KEY_BACKSPACE {
                        KeyRole::Backspace
                    } else {
                        KeyRole::Other
                    };
                    let event = KeyEvent::new(key, text, timestamp, kind, role);
                    for metric in &mut self.metrics {
                        metric.process(&event);
                    }
                }
            }
        }
    }

    fn update_variants(&mut self) {
        self.available_variants = xkb_helper::get_variants(&self.layout);
        if !self.available_variants.contains(&self.variant) {
            self.variant.clear();
        }
    }

    fn reinit_xkb(&mut self) {
        match init_keyboard_state(&self.model, &self.layout, &self.variant) {
            Ok(state) => {
                self.xkb_state = state;
                self.keyboard_error = None;
                info!(
                    "Re-initialized XKB: {} / {} / {}",
                    self.model, self.layout, self.variant
                );
            }
            Err(error) => {
                let message = format!("Could not apply keyboard configuration: {error:#}");
                error!(%message);
                self.keyboard_error = Some(message);
            }
        }
    }
}

impl App {
    fn render_capture_setup(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.heading("Capture setup");

            self.render_device_picker(ui);
            ui.add_space(4.0);
            self.render_keyboard_configuration(ui);
            ui.add_space(4.0);
            self.render_session_controls(ui);
            self.render_capture_status(ui);
        });
    }

    fn render_device_picker(&mut self, ui: &mut egui::Ui) {
        let mut request_scan = false;
        ui.horizontal_wrapped(|ui| {
            match &self.devices {
                None => {
                    ui.spinner();
                    ui.label("Scanning for keyboards…");
                }
                Some(devices) if devices.is_empty() => {
                    ui.label("No readable keyboards");
                }
                Some(devices) => {
                    let text = self
                        .selected_device
                        .and_then(|index| devices.get(index))
                        .map_or("Select a keyboard", |device| device.name.as_str());

                    ui.add_enabled_ui(self.listener.is_none(), |ui| {
                        egui::ComboBox::from_label("Keyboard")
                            .selected_text(text)
                            .show_ui(ui, |ui| {
                                for (index, device) in devices.iter().enumerate() {
                                    ui.selectable_value(
                                        &mut self.selected_device,
                                        Some(index),
                                        &device.name,
                                    )
                                    .on_hover_ui(|ui| {
                                        ui.label(format!(
                                            "{} ({})",
                                            device.physical_path, device.path
                                        ));
                                    });
                                }
                            });
                    });
                }
            }

            if ui
                .add_enabled(
                    self.listener.is_none() && self.devices.is_some(),
                    egui::Button::new("Rescan"),
                )
                .clicked()
            {
                request_scan = true;
            }
        });

        if request_scan {
            self.request_scan();
        }
    }

    fn render_keyboard_configuration(&mut self, ui: &mut egui::Ui) {
        let enabled = self.listener.is_none();
        ui.add_enabled_ui(enabled, |ui| {
            ui.horizontal_wrapped(|ui| {
                let mut changed = false;

                egui::ComboBox::from_label("Model")
                    .width(80.0)
                    .selected_text(&self.model)
                    .show_ui(ui, |ui| {
                        for model in &self.available_models {
                            if ui
                                .selectable_value(&mut self.model, model.clone(), model)
                                .clicked()
                            {
                                changed = true;
                            }
                        }
                    });

                egui::ComboBox::from_label("Layout")
                    .selected_text(&self.layout)
                    .show_ui(ui, |ui| {
                        let mut update_variants = false;
                        for layout in &self.available_layouts {
                            if ui
                                .selectable_value(&mut self.layout, layout.clone(), layout)
                                .clicked()
                            {
                                changed = true;
                                update_variants = true;
                            }
                        }
                        if update_variants {
                            self.update_variants();
                        }
                    });

                let variant_text = if self.variant.is_empty() {
                    "Default"
                } else {
                    &self.variant
                };
                egui::ComboBox::from_label("Variant")
                    .selected_text(variant_text)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_value(&mut self.variant, String::new(), "Default")
                            .clicked()
                        {
                            changed = true;
                        }
                        for variant in &self.available_variants {
                            if !variant.is_empty()
                                && ui
                                    .selectable_value(&mut self.variant, variant.clone(), variant)
                                    .clicked()
                            {
                                changed = true;
                            }
                        }
                    });

                if changed {
                    self.reinit_xkb();
                }
            });
        });
    }

    fn render_session_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if let Some(listener) = &self.listener {
                let stopping = matches!(self.listener_state, ListenerState::Stopping);
                if ui
                    .add_enabled(!stopping, egui::Button::new("Stop listening"))
                    .clicked()
                {
                    match listener.stop() {
                        Ok(()) => self.listener_state = ListenerState::Stopping,
                        Err(error) => {
                            self.listener = None;
                            self.listener_state = ListenerState::Failed(format!(
                                "Could not stop listener: {error:#}"
                            ));
                        }
                    }
                }
            } else {
                let selected_path = self
                    .devices
                    .as_ref()
                    .zip(self.selected_device)
                    .and_then(|(devices, index)| devices.get(index))
                    .map(|device| device.path.clone());
                if ui
                    .add_enabled(
                        selected_path.is_some(),
                        egui::Button::new("Start listening"),
                    )
                    .clicked()
                    && let Some(device_path) = selected_path
                {
                    self.listener = Some(listener::spawn(device_path, self.wake_signal.clone()));
                    self.listener_state = ListenerState::Connecting;
                }
            }

            if ui.button("Reset session").clicked() {
                for metric in &mut self.metrics {
                    metric.reset();
                }
            }
        });
    }

    fn render_capture_status(&self, ui: &mut egui::Ui) {
        if let Some(error) = &self.scan_error {
            ui.colored_label(egui::Color32::RED, error);
        }
        if let Some(warning) = &self.scan_warning {
            ui.colored_label(egui::Color32::YELLOW, warning);
        }
        if let Some(error) = &self.keyboard_error {
            ui.colored_label(egui::Color32::RED, error);
        }

        match &self.listener_state {
            ListenerState::Idle => {
                ui.weak("Not listening");
            }
            ListenerState::Connecting => {
                ui.label("Connecting to keyboard…");
            }
            ListenerState::Listening => {
                ui.colored_label(egui::Color32::GREEN, "Listening");
            }
            ListenerState::Stopping => {
                ui.label("Stopping listener…");
            }
            ListenerState::Failed(error) => {
                ui.colored_label(egui::Color32::RED, error);
            }
        }
    }
}

fn init_keyboard_state(model: &str, layout: &str, variant: &str) -> Result<xkb::State> {
    let context = Context::new(xkb::CONTEXT_NO_FLAGS);
    let keymap = Keymap::new_from_names(
        &context,
        "",
        model,
        layout,
        variant,
        None,
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
    .wrap_err("failed to create XKB keymap")?;
    Ok(xkb::State::new(&keymap))
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.drain_scanner_events();
        self.drain_listener_events();

        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("evtap");
            ui.label("Understand the mechanics of your everyday typing.");
            ui.small("Session data stays in memory and is discarded when evtap exits.");
            ui.add_space(8.0);

            self.render_capture_setup(ui);
            ui.add_space(8.0);

            ui.heading("Session analytics");
            ui.small("Timing tables appear as samples arrive; no raw keystroke history is saved.");
            ui.add_space(4.0);

            ScrollArea::vertical().show(ui, |ui| {
                for metric in &self.metrics {
                    ui.group(|ui| {
                        ui.set_width(ui.available_width());
                        render_metric(ui, metric.as_ref());
                    });
                    ui.add_space(6.0);
                }
            });
        });
    }
}
