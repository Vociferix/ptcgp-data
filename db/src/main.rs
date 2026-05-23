use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use rusqlite::{params, Connection};
use rust_decimal::Decimal;
use serde::Deserialize;
use tracing::{error, info, warn};

// ── Violations ────────────────────────────────────────────────────────────────

struct Violations {
    items: Vec<String>,
    lenient: bool,
}

impl Violations {
    fn new(lenient: bool) -> Self {
        Self {
            items: Vec::new(),
            lenient,
        }
    }

    fn add(&mut self, msg: impl std::fmt::Display) {
        let s = msg.to_string();
        if self.lenient {
            warn!("{s}");
        } else {
            error!("{s}");
            self.items.push(s);
        }
    }

    fn finish(self) -> Result<()> {
        if !self.items.is_empty() {
            anyhow::bail!(
                "{} constraint violation(s) — re-run with --lenient to proceed anyway",
                self.items.len()
            );
        }
        Ok(())
    }
}

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "db",
    about = "Build the PTCGP SQLite database from JSON data files"
)]
struct Cli {
    /// Path to the data directory
    #[arg(long, default_value = "data")]
    data: PathBuf,

    /// Path for the output SQLite database
    #[arg(long, default_value = "ptcgp.db")]
    output: PathBuf,

    /// Path to schema.sql (default assumes working directory is workspace root)
    #[arg(long, default_value = "db/schema.sql")]
    schema: PathBuf,

    /// Log constraint violations as warnings instead of failing
    #[arg(long)]
    lenient: bool,
}

// ── JSON models ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ElementInfo {
    symbol: Option<String>,
    name: String,
}

#[derive(Deserialize)]
struct RarityInfo {
    code: String,
    name: String,
    group: String,
    group_symbol_count: u8,
    craft_cost: Option<u32>,
    dupe_dust: Option<u32>,
}

#[derive(Deserialize)]
struct CardSourceInfo {
    code: String,
    description: Option<String>,
}

#[derive(Deserialize)]
struct PackVariantKind {
    code: String,
    name: String,
}

#[derive(Deserialize)]
struct Availability {
    start: String,
    #[serde(default)]
    end: Option<String>,
}

#[derive(Deserialize)]
struct SetSummary {
    code: String,
    name: String,
    series: String,
    availability: Option<Availability>,
    is_promo: bool,
    card_count: Option<u32>,
}

#[derive(Deserialize)]
struct SetDetail {
    packs: Vec<String>,
}

#[derive(Deserialize)]
struct BasePokemon {
    name: String,
    natdex_number: u32,
}

#[derive(Deserialize, Clone)]
struct VersionRef {
    set: String,
    number: u32,
}

#[derive(Deserialize)]
struct AbstractCard {
    id: u32,
    name: String,
    card_type: String,
    natdex_number: Option<u32>,
    element: Option<String>,
    stage: Option<String>,
    hp: Option<u32>,
    retreat_cost: Option<u32>,
    weakness: Option<String>,
    flavor: Option<String>,
    is_ex: Option<bool>,
    is_mega: Option<bool>,
    #[serde(default)]
    variants: Vec<String>,
    ability: Option<Ability>,
    #[serde(default)]
    attacks: Vec<Attack>,
    evolves_from: Option<String>,
    trainer_kind: Option<String>,
    trainer_effect: Option<String>,
}

#[derive(Deserialize, Clone)]
struct CardVersion {
    set: String,
    number: u32,
    card_id: u32,
    rarity: String,
    illustrator: Option<String>,
    is_promo: bool,
    is_foil: bool,
    is_reprint: bool,
    #[serde(default)]
    packs: Vec<String>,
    source: String,
    #[serde(default)]
    duplicates: Vec<VersionRef>,
}

#[derive(Deserialize)]
struct Ability {
    name: String,
    effect: String,
}

#[derive(Deserialize)]
struct Attack {
    name: String,
    #[serde(default)]
    cost: Vec<String>,
    damage: u32,
    damage_suffix: Option<String>,
    effect: Option<String>,
}

// ── Pull rate models ──────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct PackPullRates {
    set: String,
    subtitle: String,
    variants: HashMap<String, Option<PackVariantRates>>,
}

#[derive(Deserialize)]
struct Rate {
    numerator: Decimal,
    denominator: Decimal,
}

#[derive(Deserialize)]
struct PackVariantRates {
    rate: Rate,
    slot_count: u32,
    rarity_rates_by_slot: Vec<HashMap<String, RaritySlotRates>>,
    card_rates: HashMap<String, Vec<Option<Rate>>>,
}

#[derive(Deserialize)]
struct RaritySlotRates {
    normal: Option<Rate>,
    foil: Option<Rate>,
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "db=info".parse().unwrap()),
        )
        .init();

    let cli = Cli::parse();

    if cli.output.exists() {
        std::fs::remove_file(&cli.output)
            .with_context(|| format!("removing existing {:?}", cli.output))?;
    }

    let mut conn =
        Connection::open(&cli.output).with_context(|| format!("opening {:?}", cli.output))?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;

    let schema = std::fs::read_to_string(&cli.schema)
        .with_context(|| format!("reading {:?}", cli.schema))?;
    conn.execute_batch(&schema)?;
    info!("schema applied");

    let mut v = Violations::new(cli.lenient);

    let tx = conn.transaction()?;

    insert_static_data(&tx, &cli.data, &mut v)?;
    insert_card_sources(&tx, &cli.data)?;
    insert_rarities(&tx, &cli.data)?;
    insert_pack_variant_kinds(&tx, &cli.data)?;
    let set_map = insert_sets(&tx, &cli.data)?;
    insert_base_pokemon(&tx, &cli.data)?;
    let card_id_map = insert_abstract_cards(&tx, &cli.data, &mut v)?;
    insert_card_versions(&tx, &cli.data, &card_id_map, &set_map, &mut v)?;
    insert_pull_rates(&tx, &cli.data, &mut v)?;

    v.finish()?;
    tx.commit()?;
    conn.execute_batch("ANALYZE;")?;
    conn.execute_batch("VACUUM;")?;
    info!("database written to {:?}", cli.output);
    Ok(())
}

