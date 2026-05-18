mod client;
mod limitless;
mod models;
mod output;
mod pokeapi;
mod raenonx;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};
use futures::stream::{self, StreamExt};
use models::{
    Ability, AbstractCard, CardEntry, CardVersion, ElementInfo, LimitlessCardData, PromoSource,
    RarityInfo, SetDetail, SetSummary, VersionRef,
};
use tracing::{error, info, warn};

// ── Element symbol map ────────────────────────────────────────────────────────

/// Maps element full name to single-letter energy symbol.
/// Dragon has no energy type in PTCGP, so it is absent here.
const ELEMENT_SYMBOLS: &[(&str, &str)] = &[
    ("Grass", "G"),
    ("Fire", "R"),
    ("Water", "W"),
    ("Lightning", "L"),
    ("Fighting", "F"),
    ("Psychic", "P"),
    ("Darkness", "D"),
    ("Metal", "M"),
    ("Colorless", "C"),
];

// ── CLI definition ────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "scraper", about = "PTCGP data scraper")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Fetch the RaenonX global-master (pack IDs, craft costs, rarities)
    GlobalMaster,

    /// Scrape all set metadata from Limitless TCG and write data/sets.json
    Sets,

    /// Scrape card detail pages from Limitless TCG
    Cards {
        #[arg(long)]
        set: Option<String>,
        #[arg(long)]
        force: bool,
    },

    /// Fetch pull rate data from RaenonX for each regular pack
    PullRates {
        #[arg(long)]
        pack: Option<String>,
        #[arg(long)]
        force: bool,
    },

    /// Fetch all Pokémon species names and national dex numbers from PokéAPI
    BasePokemon,

    /// Run the complete pipeline: global-master → sets → base-pokemon → cards → pull-rates
    All {
        #[arg(long)]
        force: bool,
    },
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "scraper=info".parse().unwrap()),
        )
        .init();

    let cli = Cli::parse();
    let client = Arc::new(client::Client::new()?);

    match cli.command {
        Command::GlobalMaster => cmd_global_master(&client).await?,
        Command::Sets => cmd_sets(&client).await?,
        Command::BasePokemon => cmd_base_pokemon(&client).await?,
        Command::Cards { set, force } => cmd_cards(&client, set.as_deref(), force).await?,
        Command::PullRates { pack, force } => {
            cmd_pull_rates(&client, pack.as_deref(), force).await?
        }
        Command::All { force } => {
            cmd_global_master(&client).await?;
            cmd_sets(&client).await?;
            cmd_base_pokemon(&client).await?;
            cmd_cards(&client, None, force).await?;
            cmd_pull_rates(&client, None, force).await?;
        }
    }

    Ok(())
}

// ── Command implementations ───────────────────────────────────────────────────

async fn cmd_global_master(client: &client::Client) -> Result<()> {
    let raw = if output::global_master_exists() {
        info!("global-master already cached; loading from disk");
        output::load_global_master()?
    } else {
        info!("fetching global-master from RaenonX");
        let raw = raenonx::fetch_global_master(client).await?;
        output::write_global_master(&raw)?;
        raw
    };

    let craft_costs = raenonx::parse_craft_costs(&raw);
    let dupe_dust = raenonx::parse_dupe_dust(&raw);
    let rarity_codes = raenonx::parse_rarity_codes(&raw);

    let elements = build_elements();
    output::write_elements(&elements)?;
    info!(count = elements.len(), "elements.json written");

    let rarities = build_rarity_list(&rarity_codes, &craft_costs, &dupe_dust);
    output::write_rarities(&rarities)?;
    info!(count = rarities.len(), "rarities.json written");

    let promo_source_codes = raenonx::parse_promo_source_codes(&raw);
    let promo_sources = build_promo_sources(&promo_source_codes);
    output::write_promo_sources(&promo_sources)?;
    info!(count = promo_sources.len(), "promo_sources.json written");

    if !output::pack_names_exist() {
        info!("fetching pack names from RaenonX pack pages");
        let named_packs = raenonx::parse_named_pack_ids(&raw);
        let mut pack_names: HashMap<String, String> = HashMap::new();
        for pack_id in &named_packs {
            let url = format!("{}/{}", raenonx::PACK_PAGE_BASE, pack_id);
            match client.get_text(&url).await {
                Ok(html) => {
                    if let Some(name) = raenonx::parse_pack_name_from_title(&html) {
                        info!(pack = pack_id, name, "resolved pack name");
                        pack_names.insert(pack_id.clone(), name);
                    } else {
                        warn!(pack = pack_id, "could not parse pack name from title");
                    }
                }
                Err(e) => warn!(pack = pack_id, "failed to fetch pack page: {e}"),
            }
        }
        output::write_pack_names(&pack_names)?;
        info!(count = pack_names.len(), "pack_names.json written");
    } else {
        info!("pack_names.json already cached");
    }

    Ok(())
}

