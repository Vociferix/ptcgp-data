use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// One entry in data/sets.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetSummary {
    pub code: String,
    pub name: String,
    pub series: String,
    pub release_date: Option<String>,
    pub is_promo: bool,
    pub card_count: Option<u32>,
    pub icon_url: String,
}

/// Full set detail — data/sets/{SET}/set.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetDetail {
    pub code: String,
    pub name: String,
    pub series: String,
    pub release_date: Option<String>,
    pub is_promo: bool,
    pub card_count: Option<u32>,
    pub icon_url: String,
    pub packs: Vec<PackInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackInfo {
    /// RaenonX pack ID, e.g. "AN001_0010_00_000"
    pub raenonx_id: Option<String>,
    /// Display subtitle, e.g. "Mewtwo" (None for single-pack sets)
    pub subtitle: Option<String>,
    /// Full display name, e.g. "Mewtwo pack"
    pub display_name: String,
}

/// One entry in data/sets/{SET}/cards.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardSummary {
    pub number: u32,
    pub name: String,
    pub rarity: String,
    pub card_type: String,
    pub element: Option<String>,
}

/// Full card — data/sets/{SET}/cards/{NUM}.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Card {
    pub set: String,
    pub number: u32,
    pub name: String,
    pub rarity: String,
    pub illustrator: Option<String>,
    /// "pokemon" or "trainer"
    pub card_type: String,

    // Pokemon fields (null for trainers)
    pub element: Option<String>,
    pub stage: Option<String>,
    pub hp: Option<u32>,
    pub retreat_cost: Option<u32>,
    pub weakness: Option<String>,
    pub flavor: Option<String>,
    pub is_ex: Option<bool>,
    pub is_mega: Option<bool>,
    pub variants: Vec<String>,
    pub ability: Option<Ability>,
    pub attacks: Vec<Attack>,

    // Trainer fields (null for pokemon)
    pub trainer_kind: Option<String>,
    pub trainer_effect: Option<String>,

    /// Pack memberships with display names
    pub packs: Vec<PackRef>,
    pub images: CardImages,
    /// Other prints of the same card (different art / rarity)
    pub alternate_versions: Vec<AlternateVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attack {
    pub name: String,
    /// Ordered energy cost, e.g. ["Grass", "Colorless"]
    pub cost: Vec<String>,
    pub damage: u32,
    /// '+' or '×' suffix on damage number, if any
    pub damage_suffix: Option<String>,
    pub effect: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ability {
    pub name: String,
    pub effect: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardImages {
    pub thumbnail: String,
    pub full: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlternateVersion {
    pub set: String,
    pub number: Option<u32>,
    pub rarity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackRef {
    pub raenonx_id: Option<String>,
    pub display_name: String,
}

/// data/rarities.json
///
/// craft_cost and dupe_dust are populated from RaenonX global-master but are
/// not yet in the SQLite schema — tracked as a pending schema update.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RarityInfo {
    pub code: String,
    pub name: String,
    pub group: String,
    pub group_symbol_count: u8,
    pub craft_cost: Option<u32>,
    pub dupe_dust: Option<u32>,
}

/// Pull rate as a fraction, preserving the source representation.
///
/// When denominator is 1 the numerator is already a decimal fraction
/// (e.g. numerator=0.01538 means 1.538%). This matches the RaenonX wire
/// format exactly to avoid precision loss on conversion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rate {
    pub numerator: f64,
    pub denominator: f64,
}

impl Rate {
    pub fn as_f64(&self) -> f64 {
        if self.denominator == 0.0 {
            0.0
        } else {
            self.numerator / self.denominator
        }
    }
}

/// data/pull_rates/{PACK_ID}.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackPullRates {
    pub pack_id: String,
    pub set: String,
    pub subtitle: Option<String>,
    pub variants: PackVariants,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackVariants {
    pub normal: Option<PackVariantRates>,
    /// God pack — 0.05% base rate, all 5 slots are rare
    pub god: Option<PackVariantRates>,
    /// Premium subscriber pack — 6 cards, last slot shiny-only
    pub premium: Option<PackVariantRates>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackVariantRates {
    /// Probability of getting this pack type (0..1)
    pub rate: f64,
    pub rate_numerator: f64,
    pub rate_denominator: f64,
    pub slot_count: u32,
    /// Per-slot rarity breakdown: each element maps rarity code -> probability
    pub rarity_rates_by_slot: Vec<HashMap<String, f64>>,
    /// Per-card pull rates: RaenonX card ID -> one f64 per slot (null = card
    /// cannot appear in that slot)
    pub card_rates: HashMap<String, Vec<Option<f64>>>,
}

/// Extracted subset of RaenonX global-master — data/raenonx/global_master_summary.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalMasterSummary {
    /// Regular (openable) pack IDs in the order they appear in the source
    pub regular_pack_ids: Vec<String>,
    /// Pack ID -> expansion set code (e.g. "AN001_0010_00_000" -> "A1")
    pub pack_expansion: HashMap<String, String>,
    /// Rarity code -> craft cost in pack points
    pub craft_costs: HashMap<String, u32>,
    /// Rarity code -> dupe trade-in dust value
    pub dupe_dust: HashMap<String, u32>,
}
