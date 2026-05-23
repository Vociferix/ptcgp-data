use std::collections::{HashMap, HashSet};

use anyhow::{anyhow, bail, Result};
use regex::Regex;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::Value;
use tracing::warn;

use crate::client::Client;
use crate::models::{
    CardEntry, PackPullRates, PackVariantRates, PackVariants, RaritySlotRates, Rate,
};

const GLOBAL_MASTER_URL: &str = "https://ptcgp.raenonx.cc/api/data/global-master";

pub const PACK_PAGE_BASE: &str = "https://ptcgp.raenonx.cc/en/pack";

/// Canonical order for promo source display names in output.
const CARD_SOURCE_ORDER: &[&str] = &[
    "Pack",
    "Wonder Pick",
    "Gold Shop",
    "Shop",
    "Mission",
    "Premium Mission",
];

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
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_else(|| vec![card_id.clone()]);

        let source = val.get("source");

        let source_packs: Vec<String> = source
            .and_then(|s| s.get("pack"))
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();

        let card_source = card_source_from_raenonx(val);

        let is_foil = val.get("mirrorType").and_then(Value::as_str) == Some("normalMirror");

        entries.push(CardEntry {
            card_id: card_id.clone(),
            card_type,
            rarity,
            is_foil,
            collection_nums,
            card_ids_group,
            source_packs,
            card_source,
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
            let set_code = exp_to_code
                .get(exp_id)
                .cloned()
                .unwrap_or_else(|| normalize_expansion_id(exp_id));
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
                .is_some_and(|d| d.starts_with("PROMO_"))
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
                .is_some_and(|d| d.starts_with("PROMO_"))
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
    let after_dash = title.split_once(" - ")?.1;
    let chunk = after_dash.split(" | ").next()?.trim();
    // Multi-pack: "Set Name: Pack Name" → take the part after ": "
    // Single-pack: "Set Name" → use as-is
    let name = if let Some((_, after_colon)) = chunk.split_once(": ") {
        after_colon.trim()
    } else {
        chunk
    };
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
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
pub fn parse_card_source_codes(raw: &Value) -> Vec<String> {
    let entry_map = match raw.get("cardEntryMap").and_then(Value::as_object) {
        Some(m) => m,
        None => return Vec::new(),
    };

    let mut seen: HashSet<String> = HashSet::new();
    for val in entry_map.values() {
        if let Some(source) = card_source_from_raenonx(val) {
            seen.insert(source);
        }
    }

    let mut result: Vec<String> = CARD_SOURCE_ORDER
        .iter()
        .filter(|s| seen.remove(**s))
        .map(|s| s.to_string())
        .collect();
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
    warn!(
        id,
        "unrecognized expansion ID pattern — normalization may be wrong"
    );
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

/// Derive a card source display name from RaenonX data.
/// Prefers `promotion.sourceI18nId`; falls back to inferring from `source.*` fields
/// for cards that have no `promotion` (e.g. special non-promo mission cards).
fn card_source_from_raenonx(val: &Value) -> Option<String> {
    if let Some(source_id) = val
        .get("promotion")
        .and_then(|p| p.get("sourceI18nId"))
        .and_then(Value::as_str)
    {
        let source = val.get("source");
        let display = match source_id {
            "PACK" => "Pack",
            "FEED" => "Wonder Pick",
            "MISSION" | "CAMPAIGN" => "Mission",
            "SHOP" => {
                let item_shop = source
                    .and_then(|s| s.get("itemShop"))
                    .and_then(Value::as_array);
                let gold_shop = source
                    .and_then(|s| s.get("goldShop"))
                    .and_then(Value::as_array);
                if item_shop.is_some_and(|arr| arr.iter().any(|v| v.as_str() == Some("PREMIUM"))) {
                    "Premium Mission"
                } else if gold_shop.is_some_and(|arr| !arr.is_empty()) {
                    "Gold Shop"
                } else {
                    "Shop"
                }
            }
            other => {
                warn!(source_id = other, "unknown promotion.sourceI18nId");
                other
            }
        };
        return Some(display.to_string());
    }

    // Fallback for cards with no promotion field: infer from source.* content.
    let source = val.get("source")?.as_object()?;
    if source
        .get("pack")
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty())
    {
        return Some("Pack".to_string());
    }
    let wp = source.get("wonderPick").and_then(Value::as_object);
    if wp.is_some_and(|o| {
        o.values()
            .any(|v| v.as_array().is_some_and(|a| !a.is_empty()))
    }) {
        return Some("Wonder Pick".to_string());
    }
    if source
        .get("mission")
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty())
    {
        return Some("Mission".to_string());
    }
    if source
        .get("itemShop")
        .and_then(Value::as_array)
        .is_some_and(|arr| arr.iter().any(|v| v.as_str() == Some("PREMIUM")))
    {
        return Some("Premium Mission".to_string());
    }
    if source
        .get("goldShop")
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty())
    {
        return Some("Gold Shop".to_string());
    }
    if source
        .get("itemShop")
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty())
    {
        return Some("Shop".to_string());
    }
    None
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
    let value =
        Value::deserialize(&mut de).map_err(|e| anyhow!("JSON parse at key {key:?}: {e}"))?;
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
        .unwrap_or(Rate {
            numerator: Decimal::ONE,
            denominator: Decimal::ONE,
        });

    let mut rare_rate = type_rates
        .and_then(|m| m.get("rare"))
        .and_then(parse_rate_obj);

    let plus1_rate = type_rates
        .and_then(|m| m.get("plus1"))
        .and_then(parse_rate_obj);

    let by_rarity = pack_data.and_then(|d| d.get("byRarity"));

    let normal_rarity = slot_rarity_rates(by_rarity, "normal");
    let mut rare_rarity = slot_rarity_rates(by_rarity, "rare");
    let plus1_rarity = slot_rarity_rates(by_rarity, "plus1");

    let card_obj = card_map
        .as_object()
        .ok_or_else(|| anyhow!("cardPullProbabilityMap is not an object"))?;

    let mut normal_cards: HashMap<String, Vec<Option<Rate>>> = HashMap::new();
    let mut rare_cards: HashMap<String, Vec<Option<Rate>>> = HashMap::new();
    let mut plus1_cards: HashMap<String, Vec<Option<Rate>>> = HashMap::new();

    for (card_id, card_val) in card_obj {
        let by_pack = card_val.get("byPack").and_then(Value::as_object);
        let Some(by_pack) = by_pack else { continue };
        let Some(pack_entry) = by_pack.get(pack_id) else {
            continue;
        };
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

    // For the rare variant: check whether the extra byRarity slots (beyond the card
    // array length) are all single-rarity 1/1 slots. If so, this is a "themed rare"
    // pack embedded in the "rare" byRarity data (e.g. B2b Mega Shine). In that case:
    //   - byPackType "rare" rate → "themed" variant rate
    //   - actual "rare" rate = 1 − (normal + themed + plus1) [the remaining probability]
    //   - byRarity.rare[0..N] → "rare" rarity; byRarity.rare[N..] → "themed" rarity
    //   - rare_cards (SSR cards with N-element arrays) → "themed" card rates (no alignment)
    //   - "rare" card rates → empty (no per-card data for regular rare pack)
    // For plus1: compact single-element card arrays represent the bonus slot; align them.
    let mut themed_rate: Option<Rate> = None;
    let mut themed_rarity: Vec<HashMap<String, RaritySlotRates>> = Vec::new();
    let mut themed_cards: HashMap<String, Vec<Option<Rate>>> = HashMap::new();
    {
        let max_rare_card_len = rare_cards.values().map(Vec::len).max().unwrap_or(0);
        if rare_rarity.len() > max_rare_card_len && max_rare_card_len > 0 {
            let extra_are_themed = rare_rarity[max_rare_card_len..].iter().all(|slot| {
                slot.len() == 1
                    && slot.values().all(|r| {
                        r.normal
                            .as_ref()
                            .is_some_and(|rate| rate.numerator == rate.denominator)
                    })
            });
            if extra_are_themed {
                warn!(
                    pack_id,
                    rare_slots = max_rare_card_len,
                    themed_slots = rare_rarity.len() - max_rare_card_len,
                    "splitting embedded themed variant from rare byRarity"
                );
                // Split byRarity slots: [0..N] stay with "rare", [N..] go to "themed".
                themed_rarity = rare_rarity.split_off(max_rare_card_len);
                // rare_cards are the themed pack's cards; rare gets no per-card rates.
                // Drop all-null entries (cards that cannot appear in the themed pack).
                themed_cards = std::mem::take(&mut rare_cards);
                themed_cards.retain(|_, slots| slots.iter().any(Option::is_some));
                // byPackType "rare" rate → themed; derive actual rare rate as remainder.
                if let Some(rr) = rare_rate.take() {
                    let mut sum = normal_rate.numerator / normal_rate.denominator;
                    sum += rr.numerator / rr.denominator;
                    if let Some(ref pr) = plus1_rate {
                        sum += pr.numerator / pr.denominator;
                    }
                    let remainder = Decimal::ONE - sum;
                    let scale = remainder.scale();
                    let factor = Decimal::from(10u64.pow(scale));
                    let rn: i64 = (remainder * factor)
                        .try_into()
                        .expect("remainder rate numerator overflows i64");
                    let rd: i64 = factor
                        .try_into()
                        .expect("remainder rate denominator overflows i64");
                    let g = rate_gcd(rn.abs(), rd.abs());
                    rare_rate = Some(Rate {
                        numerator: Decimal::from(rn / g),
                        denominator: Decimal::from(rd / g),
                    });
                    themed_rate = Some(rr);
                }
            } else {
                align_card_slots_to_rarity(&mut rare_cards, rare_rarity.len());
            }
        }
    }
    align_card_slots_to_rarity(&mut plus1_cards, plus1_rarity.len());

    let normal_slot_count = normal_rarity
        .len()
        .max(normal_cards.values().map(Vec::len).max().unwrap_or(0));
    let rare_slot_count = rare_rarity
        .len()
        .max(rare_cards.values().map(Vec::len).max().unwrap_or(0));
    let themed_slot_count = themed_rarity
        .len()
        .max(themed_cards.values().map(Vec::len).max().unwrap_or(0));
    let plus1_slot_count = plus1_rarity
        .len()
        .max(plus1_cards.values().map(Vec::len).max().unwrap_or(0));

    let mut variants = PackVariants::new();

    variants.insert(
        "normal".to_string(),
        PackVariantRates {
            rate: normal_rate,
            slot_count: normal_slot_count as u32,
            rarity_rates_by_slot: normal_rarity,
            card_rates: normal_cards,
        },
    );

    if let Some(r) = rare_rate {
        variants.insert(
            "rare".to_string(),
            PackVariantRates {
                rate: r,
                slot_count: rare_slot_count as u32,
                rarity_rates_by_slot: rare_rarity,
                card_rates: rare_cards,
            },
        );
    }

    if let Some(r) = themed_rate {
        variants.insert(
            "themed".to_string(),
            PackVariantRates {
                rate: r,
                slot_count: themed_slot_count as u32,
                rarity_rates_by_slot: themed_rarity,
                card_rates: themed_cards,
            },
        );
    }

    if let Some(r) = plus1_rate {
        variants.insert(
            "plus1".to_string(),
            PackVariantRates {
                rate: r,
                slot_count: plus1_slot_count as u32,
                rarity_rates_by_slot: plus1_rarity,
                card_rates: plus1_cards,
            },
        );
    }

    Ok(variants)
}

