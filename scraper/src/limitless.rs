use anyhow::{bail, Result};
use scraper::{Html, Selector};
use tracing::warn;

use crate::client::Client;
use crate::models::{Ability, AlternateVersion, Attack, Card, CardImages, CardSummary, PackRef, SetSummary};

const BASE: &str = "https://pocket.limitlesstcg.com";

// ── Selectors ────────────────────────────────────────────────────────────────

macro_rules! sel {
    ($s:expr) => {
        Selector::parse($s).expect(concat!("bad selector: ", $s))
    };
}

// ── Public entry points ──────────────────────────────────────────────────────

/// Scrape the set index page and return a stub for every set listed.
/// Pack info is not available here — use `scrape_set_packs` for that.
pub async fn scrape_sets(client: &Client) -> Result<Vec<SetSummary>> {
    let html = client.get_text(&format!("{BASE}/cards")).await?;
    parse_sets_index(&html)
}

/// Return the pack display names found on a set's card-listing page.
/// The returned vec is ordered as displayed (pack 1, pack 2, ...).
pub async fn scrape_set_packs(client: &Client, set_code: &str) -> Result<Vec<String>> {
    let html = client.get_text(&format!("{BASE}/cards/{set_code}")).await?;
    parse_set_packs(&html)
}

/// Return the card numbers listed on a set's card-listing page.
pub async fn scrape_card_numbers(client: &Client, set_code: &str) -> Result<Vec<u32>> {
    let html = client.get_text(&format!("{BASE}/cards/{set_code}")).await?;
    parse_card_numbers(&html)
}

/// Scrape a single card page. Returns both the full `Card` and a lightweight
/// `CardSummary` suitable for the set index file.
pub async fn scrape_card(
    client: &Client,
    set_code: &str,
    number: u32,
) -> Result<(Card, CardSummary)> {
    let html = client.get_text(&format!("{BASE}/cards/{set_code}/{number}")).await?;
    parse_card(&html, set_code, number)
}

// ── Parsers ──────────────────────────────────────────────────────────────────

fn parse_sets_index(html: &str) -> Result<Vec<SetSummary>> {
    let doc = Html::parse_document(html);
    let row_sel = sel!("table.sets-table tr");
    let link_sel = sel!("td a");
    let code_sel = sel!("span.code.annotation");
    let date_sel = sel!("td.date");
    let count_sel = sel!("td.count");
    let name_sel = sel!("td.name");

    let mut sets = Vec::new();

    for row in doc.select(&row_sel) {
        let Some(link) = row.select(&link_sel).next() else { continue };
        let href = link.value().attr("href").unwrap_or("");

        // href is "/cards/A1" — extract the set code
        let set_code = match href.strip_prefix("/cards/") {
            Some(c) if !c.is_empty() => c.to_string(),
            _ => continue,
        };

        let name = row
            .select(&name_sel)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_else(|| link.text().collect::<String>().trim().to_string());

        // Prefer explicit code annotation; fall back to the href segment
        let code = row
            .select(&code_sel)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_else(|| set_code.clone());

        let release_date = row
            .select(&date_sel)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .filter(|s| !s.is_empty() && s != "—");

        let card_count = row
            .select(&count_sel)
            .next()
            .and_then(|e| e.text().collect::<String>().trim().parse::<u32>().ok());

        // Series is the leading letter(s) before the digits, or "Promo" for P-A etc.
        let series = parse_series(&code);
        let is_promo = code.contains('P') && !code.starts_with("P-");

        let icon_url =
            format!("https://s3.limitlesstcg.com/pocket/sets/{code}.webp");

        sets.push(SetSummary {
            code,
            name,
            series,
            release_date,
            is_promo,
            card_count,
            icon_url,
        });
    }

    if sets.is_empty() {
        bail!("sets-table parse returned no rows — selector may be stale");
    }
    Ok(sets)
}

