# PTCGP Collection Tracker — Project Handoff

## Project Goal

Build a **Pokemon TCG Pocket collection tracker** app. Key requirements:
- Free, open source, no ads
- Personal use first, but publicly available
- Cross-platform: **web, desktop, Android, iOS**
- Written in **Rust** (developer is an expert backend Rust dev, limited frontend experience)

## Technology Stack

### UI Framework: Dioxus
- Single Rust codebase targeting web (WASM), desktop, Android, iOS
- React-inspired API — declarative, component-based
- Supports Tailwind CSS for styling
- `dx serve --platform android` for mobile dev
- https://dioxuslabs.com/

### Why not Tauri?
Tauri requires a separate JS/TS frontend. Dioxus is pure Rust end-to-end, which fits the developer's background.

---

## Data Sources

### 1. Card Data — Limitless TCG (Primary)

**Base URL:** `https://pocket.limitlesstcg.com`

Fully server-rendered HTML (no JS execution needed). Plain `reqwest` + HTML parsing with the `scraper` crate.

**Scraping entry points:**

```
GET /cards                    → list of all sets
GET /cards/{SET}              → card number list for a set + pack names
GET /cards/{SET}/{NUM}        → full card details
```

**What each page provides:**

`/cards` — Set index:
- All set codes (A1, A1a, A2, B1, B3, P-A, etc.)
- Set names and release dates
- Card counts per set
- Set icon images: `https://s3.limitlesstcg.com/pocket/sets/{SET}.webp`

`/cards/{SET}` — Set listing:
- Pack names (from `div.pack-selection` buttons: e.g. "Charizard pack", "Mewtwo pack", "Pikachu pack", "Shared")
- All card numbers in the set (from `a[href]` links in `.card-search-grid`)
- No pull rates — this page is just a navigation grid

`/cards/{SET}/{NUM}` — Individual card page (example: `/cards/A1/1` = Bulbasaur):
- Name, type, HP: `p.card-text-title` — e.g. "Bulbasaur - Grass - 70 HP"
- Stage: `p.card-text-type` — e.g. "Pokémon - Basic"
- Attacks: `div.card-text-attack` — cost in `span.ptcg-symbol`, name + damage in text
- Weakness and retreat: `p.card-text-wrr`
- Flavor text: `div.card-text-flavor`
- Artist: `div.card-text-artist`
- Set + rarity + pack membership: `div.card-prints-current` — e.g. "#1 · ◊ · Mewtwo pack"
- All prints/variants of the card: `table.card-prints-versions`
- Thumbnail image (webp): `img.card` src attribute
- Full image (png): `img.card` data-src attribute

**Image CDN URL pattern (constructable without scraping):**
```
# Thumbnail
https://limitlesstcg.nyc3.cdn.digitaloceanspaces.com/pocket/{SET}/{SET}_{NUM}_EN_SM.webp

# Full resolution
https://limitlesstcg.nyc3.cdn.digitaloceanspaces.com/pocket/{SET}/{SET}_{NUM}_EN.png
```
Number is zero-padded to 3 digits: `001`, `002`, ... `286`

**Set icon images:**
```
https://s3.limitlesstcg.com/pocket/sets/{SET}.webp
```

**robots.txt:** Fully permissive (`User-Agent: * / Disallow:` — empty)

**Important:** Limitless has NO public card data API. The tournament platform has an API (`play.limitlesstcg.com/api`) but it's for tournament data only, not card data. Everything is HTML scraping.

---

### 2. Pull Rates — RaenonX (Primary)

**Base URL:** `https://ptcgp.raenonx.cc`

RaenonX does APK/Unity asset extraction from the game itself. Their data is authoritative and updated on set release day. Two endpoints needed:

#### Endpoint A: Global Master Data
```
GET https://ptcgp.raenonx.cc/api/data/global-master
```
Returns JSON directly (no parsing tricks needed). Contains:

- **`cardPackMap`** (48 entries): All pack IDs mapped to expansion IDs, card lists, and metadata
  ```json
  "BN006_0010_00_000": {
    "id": "BN006_0010_00_000",
    "expansionId": "B3",
    "cards": {
      "highlight": ["PK_10_016660_00", ...],
      "available": ["PK_10_016660_00", ...]
    },
    "isRegular": true,
    "packsOfSameExpansion": ["BN006_0010_00_000"],
    ...
  }
  ```
