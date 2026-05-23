BEGIN;

-- Rarity Groups
--
-- Currently the rarity groups are:
-- * Diamond
-- * Star
-- * Shiny
-- * Crown
CREATE TABLE rarity_groups (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- Diamond, Star, Shiny, or Crown
    name TEXT UNIQUE NOT NULL
);

-- Rarity Classes
--
-- This represents user facing rarities, which are the group
-- symbol repeated some number of times. Typically, there is
-- only one rarity for each rarity class. However, there are
-- actually 2 rarities that are in the Star group with count
-- 2.
CREATE TABLE rarity_classes (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- rarity_groups.id
    group_id INTEGER NOT NULL,

    -- The number of times the group symbol is repeated
    count INTEGER NOT NULL,

    FOREIGN KEY (group_id) REFERENCES rarity_groups (id),
    UNIQUE (group_id, count) ON CONFLICT FAIL
);

-- Rarity Names
CREATE TABLE rarity_names (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- Full name of a rarity, such as "Common" or "Uncommon"
    name TEXT UNIQUE NOT NULL
);

-- Card Rarity Categories
--
-- These are the rarities use internally in PTCGP.
CREATE TABLE rarities (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- rarity_classes.id
    class_id INTEGER NOT NULL,

    -- 1 to 3 letter code, such as "C", "U", and "AR"
    code TEXT UNIQUE NOT NULL,

    -- rarity_names.id - Full name of rarity, such as "Common" or "Uncommon"
    name_id INTEGER UNIQUE NOT NULL,

    -- The wonder pick point cost to craft a card of this rarity.
    -- Null when crafting is not available for this rarity.
    craft_cost INTEGER,

    -- The wonder pick points awarded for a duplicate card of this rarity.
    -- Null when duplicates do not yield points for this rarity.
    dupe_dust INTEGER,

    FOREIGN KEY (class_id) REFERENCES rarity_classes (id),
    FOREIGN KEY (name_id) REFERENCES rarity_names (id)
);

-- Card Series
--
-- Currently the only series are A and B.
CREATE TABLE series (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- The series letter, which is essentially also the name. "A" or "B"
    code TEXT UNIQUE NOT NULL
);

-- Set Names
CREATE TABLE set_names (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- The full name of a set, such as "Genetic Apex"
    name TEXT UNIQUE NOT NULL
);

-- Card Sets
--
-- These are the sets of one or more packs that are released together.
CREATE TABLE sets (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- series.id - The series this set is part of
    series_id INTEGER NOT NULL,

    -- The 2 to 3 letter set code, such as "A1" or "B2a"
    code TEXT UNIQUE NOT NULL,

    -- set_names.id - The full name of the set, such as "Genetic Apex"
    name_id INTEGER UNIQUE NOT NULL,

    FOREIGN KEY (series_id) REFERENCES series (id),
    FOREIGN KEY (name_id) REFERENCES set_names (id)
);

-- Card Set Release Dates
--
-- Promo sets don't have a release date, so they will not have a
-- corresponding row in this table.
CREATE TABLE set_release_dates (
    -- sets.id - The set
    set_id INTEGER UNIQUE NOT NULL,

    -- The release date of the set
    release_date DATETIME NOT NULL,

    FOREIGN KEY (set_id) REFERENCES sets (id)
);

-- Listing of Promo Sets
CREATE TABLE promo_sets (
    -- sets.id - each set in this table is a series promo set
    set_id INTEGER NOT NULL UNIQUE,

    FOREIGN KEY (set_id) REFERENCES sets (id)
);

-- Set Card Counts
--
-- The total number of cards in a set. Not all sets have a known card
-- count (e.g. promo sets that are still receiving new cards), so this
-- is stored out-of-line rather than as a nullable column.
CREATE TABLE set_card_counts (
    -- sets.id - The set whose card count is described
    set_id INTEGER UNIQUE NOT NULL,

    -- The total number of cards in the set
    card_count INTEGER NOT NULL,

    FOREIGN KEY (set_id) REFERENCES sets (id)
);

-- Pack Subtitles
CREATE TABLE pack_subtitles (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- The pack subtitle, such as "Charizard" or "Mega Blaziken".
    subtitle TEXT UNIQUE NOT NULL
);

-- Card Packs
--
-- Individual packs part of a set
CREATE TABLE packs (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- sets.id - The set this pack is part of
    set_id INTEGER NOT NULL,

    subtitle_id INTEGER NOT NULL,

    FOREIGN KEY (set_id) REFERENCES sets (id),
    FOREIGN KEY (subtitle_id) REFERENCES pack_subtitles (id)
);

-- Names of Cards
--
-- Distinct cards can share the same name, so the names are stored out-of-line.
CREATE TABLE card_names (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- The name of a card, such as "Alolan Vulpix", "Pokeball", or "Mega Blaziken ex"
    name TEXT UNIQUE NOT NULL
);

