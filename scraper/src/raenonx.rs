use std::collections::{HashMap, HashSet};

use anyhow::{anyhow, bail, Result};
use regex::Regex;
use serde::Deserialize;
use tracing::warn;
use serde_json::Value;

use crate::client::Client;
use crate::models::{CardEntry, PackPullRates, PackVariantRates, PackVariants, Rate};

const GLOBAL_MASTER_URL: &str = "https://ptcgp.raenonx.cc/api/data/global-master";

/// Known promo source fields in the RaenonX `source` object, mapped to display names.
/// The `pack` field is excluded — it maps to pack subtitles via `source_packs`.
const PROMO_SOURCE_FIELDS: &[(&str, &str)] = &[
    ("wonderPick", "Wonder Pick"),
    ("goldShop", "Gold Shop"),
    ("itemShop", "Shop"),
    ("mission", "Mission"),
];
pub const PACK_PAGE_BASE: &str = "https://ptcgp.raenonx.cc/en/pack";

// ── Public entry points ──────────────────────────────────────────────────────

/// Fetch the global-master endpoint and return the raw JSON.
/// If a cached copy exists on disk, it is loaded instead.
pub async fn fetch_global_master(client: &Client) -> Result<Value> {
    client.get_json(GLOBAL_MASTER_URL).await
}

/// Fetch the RaenonX pack page and extract pull rate data via RSC parsing.
pub async fn fetch_pack_pull_rates(
    client: &Client,
    pack_id: &str,
    set: &str,
    subtitle: &str,
) -> Result<PackPullRates> {
    let url = format!("{PACK_PAGE_BASE}/{pack_id}");
    let html = client.get_text(&url).await?;
    parse_rsc_pull_rates(&html, pack_id, set, subtitle)
}

// ── Global master parsers ────────────────────────────────────────────────────

/// Parse all card entries from the raw global-master JSON.
pub fn parse_card_entries(raw: &Value) -> Result<Vec<CardEntry>> {
    let entry_map = raw
        .get("cardEntryMap")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("cardEntryMap missing from global-master"))?;

    let mut entries = Vec::with_capacity(entry_map.len());

    for (card_id, val) in entry_map {
        let card_type = val
            .get("cardType")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();

        let rarity = val
            .get("rarity")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        let collection_nums: Vec<(String, u32)> = val
            .get("collectionNums")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|cn| {
                        let exp_id = cn
                            .get("expansion")
                            .and_then(|e| e.get("id"))
                            .and_then(Value::as_str)?;
                        let num = cn.get("num").and_then(Value::as_u64)? as u32;
                        Some((normalize_expansion_id(exp_id), num))
                    })
                    .collect()
            })
            .unwrap_or_default();

        let card_ids_group: Vec<String> = val
            .get("play")
            .and_then(|p| p.get("cardIds"))
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_else(|| vec![card_id.clone()]);

        let source = val.get("source");

        let source_packs: Vec<String> = source
            .and_then(|s| s.get("pack"))
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default();

        let promo_sources = extract_promo_sources(source);

        let is_foil = val
            .get("mirrorType")
            .and_then(Value::as_str)
            .map_or(false, |m| m == "normalMirror");

        entries.push(CardEntry {
            card_id: card_id.clone(),
            card_type,
            rarity,
            is_foil,
            collection_nums,
            card_ids_group,
            source_packs,
            promo_sources,
        });
    }

    Ok(entries)
}

/// Parse the ordered pack IDs for each expansion.
/// Returns: expansion_id → ordered list of pack IDs
pub fn parse_expansion_packs(raw: &Value) -> HashMap<String, Vec<String>> {
    let expansion_map = match raw.get("cardExpansionMap").and_then(Value::as_object) {
        Some(m) => m,
        None => return HashMap::new(),
    };

    expansion_map
        .iter()
        .filter_map(|(key, val)| {
            let packs: Vec<String> = val
                .get("packsInExpansion")
                .and_then(Value::as_array)?
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect();
            if packs.is_empty() {
                return None;
            }
            Some((normalize_expansion_id(key), packs))
        })
        .collect()
}

/// Parse the regular pack IDs (isRegular=true), sorted.
pub fn parse_regular_packs(raw: &Value) -> Vec<String> {
    let pack_map = match raw.get("cardPackMap").and_then(Value::as_object) {
        Some(m) => m,
        None => return Vec::new(),
    };

    let mut packs: Vec<String> = pack_map
        .iter()
        .filter(|(_, v)| v.get("isRegular").and_then(Value::as_bool).unwrap_or(false))
        .map(|(k, _)| k.clone())
        .collect();
    packs.sort();
    packs
}

