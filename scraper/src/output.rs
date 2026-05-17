use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;

use crate::models::{AbstractCard, BasePokemon, CardVersion, PackPullRates, PromoSource, RarityInfo, SetDetail, SetSummary};

// ── Path helpers ─────────────────────────────────────────────────────────────

pub fn data_dir() -> PathBuf {
    PathBuf::from("data")
}

fn cache_dir() -> PathBuf {
    PathBuf::from("cache")
}

fn set_dir(set_code: &str) -> PathBuf {
    data_dir().join("sets").join(set_code)
}

fn cards_dir(set_code: &str) -> PathBuf {
    set_dir(set_code).join("cards")
}

fn abstract_cards_dir() -> PathBuf {
    data_dir().join("cards")
}

fn pull_rates_dir(set_code: &str) -> PathBuf {
    data_dir().join("pull_rates").join(set_code)
}

fn pack_slug(subtitle: &str) -> String {
    subtitle.to_lowercase().replace(' ', "_")
}

// ── Dir setup ─────────────────────────────────────────────────────────────────

pub fn ensure_set_dirs(set_code: &str) -> Result<()> {
    std::fs::create_dir_all(cards_dir(set_code))?;
    Ok(())
}

fn ensure_abstract_cards_dir() -> Result<()> {
    std::fs::create_dir_all(abstract_cards_dir())?;
    Ok(())
}

fn ensure_pull_rates_dir(set_code: &str) -> Result<()> {
    std::fs::create_dir_all(pull_rates_dir(set_code))?;
    Ok(())
}

// ── Write helpers ─────────────────────────────────────────────────────────────

fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<()> {
    let json = serde_json::to_string_pretty(value)?;
    std::fs::write(path, json)?;
    Ok(())
}

// ── Public write functions ────────────────────────────────────────────────────

pub fn write_sets(sets: &[SetSummary]) -> Result<()> {
    std::fs::create_dir_all(data_dir())?;
    write_json(&data_dir().join("sets.json"), sets)
}

pub fn write_rarities(rarities: &[RarityInfo]) -> Result<()> {
    std::fs::create_dir_all(data_dir())?;
    write_json(&data_dir().join("rarities.json"), rarities)
}

pub fn write_promo_sources(sources: &[PromoSource]) -> Result<()> {
    std::fs::create_dir_all(data_dir())?;
    write_json(&data_dir().join("promo_sources.json"), sources)
}

pub fn write_set_detail(set: &SetDetail) -> Result<()> {
    ensure_set_dirs(&set.code)?;
    write_json(&set_dir(&set.code).join("set.json"), set)
}

/// Write data/sets/{SET}/cards/{NUM:03}.json — card version file.
pub fn write_card_version(version: &CardVersion) -> Result<()> {
    ensure_set_dirs(&version.set)?;
    let filename = format!("{:03}.json", version.number);
    write_json(&cards_dir(&version.set).join(filename), version)
}

/// Write data/cards/{ID:04}.json — abstract card file.
pub fn write_abstract_card(card: &AbstractCard) -> Result<()> {
    ensure_abstract_cards_dir()?;
    let filename = format!("{:04}.json", card.id);
    write_json(&abstract_cards_dir().join(filename), card)
}

/// Write data/pull_rates/{SET}/{subtitle_slug}.json
pub fn write_pull_rates(rates: &PackPullRates) -> Result<()> {
    ensure_pull_rates_dir(&rates.set)?;
    let filename = format!("{}.json", pack_slug(&rates.subtitle));
    write_json(&pull_rates_dir(&rates.set).join(filename), rates)
}

/// Write the raw global-master JSON to cache/global_master.json.
pub fn write_global_master(raw: &serde_json::Value) -> Result<()> {
    std::fs::create_dir_all(cache_dir())?;
    write_json(&cache_dir().join("global_master.json"), raw)
}

pub fn load_card_versions(set_code: &str) -> Result<Vec<CardVersion>> {
    let dir = cards_dir(set_code);
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut versions = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().map_or(false, |e| e == "json") {
            let json = std::fs::read_to_string(&path)?;
            versions.push(serde_json::from_str(&json)?);
        }
    }
    Ok(versions)
}

// ── Skip-if-exists helpers ───────────────────────────────────────────────────

pub fn card_version_file_exists(set_code: &str, number: u32) -> bool {
    cards_dir(set_code).join(format!("{number:03}.json")).exists()
}

pub fn abstract_card_file_exists(id: u32) -> bool {
    abstract_cards_dir().join(format!("{id:04}.json")).exists()
}

pub fn pull_rates_file_exists(set_code: &str, subtitle: &str) -> bool {
    pull_rates_dir(set_code).join(format!("{}.json", pack_slug(subtitle))).exists()
}

pub fn global_master_exists() -> bool {
    cache_dir().join("global_master.json").exists()
        || data_dir().join("global_master.json").exists()
        || data_dir().join("raenonx").join("global_master.json").exists()
}

// ── Base Pokémon ──────────────────────────────────────────────────────────────

fn base_pokemon_path() -> PathBuf {
    data_dir().join("base_pokemon.json")
}

pub fn base_pokemon_exists() -> bool {
    base_pokemon_path().exists()
}

pub fn write_base_pokemon(pokemon: &[BasePokemon]) -> Result<()> {
    std::fs::create_dir_all(data_dir())?;
    write_json(&base_pokemon_path(), pokemon)
}

pub fn load_base_pokemon() -> Result<Vec<BasePokemon>> {
    let json = std::fs::read_to_string(base_pokemon_path())?;
    Ok(serde_json::from_str(&json)?)
}

// ── Pack names ────────────────────────────────────────────────────────────────

fn pack_names_path() -> PathBuf {
    cache_dir().join("pack_names.json")
}

pub fn pack_names_exist() -> bool {
    pack_names_path().exists()
}

pub fn write_pack_names(names: &HashMap<String, String>) -> Result<()> {
    std::fs::create_dir_all(cache_dir())?;
    write_json(&pack_names_path(), names)
}

pub fn load_pack_names() -> Result<HashMap<String, String>> {
    let json = std::fs::read_to_string(pack_names_path())?;
    Ok(serde_json::from_str(&json)?)
}

/// Load an existing abstract card, update its versions list, and write it back.
pub fn update_abstract_card_versions(
    id: u32,
    versions: &[crate::models::VersionRef],
) -> Result<()> {
    let path = abstract_cards_dir().join(format!("{id:04}.json"));
    let json = std::fs::read_to_string(&path)?;
    let mut card: AbstractCard = serde_json::from_str(&json)?;
    card.versions = versions.to_vec();
    write_json(&path, &card)
}

/// Load global-master from disk (tries cache/, then legacy data/ paths).
pub fn load_global_master() -> Result<serde_json::Value> {
    let cache_path = cache_dir().join("global_master.json");
    let legacy_data_path = data_dir().join("global_master.json");
    let legacy_raenonx_path = data_dir().join("raenonx").join("global_master.json");

    let path = if cache_path.exists() {
        cache_path
    } else if legacy_data_path.exists() {
        legacy_data_path
    } else if legacy_raenonx_path.exists() {
        legacy_raenonx_path
    } else {
        anyhow::bail!("global_master.json not found — run `global-master` command first");
    };

    let json = std::fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&json)?)
}