async fn cmd_sets(client: &client::Client) -> Result<()> {
    info!("scraping set index from Limitless TCG");
    let sets = limitless::scrape_sets(client).await?;
    output::write_sets(&sets)?;
    info!(count = sets.len(), "sets.json written");

    // Load pack names (built by cmd_global_master) and expansion pack ordering
    let pack_names = if output::pack_names_exist() {
        output::load_pack_names()?
    } else {
        warn!("pack_names.json not found — run `global-master` first for accurate pack names");
        HashMap::new()
    };

    let expansion_packs = if output::global_master_exists() {
        let raw = output::load_global_master()?;
        raenonx::parse_expansion_packs(&raw)
    } else {
        HashMap::new()
    };

    for set in &sets {
        // Promo sets don't have openable packs; source info is on card versions
        let subtitles: Vec<String> = if set.is_promo {
            vec![]
        } else if let Some(pack_ids) = expansion_packs.get(&set.code) {
            pack_ids
                .iter()
                .filter_map(|id| pack_names.get(id).cloned())
                .collect()
        } else {
            vec![]
        };

        let detail = SetDetail {
            code: set.code.clone(),
            name: set.name.clone(),
            series: set.series.clone(),
            release_date: set.release_date.clone(),
            is_promo: set.is_promo,
            card_count: set.card_count,
            packs: subtitles,
        };

        output::write_set_detail(&detail)?;
        info!(set = set.code, "set detail written");
    }

    Ok(())
}

async fn cmd_base_pokemon(client: &Arc<client::Client>) -> Result<()> {
    if output::base_pokemon_exists() {
        info!("base_pokemon.json already cached");
        return Ok(());
    }
    let pokemon = pokeapi::fetch_all_species(client).await?;
    output::write_base_pokemon(&pokemon)?;
    info!(count = pokemon.len(), "base_pokemon.json written");
    Ok(())
}

