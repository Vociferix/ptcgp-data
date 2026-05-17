use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;

use crate::models::{Card, CardSummary, PackPullRates, RarityInfo, SetDetail, SetSummary};

// ── Path helpers ─────────────────────────────────────────────────────────────

/// Root of the data directory (sibling of the `scraper/` crate directory).
pub fn data_dir() -> PathBuf {
    // When run via `cargo run` from the workspace root, the working directory
    // is the workspace root. Adjust if you run from elsewhere.
    PathBuf::from("data")
}

fn set_dir(set_code: &str) -> PathBuf {
    data_dir().join("sets").join(set_code)
}

fn cards_dir(set_code: &str) -> PathBuf {
    set_dir(set_code).join("cards")
}

fn pull_rates_dir() -> PathBuf {
    data_dir().join("pull_rates")
}

fn raenonx_dir() -> PathBuf {
    data_dir().join("raenonx")
}

/// Create all directories needed for a set's output files.
pub fn ensure_set_dirs(set_code: &str) -> Result<()> {
    std::fs::create_dir_all(cards_dir(set_code))?;
    Ok(())
}

pub fn ensure_pull_rates_dir() -> Result<()> {
    std::fs::create_dir_all(pull_rates_dir())?;
    Ok(())
}

pub fn ensure_raenonx_dir() -> Result<()> {
    std::fs::create_dir_all(raenonx_dir())?;
    Ok(())
}

// ── Write helpers ─────────────────────────────────────────────────────────────

fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<()> {
    let json = serde_json::to_string_pretty(value)?;
    std::fs::write(path, json)?;
    Ok(())
}

// ── Public write functions ────────────────────────────────────────────────────

/// Write data/sets.json — the top-level set index.
pub fn write_sets(sets: &[SetSummary]) -> Result<()> {
    std::fs::create_dir_all(data_dir())?;
    write_json(&data_dir().join("sets.json"), sets)
}

/// Write data/rarities.json
pub fn write_rarities(rarities: &[RarityInfo]) -> Result<()> {
    std::fs::create_dir_all(data_dir())?;
    write_json(&data_dir().join("rarities.json"), rarities)
}

/// Write data/sets/{SET}/set.json
pub fn write_set_detail(set: &SetDetail) -> Result<()> {
    ensure_set_dirs(&set.code)?;
    write_json(&set_dir(&set.code).join("set.json"), set)
}

/// Write data/sets/{SET}/cards.json — lightweight index of all cards in the set.
pub fn write_card_index(set_code: &str, summaries: &[CardSummary]) -> Result<()> {
    ensure_set_dirs(set_code)?;
    write_json(&set_dir(set_code).join("cards.json"), summaries)
}

/// Write data/sets/{SET}/cards/{NUM}.json
pub fn write_card(card: &Card) -> Result<()> {
    ensure_set_dirs(&card.set)?;
    let filename = format!("{:03}.json", card.number);
    write_json(&cards_dir(&card.set).join(filename), card)
}

/// Write data/pull_rates/{PACK_ID}.json
pub fn write_pull_rates(rates: &PackPullRates) -> Result<()> {
    ensure_pull_rates_dir()?;
    let filename = format!("{}.json", rates.pack_id);
    write_json(&pull_rates_dir().join(filename), rates)
}

/// Write data/raenonx/global_master.json — raw response for reference.
pub fn write_global_master_raw(raw: &serde_json::Value) -> Result<()> {
    ensure_raenonx_dir()?;
    write_json(&raenonx_dir().join("global_master.json"), raw)
}

/// Write data/raenonx/global_master_summary.json — parsed summary.
pub fn write_global_master_summary(summary: &crate::models::GlobalMasterSummary) -> Result<()> {
    ensure_raenonx_dir()?;
    write_json(&raenonx_dir().join("global_master_summary.json"), summary)
}

// ── Skip-if-exists helpers ───────────────────────────────────────────────────

pub fn card_file_exists(set_code: &str, number: u32) -> bool {
    cards_dir(set_code).join(format!("{number:03}.json")).exists()
}

pub fn pull_rates_file_exists(pack_id: &str) -> bool {
    pull_rates_dir().join(format!("{pack_id}.json")).exists()
}

pub fn global_master_exists() -> bool {
    raenonx_dir().join("global_master.json").exists()
}