-- Cards
--
-- Each row is a card, possibly with multiple versions. A row represents
-- one or more cards with the same name and content, but possibly different
-- art or other aesthetic differences.
CREATE TABLE cards (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- card_names.id - The name of this card
    name_id INTEGER NOT NULL,

    FOREIGN KEY (name_id) REFERENCES card_names (id)
);

-- Card Illustrators
--
-- Each card version has an illustrator for the artwork.
CREATE TABLE illustrators (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- The name or alias of the artist
    name TEXT UNIQUE NOT NULL
);

-- Card Versions
--
-- These are distinct cards. Each can have alternate versions with different
-- artwork or other aesthetic differences.
CREATE TABLE card_versions (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- cards.id - The non-distinct card this card is a version of
    card_id INTEGER NOT NULL,

    -- sets.id - The set this card is part of
    set_id INTEGER NOT NULL,

    -- rarities.id - The rarity of this card
    rarity_id INTEGER NOT NULL,

    -- The number of this card in its set
    number INTEGER,

    FOREIGN KEY (card_id) REFERENCES cards (id),
    FOREIGN KEY (set_id) REFERENCES sets (id),
    FOREIGN KEY (rarity_id) REFERENCES rarities (id)
);

-- Card Version Illustrators
--
-- The illustrator of a card version's artwork. Not all card versions
-- have a known illustrator, so this is stored out-of-line.
CREATE TABLE card_version_illustrators (
    -- card_versions.id - The card version whose illustrator is described
    card_version_id INTEGER UNIQUE NOT NULL,

    -- illustrators.id - The illustrator of the card art
    illustrator_id INTEGER NOT NULL,

    FOREIGN KEY (card_version_id) REFERENCES card_versions (id),
    FOREIGN KEY (illustrator_id) REFERENCES illustrators (id)
);

-- Card Source Descriptions
CREATE TABLE card_source_descriptions (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- Human-readable description of a card source
    description TEXT UNIQUE NOT NULL
);

-- Card Sources
--
-- The various channels through which cards can be obtained.
-- Examples: "Pack", "Wonder Pick", "Gold Shop", "Shop", "Mission", "Premium Mission"
CREATE TABLE card_sources (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- Short code / display name for this source
    code TEXT UNIQUE NOT NULL,

    -- card_source_descriptions.id - Human-readable description
    description_id INTEGER NOT NULL,

    FOREIGN KEY (description_id) REFERENCES card_source_descriptions (id)
);

-- Card Version Source Mapping
--
-- Each row assigns a card source to a card version.
-- A card can be available from multiple sources.
CREATE TABLE card_version_card_sources (
    -- card_versions.id - The card version
    card_version_id INTEGER NOT NULL,

    -- card_sources.id - The source for this card
    card_source_id INTEGER NOT NULL,

    FOREIGN KEY (card_version_id) REFERENCES card_versions (id),
    FOREIGN KEY (card_source_id) REFERENCES card_sources (id),
    UNIQUE (card_version_id, card_source_id) ON CONFLICT IGNORE
);

-- Promo Stamp Card Versions
--
-- Card versions in this table are from a promo set (P-A, P-B, etc.) and
-- carry a visible promo stamp on the card art, making them visually
-- distinct from any non-promo printing of the same artwork.
CREATE TABLE promo_card_versions (
    -- card_versions.id - The card version with a promo stamp
    card_version_id INTEGER UNIQUE NOT NULL,

    FOREIGN KEY (card_version_id) REFERENCES card_versions (id)
);

-- Foil Card Versions
--
-- Card versions in this table have a foil/mirror finish
CREATE TABLE foil_card_versions (
    -- card_versions.id - The card version with a foil finish
    card_version_id INTEGER UNIQUE NOT NULL,

    FOREIGN KEY (card_version_id) REFERENCES card_versions (id)
);

-- Card Version Duplicates
--
-- Each row identifies a card version as part of a duplicate group —
-- cards with identical content (rarity, illustrator, promo stamp, and
-- foil status) released in different sets. Every member of a group has
-- a row here, including the original, which carries a self-referential
-- original_version_id. The original is the earliest release in the group.
-- Standalone card versions (no duplicates) are absent from this table.
CREATE TABLE card_version_duplicates (
    -- card_versions.id - A card version that is part of a duplicate group
    card_version_id INTEGER UNIQUE NOT NULL,

    -- card_versions.id - The earliest known identical card version (the original).
    --
    -- For the original itself, this is self-referential (card_version_id = original_version_id).
    original_version_id INTEGER NOT NULL,

    FOREIGN KEY (card_version_id) REFERENCES card_versions (id),
    FOREIGN KEY (original_version_id) REFERENCES card_versions (id)
);

