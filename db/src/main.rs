use anyhow::{Context, Result};
use clap::Parser;
use rusqlite::{Connection, params};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use tracing::info;

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "db", about = "Build the PTCGP SQLite database from JSON data files")]
struct Cli {
    /// Path to the data directory (default: ./data)
    #[arg(long, default_value = "data")]
    data: PathBuf,

    /// Path for the output SQLite database
    #[arg(long, default_value = "ptcgp.db")]
    output: PathBuf,

    /// Path to schema.sql
    #[arg(long, default_value = "schema.sql")]
    schema: PathBuf,
}

// ── Mirrored JSON models (subset needed for DB insertion) ─────────────────────

#[derive(Debug, Deserialize)]
struct RarityInfo {
    code: String,
    name: String,
    group: String,
    group_symbol_count: u8,
    craft_cost: Option<u32>,
    dupe_dust: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct SetSummary {
    code: String,
    name: String,
    series: String,
    release_date: Option<String>,
    is_promo: bool,
}

#[derive(Debug, Deserialize)]
struct SetDetail {
    #[allow(dead_code)]
    code: String,
    packs: Vec<PackInfo>,
}

#[derive(Debug, Deserialize)]
struct PackInfo {
    #[allow(dead_code)]
    raenonx_id: Option<String>,
    subtitle: Option<String>,
    #[allow(dead_code)]
    display_name: String,
}

#[derive(Debug, Deserialize)]
struct Card {
    set: String,
    number: u32,
    name: String,
    rarity: String,
    illustrator: Option<String>,
    card_type: String,
    // pokemon fields
    element: Option<String>,
    stage: Option<String>,
    hp: Option<u32>,
    retreat_cost: Option<u32>,
    weakness: Option<String>,
    flavor: Option<String>,
    is_ex: Option<bool>,
    is_mega: Option<bool>,
    variants: Vec<String>,
    ability: Option<Ability>,
    attacks: Vec<Attack>,
    // trainer fields
    trainer_kind: Option<String>,
    trainer_effect: Option<String>,
    packs: Vec<PackRef>,
}

#[derive(Debug, Deserialize)]
struct PackRef {
    #[allow(dead_code)]
    raenonx_id: Option<String>,
    display_name: String,
}

#[derive(Debug, Deserialize)]
struct Attack {
    name: String,
    cost: Vec<String>,
    damage: u32,
    damage_suffix: Option<String>,
    effect: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Ability {
    name: String,
    effect: String,
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

    let mut conn = Connection::open(&cli.output)
        .with_context(|| format!("opening {:?}", cli.output))?;

    conn.execute_batch("PRAGMA foreign_keys = ON;")?;

    // Create schema
    let schema = std::fs::read_to_string(&cli.schema)
        .with_context(|| format!("reading {:?}", cli.schema))?;
    conn.execute_batch(&schema)?;
    info!("schema applied");

    let tx = conn.transaction()?;

    // Insertion order must respect foreign key dependencies.
    insert_static_data(&tx)?;
    insert_rarities(&tx, &cli.data)?;
    insert_sets(&tx, &cli.data)?;
    insert_cards(&tx, &cli.data)?;

    tx.commit()?;
    info!("database written to {:?}", cli.output);
    Ok(())
}

// ── Static reference data ─────────────────────────────────────────────────────

fn insert_static_data(tx: &rusqlite::Transaction) -> Result<()> {
    // Elements
    let elements = [
        "Grass", "Fire", "Water", "Lightning", "Fighting",
        "Psychic", "Darkness", "Metal", "Dragon", "Colorless",
    ];
    for el in &elements {
        tx.execute(
            "INSERT OR IGNORE INTO elements (name) VALUES (?1)",
            params![el],
        )?;
    }

    // Stages
    let stages = ["Basic", "Stage 1", "Stage 2"];
    for stage in &stages {
        tx.execute(
            "INSERT OR IGNORE INTO stages (name) VALUES (?1)",
            params![stage],
        )?;
    }

    // Trainer kinds
    let trainer_kinds = ["Item", "Stadium", "Supporter", "Tool"];
    for kind in &trainer_kinds {
        tx.execute(
            "INSERT OR IGNORE INTO trainer_kinds (name) VALUES (?1)",
            params![kind],
        )?;
    }

    // Rarity groups
    let groups = ["Diamond", "Star", "Shiny", "Crown", "Promo"];
    for group in &groups {
        tx.execute(
            "INSERT OR IGNORE INTO rarity_groups (name) VALUES (?1)",
            params![group],
        )?;
    }

    // Pack variant names (normal / god / premium)
    let variant_names = ["Normal", "God Pack", "Premium"];
    for name in &variant_names {
        tx.execute(
            "INSERT OR IGNORE INTO pack_variant_names (name) VALUES (?1)",
            params![name],
        )?;
    }

    info!("static reference data inserted");
    Ok(())
}

// ── Rarities ──────────────────────────────────────────────────────────────────

fn insert_rarities(tx: &rusqlite::Transaction, data: &Path) -> Result<()> {
    let path = data.join("rarities.json");
    if !path.exists() {
        tracing::warn!("rarities.json not found, skipping");
        return Ok(());
    }

    let rarities: Vec<RarityInfo> = serde_json::from_str(&std::fs::read_to_string(&path)?)?;

    for r in &rarities {
        // Ensure rarity_class exists
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

        tx.execute(
            "INSERT OR IGNORE INTO rarities (class_id, code, name, craft_cost, dupe_dust) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![class_id, r.code, r.name, r.craft_cost, r.dupe_dust],
        )?;
    }

    info!(count = rarities.len(), "rarities inserted");
    Ok(())
}

// ── Sets and packs ────────────────────────────────────────────────────────────

fn insert_sets(tx: &rusqlite::Transaction, data: &Path) -> Result<()> {
    let sets_path = data.join("sets.json");
    if !sets_path.exists() {
        tracing::warn!("sets.json not found, skipping");
        return Ok(());
    }

    let sets: Vec<SetSummary> = serde_json::from_str(&std::fs::read_to_string(&sets_path)?)?;

    for set in &sets {
        // Ensure series exists
        tx.execute(
            "INSERT OR IGNORE INTO series (code) VALUES (?1)",
            params![set.series],
        )?;
        let series_id: i64 = tx.query_row(
            "SELECT id FROM series WHERE code = ?1",
            params![set.series],
            |row| row.get(0),
        )?;

        tx.execute(
            "INSERT OR IGNORE INTO sets (series_id, code, name, release_date) \
             VALUES (?1, ?2, ?3, ?4)",
            params![series_id, set.code, set.name, set.release_date],
        )?;

        let set_id: i64 = tx.query_row(
            "SELECT id FROM sets WHERE code = ?1",
            params![set.code],
            |row| row.get(0),
        )?;

        if set.is_promo {
            tx.execute(
                "INSERT OR IGNORE INTO promo_sets (set_id) VALUES (?1)",
                params![set_id],
            )?;
        }

        // Load pack details from set.json
        let detail_path = data.join("sets").join(&set.code).join("set.json");
        if !detail_path.exists() {
            continue;
        }
        let detail: SetDetail =
            serde_json::from_str(&std::fs::read_to_string(&detail_path)?)?;

        for pack in &detail.packs {
            tx.execute(
                "INSERT OR IGNORE INTO packs (set_id) VALUES (?1)",
                params![set_id],
            )?;
            let pack_id: i64 = tx.last_insert_rowid();

            if let Some(subtitle) = &pack.subtitle {
                tx.execute(
                    "INSERT OR IGNORE INTO pack_subtitles (pack_id, subtitle) VALUES (?1, ?2)",
                    params![pack_id, subtitle],
                )?;
            }
        }
    }

    info!(count = sets.len(), "sets inserted");
    Ok(())
}

// ── Cards ─────────────────────────────────────────────────────────────────────

fn insert_cards(tx: &rusqlite::Transaction, data: &Path) -> Result<()> {
    let sets_path = data.join("sets.json");
    if !sets_path.exists() {
        return Ok(());
    }

    let sets: Vec<SetSummary> = serde_json::from_str(&std::fs::read_to_string(&sets_path)?)?;
    let mut total = 0usize;

    for set in &sets {
        let cards_dir = data.join("sets").join(&set.code).join("cards");
        if !cards_dir.exists() {
            continue;
        }

        let mut entries: Vec<_> = std::fs::read_dir(&cards_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map_or(false, |x| x == "json"))
            .collect();
        entries.sort_by_key(|e| e.path());

        for entry in entries {
            let card: Card =
                serde_json::from_str(&std::fs::read_to_string(entry.path())?)?;
            if let Err(e) = insert_card(tx, &card) {
                tracing::error!(
                    set = card.set,
                    number = card.number,
                    "insert failed: {e}"
                );
            } else {
                total += 1;
            }
        }
    }

    info!(total, "cards inserted");
    Ok(())
}

fn insert_card(tx: &rusqlite::Transaction, card: &Card) -> Result<()> {
    // card_names (dedup by name)
    tx.execute(
        "INSERT OR IGNORE INTO card_names (name) VALUES (?1)",
        params![card.name],
    )?;
    let name_id: i64 = tx.query_row(
        "SELECT id FROM card_names WHERE name = ?1",
        params![card.name],
        |row| row.get(0),
    )?;

    // cards (one abstract card per name+type combination)
    // For simplicity we create a new card row per scrape entry; the DB builder
    // can be extended to merge cards with the same name and matching content.
    tx.execute(
        "INSERT INTO cards (name_id) VALUES (?1)",
        params![name_id],
    )?;
    let card_id = tx.last_insert_rowid();

    // illustrators
    let illustrator_id: Option<i64> = if let Some(name) = &card.illustrator {
        tx.execute(
            "INSERT OR IGNORE INTO illustrators (name) VALUES (?1)",
            params![name],
        )?;
        Some(tx.query_row(
            "SELECT id FROM illustrators WHERE name = ?1",
            params![name],
            |row| row.get(0),
        )?)
    } else {
        None
    };

    // Look up set and rarity IDs
    let set_id: i64 = tx.query_row(
        "SELECT id FROM sets WHERE code = ?1",
        params![card.set],
        |row| row.get(0),
    )?;

    // Cards with no rarity text (some promos) fall back to "Promo pack"
    let rarity_code = if card.rarity.is_empty() { "Promo pack" } else { &card.rarity };
    let rarity_id: i64 = tx.query_row(
        "SELECT id FROM rarities WHERE code = ?1",
        params![rarity_code],
        |row| row.get(0),
    )?;

    // card_versions
    let illustrator_id = illustrator_id.unwrap_or_else(|| {
        // Insert a placeholder illustrator if none listed
        tx.execute(
            "INSERT OR IGNORE INTO illustrators (name) VALUES ('Unknown')",
            [],
        ).ok();
        tx.query_row(
            "SELECT id FROM illustrators WHERE name = 'Unknown'",
            [],
            |row| row.get(0),
        ).unwrap_or(1)
    });

    tx.execute(
        "INSERT INTO card_versions (card_id, set_id, rarity_id, illustrator_id, number) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![card_id, set_id, rarity_id, illustrator_id, card.number],
    )?;
    let version_id = tx.last_insert_rowid();

    // Type-specific data
    if card.card_type == "pokemon" {
        insert_pokemon_card(tx, card_id, card)?;
    } else if card.card_type == "trainer" {
        insert_trainer_card(tx, card_id, card)?;
    }

    // card_packs — best-effort: match by display name against pack_subtitles
    for pack_ref in &card.packs {
        // Look up the pack by subtitle or by being the only pack in the set
        let pack_id: Option<i64> = if let Some(subtitle) = pack_ref.display_name.strip_suffix(" pack") {
            tx.query_row(
                "SELECT p.id FROM packs p \
                 JOIN pack_subtitles ps ON p.id = ps.pack_id \
                 WHERE p.set_id = ?1 AND ps.subtitle = ?2",
                params![set_id, subtitle],
                |row| row.get(0),
            ).ok()
        } else {
            // Single-pack set — take the first pack in the set
            tx.query_row(
                "SELECT id FROM packs WHERE set_id = ?1 LIMIT 1",
                params![set_id],
                |row| row.get(0),
            ).ok()
        };

        if let Some(pid) = pack_id {
            tx.execute(
                "INSERT OR IGNORE INTO card_packs (card_version_id, pack_id) VALUES (?1, ?2)",
                params![version_id, pid],
            )?;
        }
    }

    Ok(())
}

fn insert_pokemon_card(tx: &rusqlite::Transaction, card_id: i64, card: &Card) -> Result<()> {
    // base_pokemon — insert by name (natdex_number filled in later from PokeAPI or manual data)
    let base_name = derive_base_name(&card.name);
    tx.execute(
        "INSERT OR IGNORE INTO base_pokemon (name, natdex_number) VALUES (?1, NULL)",
        params![base_name],
    )?;
    let base_id: i64 = tx.query_row(
        "SELECT id FROM base_pokemon WHERE name = ?1",
        params![base_name],
        |row| row.get(0),
    )?;

    let element_id: i64 = if let Some(el) = &card.element {
        tx.query_row(
            "SELECT id FROM elements WHERE name = ?1",
            params![el],
            |row| row.get(0),
        )?
    } else {
        return Ok(());
    };

    let stage_id: i64 = if let Some(st) = &card.stage {
        tx.query_row(
            "SELECT id FROM stages WHERE name = ?1",
            params![st],
            |row| row.get(0),
        )?
    } else {
        return Ok(());
    };

    tx.execute(
        "INSERT OR IGNORE INTO pokemon_cards \
         (card_id, base_id, element_id, stage_id, retreat_cost, hp) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            card_id,
            base_id,
            element_id,
            stage_id,
            card.retreat_cost.unwrap_or(0),
            card.hp.unwrap_or(0),
        ],
    )?;