/// Parse pack_id → expansion set code mapping.
pub fn parse_pack_expansion(raw: &Value) -> HashMap<String, String> {
    let pack_map = match raw.get("cardPackMap").and_then(Value::as_object) {
        Some(m) => m,
        None => return HashMap::new(),
    };
    let expansion_map = match raw.get("cardExpansionMap").and_then(Value::as_object) {
        Some(m) => m,
        None => return HashMap::new(),
    };

    // Build expansion internal-ID -> normalized set code
    let mut exp_to_code: HashMap<String, String> = HashMap::new();
    for (key, _) in expansion_map {
        exp_to_code.insert(key.clone(), normalize_expansion_id(key));
    }

    pack_map
        .iter()
        .filter_map(|(pack_id, val)| {
            let exp_id = val.get("expansionId").and_then(Value::as_str)?;
            let set_code = exp_to_code.get(exp_id).cloned().unwrap_or_else(|| normalize_expansion_id(exp_id));
            Some((pack_id.clone(), set_code))
        })
        .collect()
}

/// Parse the description IDs for promo packs (AP*** and BP***).
/// Returns: pack_id → descriptionId (used as subtitle for promo packs)
pub fn parse_promo_pack_subtitles(raw: &Value) -> HashMap<String, String> {
    let pack_map = match raw.get("cardPackMap").and_then(Value::as_object) {
        Some(m) => m,
        None => return HashMap::new(),
    };

    pack_map
        .iter()
        .filter(|(_, val)| {
            val.get("descriptionId")
                .and_then(Value::as_str)
                .map_or(false, |d| d.starts_with("PROMO_"))
        })
        .filter_map(|(pack_id, val)| {
            let desc = val.get("descriptionId").and_then(Value::as_str)?;
            Some((pack_id.clone(), desc.to_string()))
        })
        .collect()
}

/// Parse the IDs of named regular packs (AN***, BN***).
/// These are packs whose names must be fetched from the RaenonX pack page.
pub fn parse_named_pack_ids(raw: &Value) -> Vec<String> {
    let pack_map = match raw.get("cardPackMap").and_then(Value::as_object) {
        Some(m) => m,
        None => return Vec::new(),
    };

    let mut packs: Vec<String> = pack_map
        .iter()
        .filter(|(_, v)| v.get("isRegular").and_then(Value::as_bool).unwrap_or(false))
        .filter(|(_, v)| {
            !v.get("descriptionId")
                .and_then(Value::as_str)
                .map_or(false, |d| d.starts_with("PROMO_"))
        })
        .map(|(k, _)| k.clone())
        .collect();
    packs.sort();
    packs
}

/// Extract pack name from a RaenonX pack page HTML title.
///
/// Two formats observed:
/// - Multi-pack set: "Card Pack Info - Genetic Apex: Mewtwo | RaenonX..." → "Mewtwo"
/// - Single-pack set: "Card Pack Info - Mythical Island | RaenonX..." → "Mythical Island"
pub fn parse_pack_name_from_title(html: &str) -> Option<String> {
    let start = html.find("<title>")?;
    let end = html.find("</title>")?;
    let title = &html[start + 7..end];
    let after_dash = title.splitn(2, " - ").nth(1)?;
    let chunk = after_dash.split(" | ").next()?.trim();
    // Multi-pack: "Set Name: Pack Name" → take the part after ": "
    // Single-pack: "Set Name" → use as-is
    let name = if let Some(after_colon) = chunk.splitn(2, ": ").nth(1) {
        after_colon.trim()
    } else {
        chunk
    };
    if name.is_empty() { None } else { Some(name.to_string()) }
}

/// Parse craft costs from cardPackPointMap.
pub fn parse_craft_costs(raw: &Value) -> HashMap<String, u32> {
    parse_rarity_u32_map(raw, "cardPackPointMap")
}

/// Parse dupe dust values from cardDupeShineDustMap.
pub fn parse_dupe_dust(raw: &Value) -> HashMap<String, u32> {
    parse_rarity_u32_map(raw, "cardDupeShineDustMap")
}