/// Shift card probability arrays forward when byRarity has more slots than
/// the arrays. This aligns compact card arrays (starting at index 0) with
/// their correct byRarity slots at the end of the slot list.
fn align_card_slots_to_rarity(cards: &mut HashMap<String, Vec<Option<Rate>>>, rarity_len: usize) {
    let max_card_len = cards.values().map(Vec::len).max().unwrap_or(0);
    if rarity_len <= max_card_len {
        return;
    }
    let offset = rarity_len - max_card_len;
    for slots in cards.values_mut() {
        let mut shifted = vec![None; offset];
        shifted.append(slots);
        *slots = shifted;
    }
}

fn parse_slot_rates(val: &Value) -> Vec<Option<Rate>> {
    val.as_array()
        .map(|arr| {
            arr.iter()
                .map(|v| if v.is_null() { None } else { parse_rate_obj(v) })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_rate_obj(val: &Value) -> Option<Rate> {
    let num_val = val.get("numerator")?;
    let den_val = val.get("denominator")?;
    let numerator: Decimal = serde_json::from_value(num_val.clone()).ok()?;
    let denominator: Decimal = serde_json::from_value(den_val.clone()).ok()?;
    let (n, d) = normalize_rate(numerator, denominator);
    Some(Rate {
        numerator: Decimal::from(n),
        denominator: Decimal::from(d),
    })
}

/// Normalize a rate to an exact integer fraction.
///
/// The API returns rarity rates as clean integer fractions but per-card rates as
/// IEEE 754 floats with denominator 1 (e.g. 1/75 ≈ 0.013333.../1). Direct decimal
/// scaling of such floats produces huge denominators that overflow LCM computations.
/// We detect these cases and recover the intended exact fraction via continued fractions.
fn normalize_rate(num: Decimal, den: Decimal) -> (i64, i64) {
    let scale = num.scale().max(den.scale());
    // 10^19 fits in u64 but 10^20 does not; skip direct scaling for large scales
    // since they indicate float approximations that produce huge denominators anyway.
    if scale <= 19 {
        if let Some(factor) = 10u64.checked_pow(scale) {
            let factor = Decimal::from(factor);
            let n_opt: Option<i64> = (num * factor).try_into().ok();
            let d_opt: Option<i64> = (den * factor).try_into().ok();
            if let (Some(n), Some(d)) = (n_opt, d_opt) {
                let g = rate_gcd(n.abs(), d.abs());
                let (nr, dr) = (n / g, d / g);
                if dr <= 2_000_000_000 {
                    return (nr, dr);
                }
            }
        }
    }
    // Float artifact: use continued fractions to recover the intended exact fraction.
    use rust_decimal::prelude::ToPrimitive;
    let x = (num / den).to_f64().expect("rate not representable as f64");
    cf_rational(x, 2_000_000_000)
        .unwrap_or_else(|| panic!("cannot find rational approximation for rate {num}/{den}"))
}

fn rate_gcd(a: i64, b: i64) -> i64 {
    if b == 0 {
        a
    } else {
        rate_gcd(b, a % b)
    }
}

/// Find the simplest rational p/q with q ≤ max_denom that approximates x within 1e-9.
/// Uses the continued fraction (convergents) algorithm.
fn cf_rational(x: f64, max_denom: i64) -> Option<(i64, i64)> {
    let neg = x < 0.0;
    let x = x.abs();
    let (mut h0, mut k0) = (1i64, 0i64);
    let (mut h1, mut k1) = (x.floor() as i64, 1i64);
    let mut frac = x - x.floor();
    loop {
        let approx = h1 as f64 / k1 as f64;
        if frac < 1e-10 || (approx - x).abs() < 1e-9f64.max(x * 1e-9) {
            return Some(if neg { (-h1, k1) } else { (h1, k1) });
        }
        let recip = 1.0 / frac;
        let a = recip.floor() as i64;
        frac = recip - a as f64;
        let h2 = a * h1 + h0;
        let k2 = a * k1 + k0;
        if k2 > max_denom || k2 < 0 {
            return Some(if neg { (-h1, k1) } else { (h1, k1) });
        }
        (h0, k0) = (h1, k1);
        (h1, k1) = (h2, k2);
    }
}

/// Derive card pull rates from rarity rates, replacing the API-provided float values.
///
/// Per-card rates are not stored in the PTCGP APK; RaenonX computes them from rarity
/// rates. For normal packs: per-card rate = rarity_rate / n_cards_of_that_rarity.
///
/// For packs with foil (mirror) cards (currently only A4b): foil cards across all
/// rarities share a single uniform "mirror rate" per slot. Non-foil rates for mixed
/// rarities are derived as the residual after subtracting the foil contribution.
///
/// The API-provided values are compared against derived rates; significant mismatches
/// are logged as warnings.
pub fn fix_card_rates_from_rarity(
    rates: &mut crate::models::PackPullRates,
    card_id_to_rarity: &HashMap<String, String>,
    card_id_to_is_foil: &HashMap<String, bool>,
) {
    use rust_decimal::prelude::ToPrimitive;

    for variant in rates.variants.values_mut() {
        let slot_count = variant.rarity_rates_by_slot.len().max(
            variant
                .card_rates
                .values()
                .map(|v| v.len())
                .max()
                .unwrap_or(0),
        );

        for slot_idx in 0..slot_count {
            // Count foil and non-foil cards per rarity in this slot.
            let mut foil_counts: HashMap<&str, usize> = HashMap::new();
            let mut nonfoil_counts: HashMap<&str, usize> = HashMap::new();
            for (card_id, slot_rates) in variant.card_rates.iter() {
                if slot_rates.get(slot_idx).is_some_and(|r| r.is_some()) {
                    let rarity = card_id_to_rarity
                        .get(card_id)
                        .map(String::as_str)
                        .unwrap_or("");
                    if card_id_to_is_foil.get(card_id).copied().unwrap_or(false) {
                        *foil_counts.entry(rarity).or_insert(0) += 1;
                    } else {
                        *nonfoil_counts.entry(rarity).or_insert(0) += 1;
                    }
                }
            }

            let rarity_map = variant.rarity_rates_by_slot.get(slot_idx);

            // Determine the uniform mirror rate from any rarity where ALL cards are foil.
            // In A4b, C and U are all-foil, giving mirror_rate = rarity_rate / foil_count.
            let mut mirror_rate: Option<(i64, i64)> = None;
            for (rarity, &fc) in &foil_counts {
                if nonfoil_counts.get(rarity).copied().unwrap_or(0) == 0 {
                    if let Some(rr) = rarity_map.and_then(|m| m.get(*rarity)) {
                        let Some(total) = &rr.normal else { continue };
                        let Ok(rn): Result<i64, _> = total.numerator.try_into() else {
                            continue;
                        };
                        let Ok(rd): Result<i64, _> = total.denominator.try_into() else {
                            continue;
                        };
                        let den = rd * fc as i64;
                        let g = rate_gcd(rn.abs(), den.abs());
                        let candidate = (rn / g, den / g);
                        if let Some(existing) = mirror_rate {
                            if existing != candidate {
                                warn!(
                                    slot = slot_idx,
                                    rarity,
                                    "inconsistent mirror rate candidates: {existing:?} vs {candidate:?}"
                                );
                            }
                        }
                        mirror_rate = Some(candidate);
                    }
                }
            }

            // Update per-card rates.
            for (card_id, slot_rates) in variant.card_rates.iter_mut() {
                let Some(Some(api_rate)) = slot_rates.get(slot_idx) else {
                    continue;
                };
                let rarity = card_id_to_rarity
                    .get(card_id)
                    .map(String::as_str)
                    .unwrap_or("");
                let is_foil = card_id_to_is_foil.get(card_id).copied().unwrap_or(false);

                let derived = if is_foil {
                    if let Some((mn, md)) = mirror_rate {
                        Some((mn, md))
                    } else {
                        warn!(
                            card_id,
                            slot = slot_idx,
                            "foil card but no mirror rate determined; keeping API value"
                        );
                        None
                    }
                } else {
                    let Some(rarity_rate) = rarity_map.and_then(|m| m.get(rarity)) else {
                        warn!(
                            card_id,
                            slot = slot_idx,
                            rarity,
                            "card rarity not found in slot rarity rates; keeping API value"
                        );
                        continue;
                    };
                    let Some(total) = &rarity_rate.normal else {
                        continue;
                    };
                    let Ok(rn): Result<i64, _> = total.numerator.try_into() else {
                        continue;
                    };
                    let Ok(rd): Result<i64, _> = total.denominator.try_into() else {
                        continue;
                    };
                    let nf_count = nonfoil_counts.get(rarity).copied().unwrap_or(0) as i64;
                    let f_count = foil_counts.get(rarity).copied().unwrap_or(0) as i64;

                    if f_count > 0 {
                        let Some((mn, md)) = mirror_rate else {
                            warn!(
                                card_id,
                                slot = slot_idx,
                                rarity,
                                "mixed rarity but no mirror rate; keeping API value"
                            );
                            continue;
                        };
                        let num = rn * md - rd * f_count * mn;
                        let den = rd * md * nf_count;
                        let g = rate_gcd(num.abs(), den.abs());
                        Some((num / g, den / g))
                    } else {
                        let den = rd * nf_count;
                        let g = rate_gcd(rn.abs(), den.abs());
                        Some((rn / g, den / g))
                    }
                };

                let Some((derived_num, derived_den)) = derived else {
                    continue;
                };

                let api_f64 = api_rate.numerator.to_f64().unwrap_or(f64::NAN)
                    / api_rate.denominator.to_f64().unwrap_or(1.0);
                let derived_f64 = derived_num as f64 / derived_den as f64;
                if api_f64.is_finite() && (api_f64 - derived_f64).abs() > api_f64.abs() * 1e-4 {
                    warn!(
                        card_id,
                        slot = slot_idx,
                        rarity,
                        is_foil,
                        api_rate = api_f64,
                        derived_rate = derived_f64,
                        "card rate mismatch: API value does not match derived rate"
                    );
                }

                slot_rates[slot_idx] = Some(crate::models::Rate {
                    numerator: Decimal::from(derived_num),
                    denominator: Decimal::from(derived_den),
                });
            }

            // Split each rarity's total rate into normal and foil sub-rates.
            if let Some(slot_rarity) = variant.rarity_rates_by_slot.get_mut(slot_idx) {
                for (rarity, rarity_rates) in slot_rarity.iter_mut() {
                    let fc = foil_counts.get(rarity.as_str()).copied().unwrap_or(0) as i64;
                    let nfc = nonfoil_counts.get(rarity.as_str()).copied().unwrap_or(0) as i64;

                    if fc == 0 {
                        continue; // all non-foil: keep as normal only
                    }

                    let Some(total) = rarity_rates.normal.take() else {
                        continue;
                    };

                    if nfc == 0 {
                        // All foil: total rate belongs entirely to foil.
                        rarity_rates.foil = Some(total);
                    } else if let Some((mn, md)) = mirror_rate {
                        // Mixed: foil portion = fc × mirror_rate; normal = total − foil.
                        let Ok(rn): Result<i64, _> = total.numerator.try_into() else {
                            rarity_rates.normal = Some(total);
                            continue;
                        };
                        let Ok(rd): Result<i64, _> = total.denominator.try_into() else {
                            rarity_rates.normal = Some(total);
                            continue;
                        };

                        let foil_raw_n = fc * mn;
                        let foil_raw_d = md;
                        let gf = rate_gcd(foil_raw_n.abs(), foil_raw_d.abs());

                        let normal_num = rn * md - rd * fc * mn;
                        let normal_den = rd * md;
                        let gn = rate_gcd(normal_num.abs(), normal_den.abs());

                        rarity_rates.foil = Some(crate::models::Rate {
                            numerator: Decimal::from(foil_raw_n / gf),
                            denominator: Decimal::from(foil_raw_d / gf),
                        });
                        rarity_rates.normal = Some(crate::models::Rate {
                            numerator: Decimal::from(normal_num / gn),
                            denominator: Decimal::from(normal_den / gn),
                        });
                    } else {
                        rarity_rates.normal = Some(total); // no mirror rate, leave unchanged
                    }
                }
            }
        }
    }
}

fn slot_rarity_rates(
    by_rarity: Option<&Value>,
    variant: &str,
) -> Vec<HashMap<String, RaritySlotRates>> {
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
                            parse_rate_obj(v).map(|r| {
                                (
                                    rarity.clone(),
                                    RaritySlotRates {
                                        normal: Some(r),
                                        foil: None,
                                    },
                                )
                            })
                        })
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect()
}
