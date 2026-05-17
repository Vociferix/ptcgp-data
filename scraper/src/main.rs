mod client;
mod limitless;
mod models;
mod output;
mod raenonx;

use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};
use futures::stream::{self, StreamExt};
use models::{PackInfo, RarityInfo, SetDetail};
use tracing::{error, info, warn};

// ── CLI definition ────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "scraper", about = "PTCGP data scraper")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Fetch and save the RaenonX global-master JSON (pack IDs, craft costs, etc.)
    GlobalMaster,

    /// Scrape all set metadata from Limitless TCG and write data/sets.json
    Sets,

    /// Scrape card detail pages from Limitless TCG
    Cards {
        /// Limit to a single set code, e.g. A1 (default: all sets)
        #[arg(long)]
        set: Option<String>,

        /// Re-fetch cards even if output file already exists
        #[arg(long)]
        force: bool,
    },

    /// Fetch pull rate data from RaenonX for each regular pack
    PullRates {
        /// Limit to a single pack ID (default: all regular packs)
        #[arg(long)]
        pack: Option<String>,

        /// Re-fetch even if output file already exists
        #[arg(long)]
        force: bool,
    },

    /// Run the complete pipeline: global-master → sets → cards → pull-rates
    All {
        /// Re-fetch everything even if output files already exist
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
        Command::Cards { set, force } => cmd_cards(&client, set.as_deref(), force).await?,
        Command::PullRates { pack, force } => {
            cmd_pull_rates(&client, pack.as_deref(), force).await?
        }
        Command::All { force } => {
            cmd_global_master(&client).await?;
            cmd_sets(&client).await?;
            cmd_cards(&client, None, force).await?;
            cmd_pull_rates(&client, None, force).await?;
        }
    }

    Ok(())
}

// ── Command implementations ───────────────────────────────────────────────────

async fn cmd_global_master(client: &client::Client) -> Result<()> {
    if output::global_master_exists() {
        info!("global-master already cached; loading from disk");
    } else {
        info!("fetching global-master from RaenonX");
    }

    let (raw, summary) = raenonx::fetch_global_master(client).await?;
    output::write_global_master_raw(&raw)?;
    output::write_global_master_summary(&summary)?;

    info!(
        packs = summary.regular_pack_ids.len(),
        rarities = summary.craft_costs.len(),
        "global-master saved"
    );

    // Also emit the known rarities (with craft costs from global-master)
    let rarities = build_rarity_list(&summary);
    output::write_rarities(&rarities)?;
    info!(count = rarities.len(), "rarities.json written");

    Ok(())
}

async fn cmd_sets(client: &client::Client) -> Result<()> {
    info!("scraping set index from Limitless TCG");
    let sets = limitless::scrape_sets(client).await?;
    output::write_sets(&sets)?;
    info!(count = sets.len(), "sets.json written");

    // Write per-set detail files with pack information
    for set in &sets {
        let pack_names = match limitless::scrape_set_packs(client, &set.code).await {
            Ok(names) => names,
            Err(e) => {
                warn!(set = set.code, "failed to scrape packs: {e}");
                vec![]
            }
        };

        let packs: Vec<PackInfo> = pack_names
            .into_iter()
            .enumerate()
            .map(|(_i, display_name)| {
                // subtitle is the display name without the trailing " pack"
                let subtitle = display_name
                    .strip_suffix(" pack")
                    .map(str::to_string)
                    .or_else(|| {
                        display_name
                            .strip_suffix(" Pack")
                            .map(str::to_string)
                    });
                PackInfo {
                    raenonx_id: None, // filled in by merge step
                    subtitle,
                    display_name,
                }
            })
            .collect();

        let detail = SetDetail {
            code: set.code.clone(),
            name: set.name.clone(),
            series: set.series.clone(),
            release_date: set.release_date.clone(),
            is_promo: set.is_promo,
            card_count: set.card_count,
            icon_url: set.icon_url.clone(),
            packs,
        };

        output::write_set_detail(&detail)?;
        info!(set = set.code, "set detail written");
    }

    Ok(())
}