-- Packs each card is in
--
-- Each row is a single card version and pack pair. Each card version can
-- potentially be in multiple packs, and thus may be in multiple rows.
CREATE TABLE card_packs (
    -- card_versions.id - The card in the pack identified by pack_id
    card_version_id INTEGER NOT NULL,
    -- packs.id - One of the packs this card is in
    pack_id INTEGER NOT NULL,

    FOREIGN KEY (card_version_id) REFERENCES card_versions (id),
    FOREIGN KEY (pack_id) REFERENCES packs (id)
);

-- Trainer Card Kinds
--
-- Currently the kinds of trainer cards are:
-- * Item
-- * Stadium
-- * Supporter
-- * Tool
CREATE TABLE trainer_kinds (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- The name of the trainer kind
    name TEXT NOT NULL
);

-- Effect text of trainer cards
--
-- The effect text of a trainer card isn't necessarily unique for
-- each trainer card, so the strings are stored out-of-line.
CREATE TABLE trainer_effects (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- The trainer card's effect text
    effect TEXT NOT NULL
);

-- Trainer Card Data
--
-- Each row describes a card in the cards table. Having a row in this
-- table identifies the card as a trainer card.
CREATE TABLE trainer_cards (
    -- cards.id - The trainer card described
    card_id INTEGER UNIQUE NOT NULL,

    -- trainer_kinds.id - The kind of trainer card this is
    kind_id INTEGER NOT NULL,

    -- trainer_effects.id - The effect text on this trainer card
    effect_id INTEGER NOT NULL,

    FOREIGN KEY (card_id) REFERENCES cards (id),
    FOREIGN KEY (kind_id) REFERENCES trainer_kinds (id),
    FOREIGN KEY (effect_id) REFERENCES trainer_effects (id)
);

-- Elements (types)
--
-- Currently the only elements are:
-- * Grass
-- * Fire
-- * Water
-- * Lightning
-- * Fighting
-- * Psychic
-- * Darkness
-- * Metal
-- * Dragon
-- * Colorless
CREATE TABLE elements (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- Single-letter energy symbol (e.g. "G" for Grass, "R" for Fire).
    -- Null for Dragon, which has no dedicated energy type in PTCGP.
    symbol TEXT UNIQUE,

    -- The name of the element, such as "Grass" or "Fire"
    name TEXT UNIQUE NOT NULL
);

-- Names of Abilities
--
-- Abilities can potentially have common names, so their names
-- are stored out-of-line.
CREATE TABLE ability_names (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- The ability name
    name TEXT UNIQUE NOT NULL
);

-- Ability Effect Text
--
-- Abilities can potentially have common effect text, so the
-- strings are stored out-of-line.
CREATE TABLE ability_effects (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- The ability effect text
    effect TEXT UNIQUE NOT NULL
);

-- Abilities
--
-- Each row describes a distinct pokemon card ability
CREATE TABLE abilities (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- ability_names.id - The name of the ability
    name_id INTEGER NOT NULL,

    -- ability_effects.id - The effect text of the ability
    effect_id INTEGER NOT NULL,

    FOREIGN KEY (name_id) REFERENCES ability_names (id),
    FOREIGN KEY (effect_id) REFERENCES ability_effects (id)
);

-- Attack Names
--
-- Attacks can potentially have common names, so the names
-- are stored out-of-line.
CREATE TABLE attack_names (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- The attack name
    name TEXT UNIQUE NOT NULL
);

-- Attack Effect Text
--
-- Attacks can potentially have common effect text, so the
-- strings are stored out-of-line.
CREATE TABLE attack_effects (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- The attack effect text
    effect TEXT UNIQUE NOT NULL
);

-- Attacks
--
-- Each row describes a distinct pokemon chard attack
CREATE TABLE attacks (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- attack_names.id - The name of the attack
    name_id TEXT NOT NULL,

    -- attack_effects.id - The effect text of the attack.
    --
    -- Not all effects have effect text. When that is the case,
    -- this column will be null.
    effect_id INTEGER,

    -- The base damage number of the attack as displayed on a card.
    --
    -- Attacks that don't deal damage will have a base damage of 0,
    -- but note that non-damaging attacks do not display a damage
    -- number on the card.
    base_damage INTEGER NOT NULL,

    -- The damage modifier suffix.
    --
    -- This number is a single Unicode codepoint, which is the symbol
    -- appended to the damage number on a card. This column will be
    -- null when there is no damage suffix.
    --
    -- Currently, the only damage suffixes are:
    -- * null - Flat damage
    -- * U+002B ('+') - See effect text for additional damage
    -- * U+00D7 ('×') - See effect text for applying damage multiple times
    damage_suffix_codepoint INTEGER,

    FOREIGN KEY (name_id) REFERENCES attack_names (id),
    FOREIGN KEY (effect_id) REFERENCES attack_effects (id)
);

