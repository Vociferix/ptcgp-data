BEGIN;

-- Rarity Groups
--
-- Currently there are only 4 rarity groups:
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

    -- Full name of rarity, such as "Common" or "Uncommon"
    name TEXT UNIQUE NOT NULL,

    FOREIGN KEY (class_id) REFERENCES rarity_classes (id)
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

    -- The full name of the set, such as "Genetic Apex"
    name TEXT UNIQUE NOT NULL,

    -- The date the set was released.
    --
    -- This is null for promo sets.
    release_date DATETIME,

    FOREIGN KEY (series_id) REFERENCES series (id)
);

-- Listing of Promo Sets
CREATE TABLE promo_sets (
    -- sets.id - each set in this table is a series promo set
    set_id INTEGER NOT NULL UNIQUE,

    FOREIGN KEY (set_id) REFERENCES sets (id)
);

-- Card Packs
--
-- Individual packs part of a set
CREATE TABLE packs (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- sets.id - The set this pack is part of
    set_id INTEGER NOT NULL,

    FOREIGN KEY (set_id) REFERENCES sets (id)
);

-- Pack Subtitles
--
-- When a set has multiple packs, each pack has a subtitle to identify it.
CREATE TABLE pack_subtitles (
    -- packs.id - The pack that has the this row's subtitle
    pack_id INTEGER UNIQUE NOT NULL,

    -- The subtitle for this pack, such as "Charizard" or "Mega Blaziken".
    subtitle TEXT NOT NULL,

    FOREIGN KEY (pack_id) REFERENCES packs (id)
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

    -- illustrators.id - The illustrator of this card
    illustrator_id INTEGER NOT NULL,

    -- The number of this card in its set
    number INTEGER,

    FOREIGN KEY (card_id) REFERENCES cards (id),
    FOREIGN KEY (set_id) REFERENCES sets (id),
    FOREIGN KEY (rarity_id) REFERENCES rarities (id),
    FOREIGN KEY (illustrator_id) REFERENCES illustrators (id)
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

-- Base Pokemon
--
-- This table contains the names and national pokedex numbers of all
-- pokemon from main series video games.
CREATE TABLE base_pokemon (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- The name of the pokemon
    name TEXT UNIQUE NOT NULL,

    -- The national pokedex number of the pokemon
    natdex_number INTEGER UNIQUE NOT NULL
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

    -- base_pokemon.id - The base pokemon of the card
    base_id INTEGER NOT NULL,

    -- elements.id - The element (type) of the pokemon
    element_id INTEGER NOT NULL,

    -- stages.id - The stage of the pokemon card
    stage_id INTEGER NOT NULL,

    -- The energy cost for the card to retreat
    retreat_cost INTEGER NOT NULL,

    -- The HP of the pokemon card
    hp INTEGER NOT NULL,

    FOREIGN KEY (base_id) REFERENCES base_pokemon (id),
    FOREIGN KEY (card_id) REFERENCES cards (id),
    FOREIGN KEY (element_id) REFERENCES elements (id),
    FOREIGN KEY (stage_id) REFERENCES stages (id)
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

    -- The flavor text
    flavor TEXT NOT NULL,

    FOREIGN KEY (card_id) REFERENCES cards (id)
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

-- Names of Pack Variants
--
-- Pack variants generally share names across sets, so the
-- strings are stored out-of-line.
CREATE TABLE pack_variant_names (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- The pack variant name
    name TEXT NOT NULL
);

-- Pack Variants for Each Set
--
-- Each set generally has multiple variants that can be
-- acquired when opening packs. Each variant has a unique
-- pull rate when opening packs of its set, and has distinct
-- pull rates for rarities and cards from other variants in
-- the same set. Note that pack variants apply to all packs
-- with a set.
CREATE TABLE pack_variants (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT NULL,

    -- pack_variant_names.id - The name of the pack variant
    name_id INTEGER NOT NULL,

    -- sets.id - The set this pack variant belongs to
    set_id INTEGER NOT NULL,

    -- The numerator of the pull rate for this pack variant.
    --
    -- The denominator of the pull rate is specified in the
    -- pack_variant_rate_denominators table. Together the
    -- numerator and denominator define a probability ratio
    -- between 0 and 1.
    rate_numerator INTEGER NOT NULL,

    FOREIGN KEY (name_id) REFERENCES pack_variant_names (id),
    FOREIGN KEY (set_id) REFERENCES sets (id),
    UNIQUE (name_id, set_id) ON CONFLICT FAIL
);

-- Pack Variant Rate Denominators
--
-- This table provides the denominator of pack variant pull
-- rates. The numerator is specified in the pack_variants
-- table.
CREATE TABLE pack_variant_rate_denominators (
    -- sets.id - The set the pull rate applies to
    set_id INTEGER UNIQUE NOT NULL,

    -- The pull rate denominator
    rate_denominator INTEGER NOT NULL,

    FOREIGN KEY (set_id) REFERENCES sets (id)
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
-- Each row represents the probability of pulling card of a
-- particular rarity for a particular pack variant slot. Note that
-- this information is mostly supplementary for users and does not
-- define the probability of pulling any card of the specified
-- rarity. In some cases, not all cards of the specified rarity
-- are obtainable from the slot.
CREATE TABLE rarity_pull_rates (
    -- pack_slots.id - The slot this pull rate is for
    slot_id INTEGER NOT NULL,

    -- rarities.id - The rarity with the pull rate described
    rarity_id INTEGER NOT NULL,

    -- The numerator of the pull rate for this rarity.
    --
    -- The denominator of the pull rate is specified in the
    -- pack_slots table. Together the numerator and denominator
    -- define a probability ratio between 0 and 1.
    rate_numerator INTEGER NOT NULL,

    FOREIGN KEY (slot_id) REFERENCES pack_slots (id),
    FOREIGN KEY (rarity_id) REFERENCES rarities (id),
    UNIQUE (slot_id, rarity_id) ON CONFLICT FAIL
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

COMMIT;