fn parse_set_packs(html: &str) -> Result<Vec<String>> {
    let doc = Html::parse_document(html);
    let btn_sel = sel!("div.pack-selection button[data-value]");

    let packs: Vec<String> = doc
        .select(&btn_sel)
        .map(|e| e.text().collect::<String>().trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    Ok(packs)
}

fn parse_card_numbers(html: &str) -> Result<Vec<u32>> {
    let doc = Html::parse_document(html);
    let link_sel = sel!("div.card-search-grid a[href]");

    let mut numbers = Vec::new();
    for el in doc.select(&link_sel) {
        let href = el.value().attr("href").unwrap_or("");
        // href is "/cards/A1/1"
        if let Some(num_str) = href.split('/').last() {
            if let Ok(n) = num_str.parse::<u32>() {
                numbers.push(n);
            }
        }
    }

    numbers.sort_unstable();
    numbers.dedup();
    Ok(numbers)
}

fn parse_card(html: &str, set_code: &str, number: u32) -> Result<(Card, CardSummary)> {
    let doc = Html::parse_document(html);

    // ── Title: "Name - Element - HP HP" or "Name - Element" ─────────────────
    let title_sel = sel!("p.card-text-title");
    let title_text = doc
        .select(&title_sel)
        .next()
        .map(|e| e.text().collect::<String>())
        .unwrap_or_default();
    let title_text = title_text.trim();

    // ── Type line: "Pokémon - Basic", "Trainer - Item", etc. ────────────────
    let type_sel = sel!("p.card-text-type");
    let type_text = doc
        .select(&type_sel)
        .next()
        .map(|e| e.text().collect::<String>())
        .unwrap_or_default();
    let type_text = type_text.trim();

    let (card_type, stage, trainer_kind) = parse_type_line(type_text);

    // ── Parse title based on card type ──────────────────────────────────────
    let (name, element, hp) = parse_title(title_text, &card_type);

    // ── Rarity + pack name from current-print line ───────────────────────────
    let prints_sel = sel!("div.card-prints-current");
    let prints_text = doc
        .select(&prints_sel)
        .next()
        .map(|e| e.text().collect::<String>())
        .unwrap_or_default();
    let (rarity, pack_display_names) = parse_prints_current(&prints_text);

    // ── Weakness / Retreat ───────────────────────────────────────────────────
    let wrr_sel = sel!("p.card-text-wrr");
    let wrr_text = doc
        .select(&wrr_sel)
        .next()
        .map(|e| e.text().collect::<String>())
        .unwrap_or_default();
    let (weakness, retreat_cost) = parse_wrr(&wrr_text);

    // ── Flavor text ──────────────────────────────────────────────────────────
    let flavor_sel = sel!("div.card-text-flavor");
    let flavor = doc
        .select(&flavor_sel)
        .next()
        .map(|e| e.text().collect::<String>().trim().to_string())
        .filter(|s| !s.is_empty());

    // ── Illustrator ──────────────────────────────────────────────────────────
    let artist_sel = sel!("div.card-text-artist a");
    let illustrator = doc
        .select(&artist_sel)
        .next()
        .map(|e| e.text().collect::<String>().trim().to_string())
        .filter(|s| !s.is_empty());

    // ── Attacks ──────────────────────────────────────────────────────────────
    let attack_sel = sel!("div.card-text-attack");
    let attacks: Vec<Attack> = doc
        .select(&attack_sel)
        .filter_map(|el| parse_attack(el))
        .collect();

    // ── Ability ──────────────────────────────────────────────────────────────
    let ability_sel = sel!("div.card-text-ability");
    let ability = doc.select(&ability_sel).next().and_then(parse_ability);

    // ── Trainer effect ───────────────────────────────────────────────────────
    let effect_sel = sel!("div.card-text-effect");
    let trainer_effect = doc
        .select(&effect_sel)
        .next()
        .map(|e| e.text().collect::<String>().trim().to_string())
        .filter(|s| !s.is_empty());

    // ── Alternate versions ───────────────────────────────────────────────────
    let alt_row_sel = sel!("table.card-prints-versions tr");
    let alt_link_sel = sel!("td a[href]");
    let alt_rarity_sel = sel!("td.rarity");
    let mut alternate_versions = Vec::new();
    for row in doc.select(&alt_row_sel) {
        let href = row
            .select(&alt_link_sel)
            .next()
            .and_then(|e| e.value().attr("href"))
            .unwrap_or("");
        // "/cards/A1a/5" → set=A1a, number=5
        let parts: Vec<&str> = href.trim_start_matches('/').split('/').collect();
        if parts.len() >= 3 && parts[0] == "cards" {
            let alt_set = parts[1].to_string();
            let alt_num = parts[2].parse::<u32>().ok();
            let alt_rarity = row
                .select(&alt_rarity_sel)
                .next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            // Skip the current card's own entry
            if alt_set != set_code || alt_num != Some(number) {
                alternate_versions.push(AlternateVersion {
                    set: alt_set,
                    number: alt_num,
                    rarity: alt_rarity,
                });
            }
        }
    }

    // ── Images ───────────────────────────────────────────────────────────────
    let img_sel = sel!("img.card");
    let (thumb_url, full_url) = doc
        .select(&img_sel)
        .next()
        .map(|el| {
            let src = el.value().attr("src").unwrap_or("").to_string();
            let data_src = el.value().attr("data-src").unwrap_or("").to_string();
            (src, data_src)
        })
        .unwrap_or_else(|| {
            // Construct from CDN pattern if img tag is missing
            (cdn_thumb_url(set_code, number), cdn_full_url(set_code, number))
        });

    let thumbnail = if thumb_url.is_empty() { cdn_thumb_url(set_code, number) } else { thumb_url };
    let full = if full_url.is_empty() { cdn_full_url(set_code, number) } else { full_url };

    // ── is_ex / is_mega / variants ───────────────────────────────────────────
    let is_ex = name.ends_with(" ex") || name.contains(" ex ");
    let is_mega = name.starts_with("Mega ") || name.contains(" Mega ");
    let variants = extract_variants(&name);

    let packs: Vec<PackRef> = pack_display_names
        .into_iter()
        .map(|display_name| PackRef { raenonx_id: None, display_name })
        .collect();

    let card = Card {
        set: set_code.to_string(),
        number,
        name: name.clone(),
        rarity: rarity.clone(),
        illustrator,
        card_type: card_type.clone(),
        element: element.clone(),
        stage,
        hp,
        retreat_cost,
        weakness,
        flavor,
        is_ex: if card_type == "pokemon" { Some(is_ex) } else { None },
        is_mega: if card_type == "pokemon" { Some(is_mega) } else { None },
        variants,
        ability,
        attacks,
        trainer_kind,
        trainer_effect,
        packs,
        images: CardImages { thumbnail, full },
        alternate_versions,
    };

    let summary = CardSummary {
        number,
        name,
        rarity,
        card_type,
        element,
    };

    Ok((card, summary))
}

// ── Sub-parsers ──────────────────────────────────────────────────────────────

/// "Pokémon - Basic"  → ("pokemon", Some("Basic"), None)
/// "Trainer - Item"   → ("trainer", None, Some("Item"))
fn parse_type_line(text: &str) -> (String, Option<String>, Option<String>) {
    let parts: Vec<&str> = text.splitn(2, '-').map(str::trim).collect();
    match parts.as_slice() {
        [kind, sub] => {
            let kind_lower = kind.to_lowercase();
            if kind_lower.contains("pokémon") || kind_lower.contains("pokemon") {
                ("pokemon".into(), Some(sub.to_string()), None)
            } else if kind_lower.contains("trainer") {
                ("trainer".into(), None, Some(sub.to_string()))
            } else {
                warn!("unknown card type line: {text:?}");
                ("unknown".into(), None, None)
            }
        }
        _ => {
            warn!("could not parse type line: {text:?}");
            ("unknown".into(), None, None)
        }
    }
}

/// "Bulbasaur - Grass - 70 HP" → ("Bulbasaur", Some("Grass"), Some(70))
/// "Potion - Item"             → ("Potion", None, None)  [already split by type line]
fn parse_title(
    text: &str,
    card_type: &str,
) -> (String, Option<String>, Option<u32>) {
    let parts: Vec<&str> = text.splitn(3, '-').map(str::trim).collect();
    match (card_type, parts.as_slice()) {
        // Pokemon: Name - Element - N HP
        ("pokemon", [name, element, hp_part]) => {
            let hp = hp_part
                .split_whitespace()
                .next()
                .and_then(|s| s.parse::<u32>().ok());
            (name.to_string(), Some(element.to_string()), hp)
        }
        // Pokemon with no HP shown (some promos / unusual cards)
        ("pokemon", [name, element]) => {
            (name.to_string(), Some(element.to_string()), None)
        }
        // Trainer or any unrecognised format: just take the first segment as the name
        (_, [name, ..]) => (name.to_string(), None, None),
        _ => (text.to_string(), None, None),
    }
}

/// "#1 · ◊ · Mewtwo pack" → ("C", vec!["Mewtwo pack"])
/// "#1 · ◊ · Shared"      → ("C", vec!["Shared"])
fn parse_prints_current(text: &str) -> (String, Vec<String>) {
    let parts: Vec<&str> = text.split('·').map(str::trim).collect();
    let rarity_raw = parts.get(1).copied().unwrap_or("").trim();
    let rarity = symbol_to_rarity_code(rarity_raw)
        .map(str::to_string)
        .unwrap_or_else(|| rarity_raw.to_string());

    let pack_names: Vec<String> = parts
        .get(2..)
        .unwrap_or(&[])
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    (rarity, pack_names)
}

/// "Weakness: Fire ×2 Retreat: ●●" → (Some("Fire"), Some(2))
fn parse_wrr(text: &str) -> (Option<String>, Option<u32>) {
    let text = text.replace('\n', " ");
    let weakness = parse_wrr_field(&text, "Weakness");
    let retreat_str = parse_wrr_field(&text, "Retreat");
    let retreat_cost = retreat_str.as_deref().map(count_retreat_pips);
    (weakness, retreat_cost)
}

fn parse_wrr_field(text: &str, key: &str) -> Option<String> {
    let search = format!("{key}:");
    let start = text.find(&search)? + search.len();
    let remainder = text[start..].trim();
    // Value ends at the next keyword or end-of-string
    let end = ["Weakness:", "Retreat:", "Resistance:"]
        .iter()
        .filter_map(|kw| remainder.find(kw))
        .min()
        .unwrap_or(remainder.len());
    let value = remainder[..end].trim().to_string();
    if value.is_empty() || value == "—" || value.to_lowercase() == "none" {
        None
    } else {
        Some(value)
    }
}

/// Count the number of ● or C (colorless) pip characters in a retreat string.
fn count_retreat_pips(s: &str) -> u32 {
    // The site uses filled circles or similar; count them
    let pip_chars: &[char] = &['●', '•', 'C'];
    let count = s.chars().filter(|c| pip_chars.contains(c)).count() as u32;
    if count == 0 {
        // Fallback: try to parse a number directly
        s.split_whitespace()
            .find_map(|w| w.parse::<u32>().ok())
            .unwrap_or(0)
    } else {
        count
    }
}

fn parse_attack(el: scraper::ElementRef) -> Option<Attack> {
    // Energy cost symbols
    let sym_sel = sel!("span.ptcg-symbol");
    let cost: Vec<String> = el
        .select(&sym_sel)
        .map(|s| element_from_symbol_span(s))
        .collect();

    // Attack name: first non-symbol, non-damage text node in the info line
    let name_sel = sel!("span.card-text-attack-name");
    let name = el
        .select(&name_sel)
        .next()
        .map(|e| e.text().collect::<String>().trim().to_string())
        .filter(|s| !s.is_empty());

    // Damage
    let dmg_sel = sel!("span.card-text-attack-damage");
    let damage_raw = el
        .select(&dmg_sel)
        .next()
        .map(|e| e.text().collect::<String>().trim().to_string())
        .unwrap_or_default();

    let (damage, damage_suffix) = parse_damage(&damage_raw);

    // Effect text
    let fx_sel = sel!("div.card-text-attack-effect, p.card-text-attack-effect");
    let effect = el
        .select(&fx_sel)
        .next()
        .map(|e| e.text().collect::<String>().trim().to_string())
        .filter(|s| !s.is_empty());

    // If we couldn't get a name at all, try to extract it from plain text
    // between the cost symbols and the damage number
    let name = name.unwrap_or_else(|| {
        // Walk child text nodes; skip leading symbols and trailing damage
        let all_text: String = el
            .text()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        all_text
            .split_whitespace()
            .filter(|w| !w.chars().all(|c| "●◆◇☆♛+×0123456789".contains(c)))
            .take_while(|w| w.parse::<u32>().is_err())
            .collect::<Vec<_>>()
            .join(" ")
    });

    if name.is_empty() {
        warn!("could not parse attack name from element");
        return None;
    }

    Some(Attack { name, cost, damage, damage_suffix, effect })
}

/// Parse the ability block. Limitless renders it similarly to an attack but
/// with a different container and an "Ability" label.
fn parse_ability(el: scraper::ElementRef) -> Option<Ability> {
    let name_sel = sel!("span.card-text-ability-name, p.card-text-ability-name");
    let effect_sel = sel!("div.card-text-ability-effect, p.card-text-ability-effect");

    let name = el
        .select(&name_sel)
        .next()
        .map(|e| e.text().collect::<String>().trim().to_string())
        .filter(|s| !s.is_empty())?;

    let effect = el
        .select(&effect_sel)
        .next()
        .map(|e| e.text().collect::<String>().trim().to_string())
        .unwrap_or_default();

    Some(Ability { name, effect })
}

// ── Energy / symbol helpers ──────────────────────────────────────────────────

/// Extract the element name from a `span.ptcg-symbol` element.
///
/// Limitless encodes element type as an extra class such as `ptcg-symbol-G`
/// (single letter) or `ptcg-symbol-grass` (full name). Falls back to the
/// span's text content when no recognised class is found.
fn element_from_symbol_span(el: scraper::ElementRef) -> String {
    let classes = el.value().classes();
    for cls in classes {
        if let Some(suffix) = cls.strip_prefix("ptcg-symbol-") {
            if let Some(name) = letter_to_element(suffix) {
                return name.to_string();
            }
            // Full-name class e.g. "ptcg-symbol-grass"
            let titled = title_case(suffix);
            return titled;
        }
    }
    // Fall back to text content
    let text = el.text().collect::<String>();
    let text = text.trim();
    letter_to_element(text)
        .map(str::to_string)
        .unwrap_or_else(|| text.to_string())
}

fn letter_to_element(s: &str) -> Option<&'static str> {
    match s.to_uppercase().as_str() {
        "G" => Some("Grass"),
        "R" => Some("Fire"),
        "W" => Some("Water"),
        "L" => Some("Lightning"),
        "F" => Some("Fighting"),
        "P" => Some("Psychic"),
        "D" => Some("Darkness"),
        "M" => Some("Metal"),
        "Y" | "N" => Some("Dragon"),
        "C" => Some("Colorless"),
        _ => None,
    }
}

