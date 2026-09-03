mod client;
mod limitless;
mod models;
mod output;
mod pokeapi;
mod raenonx;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};
use futures::stream::{self, StreamExt};
use models::{
    canonicalize_apostrophes, Ability, AbstractCard, CardEntry, CardSource, CardVersion,
    ElementInfo, LimitlessCardData, PackVariantName, RarityInfo, SetSummary, VersionRef,
};
use rust_decimal::Decimal;
use serde_json::Value;
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

    /// Bail if anything has been recorded so far, without consuming the collector.
    /// Used to stop before a write step rather than after it.
    fn check(&self) -> Result<()> {
        if !self.items.is_empty() {
            anyhow::bail!(
                "{} constraint violation(s) — re-run with --lenient to proceed anyway",
                self.items.len()
            );
        }
        Ok(())
    }

    fn finish(self) -> Result<()> {
        self.check()
    }
}

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
        /// Log constraint violations as warnings instead of failing
        #[arg(long)]
        lenient: bool,
        /// Omit cards Limitless has no page for yet (ignored with --set)
        #[arg(long)]
        skip_incomplete: bool,
    },

    /// Fetch pull rate data from RaenonX for each regular pack
    PullRates {
        #[arg(long)]
        pack: Option<String>,
        #[arg(long)]
        force: bool,
        /// Omit cards with no card_version file (see `cards --skip-incomplete`)
        #[arg(long)]
        skip_incomplete: bool,
    },

    /// Fetch all Pokémon species names and national dex numbers from PokéAPI
    BasePokemon,

    /// Check everything in data/ for internal consistency (no network)
    Validate {
        /// Log constraint violations as warnings instead of failing
        #[arg(long)]
        lenient: bool,
    },

    /// Run the complete pipeline: global-master → sets → base-pokemon → cards → pull-rates
    All {
        #[arg(long)]
        force: bool,
        /// Log constraint violations as warnings instead of failing
        #[arg(long)]
        lenient: bool,
        /// Omit cards Limitless has no page for yet
        #[arg(long)]
        skip_incomplete: bool,
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
        Command::Cards {
            set,
            force,
            lenient,
            skip_incomplete,
        } => cmd_cards(&client, set.as_deref(), force, lenient, skip_incomplete).await?,
        Command::PullRates {
            pack,
            force,
            skip_incomplete,
        } => cmd_pull_rates(&client, pack.as_deref(), force, skip_incomplete).await?,
        Command::Validate { lenient } => cmd_validate(lenient)?,
        Command::All {
            force,
            lenient,
            skip_incomplete,
        } => {
            let raw = raenonx::fetch_global_master(&client).await?;
            write_reference_data(&raw)?;
            let pack_names = fetch_pack_names(&client, &raw).await;
            let expansion_packs = raenonx::parse_expansion_packs(&raw);
            let promo_subtitles = raenonx::parse_promo_pack_subtitles(&raw);
            let set_end_dates = raenonx::parse_set_end_dates(&raw);
            run_sets(
                &client,
                &expansion_packs,
                &pack_names,
                &promo_subtitles,
                &set_end_dates,
            )
            .await?;
            cmd_base_pokemon(&client).await?;
            run_cards(
                &client,
                &raw,
                &pack_names,
                None,
                force,
                lenient,
                skip_incomplete,
            )
            .await?;
            run_pull_rates(&client, &raw, &pack_names, None, force, skip_incomplete).await?;
        }
    }

    Ok(())
}

// ── Command implementations ───────────────────────────────────────────────────

async fn cmd_global_master(client: &client::Client) -> Result<()> {
    info!("fetching global-master from RaenonX");
    let raw = raenonx::fetch_global_master(client).await?;
    write_reference_data(&raw)
}

fn write_reference_data(raw: &Value) -> Result<()> {
    let elements = build_elements();
    output::write_elements(&elements)?;
    info!(count = elements.len(), "elements.json written");

    let craft_costs = raenonx::parse_craft_costs(raw);
    let dupe_dust = raenonx::parse_dupe_dust(raw);
    let rarity_codes = raenonx::parse_rarity_codes(raw);
    let rarities = build_rarity_list(&rarity_codes, &craft_costs, &dupe_dust);
    output::write_rarities(&rarities)?;
    info!(count = rarities.len(), "rarities.json written");

    let card_source_codes = raenonx::parse_card_source_codes(raw);
    let card_sources = build_card_sources(&card_source_codes);
    output::write_card_sources(&card_sources)?;
    info!(count = card_sources.len(), "card_sources.json written");

    let pack_variant_names = build_pack_variant_names();
    output::write_pack_variant_names(&pack_variant_names)?;
    info!(
        count = pack_variant_names.len(),
        "pack_variant_names.json written"
    );

    Ok(())
}

fn build_pack_variant_names() -> Vec<PackVariantName> {
    vec![
        PackVariantName {
            code: "normal".into(),
            name: "Regular Pack".into(),
        },
        PackVariantName {
            code: "rare".into(),
            name: "Rare Pack".into(),
        },
        PackVariantName {
            code: "plus1".into(),
            name: "Regular Pack +1".into(),
        },
        PackVariantName {
            code: "themed".into(),
            name: "Themed Rare Pack".into(),
        },
    ]
}

/// Resolve RaenonX pack IDs to the pack names Limitless uses.
///
/// Limitless is authoritative for the wording. A multi-pack set lists its packs on
/// its set page; a single-pack set has no pack selector to read, so the pack takes
/// the set's own name. RaenonX supplies the pack IDs and — for multi-pack sets only
/// — the name used to line an ID up with its Limitless label, since RaenonX orders
/// packs by their in-game order and Limitless alphabetically.
async fn fetch_pack_names(client: &client::Client, raw: &Value) -> HashMap<String, String> {
    let named_packs: HashSet<String> = raenonx::parse_named_pack_ids(raw).into_iter().collect();

    let set_names: HashMap<String, String> = match limitless::scrape_sets(client).await {
        Ok(sets) => sets.into_iter().map(|s| (s.code, s.name)).collect(),
        Err(e) => {
            warn!("failed to scrape set index — pack names unresolved: {e}");
            return HashMap::new();
        }
    };

    let mut pack_names = HashMap::new();

    for (set_code, pack_ids) in raenonx::parse_expansion_packs(raw) {
        // Promo packs are named "Vol N" downstream and never appear here.
        let pack_ids: Vec<String> = pack_ids
            .into_iter()
            .filter(|id| named_packs.contains(id))
            .collect();

        if let [pack_id] = pack_ids.as_slice() {
            match set_names.get(&set_code) {
                Some(name) => {
                    pack_names.insert(pack_id.clone(), name.clone());
                }
                None => {
                    warn!(set = %set_code, "set not listed on Limitless — pack name unresolved")
                }
            }
            continue;
        }
        if pack_ids.is_empty() {
            continue;
        }

        let labels = match limitless::scrape_set_packs(client, &set_code).await {
            Ok(labels) if !labels.is_empty() => labels,
            Ok(_) => {
                warn!(set = %set_code, "Limitless lists no packs for a multi-pack set");
                continue;
            }
            Err(e) => {
                warn!(set = %set_code, "failed to fetch set page: {e}");
                continue;
            }
        };

        for pack_id in &pack_ids {
            let Some(raenonx_name) = fetch_raenonx_pack_name(client, pack_id).await else {
                continue;
            };
            match labels
                .iter()
                .find(|l| l.eq_ignore_ascii_case(&raenonx_name))
            {
                Some(label) => {
                    pack_names.insert(pack_id.clone(), label.clone());
                }
                None => {
                    warn!(
                        pack = %pack_id,
                        name = %raenonx_name,
                        "no matching Limitless pack label — falling back to the RaenonX name"
                    );
                    pack_names.insert(pack_id.clone(), raenonx_name);
                }
            }
        }
    }

    info!(count = pack_names.len(), "pack names resolved");
    pack_names
}

