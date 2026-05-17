use std::collections::HashMap;

use anyhow::{anyhow, bail, Result};
use regex::Regex;
use serde::Deserialize;
use serde_json::Value;

use crate::client::Client;
use crate::models::{
    GlobalMasterSummary, PackPullRates, PackVariantRates, PackVariants, Rate,
};

const GLOBAL_MASTER_URL: &str = "https://ptcgp.raenonx.cc/api/data/global-master";
const PACK_PAGE_BASE: &str = "https://ptcgp.raenonx.cc/en/pack";

// ── Public entry points ──────────────────────────────────────────────────────

/// Fetch the global-master endpoint and return both the raw JSON (for
/// archival) and a parsed summary of the fields we care about.
pub async fn fetch_global_master(client: &Client) -> Result<(Value, GlobalMasterSummary)> {
    let raw = client.get_json(GLOBAL_MASTER_URL).await?;
    let summary = parse_global_master(&raw)?;
    Ok((raw, summary))
}

/// Fetch the RaenonX pack page and extract pull rate data via RSC parsing.
pub async fn fetch_pack_pull_rates(
    client: &Client,
    pack_id: &str,
    set: &str,
    subtitle: Option<&str>,
) -> Result<PackPullRates> {
    let url = format!("{PACK_PAGE_BASE}/{pack_id}");
    let html = client.get_text(&url).await?;
    parse_rsc_pull_rates(&html, pack_id, set, subtitle)
}

// ── Global master parser ─────────────────────────────────────────────────────

pub fn parse_global_master(raw: &Value) -> Result<GlobalMasterSummary> {
    // ── Pack map ────────────────────────────────────────────────────────────
    let pack_map = raw
        .get("cardPackMap")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("cardPackMap missing from global-master"))?;

    let expansion_map = raw
        .get("cardExpansionMap")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("cardExpansionMap missing from global-master"))?;

    // Build expansion internal-ID -> set code lookup
    // Each expansion entry should have an "id" field matching the external code
    let mut expansion_code: HashMap<String, String> = HashMap::new();
    for (key, val) in expansion_map {
        // The external set code is either the key itself or a nested "id" field.
        // Try the nested field first, fall back to the map key.
        let code = val
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or(key.as_str())
            .to_string();
        expansion_code.insert(key.clone(), code);
    }

    let mut regular_pack_ids: Vec<String> = Vec::new();
    let mut pack_expansion: HashMap<String, String> = HashMap::new();

    for (pack_id, pack_val) in pack_map {
        let is_regular = pack_val
            .get("isRegular")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !is_regular {
            continue;
        }

        let exp_id = pack_val
            .get("expansionId")
            .and_then(Value::as_str)
            .unwrap_or_default();

        let set_code = expansion_code
            .get(exp_id)
            .cloned()
            .unwrap_or_else(|| exp_id.to_string());

        regular_pack_ids.push(pack_id.clone());
        pack_expansion.insert(pack_id.clone(), set_code);
    }

    regular_pack_ids.sort();

    // ── Craft costs and dupe dust ────────────────────────────────────────────
    let craft_costs = parse_rarity_u32_map(raw, "cardPackPointMap");
    let dupe_dust = parse_rarity_u32_map(raw, "cardDupeShineDustMap");

    Ok(GlobalMasterSummary {
        regular_pack_ids,
        pack_expansion,
        craft_costs,
        dupe_dust,
    })
}

fn parse_rarity_u32_map(raw: &Value, key: &str) -> HashMap<String, u32> {
    raw.get(key)
        .and_then(Value::as_object)
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| {
                    let n = v.as_u64().or_else(|| v.as_f64().map(|f| f as u64))?;
                    Some((k.clone(), n as u32))
                })
                .collect()
        })
        .unwrap_or_default()
}

// ── RSC pull rate parser ─────────────────────────────────────────────────────

