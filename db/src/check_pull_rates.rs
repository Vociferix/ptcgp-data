use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use rust_decimal::Decimal;
use rusqlite::{params, Connection};
use serde::Deserialize;

#[derive(Parser)]
#[command(
    name = "check-pull-rates",
    about = "Verify that DB pull rate fractions exactly round-trip back to the source JSON values"
)]
struct Cli {
    #[arg(long, default_value = "data")]
    data: PathBuf,

    #[arg(long, default_value = "ptcgp.db")]
    output: PathBuf,
}

#[derive(Deserialize)]
struct PackPullRates {
    #[allow(dead_code)]
    set: String,
    subtitle: String,
    variants: HashMap<String, Option<PackVariantRates>>,
}

#[derive(Deserialize)]
struct Rate {
    numerator: Decimal,
    denominator: Decimal,
}

#[derive(Deserialize)]
struct PackVariantRates {
    rate_numerator: Decimal,
    rate_denominator: Decimal,
    slot_count: u32,
    rarity_rates_by_slot: Vec<HashMap<String, Rate>>,
    card_rates: HashMap<String, Vec<Option<Rate>>>,
}

fn gcd(a: i64, b: i64) -> i64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

fn lcm(a: i64, b: i64) -> i64 {
    a / gcd(a, b) * b
}