// ── Static reference data ─────────────────────────────────────────────────────

fn insert_static_data(tx: &rusqlite::Transaction, data: &Path, v: &mut Violations) -> Result<()> {
    let elements_path = data.join("elements.json");
    if elements_path.exists() {
        let elements: Vec<ElementInfo> = load_json(&elements_path)?;
        for el in &elements {
            tx.execute(
                "INSERT OR IGNORE INTO elements (symbol, name) VALUES (?1, ?2)",
                params![el.symbol, el.name],
            )?;
        }
    } else {
        v.add("elements.json not found — run `global-master` first");
    }
    info!("static data inserted");
    Ok(())
}

// ── Reference data from JSON ──────────────────────────────────────────────────

fn insert_card_sources(tx: &rusqlite::Transaction, data: &Path) -> Result<()> {
    let path = data.join("card_sources.json");
    if !path.exists() {
        warn!("card_sources.json not found, skipping");
        return Ok(());
    }
    let sources: Vec<CardSourceInfo> = load_json(&path)?;

    // Seed card_source_descriptions alphabetically.
    let descs: BTreeSet<&str> = sources
        .iter()
        .map(|s| s.description.as_deref().unwrap_or(""))
        .collect();
    for desc in &descs {
        tx.execute(
            "INSERT OR IGNORE INTO card_source_descriptions (description) VALUES (?1)",
            params![desc],
        )?;
    }

    for s in &sources {
        let desc = s.description.as_deref().unwrap_or("");
        let desc_id: i64 = tx.query_row(
            "SELECT id FROM card_source_descriptions WHERE description = ?1",
            params![desc],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO card_sources (code, description_id) VALUES (?1, ?2)",
            params![s.code, desc_id],
        )?;
    }
    info!(count = sources.len(), "card sources inserted");
    Ok(())
}

fn insert_rarities(tx: &rusqlite::Transaction, data: &Path) -> Result<()> {
    let path = data.join("rarities.json");
    if !path.exists() {
        warn!("rarities.json not found, skipping");
        return Ok(());
    }
    let rarities: Vec<RarityInfo> = load_json(&path)?;

    // Seed rarity_names alphabetically.
    let rarity_name_set: BTreeSet<&str> = rarities.iter().map(|r| r.name.as_str()).collect();
    for name in &rarity_name_set {
        tx.execute(
            "INSERT OR IGNORE INTO rarity_names (name) VALUES (?1)",
            params![name],
        )?;
    }

    for r in &rarities {
        tx.execute(
            "INSERT OR IGNORE INTO rarity_groups (name) VALUES (?1)",
            params![r.group],
        )?;
        let group_id: i64 = tx.query_row(
            "SELECT id FROM rarity_groups WHERE name = ?1",
            params![r.group],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO rarity_classes (group_id, count) VALUES (?1, ?2)",
            params![group_id, r.group_symbol_count],
        )?;
        let class_id: i64 = tx.query_row(
            "SELECT id FROM rarity_classes WHERE group_id = ?1 AND count = ?2",
            params![group_id, r.group_symbol_count],
            |row| row.get(0),
        )?;
        let name_id: i64 = tx.query_row(
            "SELECT id FROM rarity_names WHERE name = ?1",
            params![r.name],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO rarities (class_id, code, name_id, craft_cost, dupe_dust) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![class_id, r.code, name_id, r.craft_cost, r.dupe_dust],
        )?;
    }
    info!(count = rarities.len(), "rarities inserted");
    Ok(())
}

fn insert_pack_variant_kinds(tx: &rusqlite::Transaction, data: &Path) -> Result<()> {
    let path = data.join("pack_variant_names.json");
    if !path.exists() {
        warn!("pack_variant_names.json not found, skipping");
        return Ok(());
    }
    let kinds: Vec<PackVariantKind> = load_json(&path)?;

    // Seed pack_variant_codes alphabetically.
    let codes: BTreeSet<&str> = kinds.iter().map(|k| k.code.as_str()).collect();
    for code in &codes {
        tx.execute(
            "INSERT OR IGNORE INTO pack_variant_codes (code) VALUES (?1)",
            params![code],
        )?;
    }

    // Seed pack_variant_names alphabetically.
    let names: BTreeSet<&str> = kinds.iter().map(|k| k.name.as_str()).collect();
    for name in &names {
        tx.execute(
            "INSERT OR IGNORE INTO pack_variant_names (name) VALUES (?1)",
            params![name],
        )?;
    }

    // Insert pack_variant_kinds linking codes to names.
    for k in &kinds {
        let code_id: i64 = tx.query_row(
            "SELECT id FROM pack_variant_codes WHERE code = ?1",
            params![k.code],
            |row| row.get(0),
        )?;
        let name_id: i64 = tx.query_row(
            "SELECT id FROM pack_variant_names WHERE name = ?1",
            params![k.name],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO pack_variant_kinds (code_id, name_id) VALUES (?1, ?2)",
            params![code_id, name_id],
        )?;
    }
    info!(count = kinds.len(), "pack variant kinds inserted");
    Ok(())
}