async fn fetch_raenonx_pack_name(client: &client::Client, pack_id: &str) -> Option<String> {
    let url = format!("{}/{}", raenonx::PACK_PAGE_BASE, pack_id);
    match client.get_text(&url).await {
        Ok(html) => match raenonx::parse_pack_name_from_title(&html) {
            Some(name) => Some(name),
            None => {
                warn!(pack = pack_id, "could not parse pack name from title");
                None
            }
        },
        Err(e) => {
            warn!(pack = pack_id, "failed to fetch pack page: {e}");
            None
        }
    }
}

async fn cmd_sets(client: &client::Client) -> Result<()> {
    let raw = raenonx::fetch_global_master(client).await?;
    let pack_names = fetch_pack_names(client, &raw).await;
    let expansion_packs = raenonx::parse_expansion_packs(&raw);
    let promo_subtitles = raenonx::parse_promo_pack_subtitles(&raw);
    let set_end_dates = raenonx::parse_set_end_dates(&raw);
    run_sets(
        client,
        &expansion_packs,
        &pack_names,
        &promo_subtitles,
        &set_end_dates,
    )
    .await
}

async fn run_sets(
    client: &client::Client,
    expansion_packs: &HashMap<String, Vec<String>>,
    pack_names: &HashMap<String, String>,
    promo_subtitles: &HashMap<String, String>,
    set_end_dates: &HashMap<String, String>,
) -> Result<()> {
    info!("scraping set index from Limitless TCG");
    let mut sets = limitless::scrape_sets(client).await?;

    // Patch in end dates from RaenonX for sets whose packs have a known endEpoch.
    for set in &mut sets {
        if let Some(avail) = &mut set.availability {
            if let Some(end) = set_end_dates.get(&set.code) {
                avail.end = Some(end.clone());
            }
        }
    }

    for set in &mut sets {
        set.packs = if set.is_promo {
            let prefix = format!("PROMO_{}_", set.series.to_uppercase());
            let vol_nums: BTreeSet<u32> = promo_subtitles
                .values()
                .filter(|v| v.starts_with(&prefix))
                .filter_map(|v| v.rsplit('_').next()?.parse().ok())
                .collect();
            vol_nums.into_iter().map(|n| format!("Vol {n}")).collect()
        } else if let Some(pack_ids) = expansion_packs.get(&set.code) {
            pack_ids
                .iter()
                .filter_map(|id| pack_names.get(id).cloned())
                .collect()
        } else {
            vec![]
        };
    }

    output::write_sets(&sets)?;
    info!(count = sets.len(), "sets.json written");

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
    lenient: bool,
    skip_incomplete: bool,
) -> Result<()> {
    let raw = raenonx::fetch_global_master(client).await?;
    let pack_names = fetch_pack_names(client, &raw).await;
    run_cards(
        client,
        &raw,
        &pack_names,
        filter_set,
        force,
        lenient,
        skip_incomplete,
    )
    .await
}