async fn cmd_cards(
    client: &Arc<client::Client>,
    filter_set: Option<&str>,
    force: bool,
) -> Result<()> {
    // Load dependencies
    let raw = output::load_global_master()?;
    let card_entries = raenonx::parse_card_entries(&raw)?;
    let promo_subtitles = raenonx::parse_promo_pack_subtitles(&raw);
    let pack_names = output::load_pack_names().unwrap_or_default();

    let pokemon_map: HashMap<String, u32> = if output::base_pokemon_exists() {
        output::load_base_pokemon()
            .unwrap_or_default()
            .into_iter()
            // Normalize curly apostrophe (U+2019) to straight (U+0027) so that
            // "Farfetch’d" from PokeAPI matches "Farfetch'd" from Limitless.
            .map(|bp| (bp.name.replace('\u{2019}', "'"), bp.natdex_number))
            .collect()
    } else {
        warn!("base_pokemon.json not found — run `base-pokemon` first for natdex numbers");
        HashMap::new()
    };

    let sets_path = output::data_dir().join("sets.json");
    if !sets_path.exists() {
        anyhow::bail!("data/sets.json not found — run `sets` command first");
    }
    let sets_json = std::fs::read_to_string(&sets_path)?;
    let all_sets: Vec<SetSummary> = serde_json::from_str(&sets_json)?;

    let release_dates: HashMap<String, String> = all_sets
        .iter()
        .filter_map(|s| s.release_date.as_ref().map(|d| (s.code.clone(), d.clone())))
        .collect();

    let set_is_promo: HashMap<String, bool> = all_sets
        .iter()
        .map(|s| (s.code.clone(), s.is_promo))
        .collect();

    // Build pack_id → subtitle mapping (used to resolve source.pack IDs on card entries)
    let pack_subtitle_map = build_pack_subtitle_map(&promo_subtitles, &pack_names);

    // Build abstract card groups (regardless of set filter, since abstracts span sets)
    let groups = build_abstract_groups(&card_entries, &release_dates);

    // Build lookup: (set, num) → abstract_id and (set, num) → card_entries index
    let mut version_to_abstract: HashMap<(String, u32), u32> = HashMap::new();
    let mut version_to_entry_idx: HashMap<(String, u32), usize> = HashMap::new();
    for group in &groups {
        for &entry_idx in &group.entry_indices {
            let entry = &card_entries[entry_idx];
            for (set, num) in &entry.collection_nums {
                version_to_abstract.insert((set.clone(), *num), group.abstract_id);
                version_to_entry_idx.insert((set.clone(), *num), entry_idx);
            }
        }
    }

    // Determine which canonical versions we need Limitless data for
    let canonicals_needing_data: HashSet<(String, u32)> = if filter_set.is_none() {
        groups
            .iter()
            .filter(|g| force || !output::abstract_card_file_exists(g.abstract_id))
            .map(|g| g.canonical.clone())
            .collect()
    } else {
        HashSet::new()
    };

    // Filter sets to scrape
    let sets_to_scrape: Vec<&SetSummary> = all_sets
        .iter()
        .filter(|s| filter_set.is_none_or(|f| s.code == f))
        .collect();

    if sets_to_scrape.is_empty() {
        anyhow::bail!("no matching sets found");
    }

    // ── Fetch cards from Limitless ────────────────────────────────────────────

    let mut all_card_data: HashMap<(String, u32), LimitlessCardData> = HashMap::new();

    for set in &sets_to_scrape {
        let numbers = match limitless::scrape_card_numbers(client, &set.code).await {
            Ok(n) => n,
            Err(e) => {
                error!(set = set.code, "failed to get card numbers: {e}");
                continue;
            }
        };

        let to_fetch: Vec<u32> = numbers
            .into_iter()
            .filter(|&n| {
                let key = (set.code.clone(), n);
                let need_version = force || !output::card_version_file_exists(&set.code, n);
                let need_abstract_data = canonicals_needing_data.contains(&key);
                need_version || need_abstract_data
            })
            .collect();

        if to_fetch.is_empty() {
            info!(set = set.code, "all cards already cached");
            continue;
        }

        info!(set = set.code, count = to_fetch.len(), "fetching cards");

        let set_code = set.code.clone();
        let client_arc = Arc::clone(client);

        let fetched: Vec<(u32, LimitlessCardData)> = stream::iter(to_fetch)
            .map(|n| {
                let client = Arc::clone(&client_arc);
                let set_code = set_code.clone();
                async move {
                    match limitless::scrape_card(&client, &set_code, n).await {
                        Ok(data) => Some((n, data)),
                        Err(e) => {
                            error!(set = set_code, number = n, "fetch failed: {e}");
                            None
                        }
                    }
                }
            })
            .buffer_unordered(5)
            .filter_map(|x| async { x })
            .collect()
            .await;

        let written = fetched.len();
        for (n, data) in fetched {
            all_card_data.insert((set.code.clone(), n), data);
        }

        info!(set = set.code, written, "set done");
    }

    // ── Write card version files ──────────────────────────────────────────────

    for ((set, num), card_data) in &all_card_data {
        let abstract_id = match version_to_abstract.get(&(set.clone(), *num)) {
            Some(id) => *id,
            None => {
                warn!(
                    set,
                    number = num,
                    "no abstract mapping found, skipping version"
                );
                continue;
            }
        };
        let entry_idx = match version_to_entry_idx.get(&(set.clone(), *num)) {
            Some(idx) => *idx,
            None => {
                warn!(set, number = num, "no card entry found, skipping version");
                continue;
            }
        };
        let entry = &card_entries[entry_idx];
        let is_promo = *set_is_promo.get(set).unwrap_or(&false);
        let version = build_card_version(
            set,
            *num,
            abstract_id,
            entry,
            &pack_subtitle_map,
            card_data.illustrator.clone(),
            is_promo,
            entry.is_foil,
        );
        if let Err(e) = output::write_card_version(&version) {
            error!(set, number = num, "write failed: {e}");
        }
    }

    // ── Build and write abstract cards (only without set filter) ──────────────

    if filter_set.is_none() {
        let mut written = 0usize;
        let mut updated = 0usize;

        for group in &groups {
            if !force && output::abstract_card_file_exists(group.abstract_id) {
                // Just update the versions list in case new versions were added
                if let Err(e) =
                    output::update_abstract_card_versions(group.abstract_id, &group.versions)
                {
                    error!(id = group.abstract_id, "version update failed: {e}");
                } else {
                    updated += 1;
                }
                continue;
            }

            let (canonical_set, canonical_num) = &group.canonical;
            let card_data = all_card_data.get(&(canonical_set.clone(), *canonical_num));

            let abstract_card = build_abstract_card(
                group.abstract_id,
                group,
                &card_entries,
                card_data,
                &pokemon_map,
            );

            if let Err(e) = output::write_abstract_card(&abstract_card) {
                error!(id = group.abstract_id, "write failed: {e}");
            } else {
                written += 1;
            }
        }

        info!(
            written,
            updated,
            total = groups.len(),
            "abstract cards done"
        );
    }

    compute_and_write_duplicates(&all_sets, &release_dates)?;

    Ok(())
}