/// Returns set_code → db set_id.
fn insert_sets(tx: &rusqlite::Transaction, data: &Path) -> Result<HashMap<String, i64>> {
    let path = data.join("sets.json");
    if !path.exists() {
        warn!("sets.json not found, skipping");
        return Ok(HashMap::new());
    }
    let sets: Vec<SetSummary> = load_json(&path)?;
    let mut set_map = HashMap::new();

    // Seed set_names alphabetically.
    let set_name_set: BTreeSet<&str> = sets.iter().map(|s| s.name.as_str()).collect();
    for name in &set_name_set {
        tx.execute(
            "INSERT OR IGNORE INTO set_names (name) VALUES (?1)",
            params![name],
        )?;
    }

    // Seed pack_subtitles alphabetically before inserting packs.
    let mut all_subtitles: BTreeSet<String> = BTreeSet::new();
    for set in &sets {
        let detail_path = data.join("sets").join(&set.code).join("set.json");
        if let Ok(detail) = load_json::<SetDetail>(&detail_path) {
            all_subtitles.extend(detail.packs);
        }
    }
    for subtitle in &all_subtitles {
        tx.execute(
            "INSERT OR IGNORE INTO pack_subtitles (subtitle) VALUES (?1)",
            params![subtitle],
        )?;
    }

    for set in &sets {
        tx.execute(
            "INSERT OR IGNORE INTO series (code) VALUES (?1)",
            params![set.series],
        )?;
        let series_id: i64 = tx.query_row(
            "SELECT id FROM series WHERE code = ?1",
            params![set.series],
            |row| row.get(0),
        )?;

        let name_id: i64 = tx.query_row(
            "SELECT id FROM set_names WHERE name = ?1",
            params![set.name],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO sets (series_id, code, name_id) VALUES (?1, ?2, ?3)",
            params![series_id, set.code, name_id],
        )?;
        let set_id: i64 = tx.query_row(
            "SELECT id FROM sets WHERE code = ?1",
            params![set.code],
            |row| row.get(0),
        )?;
        set_map.insert(set.code.clone(), set_id);

        if let Some(avail) = &set.availability {
            tx.execute(
                "INSERT OR IGNORE INTO set_availability \
                 (set_id, start_date, end_date) VALUES (?1, ?2, ?3)",
                params![set_id, avail.start, avail.end],
            )?;
        }
        if let Some(count) = set.card_count {
            tx.execute(
                "INSERT OR IGNORE INTO set_card_counts (set_id, card_count) VALUES (?1, ?2)",
                params![set_id, count],
            )?;
        }
        if set.is_promo {
            tx.execute(
                "INSERT OR IGNORE INTO promo_sets (set_id) VALUES (?1)",
                params![set_id],
            )?;
        }

        let detail_path = data.join("sets").join(&set.code).join("set.json");
        if !detail_path.exists() {
            continue;
        }
        let detail: SetDetail = load_json(&detail_path)?;
        for subtitle in &detail.packs {
            let subtitle_id: i64 = tx.query_row(
                "SELECT id FROM pack_subtitles WHERE subtitle = ?1",
                params![subtitle],
                |row| row.get(0),
            )?;
            tx.execute(
                "INSERT INTO packs (set_id, subtitle_id) VALUES (?1, ?2)",
                params![set_id, subtitle_id],
            )?;
        }
    }

    info!(count = sets.len(), "sets inserted");
    Ok(set_map)
}

fn insert_base_pokemon(tx: &rusqlite::Transaction, data: &Path) -> Result<()> {
    let path = data.join("base_pokemon.json");
    if !path.exists() {
        warn!("base_pokemon.json not found, skipping");
        return Ok(());
    }
    let pokemon: Vec<BasePokemon> = load_json(&path)?;

    // Seed base_pokemon_names alphabetically.
    let names: BTreeSet<&str> = pokemon.iter().map(|p| p.name.as_str()).collect();
    for name in &names {
        tx.execute(
            "INSERT OR IGNORE INTO base_pokemon_names (name) VALUES (?1)",
            params![name],
        )?;
    }

    for p in &pokemon {
        let name_id: i64 = tx.query_row(
            "SELECT id FROM base_pokemon_names WHERE name = ?1",
            params![p.name],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO base_pokemon (natdex_number, name_id) VALUES (?1, ?2)",
            params![p.natdex_number, name_id],
        )?;
    }
    info!(count = pokemon.len(), "base Pokémon inserted");
    Ok(())
}

// ── Abstract cards ────────────────────────────────────────────────────────────

