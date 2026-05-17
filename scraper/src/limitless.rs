use anyhow::{bail, Result};
use scraper::{Html, Selector};
use tracing::warn;

use crate::client::Client;
use crate::models::{Ability, Attack, LimitlessCardData, SetSummary};

const BASE: &str = "https://pocket.limitlesstcg.com";

// ── Selectors ────────────────────────────────────────────────────────────────

macro_rules! sel {
    ($s:expr) => {
        Selector::parse($s).expect(concat!("bad selector: ", $s))
    };
}

// ── Public entry points ──────────────────────────────────────────────────────

pub async fn scrape_sets(client: &Client) -> Result<Vec<SetSummary>> {
    let html = client.get_text(&format!("{BASE}/cards")).await?;
    parse_sets_index(&html)
}

pub async fn scrape_card_numbers(client: &Client, set_code: &str) -> Result<Vec<u32>> {
    let html = client.get_text(&format!("{BASE}/cards/{set_code}")).await?;
    parse_card_numbers(&html)
}

pub async fn scrape_card(
    client: &Client,
    set_code: &str,
    number: u32,
) -> Result<LimitlessCardData> {
    let html = client
        .get_text(&format!("{BASE}/cards/{set_code}/{number}"))
        .await?;
    parse_card(&html, set_code)
}

// ── Set index parser ─────────────────────────────────────────────────────────

fn parse_sets_index(html: &str) -> Result<Vec<SetSummary>> {
    let doc = Html::parse_document(html);
    let row_sel = sel!("table.sets-table tr");
    let td_sel = sel!("td");
    let link_sel = sel!("td a");
    let code_sel = sel!("span.code.annotation");

    let mut sets = Vec::new();

    for row in doc.select(&row_sel) {
        let Some(link) = row.select(&link_sel).next() else {
            continue;
        };
        let href = link.value().attr("href").unwrap_or("");

        let set_code = match href.strip_prefix("/cards/") {
            Some(c) if !c.is_empty() => c.to_string(),
            _ => continue,
        };

        let code = row
            .select(&code_sel)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_else(|| set_code.clone());

        let link_text = link.text().collect::<String>();
        let name = link_text
            .replace(&code, "")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        let tds: Vec<_> = row.select(&td_sel).collect();

        let release_date = tds
            .get(1)
            .map(|e| e.text().collect::<String>().trim().to_string())
            .filter(|s| !s.is_empty() && s != "—")
            .and_then(|s| parse_release_date(&s));

        let card_count = tds
            .get(2)
            .and_then(|e| e.text().collect::<String>().trim().parse::<u32>().ok());

        let series = parse_series(&code);
        let is_promo = code.starts_with("P-");

        sets.push(SetSummary {
            code,
            name,
            series,
            release_date,
            is_promo,
            card_count,
        });
    }

    if sets.is_empty() {
        bail!("sets-table parse returned no rows — selector may be stale");
    }
    Ok(sets)
}

// ── Card number list parser ──────────────────────────────────────────────────

fn parse_card_numbers(html: &str) -> Result<Vec<u32>> {
    let doc = Html::parse_document(html);
    let link_sel = sel!("div.card-search-grid a[href]");

    let mut numbers = Vec::new();
    for el in doc.select(&link_sel) {
        let href = el.value().attr("href").unwrap_or("");
        if let Some(num_str) = href.split('/').next_back() {
            if let Ok(n) = num_str.parse::<u32>() {
                numbers.push(n);
            }
        }
    }

    numbers.sort_unstable();
    numbers.dedup();
    Ok(numbers)
}

// ── Individual card parser ───────────────────────────────────────────────────

