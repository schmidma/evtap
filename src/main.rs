use color_eyre::{Result, eyre::eyre};
use tracing::level_filters::LevelFilter;
use tracing_subscriber::EnvFilter;

use crate::app::App;

mod app;
mod listener;
mod metric;
mod scanner;
mod wake;
mod xkb_helper;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;

    let filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "evtap",
        options,
        Box::new(|creation_context| Ok(Box::new(App::new(creation_context)?))),
    )
    .map_err(|error| eyre!("failed to run evtap: {error}"))?;

    Ok(())
}
