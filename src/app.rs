use color_eyre::{Result, eyre::ContextCompat};
use eframe::egui::{self, ScrollArea};
use tracing::{error, info};
use xkbcommon::xkb::{self, Context, Keymap};

use crate::{
    listener::{self, ListenerHandle},
    metric::{
        KeyContext, Metric, bigram_speed::BigramSpeed, dwell_time::DwellTime,
        error_rate::ErrorRate, flight_time::FlightTime, heatmap::HeatMap,
        total_presses::TotalPresses,
    },
    scanner::{self, DeviceMetadata, ScannerHandle},
    xkb_helper,
};

pub struct App {
    devices: Option<Vec<DeviceMetadata>>,
    selected_device: Option<usize>,
    scanner: ScannerHandle,
    listener: Option<ListenerHandle>,
    metrics: Vec<Box<dyn Metric>>,

    // Keyboard configuration
    model: String,
    layout: String,
    variant: String,

    // Available options
    available_models: Vec<String>,
    available_layouts: Vec<String>,
    available_variants: Vec<String>,

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
            Box::new(ErrorRate::default()),
            Box::new(FlightTime::default()),
            Box::new(DwellTime::default()),
            Box::new(BigramSpeed::default()),
        ];

        let model = "".to_string();
        let layout = "".to_string();
        let variant = "".to_string();

        let available_models = xkb_helper::get_models();
        let available_layouts = xkb_helper::get_layouts();
        let available_variants = xkb_helper::get_variants(&layout);

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
            available_models,
            available_layouts,
            available_variants,
            xkb_state,
        }
    }

    fn update_variants(&mut self) {
        self.available_variants = xkb_helper::get_variants(&self.layout);
        if !self.available_variants.contains(&self.variant) {
            self.variant = "".to_string();
        }
    }

    fn reinit_xkb(&mut self) {
        match init_keyboard_state(&self.model, &self.layout, &self.variant) {
            Ok(state) => {
                self.xkb_state = state;
                info!(
                    "Re-initialized XKB: {} / {} / {}",
                    self.model, self.layout, self.variant
                );
            }
            Err(err) => {
                error!("Error initializing keyboard state: {err:?}");
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
            ctx.request_repaint();
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
                                if ui
                                    .selectable_value(
                                        &mut self.selected_device,
                                        Some(i),
                                        &device.name,
                                    )
                                    .on_hover_ui(|ui| {
                                        ui.label(format!(
                                            "{} ({})",
                                            device.physical_path, device.path
                                        ));
                                    })
                                    .clicked()
                                {
                                    // Optionally auto-start listening or reset something
                                }
                            }
                        });
                }

                ui.separator();

                let mut changed = false;

                egui::ComboBox::from_label("Model")
                    .width(80.0)
                    .selected_text(&self.model)
                    .show_ui(ui, |ui| {
                        for m in &self.available_models {
                            if ui.selectable_value(&mut self.model, m.clone(), m).clicked() {
                                changed = true;
                            }
                        }
                    });

                egui::ComboBox::from_label("Layout")
                    .selected_text(&self.layout)
                    .show_ui(ui, |ui| {
                        let mut update_variants = false;
                        for l in &self.available_layouts {
                            if ui
                                .selectable_value(&mut self.layout, l.clone(), l)
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
                            .selectable_value(&mut self.variant, "".to_string(), "Default")
                            .clicked()
                        {
                            changed = true;
                        }
                        for v in &self.available_variants {
                            if v.is_empty() {
                                continue;
                            }
                            if ui
                                .selectable_value(&mut self.variant, v.clone(), v)
                                .clicked()
                            {
                                changed = true;
                            }
                        }
                    });

                if changed {
                    self.reinit_xkb();
                }

                if let (Some(devices), Some(index)) = (&self.devices, self.selected_device) {
                    let btn_text = if self.listener.is_some() {
                        "Stop"
                    } else {
                        "Listen"
                    };
                    if ui.button(btn_text).clicked() {
                        if self.listener.is_some() {
                            if let Some(l) = &self.listener {
                                let _ = l.stop();
                            }
                        } else {
                            let device_path = devices[index].path.clone();
                            self.listener = Some(listener::spawn(device_path));
                        }
                    }
                }
            });

            ui.separator();

            ScrollArea::vertical().show(ui, |ui| {
                for metric in &self.metrics {
                    metric.ui(ui);
                    ui.separator();
                }
            });
        });
    }
}
