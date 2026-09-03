use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;

use crate::models::{
    AbstractCard, BasePokemon, CardSource, CardVersion, ElementInfo, PackPullRates,
    PackVariantName, RarityInfo, SetSummary,
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

/// Filesystem-safe slug for a pack subtitle: lowercased, with every run of
/// non-alphanumeric characters collapsed to a single underscore
/// ("Deluxe Pack: ex" → `deluxe_pack_ex`). Hyphens are kept as-is, since they
/// are part of the name rather than a separator ("Ho-Oh" → `ho-oh`).
fn pack_slug(subtitle: &str) -> String {
    let mut slug = String::with_capacity(subtitle.len());
    for c in subtitle.to_lowercase().chars() {
        if c.is_alphanumeric() || c == '-' {
            slug.push(c);
        } else if !slug.ends_with('_') {
            slug.push('_');
        }
    }
    slug.trim_matches('_').to_string()
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

/// Delete data/cards files whose ID is not in `keep`.
///
/// IDs are stable across runs and therefore not contiguous, so "everything above
/// the count" is not a safe rule — the surviving set has to be named explicitly.
/// Stale files have to go: the validation pass and `collect_card_names` read the
/// whole directory, so a leftover keeps reporting violations for a card that no
/// longer exists.
pub fn remove_abstract_cards_not_in(keep: &HashSet<u32>) -> Result<usize> {
    let dir = abstract_cards_dir();
    if !dir.exists() {
        return Ok(0);
    }
    let mut removed = 0;
    for entry in std::fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "json") {
            let id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse::<u32>().ok());
            if id.is_some_and(|id| !keep.contains(&id)) {
                std::fs::remove_file(&path)?;
                removed += 1;
            }
        }
    }
    Ok(removed)
}

/// Which abstract ID currently owns each card version, plus the highest ID in use.
///
/// Abstract IDs must survive regeneration: `card_versions` embed them, so an ID
/// that shifts silently repoints a version at a different card.
pub struct AbstractIdIndex {
    /// (set, number) → the abstract ID whose file lists that version
    pub by_version: HashMap<(String, u32), u32>,
    /// Highest ID currently on disk. New cards are minted above it, so an ID is
    /// never handed to a second card while the first still exists.
    pub max_id: u32,
}

/// Read data/cards to learn the IDs already assigned. Missing or unreadable files
/// yield an empty index, which makes the next run mint IDs from scratch.
pub fn load_abstract_id_index() -> Result<AbstractIdIndex> {
    let dir = abstract_cards_dir();
    let mut by_version = HashMap::new();
    let mut max_id = 0;
    if !dir.exists() {
        return Ok(AbstractIdIndex { by_version, max_id });
    }
    for entry in std::fs::read_dir(&dir)? {
        let path = entry?.path();
        if !path.extension().is_some_and(|e| e == "json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(card) = serde_json::from_str::<AbstractCard>(&text) else {
            continue;
        };
        max_id = max_id.max(card.id);
        for v in &card.versions {
            by_version.insert((v.set.clone(), v.number), card.id);
        }
    }
    Ok(AbstractIdIndex { by_version, max_id })
}

/// Every pull rate file on disk, as (set code from the directory name, parsed file).
/// A file that fails to parse is returned as an Err alongside its path so the
/// caller can report it rather than aborting the whole scan.
#[allow(clippy::type_complexity)]
pub fn load_all_pull_rates() -> Result<Vec<(String, PathBuf, Result<PackPullRates>)>> {
    let root = data_dir().join("pull_rates");
    let mut out = Vec::new();
    if !root.exists() {
        return Ok(out);
    }
    let mut set_dirs: Vec<_> = std::fs::read_dir(&root)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .collect();
    set_dirs.sort_by_key(|e| e.path());
    for set_dir in set_dirs {
        let set_code = set_dir.file_name().to_string_lossy().into_owned();
        let mut files: Vec<_> = std::fs::read_dir(set_dir.path())?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .collect();
        files.sort_by_key(|e| e.path());
        for file in files {
            let path = file.path();
            let parsed = std::fs::read_to_string(&path)
                .map_err(anyhow::Error::from)
                .and_then(|t| serde_json::from_str::<PackPullRates>(&t).map_err(Into::into));
            out.push((set_code.clone(), path, parsed));
        }
    }
    Ok(out)
}

/// Every abstract card on disk, sorted by ID, with parse failures reported per file.
#[allow(clippy::type_complexity)]
pub fn load_all_abstract_cards() -> Result<Vec<(PathBuf, Result<AbstractCard>)>> {
    let dir = abstract_cards_dir();
    let mut out = Vec::new();
    if !dir.exists() {
        return Ok(out);
    }
    let mut files: Vec<_> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort_by_key(|e| e.path());
    for file in files {
        let path = file.path();
        let parsed = std::fs::read_to_string(&path)
            .map_err(anyhow::Error::from)
            .and_then(|t| serde_json::from_str::<AbstractCard>(&t).map_err(Into::into));
        out.push((path, parsed));
    }
    Ok(out)
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
