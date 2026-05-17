use std::sync::Arc;

use anyhow::Result;
use futures::stream::{self, StreamExt};
use serde_json::Value;
use tracing::warn;

use crate::client::Client;
use crate::models::BasePokemon;

const SPECIES_LIST_BASE: &str = "https://pokeapi.co/api/v2/pokemon-species";

pub async fn fetch_all_species(client: &Arc<Client>) -> Result<Vec<BasePokemon>> {
    let client = Arc::clone(client);

    // Fetch with limit=1 to read the total count, then re-fetch everything.
    let probe = client
        .get_text(&format!("{SPECIES_LIST_BASE}?limit=1"))
        .await?;
    let probe_json: Value = serde_json::from_str(&probe)?;
    let total_count = probe_json["count"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("expected 'count' in pokemon-species response"))?;

    let url = format!("{SPECIES_LIST_BASE}?limit={total_count}");
    let text = client.get_text(&url).await?;
    let list: Value = serde_json::from_str(&text)?;

    let results = list["results"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("expected 'results' array in pokemon-species response"))?;

    let urls: Vec<(u32, String)> = results
        .iter()
        .filter_map(|r| {
            let url = r["url"].as_str()?;
            let id: u32 = url.trim_end_matches('/').rsplit('/').next()?.parse().ok()?;
            Some((id, url.to_string()))
        })
        .collect();

    let total = urls.len();
    tracing::info!(total, "fetching species from PokeAPI");

    let mut all: Vec<BasePokemon> = stream::iter(urls)
        .map(|(id, url)| {
            let c = Arc::clone(&client);
            async move {
                match fetch_species_en_name(&c, &url).await {
                    Ok(name) => Some(BasePokemon {
                        name,
                        natdex_number: id,
                    }),
                    Err(e) => {
                        warn!(id, "failed to fetch species: {e}");
                        None
                    }
                }
            }
        })
        .buffer_unordered(5)
        .filter_map(|x| async { x })
        .collect()
        .await;

    all.sort_by_key(|bp| bp.natdex_number);
    tracing::info!(fetched = all.len(), total, "PokeAPI species fetch complete");
    Ok(all)
}

async fn fetch_species_en_name(client: &Client, url: &str) -> Result<String> {
    let text = client.get_text(url).await?;
    let species: Value = serde_json::from_str(&text)?;

    let name = species["names"]
        .as_array()
        .and_then(|names| {
            names
                .iter()
                .find(|n| n["language"]["name"].as_str() == Some("en"))
        })
        .and_then(|n| n["name"].as_str())
        .ok_or_else(|| anyhow::anyhow!("no English name in species response for {url}"))?;

    Ok(name.to_string())
}