fn rate_to_integers(rate: &Rate) -> (i64, i64) {
    let scale = rate.numerator.scale().max(rate.denominator.scale());
    let factor = Decimal::from(10u64.pow(scale));
    let n: i64 = (rate.numerator * factor)
        .try_into()
        .expect("rate numerator overflows i64");
    let d: i64 = (rate.denominator * factor)
        .try_into()
        .expect("rate denominator overflows i64");
    let g = gcd(n, d);
    (n / g, d / g)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let conn = Connection::open(&cli.output)
        .with_context(|| format!("opening {:?}", cli.output))?;

    let mut mismatches: Vec<String> = Vec::new();
    let mut checked = 0usize;

    let pull_rates_dir = cli.data.join("pull_rates");
    if !pull_rates_dir.exists() {
        anyhow::bail!("pull_rates/ directory not found under {:?}", cli.data);
    }

    let mut set_dirs: Vec<_> = std::fs::read_dir(&pull_rates_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .collect();
    set_dirs.sort_by_key(|e| e.path());

    for set_dir in set_dirs {
        let set_code = set_dir.file_name().to_string_lossy().into_owned();

        let set_id: i64 = match conn.query_row(
            "SELECT id FROM sets WHERE code = ?1",
            params![set_code],
            |row| row.get(0),
        ) {
            Ok(id) => id,
            Err(_) => {
                mismatches.push(format!("{set_code}: set not found in DB"));
                continue;
            }
        };

        let mut rate_files: Vec<_> = std::fs::read_dir(set_dir.path())?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .collect();
        rate_files.sort_by_key(|e| e.path());

        for rate_file in rate_files {
            let path = rate_file.path();
            let text = std::fs::read_to_string(&path)?;
            let rates: PackPullRates = serde_json::from_str(&text)
                .with_context(|| format!("parsing {}", path.display()))?;

            let file_prefix = format!("{set_code}/{}", rates.subtitle);

            let pack_id: i64 = match conn.query_row(
                "SELECT p.id FROM packs p \
                 JOIN pack_subtitles ps ON ps.id = p.subtitle_id \
                 WHERE p.set_id = ?1 AND ps.subtitle = ?2",
                params![set_id, rates.subtitle],
                |row| row.get(0),
            ) {
                Ok(id) => id,
                Err(_) => {
                    mismatches.push(format!("{file_prefix}: pack not found in DB"));
                    continue;
                }
            };

            // Compute the expected LCM denominator for this pack's variant rates.
            let expected_pack_denom: i64 = rates
                .variants
                .values()
                .flatten()
                .map(|v| {
                    rate_to_integers(&Rate {
                        numerator: v.rate_numerator,
                        denominator: v.rate_denominator,
                    })
                    .1
                })
                .fold(1i64, lcm);

            // Verify the stored pack denominator.
            match conn.query_row::<i64, _, _>(
                "SELECT rate_denominator FROM pack_variant_rate_denominators WHERE pack_id = ?1",
                params![pack_id],
                |row| row.get(0),
            ) {
                Ok(db_denom) => {
                    if db_denom != expected_pack_denom {
                        mismatches.push(format!(
                            "{file_prefix}: rate_denominator mismatch — \
                             db={db_denom} expected={expected_pack_denom}"
                        ));
                    }
                    checked += 1;
                }
                Err(_) => {
                    if rates.variants.values().any(|v| v.is_some()) {
                        mismatches.push(format!(
                            "{file_prefix}: rate_denominator row missing from DB"
                        ));
                    }
                }
            }

            for (variant_code, maybe_variant) in &rates.variants {
                let Some(variant) = maybe_variant else { continue };
                let variant_prefix = format!("{file_prefix}/{variant_code}");

                // Look up the pack_variant row.
                let (db_variant_id, db_rate_num): (i64, i64) = match conn.query_row(
                    "SELECT pv.id, pv.rate_numerator \
                     FROM pack_variants pv \
                     JOIN pack_variant_kinds pvk ON pvk.id = pv.kind_id \
                     JOIN pack_variant_codes pvc ON pvc.id = pvk.code_id \
                     WHERE pvc.code = ?1 AND pv.pack_id = ?2",
                    params![variant_code, pack_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                ) {
                    Ok(r) => r,
                    Err(_) => {
                        mismatches.push(format!("{variant_prefix}: pack_variant row missing"));
                        continue;
                    }
                };

                // Verify rate_numerator scaled to the pack's LCM denominator.
                let variant_rate = Rate {
                    numerator: variant.rate_numerator,
                    denominator: variant.rate_denominator,
                };
                let (vn, vd) = rate_to_integers(&variant_rate);
                let expected_rate_num = vn * (expected_pack_denom / vd);
                if db_rate_num != expected_rate_num {
                    mismatches.push(format!(
                        "{variant_prefix}: rate_numerator mismatch — \
                         db={db_rate_num} expected={expected_rate_num} \
                         (json={}/{}, pack_denom={expected_pack_denom})",
                        variant.rate_numerator, variant.rate_denominator
                    ));
                }
                checked += 1;

                for slot_idx in 0..variant.slot_count {
                    let slot_prefix = format!("{variant_prefix}/slot{slot_idx}");

                    // Compute the expected LCM denominator for this slot.
                    let mut expected_slot_denom = 1i64;
                    if let Some(rarity_rates) =
                        variant.rarity_rates_by_slot.get(slot_idx as usize)
                    {
                        for rate in rarity_rates.values() {
                            let (_, d) = rate_to_integers(rate);
                            expected_slot_denom = lcm(expected_slot_denom, d);
                        }
                    }
                    for slot_rates in variant.card_rates.values() {
                        if let Some(Some(rate)) = slot_rates.get(slot_idx as usize) {
                            let (_, d) = rate_to_integers(rate);
                            expected_slot_denom = lcm(expected_slot_denom, d);
                        }
                    }

                    let (slot_id, db_slot_denom): (i64, i64) = match conn.query_row(
                        "SELECT id, rate_denominator FROM pack_slots \
                         WHERE pack_variant_id = ?1 AND pull_number = ?2",
                        params![db_variant_id, slot_idx],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    ) {
                        Ok(r) => r,
                        Err(_) => {
                            mismatches.push(format!("{slot_prefix}: slot row missing"));
                            continue;
                        }
                    };

                    if db_slot_denom != expected_slot_denom {
                        mismatches.push(format!(
                            "{slot_prefix}: slot denominator mismatch — \
                             db={db_slot_denom} expected={expected_slot_denom}"
                        ));
                    }
                    checked += 1;

                    // Rarity rates for this slot.
                    if let Some(rarity_rates) =
                        variant.rarity_rates_by_slot.get(slot_idx as usize)
                    {
                        for (rarity_code, rate) in rarity_rates {
                            let (rn, rd) = rate_to_integers(rate);
                            let expected_num = rn * (expected_slot_denom / rd);
                            match conn.query_row::<i64, _, _>(
                                "SELECT rpr.rate_numerator \
                                 FROM rarity_pull_rates rpr \
                                 JOIN rarities r ON r.id = rpr.rarity_id \
                                 WHERE rpr.slot_id = ?1 AND r.code = ?2",
                                params![slot_id, rarity_code],
                                |row| row.get(0),
                            ) {
                                Ok(db_num) => {
                                    if db_num != expected_num {
                                        mismatches.push(format!(
                                            "{slot_prefix}/rarity={rarity_code}: \
                                             numerator mismatch — db={db_num} \
                                             expected={expected_num} \
                                             (json={}/{})",
                                            rate.numerator, rate.denominator
                                        ));
                                    }
                                    checked += 1;
                                }
                                Err(_) => mismatches.push(format!(
                                    "{slot_prefix}/rarity={rarity_code}: row missing"
                                )),
                            }
                        }
                    }

                    // Card rates for this slot.
                    for (card_key, slot_rates) in &variant.card_rates {
                        let Some(Some(rate)) = slot_rates.get(slot_idx as usize) else {
                            continue;
                        };
                        let card_num: u32 = match card_key.parse() {
                            Ok(n) => n,
                            Err(_) => {
                                mismatches.push(format!(
                                    "{slot_prefix}: invalid card key {card_key:?}"
                                ));
                                continue;
                            }
                        };
                        let (cn, cd) = rate_to_integers(rate);
                        let expected_num = cn * (expected_slot_denom / cd);
                        match conn.query_row::<i64, _, _>(
                            "SELECT cpr.rate_numerator \
                             FROM card_pull_rates cpr \
                             JOIN card_versions cv ON cv.id = cpr.card_version_id \
                             WHERE cpr.slot_id = ?1 \
                               AND cv.set_id = ?2 \
                               AND cv.number = ?3",
                            params![slot_id, set_id, card_num],
                            |row| row.get(0),
                        ) {
                            Ok(db_num) => {
                                if db_num != expected_num {
                                    mismatches.push(format!(
                                        "{slot_prefix}/card={card_num:03}: \
                                         numerator mismatch — db={db_num} \
                                         expected={expected_num} \
                                         (json={}/{})",
                                        rate.numerator, rate.denominator
                                    ));
                                }
                                checked += 1;
                            }
                            Err(_) => mismatches.push(format!(
                                "{slot_prefix}/card={card_num:03}: row missing"
                            )),
                        }
                    }
                }
            }
        }
    }

    if mismatches.is_empty() {
        println!("OK — {checked} values checked, all match");
    } else {
        for m in &mismatches {
            eprintln!("FAIL: {m}");
        }
        eprintln!("\n{} mismatch(es) out of {checked} checked values", mismatches.len());
        std::process::exit(1);
    }

    Ok(())
}
