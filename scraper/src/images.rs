use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use futures::stream::{self, StreamExt};
use tracing::{info, warn};

use crate::client::Client;
use crate::models::SetSummary;
use crate::output;

// ── CDN constants ─────────────────────────────────────────────────────────────

const CARD_CDN: &str =
    "https://limitlesstcg.nyc3.cdn.digitaloceanspaces.com/pocket";
const SET_LOGO_CDN: &str = "https://s3.limitlesstcg.com/pocket/sets";
const FONT_URL: &str = "https://pocket.limitlesstcg.com/fonts/ptcg-font-19.ttf";

const CONCURRENT_DOWNLOADS: usize = 5;

// ── Path helpers (public so callers can compute paths from data) ──────────────

pub fn images_dir() -> PathBuf {
    PathBuf::from("../ptcgp-images")
}

/// `../ptcgp-images/cards/{SET}/{NUM:03}.png`
pub fn card_image_path(set_code: &str, number: u32) -> PathBuf {
    images_dir()
        .join("cards")
        .join(set_code)
        .join(format!("{number:03}.png"))
}

/// `../ptcgp-images/sets/{SET}/logo.webp`
pub fn set_logo_path(set_code: &str) -> PathBuf {
    images_dir()
        .join("sets")
        .join(set_code)
        .join("logo.webp")
}

/// `../ptcgp-images/font/ptcg-font.ttf`
pub fn font_path() -> PathBuf {
    images_dir().join("font").join("ptcg-font.ttf")
}

// ── URL helpers ───────────────────────────────────────────────────────────────

pub fn card_image_url(set_code: &str, number: u32) -> String {
    format!("{CARD_CDN}/{set_code}/{set_code}_{number:03}_EN.png")
}

pub fn set_logo_url(set_code: &str) -> String {
    format!("{SET_LOGO_CDN}/{set_code}.webp")
}

// ── Download helpers ──────────────────────────────────────────────────────────

async fn download_file(client: &Client, url: &str, path: &Path, force: bool) -> Result<bool> {
    if !force && path.exists() {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = client.get_bytes(url).await?;
    std::fs::write(path, &bytes)?;
    Ok(true)
}

// ── Main entry point ──────────────────────────────────────────────────────────

pub async fn run_images(client: &Arc<Client>, force: bool) -> Result<()> {
    let sets_path = output::data_dir().join("sets.json");
    if !sets_path.exists() {
        anyhow::bail!("data/sets.json not found — run `sets` command first");
    }
    let sets: Vec<SetSummary> =
        serde_json::from_str(&std::fs::read_to_string(&sets_path)?)?;

    // ── Set logos (WebP — WebP is the only format available) ─────────────────

    let mut logo_dl = 0usize;
    let mut logo_skip = 0usize;
    for set in &sets {
        let url = set_logo_url(&set.code);
        let path = set_logo_path(&set.code);
        match download_file(client, &url, &path, force).await {
            Ok(true) => logo_dl += 1,
            Ok(false) => logo_skip += 1,
            Err(e) => warn!(set = %set.code, "set logo download failed: {e}"),
        }
    }
    info!(
        downloaded = logo_dl,
        skipped = logo_skip,
        "set logos done"
    );

    // ── Card images (PNG, concurrent) ─────────────────────────────────────────

    let mut card_tasks: Vec<(String, u32)> = Vec::new();
    for set in &sets {
        let versions = output::load_card_versions(&set.code)?;
        for v in versions {
            card_tasks.push((v.set, v.number));
        }
    }

    let total_cards = card_tasks.len();
    info!(total = total_cards, "downloading card images");

    let results: Vec<Result<bool, String>> = stream::iter(card_tasks)
        .map(|(set, num)| {
            let client = Arc::clone(client);
            async move {
                let url = card_image_url(&set, num);
                let path = card_image_path(&set, num);
                download_file(&client, &url, &path, force)
                    .await
                    .map_err(|e| format!("{set}/{num:03}: {e}"))
            }
        })
        .buffer_unordered(CONCURRENT_DOWNLOADS)
        .collect()
        .await;

    let card_dl = results.iter().filter(|r| matches!(r, Ok(true))).count();
    let card_skip = results.iter().filter(|r| matches!(r, Ok(false))).count();
    let card_err = results.iter().filter(|r| r.is_err()).count();
    for r in results.iter().filter(|r| r.is_err()) {
        if let Err(e) = r {
            warn!("card image error: {e}");
        }
    }
    info!(
        downloaded = card_dl,
        skipped = card_skip,
        errors = card_err,
        "card images done"
    );

    // ── PTCG energy-type font ─────────────────────────────────────────────────

    let fp = font_path();
    match download_file(client, FONT_URL, &fp, force).await {
        Ok(true) => info!("downloaded ptcg-font.ttf"),
        Ok(false) => info!("ptcg-font.ttf already present, skipping"),
        Err(e) => warn!("font download failed: {e}"),
    }

    Ok(())
}
