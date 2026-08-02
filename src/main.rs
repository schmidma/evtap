use color_eyre::{Result, eyre::eyre};
use tracing::level_filters::LevelFilter;
use tracing_subscriber::EnvFilter;

use crate::{app::App, paths::AppPaths};

mod app;
mod database;
mod input;
mod listener;
mod metric;
mod paths;
mod private_fs;
mod scanner;
mod session;
mod settings;
mod storage;
mod wake;
mod xkb_helper;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;

    let filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let app_paths = AppPaths::discover()?;
    app_paths.prepare_data_dir()?;
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1_000.0, 700.0])
            .with_min_inner_size([800.0, 600.0])
            .with_decorations(true),
        persistence_path: Some(app_paths.eframe_file()),
        ..Default::default()
    };
    eframe::run_native(
        "evtap",
        options,
        Box::new(move |creation_context| Ok(Box::new(App::new(creation_context, app_paths)?))),
    )
    .map_err(|error| eyre!("failed to run evtap: {error}"))?;

    Ok(())
}