async fn cmd_cards(
    client: &Arc<client::Client>,
    filter_set: Option<&str>,
    force: bool,
) -> Result<()> {
    // Determine which sets to scrape
    let sets_path = output::data_dir().join("sets.json");
    if !sets_path.exists() {
        anyhow::bail!("data/sets.json not found — run `sets` command first");
    }
    let sets_json = std::fs::read_to_string(&sets_path)?;
    let sets: Vec<models::SetSummary> = serde_json::from_str(&sets_json)?;

    let sets: Vec<_> = sets
        .into_iter()
        .filter(|s| filter_set.map_or(true, |f| s.code == f))
        .collect();

    if sets.is_empty() {
        anyhow::bail!("no matching sets found");
    }

    for set in &sets {
        info!(set = set.code, "scraping card list");

        let numbers = match limitless::scrape_card_numbers(client, &set.code).await {
            Ok(n) => n,
            Err(e) => {
                error!(set = set.code, "failed to get card numbers: {e}");
                continue;
            }
        };

        info!(set = set.code, count = numbers.len(), "fetching cards");

        // Filter to cards not yet fetched (unless --force)
        let to_fetch: Vec<u32> = numbers
            .into_iter()
            .filter(|&n| force || !output::card_file_exists(&set.code, n))
            .collect();

        if to_fetch.is_empty() {
            info!(set = set.code, "all cards already fetched");
            continue;
        }

        let set_code = set.code.clone();
        let client = Arc::clone(client);

        // Fetch cards concurrently (bounded by the client's semaphore)
        let mut summaries: Vec<models::CardSummary> = stream::iter(to_fetch)
            .map(|number| {
                let client = Arc::clone(&client);
                let set_code = set_code.clone();
                async move {
                    match limitless::scrape_card(&client, &set_code, number).await {
                        Ok((card, summary)) => {
                            if let Err(e) = output::write_card(&card) {
                                error!(set = set_code, number, "write failed: {e}");
                            }
                            Some(summary)
                        }
                        Err(e) => {
                            error!(set = set_code, number, "fetch failed: {e}");
                            None
                        }
                    }
                }
            })
            .buffer_unordered(5)
            .filter_map(|x| async move { x })
            .collect()
            .await;

        summaries.sort_by_key(|s| s.number);
        output::write_card_index(&set_code, &summaries)?;
        info!(set = set_code, written = summaries.len(), "cards done");
    }

    Ok(())
}

async fn cmd_pull_rates(
    client: &Arc<client::Client>,
    filter_pack: Option<&str>,
    force: bool,
) -> Result<()> {
    // Load global-master summary for pack list
    let summary_path = output::data_dir().join("raenonx/global_master_summary.json");
    if !summary_path.exists() {
        anyhow::bail!("raenonx summary not found — run `global-master` command first");
    }
    let summary_json = std::fs::read_to_string(&summary_path)?;
    let summary: models::GlobalMasterSummary = serde_json::from_str(&summary_json)?;

    let packs: Vec<_> = summary
        .regular_pack_ids
        .iter()
        .filter(|id| filter_pack.map_or(true, |f| id.as_str() == f))
        .collect();

    if packs.is_empty() {
        anyhow::bail!("no matching packs found");
    }

    info!(count = packs.len(), "fetching pull rates");

    for pack_id in packs {
        if !force && output::pull_rates_file_exists(pack_id) {
            info!(pack = pack_id, "already fetched, skipping");
            continue;
        }

        let set = summary
            .pack_expansion
            .get(pack_id)
            .map(String::as_str)
            .unwrap_or("unknown");

        match raenonx::fetch_pack_pull_rates(client, pack_id, set, None).await {
            Ok(rates) => {
                output::write_pull_rates(&rates)?;
                let card_count = rates
                    .variants
                    .normal
                    .as_ref()
                    .map(|v| v.card_rates.len())
                    .unwrap_or(0);
                info!(pack = pack_id, cards = card_count, "pull rates written");
            }
            Err(e) => {
                error!(pack = pack_id, "pull rate fetch failed: {e}");
            }
        }
    }

    Ok(())
}

// ── Reference data ────────────────────────────────────────────────────────────

/// Build the canonical rarity list, populating craft costs from the
/// global-master summary when available.
fn build_rarity_list(summary: &models::GlobalMasterSummary) -> Vec<RarityInfo> {
    // Static rarity definitions (group, symbol count, common name)
    let definitions: &[(&str, &str, u8, &str)] = &[
        ("C",   "Diamond", 1, "Common"),
        ("U",   "Diamond", 2, "Uncommon"),
        ("R",   "Diamond", 3, "Rare"),
        ("RR",  "Diamond", 4, "Double Rare"),
        ("AR",  "Star",    1, "Art Rare"),
        ("SR",  "Star",    2, "Super Rare"),
        ("SAR", "Star",    2, "Special Art Rare"),
        ("IM",  "Star",    3, "Immersive"),
        ("S",   "Shiny",   1, "Shiny Rare"),
        ("SSR", "Shiny",   2, "Shiny Super Rare"),
        ("UR",  "Crown",   1, "Crown Rare"),
    ];

    definitions
        .iter()
        .map(|(code, group, count, name)| RarityInfo {
            code: code.to_string(),
            name: name.to_string(),
            group: group.to_string(),
            group_symbol_count: *count,
            craft_cost: summary.craft_costs.get(*code).copied(),
            dupe_dust: summary.dupe_dust.get(*code).copied(),
        })
        .collect()
}