async fn cmd_pull_rates(
    client: &Arc<client::Client>,
    filter_pack: Option<&str>,
    force: bool,
) -> Result<()> {
    let raw = output::load_global_master()?;
    let regular_packs = raenonx::parse_regular_packs(&raw);
    let pack_expansion = raenonx::parse_pack_expansion(&raw);
    let promo_subtitles = raenonx::parse_promo_pack_subtitles(&raw);
    let pack_names = output::load_pack_names().unwrap_or_default();

    let pack_subtitle_map = build_pack_subtitle_map(&promo_subtitles, &pack_names);

    // Build card entry lookup for remapping pull rate card IDs
    let card_entries = raenonx::parse_card_entries(&raw)?;
    let mut card_id_to_versions: HashMap<String, Vec<(String, u32)>> = HashMap::new();
    for entry in &card_entries {
        card_id_to_versions.insert(entry.card_id.clone(), entry.collection_nums.clone());
    }

    let packs: Vec<&String> = regular_packs
        .iter()
        .filter(|id| filter_pack.is_none_or(|f| id.as_str() == f))
        .collect();

    if packs.is_empty() {
        anyhow::bail!("no matching packs found");
    }

    info!(count = packs.len(), "fetching pull rates");

    for pack_id in packs {
        let set_code = pack_expansion
            .get(pack_id)
            .map(String::as_str)
            .unwrap_or("unknown");
        let subtitle = pack_subtitle_map
            .get(pack_id)
            .cloned()
            .unwrap_or_else(|| pack_id.clone());

        if !force && output::pull_rates_file_exists(set_code, &subtitle) {
            info!(pack = pack_id, "already fetched, skipping");
            continue;
        }

        match raenonx::fetch_pack_pull_rates(client, pack_id, set_code, &subtitle).await {
            Ok(mut rates) => {
                remap_pull_rate_card_ids(&mut rates, &card_id_to_versions);
                output::write_pull_rates(&rates)?;
                let card_count = rates
                    .variants
                    .get("normal")
                    .map(|v| v.card_rates.len())
                    .unwrap_or(0);
                info!(
                    pack = pack_id,
                    set = set_code,
                    subtitle,
                    cards = card_count,
                    "written"
                );
            }
            Err(e) => error!(pack = pack_id, "pull rate fetch failed: {e}"),
        }
    }

    Ok(())
}

// ── Helper: duplicate detection ──────────────────────────────────────────────