-- Energy Cost of Attacks
--
-- Attacks cost varying amounts of energy of one or more different elements.
-- Each row represents one energy of the attack's cost, and all rows for the
-- same attack, make up the full cost of the attack.
CREATE TABLE attack_cost (
    -- attacks.id - The attack
    attack_id INTEGER NOT NULL,

    -- elements.id - The element of the required energy for the attack
    element_id INTEGER NOT NULL,

    -- The index of this energy in the attack's cost
    --
    -- Each attack has energies with indexes ordered from 0 to N, where
    -- N is the total number of required energies. The index order
    -- matches the order in which energies are displayed on a card for
    -- the attack.
    idx INTEGER NOT NULL,

    FOREIGN KEY (attack_id) REFERENCES attacks (id),
    FOREIGN KEY (element_id) REFERENCES elements (id),
    UNIQUE (attack_id, idx) ON CONFLICT FAIL
);

-- Base Pokemon Names
CREATE TABLE base_pokemon_names (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- The base pokemon name
    name TEXT UNIQUE NOT NULL
);

-- Base Pokemon
--
-- This table contains the names and national pokedex numbers of all
-- pokemon from main series video games.
CREATE TABLE base_pokemon (
    -- Primary key and the national pokedex number
    natdex_number INTEGER PRIMARY KEY UNIQUE NOT NULL,

    -- base_pokemon_names.id - The name of the base pokemon
    name_id INTEGER UNIQUE NOT NULL,

    FOREIGN KEY (name_id) REFERENCES base_pokemon_names (id)
);

-- Pokemon Stages
--
-- Currently, the only stages for a Pokemon Card are:
-- * Basic
-- * Stage 1
-- * Stage 2
CREATE TABLE stages (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- The name of the stage
    name TEXT UNIQUE NOT NULL
);

-- Pokemon Card Data
--
-- Each row describes a card in the cards table. Having a row in this
-- table identifies the card as a pokemon card.
CREATE TABLE pokemon_cards (
    -- cards.id - The pokemon card described
    card_id INTEGER UNIQUE NOT NULL,

    -- base_pokemon.natdex_number - The base pokemon of the card
    natdex_number INTEGER NOT NULL,

    -- elements.id - The element (type) of the pokemon
    element_id INTEGER NOT NULL,

    -- stages.id - The stage of the pokemon card
    stage_id INTEGER NOT NULL,

    -- The energy cost for the card to retreat.
    -- Null when not available for this card.
    retreat_cost INTEGER,

    -- The HP of the pokemon card.
    -- Null when not available for this card.
    hp INTEGER,

    FOREIGN KEY (natdex_number) REFERENCES base_pokemon (natdex_number),
    FOREIGN KEY (card_id) REFERENCES cards (id),
    FOREIGN KEY (element_id) REFERENCES elements (id),
    FOREIGN KEY (stage_id) REFERENCES stages (id)
);

-- Pokemon Card Evolutions
--
-- Each row describes a card that evolves from another pokemon card.
-- Evolutions don't require specific cards, but rather cards matching
-- a specific name.
CREATE TABLE pokemon_evolves_from (
    -- cards.id - The evolved card
    card_id INTEGER UNIQUE NOT NULL,

    -- card_names.id - The name of cards this card evolves from
    evolves_from_id INTEGER NOT NULL,

    FOREIGN KEY (card_id) REFERENCES cards (id),
    FOREIGN KEY (evolves_from_id) REFERENCES card_names (id)
);

-- Pokemon Card Flavor Texts
--
-- Flavor text strings, stored out-of-line for deduplication.
CREATE TABLE pokemon_flavor_texts (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- The flavor text
    flavor TEXT UNIQUE NOT NULL
);

-- Pokemon Card Flavor Text
--
-- Each row represents the flavor text for a Pokemon card. Not all
-- cards have flavor text, and in those cases, the card with not have
-- a row in this table.
--
-- Flavor text is not displayed on the card. PTCGP displays this text
-- in the card details page of the collection browser.
CREATE TABLE pokemon_flavor_text (
    -- The card with this flavor text
    card_id INTEGER UNIQUE NOT NULL,

    -- pokemon_flavor_texts.id - The flavor text
    flavor_id INTEGER NOT NULL,

    FOREIGN KEY (card_id) REFERENCES cards (id),
    FOREIGN KEY (flavor_id) REFERENCES pokemon_flavor_texts (id)
);

-- Pokemon Card Weaknesses
--
-- Each row represents the element weakness of a Pokemon card.
-- Cards with no weakness will not have a row in this table.
CREATE TABLE weaknesses (
    -- cards.id - The pokemon card with a weakness
    card_id INTEGER UNIQUE NOT NULL,

    -- elements.id - The element the pokemon card is weak to
    element_id INTEGER NOT NULL,

    FOREIGN KEY (card_id) REFERENCES cards (id),
    FOREIGN KEY (element_id) REFERENCES elements (id)
);