/// Returns json abstract_id → db cards.id map.
fn insert_abstract_cards(
    tx: &rusqlite::Transaction,
    data: &Path,
    v: &mut Violations,
) -> Result<HashMap<u32, i64>> {
    let dir = data.join("cards");
    if !dir.exists() {
        v.add("data/cards/ not found — run `cards` command first");
        return Ok(HashMap::new());
    }

    let mut entries: Vec<_> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .collect();
    entries.sort_by_key(|e| e.path());

    // Phase 1: load all cards.
    let mut cards: Vec<AbstractCard> = Vec::with_capacity(entries.len());
    for entry in &entries {
        match load_json(&entry.path()) {
            Ok(c) => cards.push(c),
            Err(e) => v.add(format!("{}: failed to parse: {e}", entry.path().display())),
        }
    }

    // Phase 2: seed all string tables alphabetically.
    let mut card_names: BTreeSet<&str> = BTreeSet::new();
    let mut stages: BTreeSet<&str> = BTreeSet::new();
    let mut pokemon_variants: BTreeSet<&str> = BTreeSet::new();
    let mut ability_names: BTreeSet<&str> = BTreeSet::new();
    let mut ability_effects: BTreeSet<&str> = BTreeSet::new();
    let mut attack_names: BTreeSet<&str> = BTreeSet::new();
    let mut attack_effects: BTreeSet<&str> = BTreeSet::new();
    let mut trainer_kinds: BTreeSet<&str> = BTreeSet::new();
    let mut trainer_effects: BTreeSet<&str> = BTreeSet::new();
    let mut flavor_texts: BTreeSet<&str> = BTreeSet::new();
    for card in &cards {
        card_names.insert(&card.name);
        if let Some(s) = &card.stage {
            stages.insert(s);
        }
        for ident in &card.variants {
            pokemon_variants.insert(ident);
        }
        if let Some(from) = &card.evolves_from {
            card_names.insert(from);
        }
        if let Some(ab) = &card.ability {
            ability_names.insert(&ab.name);
            ability_effects.insert(&ab.effect);
        }
        for atk in &card.attacks {
            attack_names.insert(&atk.name);
            if let Some(fx) = &atk.effect {
                attack_effects.insert(fx);
            }
        }
        if let Some(kind) = &card.trainer_kind {
            trainer_kinds.insert(kind);
        }
        if card.card_type == "trainer" {
            trainer_effects.insert(card.trainer_effect.as_deref().unwrap_or(""));
        }
        if let Some(flavor) = &card.flavor {
            flavor_texts.insert(flavor);
        }
    }
    for name in &card_names {
        tx.execute(
            "INSERT OR IGNORE INTO card_names (name) VALUES (?1)",
            params![name],
        )?;
    }
    for name in &stages {
        tx.execute(
            "INSERT OR IGNORE INTO stages (name) VALUES (?1)",
            params![name],
        )?;
    }
    for ident in &pokemon_variants {
        tx.execute(
            "INSERT OR IGNORE INTO pokemon_variants (ident) VALUES (?1)",
            params![ident],
        )?;
    }
    for name in &ability_names {
        tx.execute(
            "INSERT OR IGNORE INTO ability_names (name) VALUES (?1)",
            params![name],
        )?;
    }
    for effect in &ability_effects {
        tx.execute(
            "INSERT OR IGNORE INTO ability_effects (effect) VALUES (?1)",
            params![effect],
        )?;
    }
    for name in &attack_names {
        tx.execute(
            "INSERT OR IGNORE INTO attack_names (name) VALUES (?1)",
            params![name],
        )?;
    }
    for effect in &attack_effects {
        tx.execute(
            "INSERT OR IGNORE INTO attack_effects (effect) VALUES (?1)",
            params![effect],
        )?;
    }
    for name in &trainer_kinds {
        tx.execute(
            "INSERT OR IGNORE INTO trainer_kinds (name) VALUES (?1)",
            params![name],
        )?;
    }
    for effect in &trainer_effects {
        tx.execute(
            "INSERT OR IGNORE INTO trainer_effects (effect) VALUES (?1)",
            params![effect],
        )?;
    }
    for flavor in &flavor_texts {
        tx.execute(
            "INSERT OR IGNORE INTO pokemon_flavor_texts (flavor) VALUES (?1)",
            params![flavor],
        )?;
    }

    // Phase 3: insert cards (string inserts in sub-functions are now no-ops).
    let mut card_id_map = HashMap::new();
    for card in &cards {
        let json_id = card.id;
        let name = card.name.clone();
        match insert_abstract_card(tx, card, v) {
            Ok(db_id) => {
                card_id_map.insert(json_id, db_id);
            }
            Err(e) => v.add(format!("card {:05} ({name}): insert failed: {e}", json_id)),
        }
    }
    info!(count = card_id_map.len(), "abstract cards inserted");
    Ok(card_id_map)
}

fn insert_abstract_card(
    tx: &rusqlite::Transaction,
    card: &AbstractCard,
    v: &mut Violations,
) -> Result<i64> {
    tx.execute(
        "INSERT OR IGNORE INTO card_names (name) VALUES (?1)",
        params![card.name],
    )?;
    let name_id: i64 = tx.query_row(
        "SELECT id FROM card_names WHERE name = ?1",
        params![card.name],
        |row| row.get(0),
    )?;

    tx.execute("INSERT INTO cards (name_id) VALUES (?1)", params![name_id])?;
    let card_id = tx.last_insert_rowid();

    match card.card_type.as_str() {
        "pokemon" => insert_pokemon_data(tx, card_id, card, v)?,
        "trainer" => insert_trainer_data(tx, card_id, card, v)?,
        other => v.add(format!(
            "card {} ({}): unknown card_type '{other}'",
            card.id, card.name
        )),
    }

    Ok(card_id)
}