fn compute_and_write_duplicates(
    all_sets: &[SetSummary],
    release_dates: &HashMap<String, String>,
) -> Result<()> {
    // Read every card version currently on disk
    let mut all_versions: Vec<CardVersion> = Vec::new();
    for set in all_sets {
        if let Ok(mut vs) = output::load_card_versions(&set.code) {
            all_versions.append(&mut vs);
        }
    }
    if all_versions.is_empty() {
        return Ok(());
    }

    // Group versions by identity: same physical card in PTCGP's eyes
    type VersionIdentity = (u32, String, Option<String>, bool, bool);
    let mut identity_groups: HashMap<VersionIdentity, Vec<(String, u32)>> = HashMap::new();
    for v in &all_versions {
        let key = (
            v.card_id,
            v.rarity.clone(),
            v.illustrator.clone(),
            v.is_promo,
            v.is_foil,
        );
        identity_groups
            .entry(key)
            .or_default()
            .push((v.set.clone(), v.number));
    }

    // Compute final (is_reprint, duplicates) for every version
    let mut new_state: HashMap<(String, u32), (bool, Vec<VersionRef>)> = HashMap::new();
    for versions in identity_groups.values() {
        let mut sorted = versions.clone();
        sorted.sort_by(|(a_s, a_n), (b_s, b_n)| {
            let a_d = release_dates
                .get(a_s.as_str())
                .map(String::as_str)
                .unwrap_or("9999-99-99");
            let b_d = release_dates
                .get(b_s.as_str())
                .map(String::as_str)
                .unwrap_or("9999-99-99");
            a_d.cmp(b_d).then(a_n.cmp(b_n))
        });
        for (i, (set, num)) in sorted.iter().enumerate() {
            let is_reprint = i > 0;
            let dups: Vec<VersionRef> = sorted
                .iter()
                .filter(|(s, n)| !(s == set && n == num))
                .map(|(s, n)| VersionRef {
                    set: s.clone(),
                    number: *n,
                })
                .collect();
            new_state.insert((set.clone(), *num), (is_reprint, dups));
        }
    }

    // Write back versions where the computed state differs from what is on disk
    let versions_map: HashMap<(String, u32), CardVersion> = all_versions
        .into_iter()
        .map(|v| ((v.set.clone(), v.number), v))
        .collect();

    let mut updated = 0usize;
    for ((set, num), (is_reprint, dups)) in &new_state {
        let v = match versions_map.get(&(set.clone(), *num)) {
            Some(v) => v,
            None => continue,
        };
        if v.is_reprint == *is_reprint && v.duplicates == *dups {
            continue;
        }
        let mut v = v.clone();
        v.is_reprint = *is_reprint;
        v.duplicates = dups.clone();
        output::write_card_version(&v)?;
        updated += 1;
    }

    info!(updated, "duplicate links written");
    Ok(())
}

// ── Helper: abstract card grouping ───────────────────────────────────────────

struct AbstractGroup {
    abstract_id: u32,
    entry_indices: Vec<usize>,
    versions: Vec<VersionRef>,
    canonical: (String, u32),
}