-- Pokemon Card Attacks
--
-- Each row represents one attack on a Pokemon card. Each card
-- can potentially have mutliple attacks, so each card may
-- appear in the table more than once.
CREATE TABLE pokemon_attacks (
    -- cards.id - The card with the attack
    card_id INTEGER NOT NULL,

    -- attacks.id - The attack
    attack_id INTEGER NOT NULL,

    -- The order of the attack as displayed on the card
    idx INTEGER NOT NULL,

    FOREIGN KEY (card_id) REFERENCES cards (id),
    FOREIGN KEY (attack_id) REFERENCES attacks (id),
    UNIQUE (card_id, idx) ON CONFLICT FAIL
);

-- Pokemon Card Abilities
--
-- Each row represents the ability of a Pokemon Card. Each
-- Pokemon card can have either zero or one ability. Cards
-- not present in this table do not have an ability.
CREATE TABLE pokemon_abilities (
    -- cards.id - The card with the ability
    card_id INTEGER UNIQUE NOT NULL,

    -- abilities.id - The ability
    ability_id INTEGER NOT NULL,

    FOREIGN KEY (card_id) REFERENCES cards (id),
    FOREIGN KEY (ability_id) REFERENCES abilities (id)
);

-- Pokemon ex Cards
--
-- Cards included in this table are Pokemon ex.
CREATE TABLE ex_cards (
    -- cards.id - The pokemon card
    card_id INTEGER UNIQUE NOT NULL,

    FOREIGN KEY (card_id) REFERENCES cards (id)
);

-- Mega Pokemon ex Cards
--
-- Cards included in this table are Mega Pokemon ex.
-- Note that all cards in this table are always also listed
-- in the ex_cards table.
CREATE TABLE mega_cards (
    -- cards.id - The pokemon card
    card_id INTEGER UNIQUE NOT NULL,

    FOREIGN KEY (card_id) REFERENCES cards (id)
);

-- Pokemon Card Variant Identifiers
--
-- Each row in this table represents a variant identifier
-- for a Pokemon used in Pokemon card names. Examples
-- include regional variants, such as "Alolan" or
-- "Paldean", and other identifiers such as "Teal Mask".
-- "Mega" and "ex" are not included as variant identifiers,
-- and are instead tracked in the ex_cards and mega_cards
-- tables. Variant identifiers can either be a prefix or
-- a suffix, which is denoted by whether an identifier is
-- present in the pokemon_variant_suffixes table.
CREATE TABLE pokemon_variants (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- The variant identifier, such as "Alolan" or "Teal Mask"
    ident TEXT UNIQUE NOT NULL
);

-- Pokemon Card Variant Identifier Suffixes
--
-- Variant identifiers in this table are suffixes. Identifiers
-- not listed in this table are implicitly prefixes. A prefix
-- identifier is displayed before the base pokemon name, and a
-- suffix identifier is displayed after the base pokemon name.
CREATE TABLE pokemon_variant_suffixes (
    -- pokemon_variants.id - The variant identifier that is a suffix
    variant_id INTEGER UNIQUE NOT NULL,

    FOREIGN KEY (variant_id) REFERENCES pokemon_variants (id)
);

-- Pokemon Card Variant Identifier Mappings
--
-- Each row assigns a variant identifier to a Pokemon card.
-- Note that it is possible for a card to have a prefix and a
-- suffix variant identifier, so a card may have up to 2 rows
-- in this table.
CREATE TABLE pokemon_variant_tags (
    -- cards.id - The card with the variant identifier
    card_id INTEGER NOT NULL,

    -- pokemon_variants.id - The variant identifier on the card
    variant_id INTEGER NOT NULL,

    FOREIGN KEY (card_id) REFERENCES cards (id),
    FOREIGN KEY (variant_id) REFERENCES pokemon_variants (id)
);

-- Pack Variant Codes
--
-- Short codes used in pull rate data for each kind of pack variant.
CREATE TABLE pack_variant_codes (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- Short code, such as "normal" or "plus1"
    code TEXT UNIQUE NOT NULL
);

-- Pack Variant Names
--
-- Display names for each kind of pack variant.
CREATE TABLE pack_variant_names (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- Display name, such as "Regular Pack" or "Rare Pack"
    name TEXT UNIQUE NOT NULL
);

-- Pack Variant Kinds
--
-- The three kinds of pack variant: normal, rare, and plus1.
CREATE TABLE pack_variant_kinds (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- pack_variant_codes.id - Short code for this variant kind
    code_id INTEGER UNIQUE NOT NULL,

    -- pack_variant_names.id - Display name for this variant kind
    name_id INTEGER UNIQUE NOT NULL,

    FOREIGN KEY (code_id) REFERENCES pack_variant_codes (id),
    FOREIGN KEY (name_id) REFERENCES pack_variant_names (id)
);