/// Parse pull rate data out of a Next.js RSC streamed page.
///
/// The page embeds data as `self.__next_f.push([1,"..."])` chunks where the
/// string content is a JSON-encoded string (one level of escaping). The chunk
/// containing `cardPullProbabilityMap` is the one we want.
pub fn parse_rsc_pull_rates(
    html: &str,
    pack_id: &str,
    set: &str,
    subtitle: Option<&str>,
) -> Result<PackPullRates> {
    let chunk = extract_rsc_chunk(html, "cardPullProbabilityMap")?;

    let card_map = extract_json_at_key(&chunk, "cardPullProbabilityMap")?;
    let pack_data = extract_json_at_key(&chunk, "packPullProbabilityData").ok();

    let variants = build_variants(pack_id, &card_map, pack_data.as_ref())?;

    Ok(PackPullRates {
        pack_id: pack_id.to_string(),
        set: set.to_string(),
        subtitle: subtitle.map(str::to_string),
        variants,
    })
}

/// Find the RSC push chunk that contains the given key, unescape it, and
/// return the inner string ready for JSON extraction.
fn extract_rsc_chunk(html: &str, key: &str) -> Result<String> {
    let re = Regex::new(r#"self\.__next_f\.push\(\[1,"((?:[^"\\]|\\.)*)"\]\)"#)?;

    for cap in re.captures_iter(html) {
        let escaped = &cap[1];
        if !escaped.contains(key) {
            continue;
        }
        // The content is a JSON-encoded string — parse it to unescape
        let json_str = format!("\"{}\"", escaped);
        let unescaped: String = serde_json::from_str(&json_str)
            .map_err(|e| anyhow!("RSC chunk unescape failed: {e}"))?;
        return Ok(unescaped);
    }

    bail!("no RSC chunk containing {key:?} found");
}

/// Locate `"key":` in the text and deserialize the JSON value that follows.
///
/// This is the Rust equivalent of Python's `json.JSONDecoder().raw_decode()`:
/// `serde_json::Deserializer` advances past exactly one value and stops,
/// ignoring any trailing content (which may be more RSC objects).
fn extract_json_at_key(text: &str, key: &str) -> Result<Value> {
    let search = format!("\"{}\":", key);
    let pos = text
        .find(&search)
        .ok_or_else(|| anyhow!("key {key:?} not found in RSC chunk"))?;
    let value_start = pos + search.len();

    let mut de = serde_json::Deserializer::from_str(&text[value_start..]);
    let value = Value::deserialize(&mut de)
        .map_err(|e| anyhow!("JSON parse at key {key:?}: {e}"))?;
    Ok(value)
}

// ── Pull rate data builder ───────────────────────────────────────────────────

fn build_variants(
    pack_id: &str,
    card_map: &Value,
    pack_data: Option<&Value>,
) -> Result<PackVariants> {
    // Pack type rates from packPullProbabilityData.byPackType
    let type_rates = pack_data
        .and_then(|d| d.get("byPackType"))
        .and_then(Value::as_object);

    let normal_rate = type_rates
        .and_then(|m| m.get("normal"))
        .and_then(parse_rate_obj)
        .unwrap_or(Rate { numerator: 1.0, denominator: 1.0 });

    let god_rate = type_rates
        .and_then(|m| m.get("rare"))
        .and_then(parse_rate_obj);

    let premium_rate = type_rates
        .and_then(|m| m.get("plus1"))
        .and_then(parse_rate_obj);

    // Rarity rates by slot from packPullProbabilityData.byRarity
    let by_rarity = pack_data.and_then(|d| d.get("byRarity"));

    let normal_rarity = slot_rarity_rates(by_rarity, "normal");
    let god_rarity = slot_rarity_rates(by_rarity, "rare");
    let premium_rarity = slot_rarity_rates(by_rarity, "plus1");

    // Per-card rates from cardPullProbabilityMap
    let card_obj = card_map
        .as_object()
        .ok_or_else(|| anyhow!("cardPullProbabilityMap is not an object"))?;

    let mut normal_cards: HashMap<String, Vec<Option<f64>>> = HashMap::new();
    let mut god_cards: HashMap<String, Vec<Option<f64>>> = HashMap::new();
    let mut premium_cards: HashMap<String, Vec<Option<f64>>> = HashMap::new();

    for (card_id, card_val) in card_obj {
        let by_pack = card_val
            .get("byPack")
            .and_then(Value::as_object);
        let Some(by_pack) = by_pack else { continue };
        let Some(pack_entry) = by_pack.get(pack_id) else { continue };
        let probs = pack_entry.get("cardProbability").and_then(Value::as_object);
        let Some(probs) = probs else { continue };

        if let Some(slots) = probs.get("normal") {
            normal_cards.insert(card_id.clone(), parse_slot_rates(slots));
        }
        if let Some(slots) = probs.get("rare") {
            god_cards.insert(card_id.clone(), parse_slot_rates(slots));
        }
        if let Some(slots) = probs.get("plus1") {
            premium_cards.insert(card_id.clone(), parse_slot_rates(slots));
        }
    }

    let normal_slot_count = normal_rarity.len().max(normal_cards.values().map(Vec::len).max().unwrap_or(5));
    let god_slot_count = god_rarity.len().max(god_cards.values().map(Vec::len).max().unwrap_or(5));
    let premium_slot_count = premium_rarity.len().max(premium_cards.values().map(Vec::len).max().unwrap_or(6));

    let normal = Some(PackVariantRates {
        rate: normal_rate.as_f64(),
        rate_numerator: normal_rate.numerator,
        rate_denominator: normal_rate.denominator,
        slot_count: normal_slot_count as u32,
        rarity_rates_by_slot: normal_rarity,
        card_rates: normal_cards,
    });

    let god = god_rate.map(|r| PackVariantRates {
        rate: r.as_f64(),
        rate_numerator: r.numerator,
        rate_denominator: r.denominator,
        slot_count: god_slot_count as u32,
        rarity_rates_by_slot: god_rarity,
        card_rates: god_cards,
    });

    let premium = premium_rate.map(|r| PackVariantRates {
        rate: r.as_f64(),
        rate_numerator: r.numerator,
        rate_denominator: r.denominator,
        slot_count: premium_slot_count as u32,
        rarity_rates_by_slot: premium_rarity,
        card_rates: premium_cards,
    });

    Ok(PackVariants { normal, god, premium })
}

/// Parse `[{"numerator":..,"denominator":..}, null, ...]` into a vec of
/// optional f64 values, one per slot.
fn parse_slot_rates(val: &Value) -> Vec<Option<f64>> {
    val.as_array()
        .map(|arr| {
            arr.iter()
                .map(|v| {
                    if v.is_null() {
                        None
                    } else {
                        parse_rate_obj(v).map(|r| r.as_f64())
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parse `{"numerator": N, "denominator": D}` into a Rate.
fn parse_rate_obj(val: &Value) -> Option<Rate> {
    let num = val.get("numerator")?.as_f64()?;
    let den = val.get("denominator")?.as_f64()?;
    Some(Rate { numerator: num, denominator: den })
}

/// Extract rarity-rate-per-slot for one pack type variant.
/// Returns a vec (one HashMap per slot) mapping rarity code -> probability.
fn slot_rarity_rates(
    by_rarity: Option<&Value>,
    variant: &str,
) -> Vec<HashMap<String, f64>> {
    let slots = by_rarity
        .and_then(|br| br.get(variant))
        .and_then(Value::as_array);
    let Some(slots) = slots else {
        return Vec::new();
    };

    slots
        .iter()
        .map(|slot| {
            slot.as_object()
                .map(|obj| {
                    obj.iter()
                        .filter_map(|(rarity, v)| {
                            parse_rate_obj(v).map(|r| (rarity.clone(), r.as_f64()))
                        })
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect()
}