fn build_abstract_groups(
    card_entries: &[CardEntry],
    release_dates: &HashMap<String, String>,
) -> Vec<AbstractGroup> {
    // Group entries by their sorted card_ids_group
    let mut groups: HashMap<Vec<String>, Vec<usize>> = HashMap::new();
    for (i, entry) in card_entries.iter().enumerate() {
        let mut key = entry.card_ids_group.clone();
        key.sort();
        groups.entry(key).or_default().push(i);
    }

    // For each group, collect all versions and sort by release date + number
    let mut group_list: Vec<(Vec<VersionRef>, Vec<usize>)> = groups
        .into_values()
        .map(|indices| {
            let mut versions: Vec<(String, u32)> = indices
                .iter()
                .flat_map(|&i| card_entries[i].collection_nums.iter().cloned())
                .collect();
            versions.sort_by(|(a_s, a_n), (b_s, b_n)| {
                let a_d = release_dates
                    .get(a_s)
                    .map(String::as_str)
                    .unwrap_or("9999-99-99");
                let b_d = release_dates
                    .get(b_s)
                    .map(String::as_str)
                    .unwrap_or("9999-99-99");
                a_d.cmp(b_d).then(a_n.cmp(b_n))
            });
            versions.dedup();
            let refs = versions
                .iter()
                .map(|(s, n)| VersionRef {
                    set: s.clone(),
                    number: *n,
                })
                .collect();
            (refs, indices)
        })
        .collect();

    // Sort all groups by their canonical (first) version's (release_date, number)
    group_list.sort_by(|(a_vers, _), (b_vers, _)| {
        let (a_s, a_n) = (&a_vers[0].set, a_vers[0].number);
        let (b_s, b_n) = (&b_vers[0].set, b_vers[0].number);
        let a_d = release_dates
            .get(a_s)
            .map(String::as_str)
            .unwrap_or("9999-99-99");
        let b_d = release_dates
            .get(b_s)
            .map(String::as_str)
            .unwrap_or("9999-99-99");
        a_d.cmp(b_d).then(a_n.cmp(&b_n))
    });

    group_list
        .into_iter()
        .enumerate()
        .map(|(i, (versions, entry_indices))| {
            let canonical = (versions[0].set.clone(), versions[0].number);
            AbstractGroup {
                abstract_id: (i + 1) as u32,
                entry_indices,
                versions,
                canonical,
            }
        })
        .collect()
}

// ── Helper: card version construction ────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn build_card_version(
    set: &str,
    number: u32,
    abstract_id: u32,
    entry: &CardEntry,
    pack_subtitle_map: &HashMap<String, String>,
    illustrator: Option<String>,
    is_promo: bool,
    is_foil: bool,
) -> CardVersion {
    // Map source.pack IDs to subtitles (filtering out unmapped IDs like TUTORIAL_*)
    let packs: Vec<String> = entry
        .source_packs
        .iter()
        .filter_map(|id| pack_subtitle_map.get(id).cloned())
        .collect();

    // Promo sources: if any source.pack entry is an AP pack, add "Pack"
    let mut promo_sources = entry.promo_sources.clone();
    if entry.source_packs.iter().any(|id| id.starts_with("AP"))
        && !promo_sources.contains(&"Pack".to_string())
    {
        promo_sources.insert(0, "Pack".to_string());
    }

    let rarity = entry.rarity.clone();

    CardVersion {
        set: set.to_string(),
        number,
        card_id: abstract_id,
        rarity,
        illustrator,
        is_promo,
        is_foil,
        is_reprint: false,
        packs,
        promo_sources,
        duplicates: Vec::new(),
    }
}

fn build_abstract_card(
    id: u32,
    group: &AbstractGroup,
    card_entries: &[CardEntry],
    card_data: Option<&LimitlessCardData>,
    pokemon_map: &HashMap<String, u32>,
) -> AbstractCard {
    if let Some(data) = card_data {
        let is_pokemon = data.card_type == "pokemon";
        let natdex_number = if is_pokemon {
            find_natdex_number(&data.name, pokemon_map)
        } else {
            None
        };
        AbstractCard {
            id,
            name: data.name.clone(),
            card_type: data.card_type.clone(),
            natdex_number,
            element: data.element.clone(),
            stage: data.stage.clone(),
            hp: data.hp,
            retreat_cost: data.retreat_cost,
            weakness: data.weakness.clone(),
            flavor: data.flavor.clone(),
            is_ex: if is_pokemon { Some(data.is_ex) } else { None },
            is_mega: if is_pokemon { Some(data.is_mega) } else { None },
            variants: if is_pokemon {
                extract_variants(&data.name, pokemon_map)
            } else {
                Vec::new()
            },
            ability: data.ability.as_ref().map(|a| Ability {
                name: a.name.clone(),
                effect: normalize_element_placeholders(&a.effect),
            }),
            attacks: data.attacks.clone(),
            evolves_from: data.evolves_from.clone(),
            trainer_kind: data.trainer_kind.clone(),
            trainer_effect: data
                .trainer_effect
                .as_deref()
                .map(normalize_element_placeholders),
            versions: group.versions.clone(),
        }
    } else {
        let (set, num) = &group.canonical;
        let card_type = group
            .entry_indices
            .first()
            .map(|&i| card_entries[i].card_type.clone())
            .unwrap_or_else(|| "unknown".to_string());
        warn!(
            id,
            set,
            number = num,
            "no Limitless data for canonical version — minimal abstract card"
        );
        AbstractCard {
            id,
            name: format!("{set}-{num:03}"),
            card_type,
            natdex_number: None,
            element: None,
            stage: None,
            hp: None,
            retreat_cost: None,
            weakness: None,
            flavor: None,
            is_ex: None,
            is_mega: None,
            variants: Vec::new(),
            ability: None,
            attacks: Vec::new(),
            evolves_from: None,
            trainer_kind: None,
            trainer_effect: None,
            versions: group.versions.clone(),
        }
    }
}