-- Pack Variants
--
-- Each pack has up to three variants (normal, rare, plus1)
-- that can be acquired when opening it. Each variant has its
-- own pull rate and distinct slot rarity/card pull rates.
CREATE TABLE pack_variants (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- pack_variant_kinds.id - The kind of this pack variant
    kind_id INTEGER NOT NULL,

    -- packs.id - The pack this variant belongs to
    pack_id INTEGER NOT NULL,

    -- The numerator of the pull rate for this pack variant.
    --
    -- The numerator of the pull rate for this pack variant.
    --
    -- The denominator is the LCM of all variant denominators for this pack,
    -- stored in pack_variant_rate_denominators. Dividing by that denominator
    -- gives the probability as a value between 0 and 1.
    rate_numerator INTEGER NOT NULL,

    FOREIGN KEY (kind_id) REFERENCES pack_variant_kinds (id),
    FOREIGN KEY (pack_id) REFERENCES packs (id),
    UNIQUE (kind_id, pack_id) ON CONFLICT FAIL
);

-- Pack Variant Rate Denominators
--
-- This table provides the common denominator for all variant pull rates
-- within a pack. Because different variants can have different denominators
-- in the source data, this value is the LCM of all per-variant denominators,
-- and each variant's rate_numerator is scaled accordingly.
CREATE TABLE pack_variant_rate_denominators (
    -- packs.id - The pack the pull rate denominator applies to
    pack_id INTEGER UNIQUE NOT NULL,

    -- The LCM of all variant rate denominators for this pack
    rate_denominator INTEGER NOT NULL,

    FOREIGN KEY (pack_id) REFERENCES packs (id)
);

-- Pack Variant Card Slots
--
-- Each pack variant has some number of cards that it will
-- yield when opened. Each slot represents one card pulled
-- in the order they are displayed to users. Each slot has
-- different pull rates for rarities and cards.
CREATE TABLE pack_slots (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- pack_variants.id - The pack variant this slot belongs to
    pack_variant_id INTEGER NOT NULL,

    -- The number of the card slot in the order cards are displayed
    -- to the user when the pack is opened.
    pull_number INTEGER NOT NULL,

    -- The denominator for rarity and card pull rates in this slot
    rate_denominator INTEGER NOT NULL,

    FOREIGN KEY (pack_variant_id) REFERENCES pack_variants (id),
    UNIQUE (pack_variant_id, pull_number) ON CONFLICT FAIL
);

-- Rarity Pull Rates
--
-- Each row represents the probability of pulling a card of a particular
-- rarity and finish (foil or normal) from a particular pack variant slot.
-- A rarity with only normal-finish cards has one row (is_foil = 0).
-- A rarity with only foil-finish cards has one row (is_foil = 1).
-- A rarity with a mix of both finishes has two rows, one for each.
CREATE TABLE rarity_pull_rates (
    -- pack_slots.id - The slot this pull rate is for
    slot_id INTEGER NOT NULL,

    -- rarities.id - The rarity with the pull rate described
    rarity_id INTEGER NOT NULL,

    -- Whether this row covers foil-finish cards (1) or normal-finish cards (0)
    is_foil INTEGER NOT NULL CHECK (is_foil IN (0, 1)),

    -- The numerator of the pull rate for this rarity + finish combination.
    --
    -- The denominator of the pull rate is specified in the
    -- pack_slots table. Together the numerator and denominator
    -- define a probability ratio between 0 and 1.
    rate_numerator INTEGER NOT NULL,

    FOREIGN KEY (slot_id) REFERENCES pack_slots (id),
    FOREIGN KEY (rarity_id) REFERENCES rarities (id),
    UNIQUE (slot_id, rarity_id, is_foil) ON CONFLICT FAIL
);

-- Card Version Pull Rates
--
-- Each row represents the probability to pull a specific card
-- version from a pack variant slot.
CREATE TABLE card_pull_rates (
    -- card_versions.id - The card version this pull rate is for
    card_version_id INTEGER NOT NULL,

    -- pack_slots.id - The slot this pull rate is for
    slot_id INTEGER NOT NULL,

    -- The numerator of the pull rate for this card version.
    --
    -- The denominator of the pull rate is specified in the
    -- pack_slots table. Together the numerator and denominator
    -- define a probability ratio between 0 and 1.
    rate_numerator INTEGER NOT NULL,

    FOREIGN KEY (card_version_id) REFERENCES card_versions (id),
    FOREIGN KEY (slot_id) REFERENCES pack_slots (id),
    UNIQUE (card_version_id, slot_id) ON CONFLICT FAIL
);

-- ── Indexes ──────────────────────────────────────────────────────────────────