/// "20+" → (20, Some("+"))
/// "80×" → (80, Some("×"))
/// "40"  → (40, None)
/// ""    → (0, None)
fn parse_damage(raw: &str) -> (u32, Option<String>) {
    if raw.is_empty() {
        return (0, None);
    }
    let mut digits = String::new();
    for c in raw.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
        }
    }
    let damage = digits.parse::<u32>().unwrap_or(0);
    let suffix = raw
        .chars()
        .find(|c| !c.is_ascii_digit() && !c.is_whitespace())
        .map(|c| c.to_string());
    (damage, suffix)
}

/// Map rarity display symbols to internal codes.
fn symbol_to_rarity_code(sym: &str) -> Option<&'static str> {
    match sym {
        "◊" => Some("C"),
        "◊◊" => Some("U"),
        "◊◊◊" => Some("R"),
        "◊◊◊◊" => Some("RR"),
        // Single star — could be AR or S; we can't distinguish from the symbol alone.
        // The full rarity code appears in the alternate-versions table; use "AR" as
        // the default and let the DB builder refine via the global-master data.
        "☆" => Some("AR"),
        "☆☆" => Some("SR"),
        "☆☆☆" => Some("IM"),
        "♛" => Some("UR"),
        _ => None,
    }
}

// ── Pokemon variant / ex helpers ─────────────────────────────────────────────