- **`cardEntryMap`** (3074 entries): Full card data including HP, attacks, energy costs, evolution chains, weakness, retreat, rarity, which packs it appears in
- **`cardExpansionMap`** (19 entries): All expansions
- **`cardPackPointMap`**: Craft costs per rarity (C=35, U=70, R=150, RR=500, SR=1250, IM=1500, UR=2500, etc.)
- **`cardDupeShineDustMap`**: Dupe dust values per rarity

**Pack ID naming convention:**
- `AN001` = A series expansion 1 (Genetic Apex)
- `BN006` = B series expansion 6 (Pulsing Aura)
- `_0010` = pack slot 1, `_0020` = pack slot 2, `_0030` = pack slot 3
- `_00_000` = regular pack, `_01_000` = god pack variant
- `AP001` = A series promo, `BP001` = B series promo

Use `isRegular: true` to filter to standard openable packs.

#### Endpoint B: Pack Pull Rates
```
GET https://ptcgp.raenonx.cc/en/pack/{packId}
```
This is a Next.js App Router page using React Server Components with streaming. The pull rate data is embedded in the streamed HTML as `self.__next_f.push([1,"..."])` chunks — NOT in a `__NEXT_DATA__` script tag. Requires parsing the RSC wire format.

**Extraction strategy:**
1. Fetch the full page HTML
2. Find all `self.__next_f.push([1,"..."])` chunks via regex
3. Find the chunk containing `cardPullProbabilityMap`
4. Unescape the chunk content (it's a JSON-encoded string: `chunk.encode('utf-8').decode('unicode_escape')`)
5. Use `json.JSONDecoder().raw_decode(text, offset)` to extract values at specific positions — do NOT use `json.loads()` on the full chunk as it contains concatenated RSC objects, not a single JSON document

**Two data structures extracted:**

`cardPullProbabilityMap` — per-card rates:
```json
"PK_10_016660_00": {
  "cardId": "PK_10_016660_00",
  "byPack": {
    "BN006_0010_00_000": {
      "packId": "BN006_0010_00_000",
      "cardProbability": {
        "normal": [
          {"numerator": 0.01538, "denominator": 1},  // slot 1
          {"numerator": 0.01538, "denominator": 1},  // slot 2
          {"numerator": 0.01538, "denominator": 1},  // slot 3
          null,                                        // slot 4 (can't appear)
          null                                         // slot 5 (can't appear)
        ],
        "rare": [null, null, null, null, null],        // god pack
        "plus1": [...]                                  // premium +1 pack
      }
    }
  }
}
```

`packPullProbabilityData` — aggregate rarity rates:
```json
{
  "packId": "BN006_0010_00_000",
  "byPackType": {
    "normal":  {"numerator": 947119,  "denominator": 1000000},   // ~94.7%
    "rare":    {"numerator": 5,       "denominator": 10000},      // 0.05% god pack
    "plus1":   {"numerator": 52381,   "denominator": 1000000}    // ~5.2% premium
  },
  "byRarity": {
    "normal": [
      {"C": {"numerator": 1, "denominator": 1}},        // slot 1: always common
      {"C": {"numerator": 1, "denominator": 1}},        // slot 2: always common
      {"C": {"numerator": 1, "denominator": 1}},        // slot 3: always common
      {"AR": {...}, "R": {...}, "U": {...}, ...},        // slot 4: rare slot
      {"AR": {...}, "R": {...}, "U": {...}, ...}         // slot 5: rare slot
    ],
    "rare": [...],   // god pack: all 5 slots are rare
    "plus1": [...]   // premium: 6 slots, last slot is shiny-only
  },
  "cardCount": {"normal": 5, "rare": 5, "plus1": 6}
}
```

All probabilities are exact fractions. Some denominators are large (up to 10^13) for precision.

**Working Python extraction script:** See `fetch_pull_rates.py` (included in this handoff). Key insight: use `json.JSONDecoder().raw_decode()` not `json.loads()`.

---

### What Pull Rates Are NOT Available From Limitless

Limitless TCG does NOT expose pull rates anywhere on their site. The individual card page and set listing page contain no rate information. Pull rates must come from RaenonX.

---

## Complete Data Pipeline

| Data | Source URL | Method |
|---|---|---|
| All set codes + metadata | `pocket.limitlesstcg.com/cards` | HTML scrape |
| Pack names per set | `pocket.limitlesstcg.com/cards/{SET}` | HTML scrape |
| Card number list per set | `pocket.limitlesstcg.com/cards/{SET}` | HTML scrape |
| Full card details | `pocket.limitlesstcg.com/cards/{SET}/{NUM}` | HTML scrape |
| Card images | CDN (constructable from set+number) | Direct download |
| Set icon images | CDN (constructable from set code) | Direct download |
| All pack IDs + expansion mapping | `ptcgp.raenonx.cc/api/data/global-master` | JSON API |
| Card stats (HP, attacks, etc.) | `ptcgp.raenonx.cc/api/data/global-master` | JSON API |
| Per-card pull rates | `ptcgp.raenonx.cc/en/pack/{packId}` | RSC HTML parse |
| Aggregate rarity rates | `ptcgp.raenonx.cc/en/pack/{packId}` | RSC HTML parse |

**Bootstrap sequence for scraper:**
1. Fetch `global-master` → get all pack IDs (filter `isRegular: true`)
2. Fetch `/cards` → get all set codes
3. For each set, fetch `/cards/{SET}` → get card number list and pack names
4. For each card, fetch `/cards/{SET}/{NUM}` → get card details (can be parallelised heavily)
5. For each regular pack, fetch `/en/pack/{packId}` → extract pull rates
6. Download images from CDN as needed

**Scale:** ~2000+ individual card pages across all sets as of May 2026. New sets release roughly monthly.

**Rate limiting:** Be polite. Add delays between requests, use a descriptive User-Agent. Neither site has robots.txt restrictions, but hammering them would be bad citizenship.

---

## Rarity Codes (from global-master)

| Code | Meaning | Craft Cost | Dupe Dust |
|---|---|---|---|
| C | Common (◊) | 35 | 10 |
| U | Uncommon (◊◊) | 70 | 20 |
| R | Rare (◊◊◊) | 150 | 100 |
| RR | Double Rare / ex (◊◊◊◊) | 500 | 300 |
| AR | Art Rare (☆) | 400 | 210 |
| SR | Super Rare (☆☆) | 1250 | 870 |
| SAR | Special Art Rare (☆☆) | 1250 | 870 |
| IM | Immersive (☆☆☆) | 1500 | 1760 |
| S | Shiny Rare | — | — |
| SSR | Shiny Super Rare | — | — |
| UR | Crown Rare (♛) | 2500 | 3600 |

---

## Key Technical Notes

### Limitless HTML Structure (CSS selectors for scraper)

Set index page (`/cards`):
- `table.sets-table tr td a` — each row is a set; href gives set code
- `img.set` — set icon image
- `span.code.annotation` — set code text

Set listing page (`/cards/{SET}`):
- `div.pack-selection button[data-value]` — pack names (data-value is internal pack name, text content is display name)
- `div.card-search-grid a[href]` — card links; parse number from href

Individual card page (`/cards/{SET}/{NUM}`):
- `<!-- CARD ID N -->` and `<!-- DATA ID N -->` HTML comments at top of main section
- `section.card-page-main` — main card content container
- `p.card-text-title` — "Name - Type - HP HP"
- `p.card-text-type` — "Pokémon - Basic" or "Pokémon - Stage 1" etc.
- `div.card-text-attack` — one per attack; `span.ptcg-symbol` for energy cost
- `p.card-text-wrr` — weakness and retreat (text parsing needed)
- `div.card-text-flavor` — flavor text
- `div.card-text-artist a` — artist name
- `div.card-prints-current span:last-child` — "#N · ◊ · Pack name"
- `table.card-prints-versions tr` — all prints of this card
- `img.card[src]` — thumbnail webp URL
- `img.card[data-src]` — full resolution png URL

### RaenonX RSC Parsing (Rust implementation notes)

The `self.__next_f.push([1,"..."])` pattern: the string inside is escaped JSON. In Rust:
- Use `regex` crate to find all push chunks
- Find chunk containing `cardPullProbabilityMap`
- Unescape: parse the outer string as a JSON string value to get the inner content (`serde_json::from_str::<String>(&format!("\"{}\"", chunk))` or similar)
- Use `serde_json::Value` and navigate to the keys with `raw_decode` equivalent — in serde_json, deserialize into `serde_json::Value` and then use pointer syntax or manual traversal

### Pack type terminology
- `normal` = standard pack (99.95% of opens)
- `rare` = god pack (0.05%)
- `plus1` = premium subscriber bonus pack (6 cards instead of 5)

---

## Files Included in This Handoff

- `fetch_pull_rates.py` — Working Python script demonstrating the RaenonX pull rate extraction. Reference implementation for the Rust scraper.

---

## What's NOT Yet Done

- Rust scraper implementation (this is the next task)
- Dioxus app scaffold
- Data schema / SQLite or local storage design
- Image caching strategy
- Update/sync mechanism for new sets