fn insert_pokemon_data(
    tx: &rusqlite::Transaction,
    card_id: i64,
    card: &AbstractCard,
    v: &mut Violations,
) -> Result<()> {
    let Some(natdex) = card.natdex_number else {
        v.add(format!(
            "pokemon {}: no natdex_number — skipping",
            card.name
        ));
        return Ok(());
    };
    let natdex = natdex as i64;
    if tx
        .query_row::<i64, _, _>(
            "SELECT natdex_number FROM base_pokemon WHERE natdex_number = ?1",
            params![natdex],
            |row| row.get(0),
        )
        .is_err()
    {
        v.add(format!(
            "pokemon {}: natdex {natdex} not in base_pokemon — inserting",
            card.name
        ));
        get_or_insert_base_pokemon(tx, &card.name, natdex)?;
    }

    let Some(el) = &card.element else {
        return Ok(());
    };
    let element_id: i64 = tx.query_row(
        "SELECT id FROM elements WHERE name = ?1",
        params![el],
        |row| row.get(0),
    )?;

    let Some(st) = &card.stage else {
        return Ok(());
    };
    tx.execute(
        "INSERT OR IGNORE INTO stages (name) VALUES (?1)",
        params![st],
    )?;
    let stage_id: i64 = tx.query_row(
        "SELECT id FROM stages WHERE name = ?1",
        params![st],
        |row| row.get(0),
    )?;

    tx.execute(
        "INSERT OR IGNORE INTO pokemon_cards \
         (card_id, natdex_number, element_id, stage_id, retreat_cost, hp) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            card_id,
            natdex,
            element_id,
            stage_id,
            card.retreat_cost,
            card.hp,
        ],
    )?;

    if let Some(flavor) = &card.flavor {
        let flavor_id: i64 = tx.query_row(
            "SELECT id FROM pokemon_flavor_texts WHERE flavor = ?1",
            params![flavor],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO pokemon_flavor_text (card_id, flavor_id) VALUES (?1, ?2)",
            params![card_id, flavor_id],
        )?;
    }

    if let Some(el_name) = &card.weakness {
        if let Ok(el_id) = tx.query_row::<i64, _, _>(
            "SELECT id FROM elements WHERE name = ?1",
            params![el_name],
            |row| row.get(0),
        ) {
            tx.execute(
                "INSERT OR IGNORE INTO weaknesses (card_id, element_id) VALUES (?1, ?2)",
                params![card_id, el_id],
            )?;
        }
    }

    if card.is_ex == Some(true) {
        tx.execute(
            "INSERT OR IGNORE INTO ex_cards (card_id) VALUES (?1)",
            params![card_id],
        )?;
    }
    if card.is_mega == Some(true) {
        tx.execute(
            "INSERT OR IGNORE INTO mega_cards (card_id) VALUES (?1)",
            params![card_id],
        )?;
    }

    for variant in &card.variants {
        tx.execute(
            "INSERT OR IGNORE INTO pokemon_variants (ident) VALUES (?1)",
            params![variant],
        )?;
        let vid: i64 = tx.query_row(
            "SELECT id FROM pokemon_variants WHERE ident = ?1",
            params![variant],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO pokemon_variant_tags (card_id, variant_id) VALUES (?1, ?2)",
            params![card_id, vid],
        )?;
    }

    if let Some(from_name) = &card.evolves_from {
        tx.execute(
            "INSERT OR IGNORE INTO card_names (name) VALUES (?1)",
            params![from_name],
        )?;
        let from_id: i64 = tx.query_row(
            "SELECT id FROM card_names WHERE name = ?1",
            params![from_name],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO pokemon_evolves_from (card_id, evolves_from_id) \
             VALUES (?1, ?2)",
            params![card_id, from_id],
        )?;
    }

    if let Some(ab) = &card.ability {
        tx.execute(
            "INSERT OR IGNORE INTO ability_names (name) VALUES (?1)",
            params![ab.name],
        )?;
        let name_id: i64 = tx.query_row(
            "SELECT id FROM ability_names WHERE name = ?1",
            params![ab.name],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO ability_effects (effect) VALUES (?1)",
            params![ab.effect],
        )?;
        let effect_id: i64 = tx.query_row(
            "SELECT id FROM ability_effects WHERE effect = ?1",
            params![ab.effect],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO abilities (name_id, effect_id) VALUES (?1, ?2)",
            params![name_id, effect_id],
        )?;
        let ability_id: i64 = tx.query_row(
            "SELECT id FROM abilities WHERE name_id = ?1 AND effect_id = ?2",
            params![name_id, effect_id],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO pokemon_abilities (card_id, ability_id) VALUES (?1, ?2)",
            params![card_id, ability_id],
        )?;
    }

    for (idx, attack) in card.attacks.iter().enumerate() {
        let attack_id = insert_attack(tx, attack)?;
        tx.execute(
            "INSERT OR IGNORE INTO pokemon_attacks (card_id, attack_id, idx) \
             VALUES (?1, ?2, ?3)",
            params![card_id, attack_id, idx as i64],
        )?;
    }

    Ok(())
}

fn insert_trainer_data(
    tx: &rusqlite::Transaction,
    card_id: i64,
    card: &AbstractCard,
    v: &mut Violations,
) -> Result<()> {
    let Some(kind) = card.trainer_kind.as_deref() else {
        v.add(format!(
            "trainer {}: missing trainer_kind — skipped",
            card.name
        ));
        return Ok(());
    };
    tx.execute(
        "INSERT OR IGNORE INTO trainer_kinds (name) VALUES (?1)",
        params![kind],
    )?;
    let kind_id: i64 = tx.query_row(
        "SELECT id FROM trainer_kinds WHERE name = ?1",
        params![kind],
        |row| row.get(0),
    )?;

    let effect_text = card.trainer_effect.as_deref().unwrap_or("");
    tx.execute(
        "INSERT OR IGNORE INTO trainer_effects (effect) VALUES (?1)",
        params![effect_text],
    )?;
    let effect_id: i64 = tx.query_row(
        "SELECT id FROM trainer_effects WHERE effect = ?1",
        params![effect_text],
        |row| row.get(0),
    )?;

    tx.execute(
        "INSERT OR IGNORE INTO trainer_cards (card_id, kind_id, effect_id) VALUES (?1, ?2, ?3)",
        params![card_id, kind_id, effect_id],
    )?;
    Ok(())
}

fn insert_attack(tx: &rusqlite::Transaction, attack: &Attack) -> Result<i64> {
    tx.execute(
        "INSERT OR IGNORE INTO attack_names (name) VALUES (?1)",
        params![attack.name],
    )?;
    let name_id: i64 = tx.query_row(
        "SELECT id FROM attack_names WHERE name = ?1",
        params![attack.name],
        |row| row.get(0),
    )?;

    let effect_id: Option<i64> = if let Some(fx) = &attack.effect {
        tx.execute(
            "INSERT OR IGNORE INTO attack_effects (effect) VALUES (?1)",
            params![fx],
        )?;
        Some(tx.query_row(
            "SELECT id FROM attack_effects WHERE effect = ?1",
            params![fx],
            |row| row.get(0),
        )?)
    } else {
        None
    };

    let suffix_cp: Option<i64> = attack
        .damage_suffix
        .as_deref()
        .and_then(|s| s.chars().next().map(|c| c as i64));

    tx.execute(
        "INSERT OR IGNORE INTO attacks \
         (name_id, effect_id, base_damage, damage_suffix_codepoint) \
         VALUES (?1, ?2, ?3, ?4)",
        params![name_id, effect_id, attack.damage, suffix_cp],
    )?;

    let attack_id: i64 = tx.query_row(
        "SELECT id FROM attacks \
         WHERE name_id = ?1 AND base_damage = ?2 \
         AND (effect_id IS ?3) AND (damage_suffix_codepoint IS ?4)",
        params![name_id, attack.damage, effect_id, suffix_cp],
        |row| row.get(0),
    )?;

    for (idx, element) in attack.cost.iter().enumerate() {
        if let Ok(el_id) = tx.query_row::<i64, _, _>(
            "SELECT id FROM elements WHERE name = ?1",
            params![element],
            |row| row.get(0),
        ) {
            tx.execute(
                "INSERT OR IGNORE INTO attack_cost (attack_id, element_id, idx) \
                 VALUES (?1, ?2, ?3)",
                params![attack_id, el_id, idx as i64],
            )?;
        }
    }

    Ok(attack_id)
}