fn parse_card(html: &str, set_code: &str) -> Result<LimitlessCardData> {
    let doc = Html::parse_document(html);

    let name_sel = sel!("span.card-text-name");
    let name = doc
        .select(&name_sel)
        .next()
        .map(|e| e.text().collect::<String>().trim().to_string())
        .unwrap_or_default();

    let title_sel = sel!("p.card-text-title");
    let title_text = doc
        .select(&title_sel)
        .next()
        .map(|e| e.text().collect::<String>())
        .unwrap_or_default();

    let type_sel = sel!("p.card-text-type");
    let type_text = doc
        .select(&type_sel)
        .next()
        .map(|e| e.text().collect::<String>())
        .unwrap_or_default();

    let (card_type, stage, trainer_kind) = parse_type_line(type_text.trim());
    let evolves_from = parse_evolves_from(type_text.trim());

    let (element, hp) = if card_type == "pokemon" {
        parse_element_hp(&title_text, &name)
    } else {
        (None, None)
    };

    let wrr_sel = sel!("p.card-text-wrr");
    let wrr_text = doc
        .select(&wrr_sel)
        .next()
        .map(|e| e.text().collect::<String>())
        .unwrap_or_default();
    let (weakness, retreat_cost) = parse_wrr(&wrr_text);

    let flavor_sel = sel!("div.card-text-flavor");
    let flavor = doc
        .select(&flavor_sel)
        .next()
        .map(|e| e.text().collect::<String>().trim().to_string())
        .filter(|s| !s.is_empty());

    let artist_sel = sel!("div.card-text-artist a");
    let illustrator = doc
        .select(&artist_sel)
        .next()
        .map(|e| e.text().collect::<String>().trim().to_string())
        .filter(|s| !s.is_empty());

    let attack_sel = sel!("div.card-text-attack");
    let attacks: Vec<Attack> = doc.select(&attack_sel).filter_map(parse_attack).collect();

    let ability_sel = sel!("div.card-text-ability");
    let ability = doc.select(&ability_sel).next().and_then(parse_ability);

    let section_sel = sel!("div.card-text-section");
    let trainer_effect = if card_type == "trainer" {
        doc.select(&section_sel)
            .find(|el| {
                let title_child = Selector::parse("p.card-text-title").ok();
                let has_title = title_child
                    .map(|s| el.select(&s).next().is_some())
                    .unwrap_or(false);
                let classes = el.value().classes().collect::<Vec<_>>();
                let is_artist_or_flavor = classes
                    .iter()
                    .any(|c| *c == "card-text-artist" || *c == "card-text-flavor");
                !has_title && !is_artist_or_flavor
            })
            .map(|el| render_card_text(el))
            .filter(|s| !s.is_empty())
    } else {
        None
    };

    let is_ex = name.ends_with(" ex") || name.contains(" ex ");
    let is_mega = name.starts_with("Mega ") || name.contains(" Mega ");

    // Warn if we couldn't parse the name — indicates a selector issue
    if name.is_empty() {
        warn!(set = set_code, "card name is empty — selector may be stale");
    }

    Ok(LimitlessCardData {
        name,
        card_type,
        element,
        stage,
        hp,
        retreat_cost,
        weakness,
        flavor,
        is_ex,
        is_mega,
        ability,
        attacks,
        evolves_from,
        trainer_kind,
        trainer_effect,
        illustrator,
    })
}

// ── Sub-parsers ──────────────────────────────────────────────────────────────