// ── Helper: natdex lookup ─────────────────────────────────────────────────────

fn find_natdex_number(name: &str, pokemon_map: &HashMap<String, u32>) -> Option<u32> {
    if let Some(&n) = pokemon_map.get(name) {
        return Some(n);
    }

    // Try all contiguous word subsequences (longest first) to find the base species
    // name embedded in the card name. This handles any modifier prefix/suffix without
    // needing a hardcoded list — "Teal Mask Ogerpon ex" → "Ogerpon", "Mega Charizard X
    // ex" → "Charizard", "Castform Sunny Form" → "Castform", etc.
    let words: Vec<&str> = name.split_whitespace().collect();
    let n = words.len();
    for len in (1..n).rev() {
        for start in 0..=(n - len) {
            let candidate = words[start..start + len].join(" ");
            if let Some(&num) = pokemon_map.get(candidate.as_str()) {
                return Some(num);
            }
        }
    }

    None
}

// ── Helper: variant extraction ───────────────────────────────────────────────

/// Derive variant identifiers (e.g. "Alolan", "Teal Mask") by finding the base
/// Pokémon name in pokemon_map and treating the remaining words as the variant.
/// Known non-variant modifiers ("ex", "EX", "Mega") are stripped before returning.
fn extract_variants(name: &str, pokemon_map: &HashMap<String, u32>) -> Vec<String> {
    let words: Vec<&str> = name.split_whitespace().collect();
    let n = words.len();

    // Find the longest contiguous substring that is a known Pokémon name.
    let mut base_range: Option<(usize, usize)> = None;
    'outer: for len in (1..=n).rev() {
        for start in 0..=(n - len) {
            let candidate = words[start..start + len].join(" ");
            if pokemon_map.contains_key(candidate.as_str()) {
                base_range = Some((start, start + len));
                break 'outer;
            }
        }
    }

    let Some((base_start, base_end)) = base_range else {
        return Vec::new();
    };

    // Words before and after the base name, minus non-variant modifiers.
    const NON_VARIANT: &[&str] = &["ex", "EX", "Mega"];
    let remaining: Vec<&str> = words[..base_start]
        .iter()
        .chain(words[base_end..].iter())
        .copied()
        .filter(|w| !NON_VARIANT.contains(w))
        .collect();

    if remaining.is_empty() {
        Vec::new()
    } else {
        vec![remaining.join(" ")]
    }
}

// ── Helper: pack subtitle mapping ────────────────────────────────────────────

fn build_pack_subtitle_map(
    promo_subtitles: &HashMap<String, String>,
    pack_names: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut map = pack_names.clone();
    for (pack_id, subtitle) in promo_subtitles {
        map.insert(pack_id.clone(), subtitle.clone());
    }
    map
}

// ── Helper: remap pull rate card IDs to SET-NUM format ────────────────────────

fn remap_pull_rate_card_ids(
    rates: &mut models::PackPullRates,
    card_id_to_versions: &HashMap<String, Vec<(String, u32)>>,
) {
    let set_code = rates.set.clone();

    for variant in rates.variants.values_mut() {
        let remapped: HashMap<String, Vec<Option<f64>>> = variant
            .card_rates
            .iter()
            .filter_map(|(raenonx_id, slots)| {
                let versions = card_id_to_versions.get(raenonx_id)?;
                let (_ver_set, ver_num) = versions
                    .iter()
                    .find(|(s, _)| *s == set_code)
                    .or_else(|| versions.first())?;
                Some((format!("{ver_num:03}"), slots.clone()))
            })
            .collect();
        variant.card_rates = remapped;
    }
}