/// Collect all promo source codes seen across all card entries, in display order.
/// "Pack" is included first when any card has an AP*** promo pack source.
pub fn parse_promo_source_codes(raw: &Value) -> Vec<String> {
    let entry_map = match raw.get("cardEntryMap").and_then(Value::as_object) {
        Some(m) => m,
        None => return Vec::new(),
    };

    let field_to_display: HashMap<&str, &str> = PROMO_SOURCE_FIELDS.iter().copied().collect();
    let mut seen: HashSet<String> = HashSet::new();
    let mut has_ap_packs = false;

    for val in entry_map.values() {
        let source = match val.get("source").and_then(Value::as_object) {
            Some(s) => s,
            None => continue,
        };

        for (field, v) in source {
            if field == "pack" {
                if v.as_array().map_or(false, |arr| {
                    arr.iter().any(|p| p.as_str().map_or(false, |s| s.starts_with("AP")))
                }) {
                    has_ap_packs = true;
                }
                continue;
            }
            if !source_field_has_content(v) {
                continue;
            }
            let code = field_to_display
                .get(field.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    warn!(field, "unknown promo source field — add to PROMO_SOURCE_FIELDS in raenonx.rs");
                    field.clone()
                });
            seen.insert(code);
        }
    }

    // Preserve known-field order, then append unknowns sorted.
    let mut result = Vec::new();
    if has_ap_packs {
        result.push("Pack".to_string());
    }
    for (_, display) in PROMO_SOURCE_FIELDS {
        let s = display.to_string();
        if seen.remove(&s) {
            result.push(s);
        }
    }
    let mut unknowns: Vec<String> = seen.into_iter().collect();
    unknowns.sort();
    result.extend(unknowns);
    result
}

/// Collect all unique non-empty rarity codes seen in cardEntryMap.
pub fn parse_rarity_codes(raw: &Value) -> Vec<String> {
    let entry_map = match raw.get("cardEntryMap").and_then(Value::as_object) {
        Some(m) => m,
        None => return Vec::new(),
    };
    let mut codes: Vec<String> = entry_map
        .values()
        .filter_map(|v| v.get("rarity").and_then(Value::as_str))
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    codes.sort();
    codes.dedup();
    codes
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Normalize RaenonX expansion IDs to match Limitless set codes.
/// "PROMO-A" → "P-A", "PROMO-B" → "P-B", plain alphanumeric IDs pass through unchanged.
pub fn normalize_expansion_id(id: &str) -> String {
    if let Some(suffix) = id.strip_prefix("PROMO-") {
        return format!("P-{suffix}");
    }
    // Plain set codes (A1, B2a, etc.) pass through as-is.
    if id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return id.to_string();
    }
    warn!(id, "unrecognized expansion ID pattern — normalization may be wrong");
    id.to_string()
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

/// Extract human-readable promo source names from a card's source object.
/// Iterates over all source fields (excluding `pack`) and maps them to display names.
fn extract_promo_sources(source: Option<&Value>) -> Vec<String> {
    let Some(source) = source.and_then(Value::as_object) else {
        return Vec::new();
    };

    let field_to_display: HashMap<&str, &str> = PROMO_SOURCE_FIELDS.iter().copied().collect();

    // Collect seen display names, then re-order to match PROMO_SOURCE_FIELDS order.
    let mut seen: HashSet<String> = HashSet::new();
    for (field, val) in source {
        if field == "pack" {
            continue;
        }
        if !source_field_has_content(val) {
            continue;
        }
        let code = field_to_display
            .get(field.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                warn!(field, "unknown promo source field — add to PROMO_SOURCE_FIELDS in raenonx.rs");
                field.clone()
            });
        seen.insert(code);
    }

    let mut result = Vec::new();
    for (_, display) in PROMO_SOURCE_FIELDS {
        let s = display.to_string();
        if seen.remove(&s) {
            result.push(s);
        }
    }
    let mut unknowns: Vec<String> = seen.into_iter().collect();
    unknowns.sort();
    result.extend(unknowns);
    result
}