    // Flavor text (separate table, nullable)
    if let Some(flavor) = &card.flavor {
        tx.execute(
            "INSERT OR IGNORE INTO pokemon_flavor_text (card_id, flavor) VALUES (?1, ?2)",
            params![card_id, flavor],
        )?;
    }

    // Weakness
    if let Some(weakness_el) = &card.weakness {
        if let Ok(el_id) = tx.query_row::<i64, _, _>(
            "SELECT id FROM elements WHERE name = ?1",
            params![weakness_el],
            |row| row.get(0),
        ) {
            tx.execute(
                "INSERT OR IGNORE INTO weaknesses (card_id, element_id) VALUES (?1, ?2)",
                params![card_id, el_id],
            )?;
        }
    }

    // ex / mega
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

    // Variants
    for variant in &card.variants {
        tx.execute(
            "INSERT OR IGNORE INTO pokemon_variants (ident) VALUES (?1)",
            params![variant],
        )?;
        let variant_id: i64 = tx.query_row(
            "SELECT id FROM pokemon_variants WHERE ident = ?1",
            params![variant],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO pokemon_variant_tags (card_id, variant_id) VALUES (?1, ?2)",
            params![card_id, variant_id],
        )?;
    }

    // Ability
    if let Some(ability) = &card.ability {
        tx.execute(
            "INSERT OR IGNORE INTO ability_names (name) VALUES (?1)",
            params![ability.name],
        )?;
        let name_id: i64 = tx.query_row(
            "SELECT id FROM ability_names WHERE name = ?1",
            params![ability.name],
            |row| row.get(0),
        )?;

        tx.execute(
            "INSERT OR IGNORE INTO ability_effects (effect) VALUES (?1)",
            params![ability.effect],
        )?;
        let effect_id: i64 = tx.query_row(
            "SELECT id FROM ability_effects WHERE effect = ?1",
            params![ability.effect],
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

    // Attacks
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

fn insert_trainer_card(tx: &rusqlite::Transaction, card_id: i64, card: &Card) -> Result<()> {
    let kind_name = card.trainer_kind.as_deref().unwrap_or("Item");
    let kind_id: i64 = tx.query_row(
        "SELECT id FROM trainer_kinds WHERE name = ?1",
        params![kind_name],
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
        "INSERT OR IGNORE INTO trainer_cards (card_id, kind_id, effect_id) \
         VALUES (?1, ?2, ?3)",
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

    let suffix_cp: Option<i64> = attack.damage_suffix.as_deref().and_then(|s| {
        s.chars().next().map(|c| c as i64)
    });

    tx.execute(
        "INSERT OR IGNORE INTO attacks (name_id, effect_id, base_damage, damage_suffix_codepoint) \
         VALUES (?1, ?2, ?3, ?4)",
        params![name_id, effect_id, attack.damage, suffix_cp],
    )?;

    let attack_id: i64 = tx.query_row(
        "SELECT id FROM attacks WHERE name_id = ?1 AND base_damage = ?2 \
         AND (effect_id IS ?3) AND (damage_suffix_codepoint IS ?4)",
        params![name_id, attack.damage, effect_id, suffix_cp],
        |row| row.get(0),
    )?;

    // Energy cost
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

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Strip known modifiers to derive the base Pokémon name for the base_pokemon
/// table. National Pokédex numbers must be populated separately (e.g. from
/// PokeAPI) since neither Limitless nor RaenonX provides them.
fn derive_base_name(card_name: &str) -> String {
    let name = card_name
        .trim_end_matches(" ex")
        .trim_start_matches("Mega ");

    // Strip known regional prefix variants
    let prefixes = ["Alolan ", "Galarian ", "Hisuian ", "Paldean "];
    for prefix in &prefixes {
        if let Some(rest) = name.strip_prefix(prefix) {
            return rest.to_string();
        }
    }

    name.to_string()
}