/// "Pokémon - Basic"  → ("pokemon", Some("Basic"), None)
/// "Trainer - Item"   → ("trainer", None, Some("Item"))
fn parse_type_line(text: &str) -> (String, Option<String>, Option<String>) {
    let parts: Vec<&str> = text.splitn(2, '-').map(str::trim).collect();
    match parts.as_slice() {
        [kind, sub] => {
            let sub_first = sub.lines().next().unwrap_or(sub).trim();
            let kind_lower = kind.to_lowercase();
            if kind_lower.contains("pokémon") || kind_lower.contains("pokemon") {
                ("pokemon".into(), Some(sub_first.to_string()), None)
            } else if kind_lower.contains("trainer") {
                ("trainer".into(), None, Some(sub_first.to_string()))
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

/// Extract "Evolves from X" from the type line text.
///
/// Limitless renders the evolved-from name as an <a> link, so `.text()` puts
/// "Evolves from" and the name on consecutive lines rather than one line.
fn parse_evolves_from(type_text: &str) -> Option<String> {
    let mut lines = type_text.lines().map(|l| l.trim());
    while let Some(line) = lines.next() {
        let without_dash = line.trim_start_matches('-').trim();
        if let Some(name) = without_dash.strip_prefix("Evolves from ") {
            // Inline: "Evolves from Bulbasaur"
            let name = name.trim();
            if !name.is_empty() {
                return Some(name.to_string());
            }
        } else if without_dash == "Evolves from" {
            // Linked: name is on the next non-empty line (inside an <a> tag)
            return lines.find(|l| !l.is_empty()).map(str::to_string);
        }
    }
    None
}

fn parse_element_hp(title_text: &str, name: &str) -> (Option<String>, Option<u32>) {
    let after = title_text
        .find(name)
        .map(|i| &title_text[i + name.len()..])
        .unwrap_or(title_text);

    let parts: Vec<&str> = after
        .split('-')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();

    let element = parts
        .first()
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|s| !s.is_empty());

    let hp = parts.get(1).and_then(|s| {
        s.split_whitespace()
            .next()
            .and_then(|w| w.parse::<u32>().ok())
    });

    (element, hp)
}

/// "Weakness: Fire\nRetreat: 1\n" → (Some("Fire"), Some(1))
fn parse_wrr(text: &str) -> (Option<String>, Option<u32>) {
    let weakness = parse_wrr_field(text, "Weakness");
    let retreat_str = parse_wrr_field(text, "Retreat");
    let retreat_cost = retreat_str
        .as_deref()
        .and_then(|s| s.split_whitespace().next())
        .and_then(|s| s.parse::<u32>().ok());
    (weakness, retreat_cost)
}

fn parse_wrr_field(text: &str, key: &str) -> Option<String> {
    let search = format!("{key}:");
    let start = text.find(&search)? + search.len();
    let remainder = text[start..].trim();
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

/// Render card text, substituting energy symbol spans with [ElementName].
fn render_card_text(el: scraper::ElementRef) -> String {
    let mut out = String::new();
    for node in el.children() {
        if let Some(child) = scraper::ElementRef::wrap(node) {
            let tag = child.value().name();
            if tag == "br" {
                out.push(' ');
                continue;
            }
            if child.value().classes().any(|c| c == "copy-only") {
                continue;
            }
            if let Some(tip) = child.value().attr("data-tooltip") {
                out.push('[');
                out.push_str(tip);
                out.push(']');
            } else {
                out.push_str(&child.text().collect::<String>());
            }
        } else if let Some(text) = node.value().as_text() {
            out.push_str(text);
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse_attack(el: scraper::ElementRef) -> Option<Attack> {
    let info_sel = sel!("p.card-text-attack-info");
    let sym_sel = sel!("span.ptcg-symbol");
    let effect_sel = sel!("p.card-text-attack-effect");

    let info = el.select(&info_sel).next()?;

    let cost: Vec<String> = info
        .select(&sym_sel)
        .next()
        .map(|s| {
            s.text()
                .collect::<String>()
                .chars()
                .filter_map(|c| letter_to_element(&c.to_string()).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let symbol_text = info
        .select(&sym_sel)
        .next()
        .map(|s| s.text().collect::<String>())
        .unwrap_or_default();
    let info_text = info.text().collect::<String>();
    let name_dmg = match info_text.find(&symbol_text) {
        Some(pos) => info_text[pos + symbol_text.len()..].trim(),
        None => info_text.trim(),
    };

    let tokens: Vec<&str> = name_dmg.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }

    let (name, damage_raw) = match tokens.split_last() {
        Some((last, rest)) if last.chars().any(|c| c.is_ascii_digit()) => {
            (rest.join(" "), last.to_string())
        }
        _ => (tokens.join(" "), String::new()),
    };

    if name.is_empty() {
        warn!("could not extract attack name from: {name_dmg:?}");
        return None;
    }

    let (damage, damage_suffix) = parse_damage(&damage_raw);

    let effect = el
        .select(&effect_sel)
        .next()
        .map(|e| e.text().collect::<String>().trim().to_string())
        .filter(|s| !s.is_empty());

    Some(Attack {
        name,
        cost,
        damage,
        damage_suffix,
        effect,
    })
}

fn parse_ability(el: scraper::ElementRef) -> Option<Ability> {
    let info_sel = sel!("p.card-text-ability-info");
    let effect_sel = sel!("p.card-text-ability-effect");

    let name = el
        .select(&info_sel)
        .next()
        .map(|e| {
            let text = e.text().collect::<String>();
            text.split(':').nth(1).unwrap_or(&text).trim().to_string()
        })
        .filter(|s| !s.is_empty())?;

    let effect = el
        .select(&effect_sel)
        .next()
        .map(|e| render_card_text(e))
        .unwrap_or_default();

    Some(Ability { name, effect })
}

// ── Energy / symbol helpers ──────────────────────────────────────────────────

fn letter_to_element(s: &str) -> Option<&'static str> {
    // Single-letter energy codes are part of the PTCGP card encoding used by
    // Limitless. This mapping must be maintained manually if new types are added.
    let element = match s.to_uppercase().as_str() {
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
    };
    if element.is_none() && !s.is_empty() {
        warn!(
            letter = s,
            "unknown energy symbol — add to letter_to_element in limitless.rs"
        );
    }
    element
}

fn parse_damage(raw: &str) -> (u32, Option<String>) {
    if raw.is_empty() {
        return (0, None);
    }
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    let damage = digits.parse::<u32>().unwrap_or(0);
    let suffix = raw
        .chars()
        .find(|c| !c.is_ascii_digit() && !c.is_whitespace())
        .map(|c| c.to_string());
    (damage, suffix)
}

// ── Date parsing ─────────────────────────────────────────────────────────────

/// Parse "30 Oct 24" → Some("2024-10-30"), or already-formatted dates pass through.
pub fn parse_release_date(s: &str) -> Option<String> {
    let s = s.trim();
    // Already in YYYY-MM-DD format
    if s.len() == 10 && s.chars().nth(4) == Some('-') {
        return Some(s.to_string());
    }
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() != 3 {
        return None;
    }
    let day: u32 = parts[0].parse().ok()?;
    let month = match parts[1].to_lowercase().as_str() {
        "jan" => 1u32,
        "feb" => 2,
        "mar" => 3,
        "apr" => 4,
        "may" => 5,
        "jun" => 6,
        "jul" => 7,
        "aug" => 8,
        "sep" => 9,
        "oct" => 10,
        "nov" => 11,
        "dec" => 12,
        _ => return None,
    };
    let year_short: u32 = parts[2].parse().ok()?;
    let year = if year_short < 100 {
        2000 + year_short
    } else {
        year_short
    };
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

// ── Misc helpers ─────────────────────────────────────────────────────────────

fn parse_series(code: &str) -> String {
    if code.starts_with("P-") {
        return code
            .trim_start_matches("P-")
            .chars()
            .next()
            .unwrap_or('A')
            .to_string();
    }
    code.chars()
        .next()
        .map(|c| c.to_string())
        .unwrap_or_default()
}