fn source_field_has_content(val: &Value) -> bool {
    match val {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map_or(false, |f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(arr) => !arr.is_empty(),
        Value::Object(obj) => obj.values().any(source_field_has_content),
    }
}

// ── RSC pull rate parser ─────────────────────────────────────────────────────

pub fn parse_rsc_pull_rates(
    html: &str,
    pack_id: &str,
    set: &str,
    subtitle: &str,
) -> Result<PackPullRates> {
    let chunk = extract_rsc_chunk(html, "cardPullProbabilityMap")?;

    let card_map = extract_json_at_key(&chunk, "cardPullProbabilityMap")?;
    let pack_data = extract_json_at_key(&chunk, "packPullProbabilityData").ok();

    let variants = build_variants(pack_id, &card_map, pack_data.as_ref())?;

    Ok(PackPullRates {
        set: set.to_string(),
        subtitle: subtitle.to_string(),
        variants,
    })
}

fn extract_rsc_chunk(html: &str, key: &str) -> Result<String> {
    let re = Regex::new(r#"self\.__next_f\.push\(\[1,"((?:[^"\\]|\\.)*)"\]\)"#)?;

    for cap in re.captures_iter(html) {
        let escaped = &cap[1];
        if !escaped.contains(key) {
            continue;
        }
        let json_str = format!("\"{}\"", escaped);
        let unescaped: String = serde_json::from_str(&json_str)
            .map_err(|e| anyhow!("RSC chunk unescape failed: {e}"))?;
        return Ok(unescaped);
    }

    bail!("no RSC chunk containing {key:?} found");
}

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
    let type_rates = pack_data
        .and_then(|d| d.get("byPackType"))
        .and_then(Value::as_object);

    let normal_rate = type_rates
        .and_then(|m| m.get("normal"))
        .and_then(parse_rate_obj)
        .unwrap_or(Rate { numerator: 1.0, denominator: 1.0 });

    let rare_rate = type_rates
        .and_then(|m| m.get("rare"))
        .and_then(parse_rate_obj);

    let plus1_rate = type_rates
        .and_then(|m| m.get("plus1"))
        .and_then(parse_rate_obj);

    let by_rarity = pack_data.and_then(|d| d.get("byRarity"));

    let normal_rarity = slot_rarity_rates(by_rarity, "normal");
    let rare_rarity = slot_rarity_rates(by_rarity, "rare");
    let plus1_rarity = slot_rarity_rates(by_rarity, "plus1");

    let card_obj = card_map
        .as_object()
        .ok_or_else(|| anyhow!("cardPullProbabilityMap is not an object"))?;

    let mut normal_cards: HashMap<String, Vec<Option<f64>>> = HashMap::new();
    let mut rare_cards: HashMap<String, Vec<Option<f64>>> = HashMap::new();
    let mut plus1_cards: HashMap<String, Vec<Option<f64>>> = HashMap::new();

    for (card_id, card_val) in card_obj {
        let by_pack = card_val.get("byPack").and_then(Value::as_object);
        let Some(by_pack) = by_pack else { continue };
        let Some(pack_entry) = by_pack.get(pack_id) else { continue };
        let probs = pack_entry.get("cardProbability").and_then(Value::as_object);
        let Some(probs) = probs else { continue };

        if let Some(slots) = probs.get("normal") {
            normal_cards.insert(card_id.clone(), parse_slot_rates(slots));
        }
        if let Some(slots) = probs.get("rare") {
            rare_cards.insert(card_id.clone(), parse_slot_rates(slots));
        }
        if let Some(slots) = probs.get("plus1") {
            plus1_cards.insert(card_id.clone(), parse_slot_rates(slots));
        }
    }

    let normal_slot_count = normal_rarity.len().max(
        normal_cards.values().map(Vec::len).max().unwrap_or(0),
    );
    let rare_slot_count = rare_rarity.len().max(
        rare_cards.values().map(Vec::len).max().unwrap_or(0),
    );
    let plus1_slot_count = plus1_rarity.len().max(
        plus1_cards.values().map(Vec::len).max().unwrap_or(0),
    );

    let mut variants = PackVariants::new();

    variants.insert("normal".to_string(), PackVariantRates {
        rate: normal_rate.as_f64(),
        rate_numerator: normal_rate.numerator,
        rate_denominator: normal_rate.denominator,
        slot_count: normal_slot_count as u32,
        rarity_rates_by_slot: normal_rarity,
        card_rates: normal_cards,
    });

    if let Some(r) = rare_rate {
        variants.insert("rare".to_string(), PackVariantRates {
            rate: r.as_f64(),
            rate_numerator: r.numerator,
            rate_denominator: r.denominator,
            slot_count: rare_slot_count as u32,
            rarity_rates_by_slot: rare_rarity,
            card_rates: rare_cards,
        });
    }

    if let Some(r) = plus1_rate {
        variants.insert("plus1".to_string(), PackVariantRates {
            rate: r.as_f64(),
            rate_numerator: r.numerator,
            rate_denominator: r.denominator,
            slot_count: plus1_slot_count as u32,
            rarity_rates_by_slot: plus1_rarity,
            card_rates: plus1_cards,
        });
    }

    Ok(variants)
}

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

fn parse_rate_obj(val: &Value) -> Option<Rate> {
    let num = val.get("numerator")?.as_f64()?;
    let den = val.get("denominator")?.as_f64()?;
    Some(Rate { numerator: num, denominator: den })
}

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