async fn run_cards(
    client: &Arc<client::Client>,
    raw: &Value,
    pack_names: &HashMap<String, String>,
    filter_set: Option<&str>,
    force: bool,
    lenient: bool,
    skip_incomplete: bool,
) -> Result<()> {
    let mut violations = Violations::new(lenient);
    let mut card_entries = raenonx::parse_card_entries(raw)?;
    let promo_subtitles = raenonx::parse_promo_pack_subtitles(raw);

    let pokemon_map: HashMap<String, u32> = if output::base_pokemon_exists() {
        output::load_base_pokemon()
            .unwrap_or_default()
            .into_iter()
            .map(|bp| (canonicalize_apostrophes(&bp.name), bp.natdex_number))
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
        .filter_map(|s| {
            s.availability
                .as_ref()
                .map(|a| (s.code.clone(), a.start.clone()))
        })
        .collect();

    let set_is_promo: HashMap<String, bool> = all_sets
        .iter()
        .map(|s| (s.code.clone(), s.is_promo))
        .collect();

    // Build pack_id → subtitle mapping (used to resolve source.pack IDs on card entries)
    let pack_subtitle_map = build_pack_subtitle_map(&promo_subtitles, pack_names);

    // Build pack_id → set code mapping (used to scope source.pack IDs to the card's own set)
    let pack_expansion = raenonx::parse_pack_expansion(raw);

    // Filter sets to scrape
    let sets_to_scrape: Vec<&SetSummary> = all_sets
        .iter()
        .filter(|s| filter_set.is_none_or(|f| s.code == f))
        .collect();

    if sets_to_scrape.is_empty() {
        anyhow::bail!("no matching sets found");
    }

    // ── Card number lists from Limitless ──────────────────────────────────────
    //
    // Fetched before grouping because --skip-incomplete drops versions from the
    // entries, and abstract IDs are assigned from whatever groups survive.
    let mut set_numbers: HashMap<String, Vec<u32>> = HashMap::new();
    for set in &sets_to_scrape {
        match limitless::scrape_card_numbers(client, &set.code).await {
            Ok(numbers) => {
                set_numbers.insert(set.code.clone(), numbers);
            }
            Err(e) => error!(set = set.code, "failed to get card numbers: {e}"),
        }
    }

    // RaenonX lists cards Limitless has not published a page for yet (e.g. P-B 87+
    // at the time of writing). Their versions have no card_version file, so an
    // abstract card referencing one dangles. Drop those versions, and any card
    // left with none.
    //
    // Dropping a card renumbers the abstract IDs that card_versions embed, so this
    // half of --skip-incomplete needs a full run; the reverse gap (a Limitless page
    // RaenonX has no entry for) is handled in the fetch loop and is always safe.
    if skip_incomplete {
        if filter_set.is_some() {
            warn!("--skip-incomplete: skipping cards RaenonX has no entry for, but dropping versions Limitless lacks needs a full run, not --set");
        } else {
            let available: HashSet<(String, u32)> = set_numbers
                .iter()
                .flat_map(|(set, nums)| nums.iter().map(|n| (set.clone(), *n)))
                .collect();
            let mut dropped_versions = 0usize;
            for entry in &mut card_entries {
                entry.collection_nums.retain(|(set, num)| {
                    // A set whose list failed to fetch is left alone — a missing
                    // list is not evidence that its cards are missing.
                    let keep =
                        !set_numbers.contains_key(set) || available.contains(&(set.clone(), *num));
                    if !keep {
                        dropped_versions += 1;
                    }
                    keep
                });
            }
            let before = card_entries.len();
            card_entries.retain(|e| !e.collection_nums.is_empty());
            info!(
                versions = dropped_versions,
                cards = before - card_entries.len(),
                "dropped versions Limitless has no page for"
            );
        }
    }

    // Build abstract card groups (regardless of set filter, since abstracts span sets).
    // Existing IDs are read first so regeneration keeps them stable.
    let id_index = output::load_abstract_id_index()?;
    let groups = build_abstract_groups(&card_entries, &release_dates, &id_index);

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

    // ── Fetch cards from Limitless ────────────────────────────────────────────

    let mut all_card_data: HashMap<(String, u32), LimitlessCardData> = HashMap::new();

    for set in &sets_to_scrape {
        let Some(numbers) = set_numbers.get(&set.code) else {
            continue;
        };

        let to_fetch: Vec<u32> = numbers
            .iter()
            .copied()
            .filter(|&n| {
                let key = (set.code.clone(), n);
                // The reverse gap: Limitless publishes a page RaenonX has no entry
                // for. There is no abstract card to hang the version on, so it is
                // dropped either way — but by default that is reported as a
                // violation, which fails a strict run.
                if skip_incomplete && !version_to_abstract.contains_key(&key) {
                    return false;
                }
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
                violations.add(format!(
                    "{set}/{num:03}: no abstract card mapping — version skipped"
                ));
                continue;
            }
        };
        let entry_idx = match version_to_entry_idx.get(&(set.clone(), *num)) {
            Some(idx) => *idx,
            None => {
                violations.add(format!(
                    "{set}/{num:03}: no card entry found — version skipped"
                ));
                continue;
            }
        };
        let entry = &card_entries[entry_idx];
        let is_promo = *set_is_promo.get(set).unwrap_or(&false);
        let version = match build_card_version(
            set,
            *num,
            abstract_id,
            entry,
            &pack_subtitle_map,
            &pack_expansion,
            card_data.illustrator.clone(),
            card_data.card_source.as_deref(),
            is_promo,
            entry.is_foil,
            entry.is_tradable,
        ) {
            Some(v) => v,
            None => {
                violations.add(format!("{set}/{num:03}: no card source — version skipped"));
                continue;
            }
        };
        if let Err(e) = output::write_card_version(&version) {
            error!(set, number = num, "write failed: {e}");
        }
    }

    // ── Build and write abstract cards (only without set filter) ──────────────

    if filter_set.is_none() {
        let mut written = 0usize;
        let mut updated = 0usize;

        // A card whose canonical version has no Limitless data can only be built as
        // a name-less placeholder. Refuse before writing anything: a placeholder
        // written to disk takes an abstract ID, and the next run — which either
        // replaces or drops it — would then be free to hand that ID to a different
        // card. Bail up front so a failed run leaves no card behind.
        let incomplete: Vec<&AbstractGroup> = groups
            .iter()
            .filter(|g| {
                (force || !output::abstract_card_file_exists(g.abstract_id))
                    && !all_card_data.contains_key(&g.canonical)
            })
            .collect();
        for group in &incomplete {
            let (set, num) = &group.canonical;
            violations.add(format!(
                "{set}/{num:03}: no Limitless data — card skipped (use --skip-incomplete to omit it deliberately)"
            ));
        }
        let incomplete_ids: HashSet<u32> = incomplete.iter().map(|g| g.abstract_id).collect();
        if !incomplete_ids.is_empty() {
            violations.check()?;
        }

        for group in groups
            .iter()
            .filter(|g| !incomplete_ids.contains(&g.abstract_id))
        {
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

            let card_data = all_card_data.get(&group.canonical);

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

        let live_ids: HashSet<u32> = groups.iter().map(|g| g.abstract_id).collect();
        match output::remove_abstract_cards_not_in(&live_ids) {
            Ok(0) => {}
            Ok(n) => info!(removed = n, "stale abstract cards deleted"),
            Err(e) => error!("stale abstract card cleanup failed: {e}"),
        }

        info!(
            written,
            updated,
            total = groups.len(),
            "abstract cards done"
        );
    }

    compute_and_write_duplicates(&all_sets, &release_dates)?;

    // ── Data integrity validation pass ────────────────────────────────────────

    let known_elements =
        load_known_set::<ElementInfo, _>(&output::data_dir().join("elements.json"), |e| e.name);
    let known_rarities =
        load_known_set::<RarityInfo, _>(&output::data_dir().join("rarities.json"), |r| r.code);
    let known_card_names = collect_card_names()?;
    let base_pokemon_loaded = output::base_pokemon_exists();

    let abstract_dir = output::data_dir().join("cards");
    if abstract_dir.exists() {
        let mut entries: Vec<_> = std::fs::read_dir(&abstract_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .collect();
        entries.sort_by_key(|e| e.path());
        for entry in entries {
            match serde_json::from_str::<AbstractCard>(&std::fs::read_to_string(entry.path())?) {
                Ok(card) => validate_abstract_card(
                    &mut violations,
                    &card,
                    &known_elements,
                    &known_card_names,
                    base_pokemon_loaded,
                ),
                Err(e) => violations.add(format!("{}: parse error: {e}", entry.path().display())),
            }
        }
    }

    let set_packs: HashMap<String, HashSet<String>> = all_sets
        .iter()
        .map(|s| (s.code.clone(), s.packs.iter().cloned().collect()))
        .collect();
    let all_known_packs: HashSet<String> = set_packs.values().flatten().cloned().collect();

    for set in all_sets
        .iter()
        .filter(|s| filter_set.is_none_or(|f| s.code == f))
    {
        match output::load_card_versions(&set.code) {
            Ok(versions) => {
                for version in &versions {
                    validate_card_version(
                        &mut violations,
                        version,
                        &known_rarities,
                        &set_is_promo,
                        &set_packs,
                        &all_known_packs,
                    );
                }
            }
            Err(e) => violations.add(format!("{}: failed to load card versions: {e}", set.code)),
        }
    }

    violations.finish()?;
    Ok(())
}

async fn cmd_pull_rates(
    client: &Arc<client::Client>,
    filter_pack: Option<&str>,
    force: bool,
    skip_incomplete: bool,
) -> Result<()> {
    let raw = raenonx::fetch_global_master(client).await?;
    let pack_names = fetch_pack_names(client, &raw).await;
    run_pull_rates(
        client,
        &raw,
        &pack_names,
        filter_pack,
        force,
        skip_incomplete,
    )
    .await
}

async fn run_pull_rates(
    client: &Arc<client::Client>,
    raw: &Value,
    pack_names: &HashMap<String, String>,
    filter_pack: Option<&str>,
    force: bool,
    skip_incomplete: bool,
) -> Result<()> {
    let regular_packs = raenonx::parse_regular_packs(raw);
    let pack_expansion = raenonx::parse_pack_expansion(raw);
    let promo_subtitles = raenonx::parse_promo_pack_subtitles(raw);

    let pack_subtitle_map = build_pack_subtitle_map(&promo_subtitles, pack_names);

    // Build card entry lookup for remapping pull rate card IDs
    let card_entries = raenonx::parse_card_entries(raw)?;
    let mut card_id_to_versions: HashMap<String, Vec<(String, u32)>> = HashMap::new();
    let mut card_id_to_rarity: HashMap<String, String> = HashMap::new();
    let mut card_id_to_is_foil: HashMap<String, bool> = HashMap::new();
    for entry in &card_entries {
        // `cards --skip-incomplete` drops versions with no card_version file; a
        // pull rate naming one would point at a card that was never written.
        let versions: Vec<(String, u32)> = entry
            .collection_nums
            .iter()
            .filter(|(set, num)| !skip_incomplete || output::card_version_file_exists(set, *num))
            .cloned()
            .collect();
        card_id_to_versions.insert(entry.card_id.clone(), versions);
        card_id_to_rarity.insert(entry.card_id.clone(), entry.rarity.clone());
        card_id_to_is_foil.insert(entry.card_id.clone(), entry.is_foil);
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
                raenonx::fix_card_rates_from_rarity(
                    &mut rates,
                    &card_id_to_rarity,
                    &card_id_to_is_foil,
                );
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
    id_index: &output::AbstractIdIndex,
) -> Vec<AbstractGroup> {
    // Group entries by their sorted card_ids_group. A BTreeMap keeps iteration
    // order fixed; a HashMap here would let ties in the sort below fall out in a
    // different order on every run, since Rust randomizes HashMap iteration.
    let mut groups: BTreeMap<Vec<String>, Vec<usize>> = BTreeMap::new();
    for (i, entry) in card_entries.iter().enumerate() {
        let mut key = entry.card_ids_group.clone();
        key.sort();
        groups.entry(key).or_default().push(i);
    }

    // For each group, collect all versions and sort by release date + number
    let mut group_list: Vec<(Vec<String>, Vec<VersionRef>, Vec<usize>)> = groups
        .into_iter()
        .map(|(key, indices)| {
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
            (key, refs, indices)
        })
        .collect();

    // Sort all groups by their canonical (first) version's (release_date, number).
    // Promo sets have no release date and so all tie on the fallback; the group key
    // breaks the remaining ties to make the order total and reproducible.
    group_list.sort_by(|(a_key, a_vers, _), (b_key, b_vers, _)| {
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
        a_d.cmp(b_d)
            .then(a_n.cmp(&b_n))
            .then_with(|| a_s.cmp(b_s))
            .then_with(|| a_key.cmp(b_key))
    });

    // Reuse the ID each group already owns; mint only for genuinely new cards.
    // card_versions embed these IDs, so a group that keeps its versions must keep
    // its ID or every version pointing at it is silently repointed.
    let mut assigned: Vec<Option<u32>> = Vec::with_capacity(group_list.len());
    let mut claimed: HashSet<u32> = HashSet::new();
    for (_, versions, _) in &group_list {
        let mut candidates: Vec<u32> = versions
            .iter()
            .filter_map(|v| id_index.by_version.get(&(v.set.clone(), v.number)).copied())
            .collect();
        candidates.sort_unstable();
        candidates.dedup();

        if candidates.len() > 1 {
            warn!(
                ids = ?candidates,
                versions = ?versions
                    .iter()
                    .map(|v| format!("{}/{:03}", v.set, v.number))
                    .collect::<Vec<_>>(),
                "card merges versions that had separate IDs — keeping the lowest"
            );
        }

        // A group whose ID was already taken by an earlier group (a split upstream)
        // falls through to a fresh ID rather than colliding.
        let id = candidates.into_iter().find(|id| !claimed.contains(id));
        if let Some(id) = id {
            claimed.insert(id);
        }
        assigned.push(id);
    }

    let mut next_id = id_index.max_id;
    let mut minted = 0usize;
    let ids: Vec<u32> = assigned
        .into_iter()
        .map(|id| {
            id.unwrap_or_else(|| {
                next_id += 1;
                while claimed.contains(&next_id) {
                    next_id += 1;
                }
                claimed.insert(next_id);
                minted += 1;
                next_id
            })
        })
        .collect();

    if minted > 0 {
        info!(minted, "new abstract card IDs assigned");
    }

    group_list
        .into_iter()
        .zip(ids)
        .map(|((_, versions, entry_indices), abstract_id)| {
            let canonical = (versions[0].set.clone(), versions[0].number);
            AbstractGroup {
                abstract_id,
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
    pack_expansion: &HashMap<String, String>,
    illustrator: Option<String>,
    limitless_card_source: Option<&str>,
    is_promo: bool,
    is_foil: bool,
    is_tradable: bool,
) -> Option<CardVersion> {
    // Map source.pack IDs to subtitles, keeping only packs that belong to this card's set.
    // A single CardEntry can span multiple sets (reprints); without this filter every version
    // would inherit packs from all sets, not just its own.
    let packs: Vec<String> = entry
        .source_packs
        .iter()
        .filter(|id| pack_expansion.get(*id).is_some_and(|s| s == set))
        .filter_map(|id| pack_subtitle_map.get(id).cloned())
        .collect();

    // For promo cards: Limitless label is authoritative when present; fall back to RaenonX.
    // For non-promo cards: use RaenonX-derived source (e.g. "Pack" or "Mission").
    let card_source = if is_promo {
        limitless_card_source
            .map(|s| s.to_string())
            .or_else(|| entry.card_source.clone())
    } else {
        entry.card_source.clone()
    }?;

    let rarity = entry.rarity.clone();

    Some(CardVersion {
        set: set.to_string(),
        number,
        card_id: abstract_id,
        rarity,
        illustrator,
        is_promo,
        is_foil,
        is_reprint: false,
        is_tradable,
        packs,
        source: card_source,
        duplicates: Vec::new(),
    })
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
    for (pack_id, desc_id) in promo_subtitles {
        map.insert(pack_id.clone(), promo_vol_label(desc_id));
    }
    map
}

/// Converts a PROMO_* pack ID (e.g. "PROMO_A_3") to its in-game display name ("Vol 3").
fn promo_vol_label(pack_id: &str) -> String {
    pack_id
        .rsplit('_')
        .next()
        .and_then(|n| n.parse::<u32>().ok())
        .map(|n| format!("Vol {n}"))
        .unwrap_or_else(|| pack_id.to_string())
}

// ── Helper: remap pull rate card IDs to SET-NUM format ────────────────────────

fn remap_pull_rate_card_ids(
    rates: &mut models::PackPullRates,
    card_id_to_versions: &HashMap<String, Vec<(String, u32)>>,
) {
    let set_code = rates.set.clone();

    for variant in rates.variants.values_mut() {
        let remapped: models::CardRates = variant
            .card_rates
            .iter()
            .filter_map(|(raenonx_id, slots)| {
                let versions = card_id_to_versions.get(raenonx_id)?;
                // The key is a card number within this pack's set, so only a
                // version from that set can supply it. Borrowing another set's
                // number would point the rate at an unrelated card.
                let (_, ver_num) = versions.iter().find(|(s, _)| *s == set_code).or_else(|| {
                    warn!(
                        set = set_code,
                        card = raenonx_id,
                        "pull rate card has no version in this set — dropped"
                    );
                    None
                })?;
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

// ── Validation helpers ────────────────────────────────────────────────────────

fn load_known_set<T, F>(path: &std::path::Path, key: F) -> HashSet<String>
where
    T: for<'de> serde::Deserialize<'de>,
    F: Fn(T) -> String,
{
    (|| -> Option<HashSet<String>> {
        let text = std::fs::read_to_string(path).ok()?;
        let items: Vec<T> = serde_json::from_str(&text).ok()?;
        Some(items.into_iter().map(key).collect())
    })()
    .unwrap_or_default()
}

/// How far a sum of rates may drift from 1 before it counts as an error.
///
/// Rates are reconstructed from RaenonX's rounded percentages, so a slot can miss
/// exactly 1 by a hair (B3b "Everyday Wonders" slot 4 is off by ~1.3e-9). Anything
/// larger than this is a real discrepancy, not an artifact of the source rounding.
const RATE_SUM_TOLERANCE: Decimal = Decimal::from_parts(1, 0, 0, false, 6);

fn rate_value(r: &models::Rate) -> Option<Decimal> {
    r.numerator.checked_div(r.denominator)
}

/// Exact fraction equality by cross-multiplication, so two spellings of the same
/// value (1/2 and 500/1000) compare equal without a division rounding step.
fn rates_equal(a: &models::Rate, b: &models::Rate) -> bool {
    match (
        a.numerator.checked_mul(b.denominator),
        b.numerator.checked_mul(a.denominator),
    ) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// Sum a set of rates, reporting a violation if any one of them is unrepresentable.
fn sum_rates<'a>(rates: impl IntoIterator<Item = &'a models::Rate>) -> Option<Decimal> {
    let mut total = Decimal::ZERO;
    for r in rates {
        total = total.checked_add(rate_value(r)?)?;
    }
    Some(total)
}

fn check_sums_to_one(v: &mut Violations, total: Option<Decimal>, what: &str) {
    match total {
        Some(t) if (t - Decimal::ONE).abs() <= RATE_SUM_TOLERANCE => {}
        Some(t) => v.add(format!("{what}: rates sum to {t}, expected 1")),
        None => v.add(format!("{what}: rate could not be evaluated")),
    }
}

/// Offline consistency check over everything in data/.
///
/// This is the cross-file half of validation: the per-card rules run during
/// `cards`, but they cannot see whether a card version points at a card that
/// exists, or whether a pull rate names a card that was ever written. Those
/// links only break when two commands disagree, so they are checked here over
/// the finished data rather than mid-scrape.
fn cmd_validate(lenient: bool) -> Result<()> {
    let mut v = Violations::new(lenient);
    let data = output::data_dir();
    if !data.exists() {
        anyhow::bail!("{} not found — nothing to validate", data.display());
    }

    let known_elements = load_known_set::<ElementInfo, _>(&data.join("elements.json"), |e| e.name);
    let known_rarities = load_known_set::<RarityInfo, _>(&data.join("rarities.json"), |r| r.code);
    let known_sources =
        load_known_set::<CardSource, _>(&data.join("card_sources.json"), |s| s.code);
    let known_variant_codes =
        load_known_set::<PackVariantName, _>(&data.join("pack_variant_names.json"), |p| p.code);
    for (file, set) in [
        ("elements.json", &known_elements),
        ("rarities.json", &known_rarities),
        ("card_sources.json", &known_sources),
        ("pack_variant_names.json", &known_variant_codes),
    ] {
        if set.is_empty() {
            v.add(format!(
                "{file} missing or empty — run `global-master` first"
            ));
        }
    }

    let sets: Vec<SetSummary> = match std::fs::read_to_string(data.join("sets.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
    {
        Some(s) => s,
        None => {
            v.add("sets.json missing or unparseable — run `sets` first");
            Vec::new()
        }
    };
    let set_is_promo: HashMap<String, bool> =
        sets.iter().map(|s| (s.code.clone(), s.is_promo)).collect();
    let set_packs: HashMap<String, HashSet<String>> = sets
        .iter()
        .map(|s| (s.code.clone(), s.packs.iter().cloned().collect()))
        .collect();
    let all_known_packs: HashSet<String> = set_packs.values().flatten().cloned().collect();
    let release_dates: HashMap<String, String> = sets
        .iter()
        .filter_map(|s| {
            s.availability
                .as_ref()
                .map(|a| (s.code.clone(), a.start.clone()))
        })
        .collect();
    let known_natdex: HashSet<u32> = output::load_base_pokemon()
        .unwrap_or_default()
        .into_iter()
        .map(|bp| bp.natdex_number)
        .collect();

    // ── Abstract cards ────────────────────────────────────────────────────────
    let mut cards: Vec<AbstractCard> = Vec::new();
    for (path, parsed) in output::load_all_abstract_cards()? {
        match parsed {
            Ok(card) => cards.push(card),
            Err(e) => v.add(format!("{}: parse error: {e}", path.display())),
        }
    }
    if cards.is_empty() {
        v.add("data/cards/ is empty — run `cards` first");
    }

    let known_card_names: HashSet<String> = cards
        .iter()
        .map(|c| canonicalize_apostrophes(&c.name))
        .collect();
    let base_pokemon_loaded = !known_natdex.is_empty();

    let mut seen_ids: HashMap<u32, String> = HashMap::new();
    for card in &cards {
        validate_abstract_card(
            &mut v,
            card,
            &known_elements,
            &known_card_names,
            base_pokemon_loaded,
        );
        if let Some(prev) = seen_ids.insert(card.id, card.name.clone()) {
            v.add(format!(
                "card {:05}: duplicate ID, also used by '{prev}'",
                card.id
            ));
        }
        if let Some(n) = card.natdex_number {
            if base_pokemon_loaded && !known_natdex.contains(&n) {
                v.add(format!(
                    "card {:05} ({}): natdex {n} not in base_pokemon.json",
                    card.id, card.name
                ));
            }
        }
        let mut seen_refs: HashSet<(&str, u32)> = HashSet::new();
        for ver in &card.versions {
            if !output::card_version_file_exists(&ver.set, ver.number) {
                v.add(format!(
                    "card {:05} ({}): version {}/{:03} has no card_version file",
                    card.id, card.name, ver.set, ver.number
                ));
            }
            if !seen_refs.insert((ver.set.as_str(), ver.number)) {
                v.add(format!(
                    "card {:05} ({}): version {}/{:03} listed more than once",
                    card.id, card.name, ver.set, ver.number
                ));
            }
        }
        // Versions run oldest first, so the canonical printing is versions[0].
        let ordering: Vec<(&str, u32)> = card
            .versions
            .iter()
            .map(|r| {
                (
                    release_dates
                        .get(&r.set)
                        .map(String::as_str)
                        .unwrap_or("9999-99-99"),
                    r.number,
                )
            })
            .collect();
        if ordering.windows(2).any(|w| w[0] > w[1]) {
            v.add(format!(
                "card {:05} ({}): versions are not ordered by release date then number",
                card.id, card.name
            ));
        }
    }

    // ── Card versions ─────────────────────────────────────────────────────────
    let mut all_versions: HashMap<(String, u32), CardVersion> = HashMap::new();
    for set in &sets {
        match output::load_card_versions(&set.code) {
            Ok(versions) => {
                for version in versions {
                    all_versions.insert((version.set.clone(), version.number), version);
                }
            }
            Err(e) => v.add(format!("{}: failed to load card versions: {e}", set.code)),
        }
    }

    // Which versions each card claims, for the reverse direction of the link check.
    let card_version_refs: HashMap<u32, HashSet<(String, u32)>> = cards
        .iter()
        .map(|c| {
            (
                c.id,
                c.versions
                    .iter()
                    .map(|r| (r.set.clone(), r.number))
                    .collect(),
            )
        })
        .collect();

    let mut versions_by_set: HashMap<String, HashSet<u32>> = HashMap::new();
    for (key, version) in &all_versions {
        validate_card_version(
            &mut v,
            version,
            &known_rarities,
            &set_is_promo,
            &set_packs,
            &all_known_packs,
        );
        let loc = format!("{}/{:03}", version.set, version.number);
        versions_by_set
            .entry(version.set.clone())
            .or_default()
            .insert(version.number);

        if !known_sources.is_empty() && !known_sources.contains(&version.source) {
            v.add(format!("{loc}: unknown card source '{}'", version.source));
        }

        // Version → card, and the card must name it back. A one-way link means the
        // two files disagree about which card this printing belongs to.
        match card_version_refs.get(&version.card_id) {
            None => v.add(format!(
                "{loc}: card_id {:05} does not exist in data/cards",
                version.card_id
            )),
            Some(refs) if !refs.contains(key) => v.add(format!(
                "{loc}: card {:05} does not list this version",
                version.card_id
            )),
            Some(_) => {}
        }

        if version.is_promo != set_is_promo.get(&version.set).copied().unwrap_or(false) {
            v.add(format!(
                "{loc}: is_promo={} but set '{}' is_promo={}",
                version.is_promo,
                version.set,
                set_is_promo.get(&version.set).copied().unwrap_or(false)
            ));
        }

        if version.is_reprint && version.duplicates.is_empty() {
            v.add(format!("{loc}: is_reprint but has no duplicates"));
        }

        // Verified against the full data set: IM and UR are the only rarities with
        // no tradable version, and every non-promo untradable version is one of them.
        const UNTRADABLE_RARITIES: &[&str] = &["IM", "UR"];
        let expect_tradable =
            !version.is_promo && !UNTRADABLE_RARITIES.contains(&version.rarity.as_str());
        if version.is_tradable != expect_tradable {
            v.add(format!(
                "{loc}: is_tradable={} for a {}{} card, expected {expect_tradable}",
                version.is_tradable,
                version.rarity,
                if version.is_promo { " promo" } else { "" }
            ));
        }

        // `packs` lists where a version drops, so it is populated exactly when the
        // version comes from a pack.
        match (version.source == "Pack", version.packs.is_empty()) {
            (true, true) => v.add(format!("{loc}: source is Pack but packs is empty")),
            (false, false) => v.add(format!(
                "{loc}: source is '{}' but packs is not empty",
                version.source
            )),
            _ => {}
        }

        for dup in &version.duplicates {
            let other_key = (dup.set.clone(), dup.number);
            let dloc = format!("{}/{:03}", dup.set, dup.number);
            if other_key == *key {
                v.add(format!("{loc}: lists itself as a duplicate"));
                continue;
            }
            let Some(other) = all_versions.get(&other_key) else {
                v.add(format!("{loc}: duplicate {dloc} does not exist"));
                continue;
            };
            if !other
                .duplicates
                .iter()
                .any(|d| (d.set.as_str(), d.number) == (key.0.as_str(), key.1))
            {
                v.add(format!("{loc}: duplicate {dloc} does not link back"));
            }
            if other.card_id != version.card_id {
                v.add(format!(
                    "{loc}: duplicate {dloc} belongs to card {:05}, not {:05}",
                    other.card_id, version.card_id
                ));
            }
            // Duplicates are the same physical card, so the identifying fields must agree.
            for (field, a, b) in [
                ("rarity", version.rarity.clone(), other.rarity.clone()),
                (
                    "illustrator",
                    version.illustrator.clone().unwrap_or_default(),
                    other.illustrator.clone().unwrap_or_default(),
                ),
                (
                    "is_promo",
                    version.is_promo.to_string(),
                    other.is_promo.to_string(),
                ),
                (
                    "is_foil",
                    version.is_foil.to_string(),
                    other.is_foil.to_string(),
                ),
            ] {
                if a != b {
                    v.add(format!(
                        "{loc}: duplicate {dloc} differs in {field} ({a} vs {b})"
                    ));
                }
            }
        }
    }

    // Exactly one member of each duplicate group is the original; the rest are reprints.
    let mut seen_group: HashSet<(String, u32)> = HashSet::new();
    for (key, version) in &all_versions {
        if version.duplicates.is_empty() || seen_group.contains(key) {
            continue;
        }
        let mut group: Vec<(String, u32)> = vec![key.clone()];
        group.extend(version.duplicates.iter().map(|d| (d.set.clone(), d.number)));
        seen_group.extend(group.iter().cloned());
        let originals = group
            .iter()
            .filter(|k| all_versions.get(*k).is_some_and(|x| !x.is_reprint))
            .count();
        if originals != 1 {
            let mut names: Vec<String> = group.iter().map(|(s, n)| format!("{s}/{n:03}")).collect();
            names.sort();
            v.add(format!(
                "duplicate group [{}]: {originals} versions are not reprints, expected exactly 1",
                names.join(", ")
            ));
            continue;
        }

        // The original is the earliest printing. Promo sets carry no release date,
        // so a group involving one cannot be ordered — skip rather than guess.
        // (No duplicate group involves a promo set today.)
        let dated: Vec<(&str, &(String, u32))> = group
            .iter()
            .filter_map(|k| Some((release_dates.get(&k.0)?.as_str(), k)))
            .collect();
        if dated.len() != group.len() || dated.len() < 2 {
            continue;
        }
        let original = group
            .iter()
            .find(|k| all_versions.get(*k).is_some_and(|x| !x.is_reprint));
        let earliest = dated.iter().min_by_key(|(d, k)| (*d, k.1));
        if let (Some(orig), Some((_, first))) = (original, earliest) {
            if orig != *first {
                v.add(format!(
                    "{}/{:03} is the original, but {}/{:03} was released earlier",
                    orig.0, orig.1, first.0, first.1
                ));
            }
        }
    }

    // ── Packs declared by sets vs pull rate files ─────────────────────────────
    let rate_packs: HashSet<(String, String)> = output::load_all_pull_rates()?
        .into_iter()
        .filter_map(|(_, _, r)| r.ok().map(|r| (r.set, r.subtitle)))
        .collect();
    for (code, packs) in &set_packs {
        for pack in packs {
            if !rate_packs.contains(&(code.clone(), pack.clone())) {
                v.add(format!(
                    "{code}: pack '{pack}' is declared in sets.json but has no pull rate file"
                ));
            }
        }
    }
    for (set, subtitle) in &rate_packs {
        if !set_packs
            .get(set)
            .is_some_and(|packs| packs.contains(subtitle))
        {
            v.add(format!(
                "pull_rates/{set}: pack '{subtitle}' is not declared in sets.json"
            ));
        }
    }

    // ── Declared card counts vs versions on disk ──────────────────────────────
    for set in &sets {
        if let Some(expected) = set.card_count {
            let actual = versions_by_set.get(&set.code).map_or(0, HashSet::len) as u32;
            if actual != expected {
                v.add(format!(
                    "{}: sets.json says {expected} cards but {actual} card_version files exist",
                    set.code
                ));
            }
        }
    }

    // ── Pull rates ────────────────────────────────────────────────────────────
    for (dir_set, path, parsed) in output::load_all_pull_rates()? {
        let rates = match parsed {
            Ok(r) => r,
            Err(e) => {
                v.add(format!("{}: parse error: {e}", path.display()));
                continue;
            }
        };
        let loc = format!("pull_rates/{dir_set}/{}", rates.subtitle);
        if rates.set != dir_set {
            v.add(format!(
                "{loc}: set field '{}' does not match its directory",
                rates.set
            ));
        }
        if !set_is_promo.contains_key(&dir_set) {
            v.add(format!("{loc}: set '{dir_set}' is not in sets.json"));
        }
        // One pack is opened per attempt, so the variants partition that certainty.
        check_sums_to_one(
            &mut v,
            sum_rates(rates.variants.values().map(|x| &x.rate)),
            &format!("{loc}: pack variants"),
        );

        for (code, variant) in &rates.variants {
            if !known_variant_codes.is_empty() && !known_variant_codes.contains(code) {
                v.add(format!("{loc}: unknown pack variant code '{code}'"));
            }
            if variant.rarity_rates_by_slot.len() != variant.slot_count as usize {
                v.add(format!(
                    "{loc}: {code} has {} rarity slots but slot_count is {}",
                    variant.rarity_rates_by_slot.len(),
                    variant.slot_count
                ));
            }

            for (slot_idx, slot) in variant.rarity_rates_by_slot.iter().enumerate() {
                for rarity in slot.keys() {
                    if !known_rarities.is_empty() && !known_rarities.contains(rarity) {
                        v.add(format!("{loc}: unknown rarity '{rarity}' in {code} rates"));
                    }
                }
                // Every slot yields exactly one card, so its rarities partition it.
                check_sums_to_one(
                    &mut v,
                    sum_rates(
                        slot.values()
                            .flat_map(|r| [r.normal.as_ref(), r.foil.as_ref()])
                            .flatten(),
                    ),
                    &format!("{loc}: {code} slot {slot_idx} rarities"),
                );
            }

            for (card_key, slots) in &variant.card_rates {
                if slots.len() != variant.slot_count as usize {
                    v.add(format!(
                        "{loc}: {code} card {card_key} has {} slot entries but slot_count is {}",
                        slots.len(),
                        variant.slot_count
                    ));
                }
                let Ok(number) = card_key.parse::<u32>() else {
                    v.add(format!(
                        "{loc}: invalid card key '{card_key}' in {code} rates"
                    ));
                    continue;
                };
                if !versions_by_set
                    .get(&dir_set)
                    .is_some_and(|nums| nums.contains(&number))
                {
                    v.add(format!(
                        "{loc}: {code} rates name card {number:03}, which has no card_version in {dir_set}"
                    ));
                }
            }

            for slot_idx in 0..variant.slot_count as usize {
                let in_slot: Vec<(&String, &models::Rate)> = variant
                    .card_rates
                    .iter()
                    .filter_map(|(k, slots)| Some((k, slots.get(slot_idx)?.as_ref()?)))
                    .collect();
                if in_slot.is_empty() {
                    continue;
                }
                // The cards in a slot partition it too, so they sum to 1 as well.
                check_sums_to_one(
                    &mut v,
                    sum_rates(in_slot.iter().map(|(_, r)| *r)),
                    &format!("{loc}: {code} slot {slot_idx} cards"),
                );

                // Within a slot, every card of the same rarity and finish is equally
                // likely — the rarity's rate is split evenly across its members.
                let mut by_rarity: BTreeMap<(String, bool), Vec<(&String, &models::Rate)>> =
                    BTreeMap::new();
                for (key, rate) in &in_slot {
                    let Ok(number) = key.parse::<u32>() else {
                        continue;
                    };
                    let Some(ver) = all_versions.get(&(dir_set.clone(), number)) else {
                        continue;
                    };
                    by_rarity
                        .entry((ver.rarity.clone(), ver.is_foil))
                        .or_default()
                        .push((key, rate));
                }
                for ((rarity, is_foil), members) in &by_rarity {
                    let finish = if *is_foil { "foil" } else { "normal" };
                    if let Some(((first_key, first), (odd_key, odd))) =
                        members.split_first().and_then(|(f, rest)| {
                            rest.iter()
                                .find(|(_, r)| !rates_equal(r, f.1))
                                .map(|o| (*f, *o))
                        })
                    {
                        v.add(format!(
                            "{loc}: {code} slot {slot_idx} {rarity} ({finish}) cards differ — \
                             {first_key} is {}/{} but {odd_key} is {}/{}",
                            first.numerator, first.denominator, odd.numerator, odd.denominator
                        ));
                    }
                    // Those members together account for that rarity's share of the slot.
                    if let Some(sub) = variant
                        .rarity_rates_by_slot
                        .get(slot_idx)
                        .and_then(|s| s.get(rarity))
                        .and_then(|r| {
                            if *is_foil {
                                r.foil.as_ref()
                            } else {
                                r.normal.as_ref()
                            }
                        })
                    {
                        match (sum_rates(members.iter().map(|(_, r)| *r)), rate_value(sub)) {
                            (Some(t), Some(e)) if (t - e).abs() > RATE_SUM_TOLERANCE => {
                                v.add(format!(
                                    "{loc}: {code} slot {slot_idx} {rarity} ({finish}) cards sum to {t}, \
                                     but the rarity rate is {e}"
                                ));
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    info!(
        cards = cards.len(),
        versions = versions_by_set.values().map(HashSet::len).sum::<usize>(),
        "validation complete"
    );
    v.finish()
}

fn collect_card_names() -> Result<HashSet<String>> {
    let dir = output::data_dir().join("cards");
    if !dir.exists() {
        return Ok(HashSet::new());
    }
    let mut names = HashSet::new();
    for entry in std::fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "json") {
            if let Ok(text) = std::fs::read_to_string(&path) {
                if let Ok(card) = serde_json::from_str::<AbstractCard>(&text) {
                    names.insert(canonicalize_apostrophes(&card.name));
                }
            }
        }
    }
    Ok(names)
}

fn validate_abstract_card(
    v: &mut Violations,
    card: &AbstractCard,
    known_elements: &HashSet<String>,
    known_card_names: &HashSet<String>,
    base_pokemon_loaded: bool,
) {
    if card.name.is_empty() {
        v.add(format!("card {:05}: empty name", card.id));
    }
    if card.versions.is_empty() {
        v.add(format!("card {:05} ({}): no versions", card.id, card.name));
    }
    match card.card_type.as_str() {
        "pokemon" => validate_pokemon_card(
            v,
            card,
            known_elements,
            known_card_names,
            base_pokemon_loaded,
        ),
        "trainer" => validate_trainer_card(v, card),
        other => v.add(format!(
            "card {:05} ({}): unknown card_type '{other}'",
            card.id, card.name
        )),
    }
}

fn validate_pokemon_card(
    v: &mut Violations,
    card: &AbstractCard,
    known_elements: &HashSet<String>,
    known_card_names: &HashSet<String>,
    base_pokemon_loaded: bool,
) {
    let name = &card.name;
    if card.element.is_none() {
        v.add(format!("pokemon {name}: missing element"));
    } else if let Some(el) = &card.element {
        if !known_elements.is_empty() && !known_elements.contains(el) {
            v.add(format!("pokemon {name}: unknown element '{el}'"));
        }
    }
    if card.stage.is_none() {
        v.add(format!("pokemon {name}: missing stage"));
    }
    match card.hp {
        None => v.add(format!("pokemon {name}: missing hp")),
        Some(0) => v.add(format!("pokemon {name}: hp is 0")),
        _ => {}
    }
    if card.retreat_cost.is_none() {
        v.add(format!("pokemon {name}: missing retreat_cost"));
    }
    if card.is_ex.is_none() {
        v.add(format!("pokemon {name}: missing is_ex"));
    }
    if card.is_mega.is_none() {
        v.add(format!("pokemon {name}: missing is_mega"));
    }
    if base_pokemon_loaded && card.natdex_number.is_none() {
        v.add(format!("pokemon {name}: missing natdex_number"));
    }
    if let Some(weakness) = &card.weakness {
        if !known_elements.is_empty() && !known_elements.contains(weakness) {
            v.add(format!(
                "pokemon {name}: unknown weakness element '{weakness}'"
            ));
        }
    }
    if let Some(from) = &card.evolves_from {
        if !known_card_names.is_empty()
            && !known_card_names.contains(&canonicalize_apostrophes(from))
        {
            v.add(format!(
                "pokemon {name}: evolves_from '{from}' is not a known card name"
            ));
        }
    }
    for attack in &card.attacks {
        if attack.name.is_empty() {
            v.add(format!("pokemon {name}: attack with empty name"));
        }
        for cost_el in &attack.cost {
            if !known_elements.is_empty() && !known_elements.contains(cost_el) {
                v.add(format!(
                    "pokemon {name} attack '{}': unknown cost element '{cost_el}'",
                    attack.name
                ));
            }
        }
    }
    if card.trainer_kind.is_some() {
        v.add(format!("pokemon {name}: unexpected trainer_kind"));
    }
    if card.trainer_effect.is_some() {
        v.add(format!("pokemon {name}: unexpected trainer_effect"));
    }
}

fn validate_trainer_card(v: &mut Violations, card: &AbstractCard) {
    let name = &card.name;
    if card.trainer_kind.is_none() {
        v.add(format!("trainer {name}: missing trainer_kind"));
    }
    if card.trainer_effect.is_none() {
        v.add(format!("trainer {name}: missing trainer_effect"));
    }
    if card.element.is_some() {
        v.add(format!("trainer {name}: unexpected element"));
    }
    if card.stage.is_some() {
        v.add(format!("trainer {name}: unexpected stage"));
    }
    if card.hp.is_some() {
        v.add(format!("trainer {name}: unexpected hp"));
    }
    if card.retreat_cost.is_some() {
        v.add(format!("trainer {name}: unexpected retreat_cost"));
    }
    if !card.attacks.is_empty() {
        v.add(format!("trainer {name}: unexpected attacks"));
    }
    if card.evolves_from.is_some() {
        v.add(format!("trainer {name}: unexpected evolves_from"));
    }
}

fn validate_card_version(
    v: &mut Violations,
    version: &CardVersion,
    known_rarities: &HashSet<String>,
    set_is_promo: &HashMap<String, bool>,
    set_packs: &HashMap<String, HashSet<String>>,
    all_known_packs: &HashSet<String>,
) {
    let loc = format!("{}/{:03}", version.set, version.number);
    if !known_rarities.is_empty() && !known_rarities.contains(&version.rarity) {
        v.add(format!("{loc}: unknown rarity '{}'", version.rarity));
    }
    let is_promo = set_is_promo
        .get(&version.set)
        .copied()
        .unwrap_or(version.is_promo);
    let is_pack_card = version.source == "Pack";
    if !is_promo && is_pack_card && version.packs.is_empty() {
        v.add(format!("{loc}: pack card has no packs"));
    }
    if let Some(known_packs) = set_packs.get(&version.set) {
        for pack in &version.packs {
            if !known_packs.contains(pack) && !all_known_packs.contains(pack) {
                v.add(format!("{loc}: pack '{pack}' not found in any set"));
            }
        }
    }
}

fn build_card_sources(codes: &[String]) -> Vec<CardSource> {
    let descriptions: &[(&str, &str)] = &[
        ("Pack", "Obtainable from packs"),
        ("Wonder Pick", "Obtainable via Wonder Pick"),
        ("Gold Shop", "Purchasable in the Gold Shop"),
        ("Shop", "Purchasable in the Shop"),
        ("Mission", "Obtainable via Missions"),
        ("Premium Mission", "Obtainable via Premium Pass missions"),
    ];
    let desc_lookup: HashMap<&str, &str> = descriptions.iter().copied().collect();

    codes
        .iter()
        .map(|code| CardSource {
            code: code.clone(),
            description: desc_lookup.get(code.as_str()).map(|d| d.to_string()),
        })
        .collect()
}
