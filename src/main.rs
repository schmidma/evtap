use clap::Parser;
use color_eyre::Result;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::EnvFilter;

use crate::app::App;

mod app;
mod listener;
mod metric;
mod scanner;

#[derive(Parser)]
struct Arguments {}

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

    Ok(())
}
