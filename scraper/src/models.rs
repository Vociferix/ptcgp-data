use std::collections::HashMap;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

// ── Set metadata ──────────────────────────────────────────────────────────────

/// Availability window for a set — null for promo sets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Availability {
    /// YYYY-MM-DD date the set became available
    pub start: String,
    /// YYYY-MM-DD date the set stopped being available, if known
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub end: Option<String>,
}

/// One entry in data/sets.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetSummary {
    pub code: String,
    pub name: String,
    pub series: String,
    /// Null for promo sets, which have no fixed availability window
    pub availability: Option<Availability>,
    pub is_promo: bool,
    pub card_count: Option<u32>,
    /// Subtitle of each pack in this set (e.g. "Charizard", "Mewtwo")
    #[serde(default)]
    pub packs: Vec<String>,
}

// ── Abstract cards and card versions ─────────────────────────────────────────

/// Abstract card file — data/cards/{ID:05}.json
///
/// An abstract card represents the game content shared by all art/rarity
/// variants of the same card (same name and mechanics). Version-specific
/// data (rarity, illustrator, packs) lives in CardVersion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbstractCard {
    pub id: u32,
    pub name: String,
    /// "pokemon" or "trainer"
    pub card_type: String,
    /// National Pokédex number (null for trainers and unknown Pokémon)
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub natdex_number: Option<u32>,

    // Pokemon fields (null for trainers)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hp: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retreat_cost: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weakness: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flavor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_ex: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_mega: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub variants: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ability: Option<Ability>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub attacks: Vec<Attack>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evolves_from: Option<String>,

    // Trainer fields (null for pokemon)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trainer_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trainer_effect: Option<String>,

    /// All known versions of this abstract card
    pub versions: Vec<VersionRef>,
}

/// Reference to one specific card version
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VersionRef {
    pub set: String,
    pub number: u32,
}

/// Card version file — data/card_versions/{SET}/{NUM:03}.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardVersion {
    pub set: String,
    pub number: u32,
    /// References the abstract card by its sequential ID
    pub card_id: u32,
    pub rarity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub illustrator: Option<String>,
    /// True when this version is from a promo set (P-A, P-B, …) and carries a promo stamp
    #[serde(default)]
    pub is_promo: bool,
    /// True when this version has a foil/mirror finish (mirrorType "normalMirror" in RaenonX)
    #[serde(default)]
    pub is_foil: bool,
    /// True when an identical version (same rarity, illustrator, promo status, foil status) was released earlier
    #[serde(default)]
    pub is_reprint: bool,
    /// True when this version can be offered in a trade. False for IM/UR cards and all promo versions.
    #[serde(default = "default_tradable")]
    pub is_tradable: bool,
    /// Pack subtitles this version can be obtained from
    pub packs: Vec<String>,
    /// How this version is obtained (e.g. "Pack", "Shop", "Mission")
    pub source: String,
    /// Other versions with the same rarity, illustrator, and promo status (same physical card)
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub duplicates: Vec<VersionRef>,
}

fn default_tradable() -> bool {
    true
}

// ── Card content from Limitless scraping ─────────────────────────────────────

/// Game content scraped from a Limitless card page.
/// Not serialized to disk directly — used to build AbstractCard.
#[derive(Debug, Clone)]
pub struct LimitlessCardData {
    pub name: String,
    pub card_type: String,

    pub element: Option<String>,
    pub stage: Option<String>,
    pub hp: Option<u32>,
    pub retreat_cost: Option<u32>,
    pub weakness: Option<String>,
    pub flavor: Option<String>,
    pub is_ex: bool,
    pub is_mega: bool,
    pub ability: Option<Ability>,
    pub attacks: Vec<Attack>,
    pub evolves_from: Option<String>,

    pub trainer_kind: Option<String>,
    pub trainer_effect: Option<String>,

    /// Version-specific (moves to CardVersion)
    pub illustrator: Option<String>,
    /// Card source label scraped from Limitless (e.g. "Pack", "Mission"). None if absent.
    pub card_source: Option<String>,
}

// ── Shared card sub-types ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attack {
    pub name: String,
    pub cost: Vec<String>,
    pub damage: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub damage_suffix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ability {
    pub name: String,
    pub effect: String,
}

// ── Reference data ────────────────────────────────────────────────────────────

/// data/base_pokemon.json — one entry per national dex number
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasePokemon {
    pub name: String,
    pub natdex_number: u32,
}

/// One entry in data/elements.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementInfo {
    /// Single-letter energy symbol (e.g. "G" for Grass, "R" for Fire).
    /// Null for Dragon, which has no dedicated energy type in PTCGP.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub symbol: Option<String>,
    pub name: String,
}

/// data/rarities.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RarityInfo {
    pub code: String,
    pub name: String,
    pub group: String,
    pub group_symbol_count: u8,
    pub craft_cost: Option<u32>,
    pub dupe_dust: Option<u32>,
}

/// One entry in data/card_sources.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardSource {
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

// ── RaenonX global-master parsed data ────────────────────────────────────────

/// One entry from global-master cardEntryMap, with all fields we care about.
#[derive(Debug, Clone)]
pub struct CardEntry {
    pub card_id: String,
    pub card_type: String,
    /// Rarity code from RaenonX (e.g. "C", "AR", "UR"). Empty for promos without code.
    pub rarity: String,
    /// True when mirrorType == "normalMirror" (foil/mirror finish variant)
    pub is_foil: bool,
    /// True when this card can be offered in a trade (isTradable from RaenonX)
    pub is_tradable: bool,
    /// All (expansion_id, card_number) pairs for this entry (may span multiple sets)
    pub collection_nums: Vec<(String, u32)>,
    /// All variant card IDs that share the same abstract card (play.cardIds)
    pub card_ids_group: Vec<String>,
    /// Pack IDs this card can be obtained from (source.pack)
    pub source_packs: Vec<String>,
    /// Card source derived from RaenonX promotion/source fields
    pub card_source: Option<String>,
}

// ── Pull rate data ────────────────────────────────────────────────────────────

/// Pull rate as a fraction, preserving the source representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rate {
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub numerator: Decimal,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub denominator: Decimal,
}

/// data/pull_rates/{SET}/{SUBTITLE}.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackPullRates {
    pub set: String,
    pub subtitle: String,
    pub variants: PackVariants,
}

/// Maps variant name (e.g. "normal", "rare", "plus1") to its rates.
/// Only variants that exist for this pack appear as keys.
pub type PackVariants = HashMap<String, PackVariantRates>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackVariantRates {
    pub rate: Rate,
    pub slot_count: u32,
    /// Per-slot rarity breakdown: each element maps rarity code -> foil/normal sub-rates
    pub rarity_rates_by_slot: Vec<HashMap<String, RaritySlotRates>>,
    /// Per-card pull rates: card key -> one Rate per slot (null = cannot appear)
    pub card_rates: HashMap<String, Vec<Option<Rate>>>,
}

/// Rarity pull rate split by foil vs non-foil finish.
///
/// For most rarities in most sets only `normal` is present. A rarity where all
/// cards have a foil finish (e.g. C and U in A4b) carries only `foil`. A rarity
/// with a mix of foil and non-foil cards (e.g. R in A4b) carries both.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RaritySlotRates {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normal: Option<Rate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub foil: Option<Rate>,
}