// ── Reference data ────────────────────────────────────────────────────────────

fn build_rarity_list(
    codes: &[String],
    craft_costs: &HashMap<String, u32>,
    dupe_dust: &HashMap<String, u32>,
) -> Vec<RarityInfo> {
    // Name and group metadata keyed by rarity code.
    // Codes are sourced dynamically from global-master; this table only provides
    // display names and grouping for known codes. Unknown future codes will still
    // appear in output with a warning.
    let metadata: &[(&str, &str, u8, &str)] = &[
        ("C", "Diamond", 1, "Common"),
        ("U", "Diamond", 2, "Uncommon"),
        ("R", "Diamond", 3, "Rare"),
        ("RR", "Diamond", 4, "Double Rare"),
        ("AR", "Star", 1, "Art Rare"),
        ("SR", "Star", 2, "Super Rare"),
        ("SAR", "Star", 2, "Special Art Rare"),
        ("IM", "Star", 3, "Immersive"),
        ("S", "Shiny", 1, "Shiny Rare"),
        ("SSR", "Shiny", 2, "Shiny Super Rare"),
        ("UR", "Crown", 1, "Crown Rare"),
    ];

    let lookup: std::collections::HashMap<&str, (&str, u8, &str)> = metadata
        .iter()
        .map(|&(code, group, count, name)| (code, (group, count, name)))
        .collect();

    // Preserve metadata order for known codes; append unknown codes at the end.
    let mut ordered: Vec<String> = metadata
        .iter()
        .map(|&(code, ..)| code.to_string())
        .filter(|c| codes.contains(c))
        .collect();
    for code in codes {
        if !ordered.contains(code) {
            warn!(
                code,
                "unknown rarity code — add to metadata table in build_rarity_list"
            );
            ordered.push(code.clone());
        }
    }

    ordered
        .iter()
        .map(|code| {
            let (group, count, name) =
                lookup
                    .get(code.as_str())
                    .copied()
                    .unwrap_or(("Unknown", 0, code.as_str()));
            RarityInfo {
                code: code.clone(),
                name: name.to_string(),
                group: group.to_string(),
                group_symbol_count: count,
                craft_cost: craft_costs.get(code).copied(),
                dupe_dust: dupe_dust.get(code).copied(),
            }
        })
        .collect()
}

fn build_elements() -> Vec<ElementInfo> {
    let known: &[(&str, Option<&str>)] = &[
        ("Grass", Some("G")),
        ("Fire", Some("R")),
        ("Water", Some("W")),
        ("Lightning", Some("L")),
        ("Fighting", Some("F")),
        ("Psychic", Some("P")),
        ("Darkness", Some("D")),
        ("Metal", Some("M")),
        ("Colorless", Some("C")),
        ("Dragon", None),
    ];
    known
        .iter()
        .map(|&(name, symbol)| ElementInfo {
            symbol: symbol.map(str::to_string),
            name: name.to_string(),
        })
        .collect()
}

fn normalize_element_placeholders(text: &str) -> String {
    let mut result = text.to_string();
    for &(name, symbol) in ELEMENT_SYMBOLS {
        result = result.replace(&format!("[{name}]"), &format!("[{symbol}]"));
    }
    result
}

fn build_promo_sources(codes: &[String]) -> Vec<PromoSource> {
    // Display names and descriptions for known promo source codes.
    // Unknown future codes will still appear in output without a description.
    let descriptions: &[(&str, &str)] = &[
        ("Pack", "Obtainable from promo packs"),
        ("Wonder Pick", "Obtainable via Wonder Pick"),
        ("Gold Shop", "Purchasable in the Gold Shop"),
        ("Shop", "Purchasable in the Shop"),
        ("Mission", "Obtainable via Missions"),
    ];
    let desc_lookup: HashMap<&str, &str> = descriptions.iter().copied().collect();

    codes
        .iter()
        .map(|code| PromoSource {
            code: code.clone(),
            description: desc_lookup.get(code.as_str()).map(|d| d.to_string()),
        })
        .collect()
}