-- card_versions: foreign key columns used in joins and filters.
-- set_id and rarity_id are not leftmost in any UNIQUE constraint.
-- card_id has no UNIQUE constraint at all.
CREATE INDEX idx_card_versions_card_id   ON card_versions (card_id);
CREATE INDEX idx_card_versions_set_id    ON card_versions (set_id);
CREATE INDEX idx_card_versions_rarity_id ON card_versions (rarity_id);

-- packs: set_id is not covered by any constraint.
CREATE INDEX idx_packs_set_id ON packs (set_id);

-- pack_variants: pack_id is the second column in UNIQUE (kind_id, pack_id),
-- so lookups by pack_id alone are not covered by that index.
CREATE INDEX idx_pack_variants_pack_id ON pack_variants (pack_id);

-- card_pull_rates: slot_id is the second column in UNIQUE (card_version_id, slot_id),
-- so lookups by slot_id alone (e.g. all cards pullable from a slot) are not covered.
CREATE INDEX idx_card_pull_rates_slot_id ON card_pull_rates (slot_id);

-- card_packs: junction table with no constraints; both directions need indexes.
CREATE INDEX idx_card_packs_card_version_id ON card_packs (card_version_id);
CREATE INDEX idx_card_packs_pack_id         ON card_packs (pack_id);

-- card_version_duplicates: original_version_id has no index, needed to find
-- all members of a duplicate group given the original.
CREATE INDEX idx_card_version_duplicates_original ON card_version_duplicates (original_version_id);

-- ── Convenience views ────────────────────────────────────────────────────────

-- versions: one row per card version with all commonly needed fields resolved.
-- Covers the most painful joins in the schema (set, rarity hierarchy,
-- illustrator, and the three boolean flags stored in satellite tables).
CREATE VIEW versions AS
    SELECT
        cv.id               AS version_id,
        cv.card_id,
        cn.name             AS card_name,
        s.code              AS set_code,
        sn.name             AS set_name,
        cv.number,
        r.code              AS rarity_code,
        rn.name             AS rarity_name,
        rg.name             AS rarity_group,
        rc.count            AS rarity_count,
        i.name              AS illustrator,
        CASE WHEN pcv.card_version_id IS NOT NULL THEN 1 ELSE 0 END AS is_promo,
        CASE WHEN fcv.card_version_id IS NOT NULL THEN 1 ELSE 0 END AS is_foil,
        CASE WHEN cvd.card_version_id IS NOT NULL
              AND cvd.card_version_id != cvd.original_version_id
             THEN 1 ELSE 0 END                                       AS is_reprint
    FROM card_versions cv
    JOIN cards                    c   ON c.id   = cv.card_id
    JOIN card_names               cn  ON cn.id  = c.name_id
    JOIN sets                     s   ON s.id   = cv.set_id
    JOIN set_names                sn  ON sn.id  = s.name_id
    JOIN rarities                 r   ON r.id   = cv.rarity_id
    JOIN rarity_names             rn  ON rn.id  = r.name_id
    JOIN rarity_classes           rc  ON rc.id  = r.class_id
    JOIN rarity_groups            rg  ON rg.id  = rc.group_id
    LEFT JOIN card_version_illustrators cvi ON cvi.card_version_id = cv.id
    LEFT JOIN illustrators        i   ON i.id   = cvi.illustrator_id
    LEFT JOIN promo_card_versions pcv ON pcv.card_version_id = cv.id
    LEFT JOIN foil_card_versions  fcv ON fcv.card_version_id = cv.id
    LEFT JOIN card_version_duplicates cvd ON cvd.card_version_id = cv.id;

-- pokemon: one row per abstract pokemon card with all game data resolved.
CREATE VIEW pokemon AS
    SELECT
        c.id                AS card_id,
        cn.name             AS name,
        bpn.name            AS base_name,
        bp.natdex_number,
        e.name              AS element,
        st.name             AS stage,
        pc.hp,
        pc.retreat_cost,
        we.name             AS weakness,
        CASE WHEN exc.card_id IS NOT NULL THEN 1 ELSE 0 END AS is_ex,
        CASE WHEN mc.card_id  IS NOT NULL THEN 1 ELSE 0 END AS is_mega,
        efn.name            AS evolves_from,
        pft_text.flavor,
        abn.name            AS ability_name,
        abe.effect          AS ability_effect
    FROM cards                c
    JOIN card_names           cn  ON cn.id  = c.name_id
    JOIN pokemon_cards        pc  ON pc.card_id = c.id
    JOIN base_pokemon         bp  ON bp.natdex_number = pc.natdex_number
    JOIN base_pokemon_names   bpn ON bpn.id = bp.name_id
    JOIN elements             e   ON e.id   = pc.element_id
    JOIN stages               st  ON st.id  = pc.stage_id
    LEFT JOIN weaknesses          w   ON w.card_id  = c.id
    LEFT JOIN elements            we  ON we.id       = w.element_id
    LEFT JOIN ex_cards            exc ON exc.card_id = c.id
    LEFT JOIN mega_cards          mc  ON mc.card_id  = c.id
    LEFT JOIN pokemon_evolves_from pef ON pef.card_id = c.id
    LEFT JOIN card_names          efn ON efn.id = pef.evolves_from_id
    LEFT JOIN pokemon_flavor_text  pft      ON pft.card_id  = c.id
    LEFT JOIN pokemon_flavor_texts pft_text ON pft_text.id  = pft.flavor_id
    LEFT JOIN pokemon_abilities   pab ON pab.card_id = c.id
    LEFT JOIN abilities           ab  ON ab.id  = pab.ability_id
    LEFT JOIN ability_names       abn ON abn.id = ab.name_id
    LEFT JOIN ability_effects     abe ON abe.id = ab.effect_id;

