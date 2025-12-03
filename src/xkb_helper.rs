use std::process::Command;

use tracing::warn;

const FALLBACK_MODELS: &[&str] = &[
    "pc105",
    "pc104",
    "pc102",
    "pc101",
    "chromebook",
    "macbook79",
];

const FALLBACK_LAYOUTS: &[&str] = &["us", "de", "fr", "gb", "es", "it", "jp", "ru", "cn"];

const FALLBACK_VARIANTS_US: &[&str] = &["", "altgr-intl", "dvorak", "colemak", "workman"];

const FALLBACK_VARIANTS_DE: &[&str] = &["", "nodeadkeys", "neo", "bone"];

pub fn get_models() -> Vec<String> {
    run_localectl("list-x11-keymap-models")
        .unwrap_or_else(|| FALLBACK_MODELS.iter().map(|s| s.to_string()).collect())
}

pub fn get_layouts() -> Vec<String> {
    run_localectl("list-x11-keymap-layouts")
        .unwrap_or_else(|| FALLBACK_LAYOUTS.iter().map(|s| s.to_string()).collect())
}

pub fn get_variants(layout: &str) -> Vec<String> {
    if layout.is_empty() {
        return vec![];
    }

    run_localectl_with_args("list-x11-keymap-variants", &[layout]).unwrap_or_else(|| match layout {
        "us" => FALLBACK_VARIANTS_US.iter().map(|s| s.to_string()).collect(),
        "de" => FALLBACK_VARIANTS_DE.iter().map(|s| s.to_string()).collect(),
        _ => Vec::new(),
    })
}

fn run_localectl(command: &str) -> Option<Vec<String>> {
    run_localectl_with_args(command, &[])
}

fn run_localectl_with_args(command: &str, args: &[&str]) -> Option<Vec<String>> {
    let output = match Command::new("localectl").arg(command).args(args).output() {
        Ok(out) => out,
        Err(err) => {
            warn!("failed to execute localectl {command}: {err}");
            return None;
        }
    };
    let text = String::from_utf8_lossy(&output.stdout);

    if !output.status.success() {
        warn!("localectl {command} returned non-zero exit code: {text}");
        return None;
    }

    let lines = text
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    Some(lines)
}
