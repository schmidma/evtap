use clap::Parser;
use color_eyre::Result;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::EnvFilter;

use crate::app::App;

mod app;
mod listener;
mod scanner;

#[derive(Parser)]
struct Arguments {
    /// Keyboard model (e.g., "pc105")
    model: Option<String>,
    /// Keyboard layout (e.g., "us", "de")
    layout: Option<String>,
    /// Keyboard variant (e.g., "dvorak")
    variant: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;

    let filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let _arguments = Arguments::parse();

    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "ev-tap",
        options,
        Box::new(|creation_context| Ok(Box::new(App::new(creation_context)))),
    )
    .unwrap();

    // info!("Starting keyboard listener...");
    //
    // let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    //
    // let keymap = xkb::Keymap::new_from_names(
    //     &context,
    //     "",
    //     arguments.model.as_deref().unwrap_or(""),
    //     arguments.layout.as_deref().unwrap_or(""),
    //     arguments.variant.as_deref().unwrap_or(""),
    //     None,
    //     xkb::KEYMAP_COMPILE_NO_FLAGS,
    // )
    // .wrap_err("failed to create XKB keymap")?;
    //
    // let mut state = xkb::State::new(&keymap);
    //
    // let devices = find_input_devices()?;
    //
    // if devices.is_empty() {
    //     bail!("no suitable input devices found");
    // }
    //
    // // Print devices and select one
    // for (i, device) in devices.iter().enumerate() {
    //     println!(
    //         "{}: {} ({})",
    //         i,
    //         device.name().unwrap_or("Unknown"),
    //         device.physical_path().unwrap_or("Unknown")
    //     );
    // }
    // println!("Select a device by number:");
    // let mut input = String::new();
    // stdin().read_line(&mut input)?;
    // let index: usize = input.trim().parse().wrap_err("invalid number")?;
    // let mut device = devices
    //     .into_iter()
    //     .nth(index)
    //     .wrap_err("invalid device index")?;
    //
    // loop {
    //     for event in device.fetch_events()? {
    //         if event.event_type() == EventType::KEY {
    //             // 1 = Pressed, 0 = Released, 2 = Repeat
    //             let value = event.value();
    //
    //             // CRITICAL: Linux evdev codes are off by 8 compared to XKB standards.
    //             // We must add 8 to the kernel code to match the XKB map.
    //             let code = event.code() as u32 + 8;
    //
    //             // We must update the state on BOTH press (down) and release (up)
    //             // so the state machine knows when Shift is released.
    //             let direction = if value == 0 {
    //                 xkb::KeyDirection::Up
    //             } else {
    //                 xkb::KeyDirection::Down
    //             };
    //
    //             // Update the internal state machine
    //             state.update_key(code.into(), direction);
    //
    //             // If the key was just pressed (or repeated), try to get the text
    //             if value == 1 || value == 2 {
    //                 let utf8 = state.key_get_utf8(code.into());
    //                 if !utf8.is_empty() {
    //                     println!("Input: '{}' (Hex: {:?})", utf8, utf8.as_bytes());
    //                 }
    //
    //                 // Keys like F1, Esc, or Shift don't produce UTF8 text
    //                 let name = state.key_get_one_sym(code.into()).name();
    //                 println!("Non-text key: {:?}", name);
    //             }
    //         }
    //     }
    // }
    Ok(())
}