/// Extract named variant identifiers from a card name.
/// e.g. "Alolan Vulpix" → ["Alolan"]
///      "Galarian Slowpoke ex" → ["Galarian"]
fn extract_variants(name: &str) -> Vec<String> {
    const KNOWN_PREFIXES: &[&str] = &[
        "Alolan", "Galarian", "Hisuian", "Paldean",
    ];
    const KNOWN_SUFFIXES: &[&str] = &[
        "Teal Mask", "Hearthflame Mask", "Wellspring Mask", "Cornerstone Mask",
    ];

    let mut variants = Vec::new();
    for prefix in KNOWN_PREFIXES {
        if name.starts_with(prefix) {
            variants.push(prefix.to_string());
        }
    }
    for suffix in KNOWN_SUFFIXES {
        if name.ends_with(suffix) {
            variants.push(suffix.to_string());
        }
    }
    variants
}

// ── CDN URL helpers ──────────────────────────────────────────────────────────

pub fn cdn_thumb_url(set_code: &str, number: u32) -> String {
    format!(
        "https://limitlesstcg.nyc3.cdn.digitaloceanspaces.com/pocket/{set_code}/{set_code}_{number:03}_EN_SM.webp"
    )
}

pub fn cdn_full_url(set_code: &str, number: u32) -> String {
    format!(
        "https://limitlesstcg.nyc3.cdn.digitaloceanspaces.com/pocket/{set_code}/{set_code}_{number:03}_EN.png"
    )
}

pub fn set_icon_url(set_code: &str) -> String {
    format!("https://s3.limitlesstcg.com/pocket/sets/{set_code}.webp")
}

// ── Misc helpers ─────────────────────────────────────────────────────────────

fn parse_series(code: &str) -> String {
    // "A1" → "A", "B2a" → "B", "P-A" → "A" (promo)
    if code.starts_with("P-") {
        return code.trim_start_matches("P-").chars().next().unwrap_or('A').to_string();
    }
    code.chars().next().map(|c| c.to_string()).unwrap_or_default()
}

fn title_case(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}