-- trainers: one row per abstract trainer card with all game data resolved.
CREATE VIEW trainers AS
    SELECT
        c.id    AS card_id,
        cn.name AS name,
        tk.name AS kind,
        te.effect
    FROM cards              c
    JOIN card_names         cn ON cn.id    = c.name_id
    JOIN trainer_cards      tc ON tc.card_id = c.id
    JOIN trainer_kinds      tk ON tk.id    = tc.kind_id
    JOIN trainer_effects    te ON te.id    = tc.effect_id;

-- rarity_overview: rarities with the group name and symbol count resolved,
-- avoiding the two-hop join through rarity_classes and rarity_groups.
CREATE VIEW rarity_overview AS
    SELECT
        r.id            AS rarity_id,
        r.code,
        rn.name,
        rg.name         AS group_name,
        rc.count        AS symbol_count,
        r.craft_cost,
        r.dupe_dust
    FROM rarities           r
    JOIN rarity_names       rn ON rn.id = r.name_id
    JOIN rarity_classes     rc ON rc.id = r.class_id
    JOIN rarity_groups      rg ON rg.id = rc.group_id;

-- ── Image path views ─────────────────────────────────────────────────────────
--
-- Each view maps a database entity to its corresponding image file path
-- relative to the root of the ptcgp-images repository.

CREATE VIEW card_version_images AS
    SELECT
        cv.id   AS card_version_id,
        printf('cards/%s/%03d.png', s.code, cv.number) AS path
    FROM card_versions cv
    JOIN sets s ON s.id = cv.set_id;

CREATE VIEW element_icons AS
    SELECT
        id AS element_id,
        printf('elements/icons/%s.png', lower(name)) AS path
    FROM elements;

CREATE VIEW element_symbols AS
    SELECT
        id AS element_id,
        printf('elements/symbols/%s.png', lower(name)) AS path
    FROM elements;

-- Only packs with a subtitle have art/logo images.
CREATE VIEW pack_art AS
    SELECT
        p.id AS pack_id,
        printf('packs/art/%s/%s.png', s.code, lower(replace(ps.subtitle, ' ', '_'))) AS path
    FROM packs p
    JOIN sets s ON s.id = p.set_id
    JOIN pack_subtitles ps ON ps.id = p.subtitle_id;

CREATE VIEW pack_logos AS
    SELECT
        p.id AS pack_id,
        printf('packs/logos/%s/%s.png', s.code, lower(replace(ps.subtitle, ' ', '_'))) AS path
    FROM packs p
    JOIN sets s ON s.id = p.set_id
    JOIN pack_subtitles ps ON ps.id = p.subtitle_id;

CREATE VIEW card_source_icons AS
    SELECT
        id AS card_source_id,
        printf('card_sources/%s.png', lower(replace(code, ' ', '_'))) AS path
    FROM card_sources;

CREATE VIEW rarity_icons AS
    SELECT
        r.id AS rarity_id,
        printf('rarities/icons/%s/%d.png', lower(rg.name), rc.count) AS path
    FROM rarities r
    JOIN rarity_classes rc ON rc.id = r.class_id
    JOIN rarity_groups rg ON rg.id = rc.group_id;

CREATE VIEW rarity_symbols AS
    SELECT
        r.id AS rarity_id,
        printf('rarities/symbols/%s/%d.png', lower(rg.name), rc.count) AS path
    FROM rarities r
    JOIN rarity_classes rc ON rc.id = r.class_id
    JOIN rarity_groups rg ON rg.id = rc.group_id;

CREATE VIEW set_logos AS
    SELECT
        id AS set_id,
        printf('sets/logos/%s.png', code) AS path
    FROM sets;

CREATE VIEW set_icons AS
    SELECT
        id AS set_id,
        printf('sets/icons/%s.png', code) AS path
    FROM sets;

COMMIT;