// ── Card versions ─────────────────────────────────────────────────────────────

fn insert_card_versions(
    tx: &rusqlite::Transaction,
    data: &Path,
    card_id_map: &HashMap<u32, i64>,
    set_map: &HashMap<String, i64>,
    v: &mut Violations,
) -> Result<()> {
    let sets_path = data.join("sets.json");
    if !sets_path.exists() {
        return Ok(());
    }
    let sets: Vec<SetSummary> = load_json(&sets_path)?;

    // Phase 1: pre-load all card versions across all sets.
    let mut loaded: Vec<(i64, CardVersion)> = Vec::new();
    for set in &sets {
        let cards_dir = data.join("sets").join(&set.code).join("cards");
        if !cards_dir.exists() {
            continue;
        }
        let Some(&set_id) = set_map.get(&set.code) else {
            v.add(format!(
                "{}: set missing from DB map — versions skipped",
                set.code
            ));
            continue;
        };

        let mut entries: Vec<_> = std::fs::read_dir(&cards_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .collect();
        entries.sort_by_key(|e| e.path());

        for entry in entries {
            match load_json(&entry.path()) {
                Ok(version) => loaded.push((set_id, version)),
                Err(e) => v.add(format!("{}: failed to parse: {e}", entry.path().display())),
            }
        }
    }

    // Phase 2: seed illustrators alphabetically.
    let illustrators: BTreeSet<&str> = loaded
        .iter()
        .filter_map(|(_, cv)| cv.illustrator.as_deref())
        .collect();
    for name in &illustrators {
        tx.execute(
            "INSERT OR IGNORE INTO illustrators (name) VALUES (?1)",
            params![name],
        )?;
    }

    // Phase 3: process versions.
    let mut all_versions: Vec<CardVersion> = Vec::new();
    let mut version_db_map: HashMap<(String, u32), i64> = HashMap::new();
    for (set_id, version) in loaded {
        let Some(&card_db_id) = card_id_map.get(&version.card_id) else {
            v.add(format!(
                "{}/{:03}: abstract card {:05} not found",
                version.set, version.number, version.card_id
            ));
            continue;
        };

        let rarity_id = match tx.query_row::<i64, _, _>(
            "SELECT id FROM rarities WHERE code = ?1",
            params![version.rarity],
            |row| row.get(0),
        ) {
            Ok(id) => id,
            Err(_) => {
                v.add(format!(
                    "{}/{:03}: unknown rarity '{}'",
                    version.set, version.number, version.rarity
                ));
                continue;
            }
        };

        let source_id = match tx.query_row::<i64, _, _>(
            "SELECT id FROM card_sources WHERE code = ?1",
            params![version.source],
            |row| row.get(0),
        ) {
            Ok(id) => id,
            Err(_) => {
                v.add(format!(
                    "{}/{:03}: unknown card source '{}'",
                    version.set, version.number, version.source
                ));
                continue;
            }
        };

        tx.execute(
            "INSERT INTO card_versions (card_id, set_id, rarity_id, number, source_id) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![card_db_id, set_id, rarity_id, version.number, source_id],
        )?;
        let version_db_id = tx.last_insert_rowid();
        version_db_map.insert((version.set.clone(), version.number), version_db_id);

        if let Some(ill) = &version.illustrator {
            let ill_id: i64 = tx.query_row(
                "SELECT id FROM illustrators WHERE name = ?1",
                params![ill],
                |row| row.get(0),
            )?;
            tx.execute(
                "INSERT OR IGNORE INTO card_version_illustrators \
                 (card_version_id, illustrator_id) VALUES (?1, ?2)",
                params![version_db_id, ill_id],
            )?;
        }

        if version.is_promo {
            tx.execute(
                "INSERT OR IGNORE INTO promo_card_versions (card_version_id) VALUES (?1)",
                params![version_db_id],
            )?;
        }
        if version.is_foil {
            tx.execute(
                "INSERT OR IGNORE INTO foil_card_versions (card_version_id) VALUES (?1)",
                params![version_db_id],
            )?;
        }

        for subtitle in &version.packs {
            match lookup_pack(tx, set_id, subtitle) {
                Some(pid) => {
                    tx.execute(
                        "INSERT OR IGNORE INTO card_packs (card_version_id, pack_id) \
                         VALUES (?1, ?2)",
                        params![version_db_id, pid],
                    )?;
                }
                None => v.add(format!(
                    "{}/{:03}: pack '{subtitle}' not found",
                    version.set, version.number
                )),
            }
        }

        all_versions.push(version);
    }

    insert_version_duplicates(tx, &all_versions, &version_db_map)?;
    info!(count = version_db_map.len(), "card versions inserted");
    Ok(())
}

fn lookup_pack(tx: &rusqlite::Transaction, set_id: i64, subtitle: &str) -> Option<i64> {
    // Prefer a pack within the card version's own set; fall back to a global
    // search to handle cross-set pack references (e.g. Deluxe Pack sets whose
    // cards can be obtained from packs defined in other sets).
    tx.query_row::<i64, _, _>(
        "SELECT p.id FROM packs p \
         JOIN pack_subtitles ps ON ps.id = p.subtitle_id \
         WHERE p.set_id = ?1 AND ps.subtitle = ?2",
        params![set_id, subtitle],
        |row| row.get(0),
    )
    .ok()
    .or_else(|| {
        tx.query_row::<i64, _, _>(
            "SELECT p.id FROM packs p \
             JOIN pack_subtitles ps ON ps.id = p.subtitle_id \
             WHERE ps.subtitle = ?1",
            params![subtitle],
            |row| row.get(0),
        )
        .ok()
    })
}

fn insert_version_duplicates(
    tx: &rusqlite::Transaction,
    all_versions: &[CardVersion],
    version_db_map: &HashMap<(String, u32), i64>,
) -> Result<()> {
    let reprint_map: HashMap<(String, u32), bool> = all_versions
        .iter()
        .map(|v| ((v.set.clone(), v.number), v.is_reprint))
        .collect();

    let mut count = 0usize;
    for version in all_versions {
        if version.duplicates.is_empty() {
            continue;
        }
        let Some(&self_db_id) = version_db_map.get(&(version.set.clone(), version.number)) else {
            continue;
        };

        let original_db_id = if !version.is_reprint {
            self_db_id
        } else {
            version
                .duplicates
                .iter()
                .find(|d| reprint_map.get(&(d.set.clone(), d.number)).copied() == Some(false))
                .and_then(|d| version_db_map.get(&(d.set.clone(), d.number)).copied())
                .unwrap_or(self_db_id)
        };

        tx.execute(
            "INSERT OR IGNORE INTO card_version_duplicates \
             (card_version_id, original_version_id) VALUES (?1, ?2)",
            params![self_db_id, original_db_id],
        )?;
        count += 1;
    }
    info!(count, "version duplicates inserted");
    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn get_or_insert_base_pokemon(tx: &rusqlite::Transaction, name: &str, natdex: i64) -> Result<()> {
    tx.execute(
        "INSERT OR IGNORE INTO base_pokemon_names (name) VALUES (?1)",
        params![name],
    )?;
    let name_id: i64 = tx.query_row(
        "SELECT id FROM base_pokemon_names WHERE name = ?1",
        params![name],
        |row| row.get(0),
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO base_pokemon (natdex_number, name_id) VALUES (?1, ?2)",
        params![natdex, name_id],
    )?;
    Ok(())
}

fn gcd(a: i64, b: i64) -> i64 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

fn lcm(a: i64, b: i64) -> i64 {
    a / gcd(a, b) * b
}

/// Convert a `Rate` to an `(numerator, denominator)` integer pair.
///
/// The source API sometimes gives fractional numerators (e.g. `0.125 / 1`).
/// We clear those by scaling both values up by `10^scale` (the Decimal scale),
/// then reduce by GCD — all without introducing any floating-point arithmetic.
fn rate_to_integers(rate: &Rate) -> (i64, i64) {
    let scale = rate.numerator.scale().max(rate.denominator.scale());
    let factor = Decimal::from(10u64.pow(scale));
    let n = (rate.numerator * factor)
        .try_into()
        .expect("rate numerator overflows i64");
    let d = (rate.denominator * factor)
        .try_into()
        .expect("rate denominator overflows i64");
    let g = gcd(n, d);
    (n / g, d / g)
}

fn load_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let text = std::fs::read_to_string(path)?;
    serde_json::from_str(&text).with_context(|| format!("parsing {:?}", path))
}

// ── Pull rates ────────────────────────────────────────────────────────────────

fn insert_pull_rates(
    tx: &rusqlite::Transaction,
    data_dir: &Path,
    v: &mut Violations,
) -> Result<()> {
    let pull_rates_dir = data_dir.join("pull_rates");
    if !pull_rates_dir.exists() {
        return Ok(());
    }

    // Build lookup tables from the DB.
    let mut stmt = tx.prepare("SELECT code, id FROM sets")?;
    let set_ids: HashMap<String, i64> = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    drop(stmt);

    let mut stmt = tx.prepare("SELECT code, id FROM rarities")?;
    let rarity_ids: HashMap<String, i64> = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    drop(stmt);

    let mut set_dirs: Vec<_> = std::fs::read_dir(&pull_rates_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .collect();
    set_dirs.sort_by_key(|e| e.path());

    let mut total = 0usize;

    for set_dir_entry in set_dirs {
        let set_code = set_dir_entry.file_name().to_string_lossy().into_owned();
        let Some(&set_id) = set_ids.get(&set_code) else {
            v.add(format!("pull_rates/{set_code}: set not found in DB"));
            continue;
        };

        let mut rate_files: Vec<_> = std::fs::read_dir(set_dir_entry.path())?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .collect();
        rate_files.sort_by_key(|e| e.path());

        for rate_file in rate_files {
            let rates: PackPullRates = match load_json(&rate_file.path()) {
                Ok(r) => r,
                Err(e) => {
                    v.add(format!("{}: {e:?}", rate_file.path().display()));
                    continue;
                }
            };

            if rates.set != set_code {
                v.add(format!(
                    "pull_rates/{set_code}/{}: set field '{}' does not match directory",
                    rates.subtitle, rates.set
                ));
            }

            let pack_id: i64 = match tx.query_row(
                "SELECT p.id FROM packs p \
                 JOIN pack_subtitles ps ON ps.id = p.subtitle_id \
                 WHERE p.set_id = ?1 AND ps.subtitle = ?2",
                params![set_id, rates.subtitle],
                |row| row.get(0),
            ) {
                Ok(id) => id,
                Err(_) => {
                    v.add(format!(
                        "pull_rates/{set_code}/{}: pack '{}' not found",
                        rates.subtitle, rates.subtitle
                    ));
                    continue;
                }
            };

            // Compute LCM of all non-null variant rate denominators for this pack.
            let pack_denom: i64 = rates
                .variants
                .values()
                .flatten()
                .map(|v| rate_to_integers(&v.rate).1)
                .fold(1i64, lcm);
            tx.execute(
                "INSERT OR IGNORE INTO pack_variant_rate_denominators \
                 (pack_id, rate_denominator) VALUES (?1, ?2)",
                params![pack_id, pack_denom],
            )?;

            for (variant_code, variant) in rates
                .variants
                .iter()
                .filter_map(|(k, v)| v.as_ref().map(|v| (k, v)))
            {
                let kind_id: i64 = match tx.query_row::<i64, _, _>(
                    "SELECT pvk.id FROM pack_variant_kinds pvk \
                     JOIN pack_variant_codes pvc ON pvc.id = pvk.code_id \
                     WHERE pvc.code = ?1",
                    params![variant_code],
                    |row| row.get(0),
                ) {
                    Ok(id) => id,
                    Err(_) => {
                        v.add(format!(
                            "pull_rates/{set_code}/{}: unknown variant code '{variant_code}'",
                            rates.subtitle
                        ));
                        continue;
                    }
                };

                // Scale variant numerator to the pack's LCM denominator.
                let (vn, vd) = rate_to_integers(&variant.rate);
                let rate_num = vn * (pack_denom / vd);
                tx.execute(
                    "INSERT OR IGNORE INTO pack_variants \
                     (kind_id, pack_id, rate_numerator) VALUES (?1, ?2, ?3)",
                    params![kind_id, pack_id, rate_num],
                )?;
                let variant_id: i64 = tx.query_row(
                    "SELECT id FROM pack_variants WHERE kind_id = ?1 AND pack_id = ?2",
                    params![kind_id, pack_id],
                    |row| row.get(0),
                )?;

                for slot_idx in 0..variant.slot_count {
                    // Compute LCM of all rate denominators in this slot.
                    let mut slot_denom = 1i64;
                    if let Some(rarity_rates) = variant.rarity_rates_by_slot.get(slot_idx as usize)
                    {
                        for rates in rarity_rates.values() {
                            for rate in [rates.normal.as_ref(), rates.foil.as_ref()]
                                .into_iter()
                                .flatten()
                            {
                                let (_, d) = rate_to_integers(rate);
                                slot_denom = lcm(slot_denom, d);
                            }
                        }
                    }
                    for slot_rates in variant.card_rates.values() {
                        if let Some(Some(rate)) = slot_rates.get(slot_idx as usize) {
                            let (_, d) = rate_to_integers(rate);
                            slot_denom = lcm(slot_denom, d);
                        }
                    }

                    tx.execute(
                        "INSERT OR IGNORE INTO pack_slots \
                         (pack_variant_id, pull_number, rate_denominator) VALUES (?1, ?2, ?3)",
                        params![variant_id, slot_idx, slot_denom],
                    )?;
                    let slot_id: i64 = tx.query_row(
                        "SELECT id FROM pack_slots \
                         WHERE pack_variant_id = ?1 AND pull_number = ?2",
                        params![variant_id, slot_idx],
                        |row| row.get(0),
                    )?;

                    if let Some(rarity_rates) = variant.rarity_rates_by_slot.get(slot_idx as usize)
                    {
                        for (rarity_code, slot_rates) in rarity_rates {
                            match rarity_ids.get(rarity_code.as_str()) {
                                Some(&rarity_id) => {
                                    for (is_foil, rate) in [
                                        (0i64, slot_rates.normal.as_ref()),
                                        (1i64, slot_rates.foil.as_ref()),
                                    ] {
                                        let Some(rate) = rate else { continue };
                                        let (rn, rd) = rate_to_integers(rate);
                                        let scaled_num = rn * (slot_denom / rd);
                                        tx.execute(
                                            "INSERT OR IGNORE INTO rarity_pull_rates \
                                             (slot_id, rarity_id, is_foil, rate_numerator) \
                                             VALUES (?1, ?2, ?3, ?4)",
                                            params![slot_id, rarity_id, is_foil, scaled_num],
                                        )?;
                                    }
                                }
                                None => v.add(format!(
                                    "pull_rates/{set_code}/{}: unknown rarity '{rarity_code}'",
                                    rates.subtitle
                                )),
                            }
                        }
                    }

                    for (card_key, slot_rates) in &variant.card_rates {
                        let Some(Some(rate)) = slot_rates.get(slot_idx as usize) else {
                            continue;
                        };
                        let card_num: u32 = match card_key.parse() {
                            Ok(n) => n,
                            Err(_) => {
                                v.add(format!(
                                    "pull_rates/{set_code}/{}: invalid card key '{card_key}'",
                                    rates.subtitle
                                ));
                                continue;
                            }
                        };
                        let card_version_id: i64 = match tx.query_row(
                            "SELECT id FROM card_versions WHERE set_id = ?1 AND number = ?2",
                            params![set_id, card_num],
                            |row| row.get(0),
                        ) {
                            Ok(id) => id,
                            Err(_) => {
                                v.add(format!(
                                    "pull_rates/{set_code}/{}: card {card_num:03} not found",
                                    rates.subtitle
                                ));
                                continue;
                            }
                        };
                        let (cn, cd) = rate_to_integers(rate);
                        let scaled_num = cn * (slot_denom / cd);
                        tx.execute(
                            "INSERT OR IGNORE INTO card_pull_rates \
                             (card_version_id, slot_id, rate_numerator) VALUES (?1, ?2, ?3)",
                            params![card_version_id, slot_id, scaled_num],
                        )?;
                    }
                }
            }

            total += 1;
        }
    }

    info!(count = total, "pull rates inserted");
    Ok(())
}
