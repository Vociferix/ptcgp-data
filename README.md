# ptcgp-data

> **Disclaimer:** The literal and graphical information presented in this repository about Pokémon Trading Card Game Pocket, including card data and text, is copyright The Pokémon Company, DeNA Co., Ltd., and/or Creatures, Inc. This repository is not produced by, endorsed by, supported by, or affiliated with any of those copyright holders.

Community-maintained data for Pokémon TCG Pocket, available in two formats:

- **JSON** — human-readable, diff-friendly, accessible directly via `raw.githubusercontent.com`
- **SQLite** — pre-built database published as a [GitHub Release](https://github.com/Vociferix/ptcgp-data/releases/latest) asset (`ptcgp.db`)

Card images are in a companion repository: [Vociferix/ptcgp-images](https://github.com/Vociferix/ptcgp-images)

---

## JSON Data

All JSON lives under `data/`. Files are UTF-8 and use consistent naming conventions throughout.

### Reference tables

These files define the fixed vocabulary used across the rest of the data.

#### `data/sets.json`

Array of every set, ordered newest-first.

```json
[
  {
    "code": "B3",
    "name": "Pulsing Aura",
    "series": "B",
    "release_date": "2026-04-28",
    "is_promo": false,
    "card_count": 234
  }
]
```

| Field | Description |
|---|---|
| `code` | Short identifier used in paths throughout the data (`A1`, `B2a`, `P-A`, …) |
| `series` | Series letter (`A` or `B`). Promo sets belong to the series of their release period |
| `release_date` | ISO 8601 date, `null` for promo sets |
| `is_promo` | `true` for promo sets (`P-A`, `P-B`, …) |
| `card_count` | Total number of cards in the set, `null` if unknown |

#### `data/rarities.json`

All rarity tiers, ordered from most common to rarest.

```json
[
  {
    "code": "C",
    "name": "Common",
    "group": "Diamond",
    "group_symbol_count": 1,
    "craft_cost": 35,
    "dupe_dust": 10
  }
]
```

Rarity codes: `C`, `U`, `R`, `RR`, `AR`, `SR`, `SAR`, `IM`, `S`, `SSR`, `UR`

`craft_cost` and `dupe_dust` are Wonder Pick point values. Both are `null` for rarities where crafting/duplicates don't apply.

#### `data/elements.json`

Energy types. Keys are element names; values include an optional single-letter `symbol` (Dragon has no symbol) and a `name`.

```json
[
  { "symbol": "G", "name": "Grass" },
  { "symbol": null, "name": "Dragon" }
]
```

#### `data/base_pokemon.json`

Pokédex reference. Array of objects with `natdex_number` and `name`.

#### `data/promo_sources.json`

How promo cards can be obtained. Each entry has a `code` and optional `description`.

#### `data/pack_variant_names.json`

Display names for pack variant codes (`normal`, `rare`, `plus1`, `themed`).

---

### Sets

#### `data/sets/{SET}/set.json`

Full detail for one set, including the list of pack subtitles.

```json
{
  "code": "A1",
  "name": "Genetic Apex",
  "series": "A",
  "release_date": "2024-10-30",
  "is_promo": false,
  "card_count": 286,
  "packs": ["Mewtwo", "Charizard", "Pikachu"]
}
```

`packs` is absent for promo sets, which have no booster packs.

---

### Cards

Cards are split into two layers:

- **Abstract card** (`data/cards/{ID:05}.json`) — game mechanics shared by all art/rarity variants of the same Pokémon or Trainer card
- **Card version** (`data/sets/{SET}/cards/{NUM:03}.json`) — one specific physical card: a set, collector number, rarity, illustrator, and finish

#### `data/cards/{ID:05}.json`

```json
{
  "id": 1,
  "name": "Bulbasaur",
  "card_type": "pokemon",
  "natdex_number": 1,
  "element": "Grass",
  "stage": "Basic",
  "hp": 70,
  "retreat_cost": 1,
  "weakness": "Fire",
  "flavor": "There is a plant seed on its back right from the day this Pokémon is born.",
  "is_ex": false,
  "is_mega": false,
  "attacks": [
    {
      "name": "Vine Whip",
      "cost": ["Grass", "Colorless"],
      "damage": 40,
      "damage_suffix": null,
      "effect": null
    }
  ],
  "evolves_from": "Kakuna",
  "ability": null,
  "versions": [
    { "set": "A1", "number": 1 },
    { "set": "A4b", "number": 1 }
  ]
}
```

Trainer cards omit Pokémon fields and instead have `trainer_kind` and `trainer_effect`. Fields that don't apply to a card are omitted rather than set to `null`.

The `versions` array lists every card version that shares this abstract card's mechanics.

#### `data/sets/{SET}/cards/{NUM:03}.json`

```json
{
  "set": "A1",
  "number": 1,
  "card_id": 1,
  "rarity": "C",
  "illustrator": "Narumi Sato",
  "is_promo": false,
  "is_foil": false,
  "is_reprint": false,
  "packs": ["Mewtwo", "ex"],
  "promo_sources": [],
  "duplicates": [
    { "set": "A4b", "number": 1 }
  ]
}
```

| Field | Description |
|---|---|
| `card_id` | References the abstract card in `data/cards/` |
| `rarity` | Rarity code from `rarities.json` |
| `is_promo` | `true` when the card carries a promo stamp |
| `is_foil` | `true` when the card has a mirror/foil finish |
| `is_reprint` | `true` when an identical version was released in an earlier set |
| `packs` | Pack subtitles this version can be pulled from (empty for promo-only cards) |
| `promo_sources` | Acquisition methods from `promo_sources.json` (empty for non-promo cards) |
| `duplicates` | Other versions with the same rarity, illustrator, and finish (i.e. the same physical card released again) |

---

### Pull rates

#### `data/pull_rates/{SET}/{subtitle}.json`

Pull rate data for one pack. Rates are stored as exact integer fractions to avoid floating-point rounding errors. All rates for a given scope share a common LCM denominator stored separately, so numerators can be compared directly as integers.

```json
{
  "set": "A1",
  "subtitle": "Charizard",
  "variants": {
    "normal": {
      "rate": { "numerator": 1999, "denominator": 2000 },
      "slot_count": 5,
      "rarity_rates_by_slot": [
        {
          "C": { "normal": { "numerator": 1, "denominator": 1 } }
        },
        "..."
      ],
      "card_rates": {
        "1": [
          { "numerator": 1, "denominator": 33 },
          { "numerator": 1, "denominator": 33 },
          { "numerator": 1, "denominator": 33 },
          null,
          null
        ]
      }
    }
  }
}
```

**`variants`** maps a variant code to its data. Variant codes are defined in `pack_variant_names.json`. Most packs have `normal`, `rare`, and `plus1`. Some packs (e.g. B2b Mega Shine) also have `themed`.

| Variant | Probability | Cards |
|---|---|---|
| `normal` | ~94–99.95% | 5 cards |
| `plus1` | ~5–8% | 6 cards (bonus slot) |
| `rare` | 0.05% | 5 star-rarity cards |
| `themed` | 0.005% | 5 guaranteed featured cards |

**`rate`** — probability of opening this variant as an exact fraction `{ numerator, denominator }`. The numerators across all variants for a pack sum to the denominator (i.e. the rates sum to 1).

**`rarity_rates_by_slot`** — array with one entry per card slot. Each entry maps a rarity code to a `{ normal, foil }` pair. A rarity with only non-foil cards has only a `normal` sub-object; a rarity where all cards have a foil finish has only a `foil` sub-object; a rarity with a mix has both. Each sub-object is an exact fraction `{ numerator, denominator }`.

**`card_rates`** — maps a card number (as a string) to an array of per-slot rates, one element per slot. `null` means the card cannot appear in that slot. The denominator for card rates in each slot is the LCM of all rate denominators in that slot (see the database for the stored LCM values).

Promo sets have no pull rate files.

---

## SQLite Database

The pre-built `ptcgp.db` is published with each [release](https://github.com/Vociferix/ptcgp-data/releases/latest). It contains the same data as the JSON files in a fully relational schema with convenience views.

### Key tables

| Table | Description |
|---|---|
| `sets` | All sets with codes and series |
| `cards` | Abstract cards (one row per unique card identity) |
| `card_versions` | Physical cards: set, number, rarity, finish |
| `packs` | One row per pack subtitle per set |
| `pack_variants` | Pack opening variant (normal/rare/plus1/themed) with rate numerator |
| `pack_variant_rate_denominators` | LCM denominator shared by all variant rates for a pack |
| `pack_slots` | Individual card slots within a pack variant, with slot rate denominator |
| `rarity_pull_rates` | Per-slot rarity probabilities, split by foil vs non-foil finish |
| `card_pull_rates` | Per-slot per-card probabilities |
| `rarities` | Rarity codes, names, groups, craft costs |
| `elements` | Energy types |

### Convenience views

| View | Description |
|---|---|
| `versions` | Flat join of card_versions with set, card name, rarity, illustrator, and finish flags |
| `pokemon` | All Pokémon abstract cards with stats, attacks, and ability |
| `trainers` | All Trainer abstract cards with kind and effect |
| `rarity_overview` | Rarities with group name, symbol count, craft cost, and dupe dust |
| `card_version_images` | Card version ID → image path (see [ptcgp-images](https://github.com/Vociferix/ptcgp-images)) |
| `pack_art` | Pack ID → pack art image path |
| `pack_logos` | Pack ID → pack logo image path |
| `set_icons` | Set ID → set icon image path |
| `set_logos` | Set ID → set logo image path |
| `rarity_icons` | Rarity ID → rarity icon image path |
| `element_icons` | Element ID → element icon image path |
| `promo_source_icons` | Promo source ID → icon image path |

### Pull rate fractions

Rates are stored as integer fractions using a shared LCM denominator per scope:

- **Pack variant rates**: all variants for a pack share a denominator stored in `pack_variant_rate_denominators`. Dividing `pack_variants.rate_numerator` by that denominator gives the probability.
- **Slot rates**: each slot in `pack_slots` has its own `rate_denominator`. Dividing any `rarity_pull_rates.rate_numerator` or `card_pull_rates.rate_numerator` for that slot by the slot's denominator gives the probability.

Example query — pack variant probabilities as percentages:

```sql
SELECT
    s.code AS set,
    ps.subtitle AS pack,
    pvc.code AS variant,
    ROUND(100.0 * pv.rate_numerator / pvrd.rate_denominator, 4) AS pct
FROM pack_variants pv
JOIN pack_variant_kinds pvk ON pvk.id = pv.kind_id
JOIN pack_variant_codes pvc ON pvc.id = pvk.code_id
JOIN packs p ON p.id = pv.pack_id
JOIN pack_subtitles ps ON ps.id = p.subtitle_id
JOIN sets s ON s.id = p.set_id
JOIN pack_variant_rate_denominators pvrd ON pvrd.pack_id = p.id
ORDER BY s.code, ps.subtitle, pvc.code;
```

---

## Generating the data

### Prerequisites

- [Rust](https://rustup.rs/) (stable toolchain)

### Build the database

```sh
cargo run --release --bin db
```

This reads all JSON from `data/` and writes `ptcgp.db` to the current directory.

### Verify pull rates

```sh
cargo run --release --bin check-pull-rates
```

Confirms that every pull rate in the database round-trips exactly back to the source JSON fractions.

### Refresh pull rate JSON

Pull rate data is scraped from [RaenonX](https://ptcgp.raenonx.cc/). To update all packs:

```sh
cargo run --release --bin scraper -- pull-rates
```

To refresh a single pack by its RaenonX pack ID:

```sh
cargo run --release --bin scraper -- pull-rates --pack <PACK_ID> --force
```
