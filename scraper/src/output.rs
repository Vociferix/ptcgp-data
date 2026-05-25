use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;

use crate::models::{
    AbstractCard, BasePokemon, CardSource, CardVersion, ElementInfo, PackPullRates, PackVariantName,
    RarityInfo, SetSummary,
};

// ── Path helpers ─────────────────────────────────────────────────────────────

pub fn data_dir() -> PathBuf {
    PathBuf::from("data")
}

fn cards_dir(set_code: &str) -> PathBuf {
    data_dir().join("card_versions").join(set_code)
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

pub fn write_elements(elements: &[ElementInfo]) -> Result<()> {
    std::fs::create_dir_all(data_dir())?;
    write_json(&data_dir().join("elements.json"), elements)
}

pub fn write_rarities(rarities: &[RarityInfo]) -> Result<()> {
    std::fs::create_dir_all(data_dir())?;
    write_json(&data_dir().join("rarities.json"), rarities)
}

pub fn write_card_sources(sources: &[CardSource]) -> Result<()> {
    std::fs::create_dir_all(data_dir())?;
    write_json(&data_dir().join("card_sources.json"), sources)
}

pub fn write_pack_variant_names(variants: &[PackVariantName]) -> Result<()> {
    std::fs::create_dir_all(data_dir())?;
    write_json(&data_dir().join("pack_variant_names.json"), variants)
}

/// Write data/card_versions/{SET}/{NUM:03}.json — card version file.
pub fn write_card_version(version: &CardVersion) -> Result<()> {
    ensure_set_dirs(&version.set)?;
    let filename = format!("{:03}.json", version.number);
    write_json(&cards_dir(&version.set).join(filename), version)
}

/// Write data/cards/{ID:05}.json — abstract card file.
pub fn write_abstract_card(card: &AbstractCard) -> Result<()> {
    ensure_abstract_cards_dir()?;
    let filename = format!("{:05}.json", card.id);
    write_json(&abstract_cards_dir().join(filename), card)
}

/// Write data/pull_rates/{SET}/{subtitle_slug}.json
pub fn write_pull_rates(rates: &PackPullRates) -> Result<()> {
    ensure_pull_rates_dir(&rates.set)?;
    let filename = format!("{}.json", pack_slug(&rates.subtitle));
    write_json(&pull_rates_dir(&rates.set).join(filename), rates)
}

pub fn load_card_versions(set_code: &str) -> Result<Vec<CardVersion>> {
    let dir = cards_dir(set_code);
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut versions = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "json") {
            let json = std::fs::read_to_string(&path)?;
            versions.push(serde_json::from_str(&json)?);
        }
    }
    Ok(versions)
}

// ── Skip-if-exists helpers ───────────────────────────────────────────────────

pub fn card_version_file_exists(set_code: &str, number: u32) -> bool {
    cards_dir(set_code)
        .join(format!("{number:03}.json"))
        .exists()
}

pub fn abstract_card_file_exists(id: u32) -> bool {
    abstract_cards_dir().join(format!("{id:05}.json")).exists()
}

pub fn pull_rates_file_exists(set_code: &str, subtitle: &str) -> bool {
    pull_rates_dir(set_code)
        .join(format!("{}.json", pack_slug(subtitle)))
        .exists()
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

/// Load an existing abstract card, update its versions list, and write it back.
pub fn update_abstract_card_versions(
    id: u32,
    versions: &[crate::models::VersionRef],
) -> Result<()> {
    let path = abstract_cards_dir().join(format!("{id:05}.json"));
    let json = std::fs::read_to_string(&path)?;
    let mut card: AbstractCard = serde_json::from_str(&json)?;
    card.versions = versions.to_vec();
    write_json(&path, &card)
}
