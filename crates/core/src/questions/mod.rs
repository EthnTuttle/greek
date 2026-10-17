//! Measured-assessment engine: auto-generated EN→GR multiple-choice
//! questions plus accent-tolerant typed-answer grading.
//!
//! Unlike self-rated flashcards, practice questions produce an objective
//! accuracy signal. The SRS layer records their results with
//! `source = 'practice'` so accuracy can be measured separately.

use std::collections::HashSet;

use crate::morphology::normalize_form;
use crate::Card;

/// One EN→GR multiple-choice question: `prompt_gloss` is the card's English
/// back, `choices` are Greek fronts (4 options including the correct one),
/// and `correct_index` marks the front belonging to `card_id`.
#[derive(Debug, Clone, PartialEq)]
pub struct Mcq {
    pub card_id: String,
    pub prompt_gloss: String,
    pub choices: Vec<String>,
    pub correct_index: usize,
}

/// Grade for a typed (free-form) answer against a card's Greek front.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypedGrade {
    /// Byte-identical after whitespace cleanup — accents must match.
    Exact,
    /// Equal once accents/diacritics are stripped and case is folded.
    Lenient,
    /// Anything else.
    Wrong,
}

/// Generate up to `limit` MCQs from `cards`. Pure: no DB, no clock —
/// output depends only on the slice and `seed`.
///
/// Randomness: a xorshift64 PRNG (Marsaglia's xorshift family —
/// state ^= state << 13; state ^= state >> 7; state ^= state << 17).
/// A zero seed is remapped to a fixed non-zero constant (the xorshift
/// state must never be 0). All draws use modulo reduction, and Fisher–Yates
/// shuffles consume the stream in index order, so the same seed always
/// reproduces the identical question set on any platform.
///
/// Prompt selection walks the slice in seeded-random order while respecting
/// caller priority: eligible cards (non-empty trimmed front AND back) are
/// processed in caller-ordered windows of `limit` cards; each window is
/// shuffled with the seeded RNG and fully consumed before the next window
/// starts. Callers that want a preference (e.g. reviewed cards first)
/// simply pass an ordered slice.
///
/// Distractor selection per prompt card follows a strict widening policy:
///   1. same `pos` AND same chapter
///   2. same `pos` AND shares at least one tag (same textbook/volume)
///   3. same `pos`, anywhere
///   4. any eligible card
/// Each pool is consumed in seeded-random order until 3 distractors are
/// found. Choices are deduped by front text (including against the correct
/// front), and a card is omitted from the set entirely when even policy 4
/// cannot yield 3 distinct distractors.
pub fn generate_mcq_set(cards: &[Card], limit: usize, seed: u64) -> Vec<Mcq> {
    if limit == 0 {
        return Vec::new();
    }
    let mut rng = Xorshift64::new(seed);

    // Eligible = usable as a prompt AND as a distractor (skipped entirely).
    let eligible: Vec<usize> = (0..cards.len())
        .filter(|&i| !cards[i].front.trim().is_empty() && !cards[i].back.trim().is_empty())
        .collect();

    let mut out = Vec::new();
    let mut start = 0;
    while start < eligible.len() && out.len() < limit {
        let end = (start + limit).min(eligible.len());
        // Sample without replacement within this window.
        for w in shuffled_indices(end - start, &mut rng) {
            if out.len() >= limit {
                break;
            }
            let target = eligible[start + w];
            if let Some(mcq) = build_mcq(cards, &eligible, target, &mut rng) {
                out.push(mcq);
            }
        }
        start = end;
    }
    out
}

/// Grade a typed answer against a card's Greek front. Both sides are
/// trimmed and have internal runs of whitespace collapsed first. The
/// lenient check reuses [`normalize_form`] from the morphology module
/// (accent/diacritic stripping + case folding) so grading can never drift
/// from dictionary normalization.
pub fn grade_typed(expected_front: &str, answer: &str) -> TypedGrade {
    let expected = collapse_whitespace(expected_front);
    let given = collapse_whitespace(answer);
    if given == expected {
        return TypedGrade::Exact;
    }
    if normalize_form(&given) == normalize_form(&expected) {
        return TypedGrade::Lenient;
    }
    TypedGrade::Wrong
}

/// Trim leading/trailing whitespace and collapse internal whitespace runs
/// to a single space (any Unicode whitespace counts).
fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// xorshift64 PRNG: tiny, fast, and fully deterministic across platforms.
struct Xorshift64(u64);

impl Xorshift64 {
    fn new(seed: u64) -> Self {
        // xorshift's state must be non-zero; remap 0 to a fixed constant.
        Self(if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed })
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Index in 0..n via modulo reduction (uniform enough for shuffling).
    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

/// Fisher–Yates shuffle of the index range 0..len, driven by `rng`.
fn shuffled_indices(len: usize, rng: &mut Xorshift64) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..len).collect();
    for i in (1..len).rev() {
        let j = rng.below(i + 1);
        idx.swap(i, j);
    }
    idx
}

/// Build one MCQ for `target`, or `None` when even the widest distractor
/// policy cannot produce 3 distinct distractor fronts.
fn build_mcq(
    cards: &[Card],
    eligible: &[usize],
    target: usize,
    rng: &mut Xorshift64,
) -> Option<Mcq> {
    let t = &cards[target];
    let mut used: HashSet<&str> = HashSet::new();
    used.insert(t.front.as_str());
    let mut distractors: Vec<String> = Vec::new();

    // The four distractor policies in strict widening order.
    let policies: [fn(&Card, &Card) -> bool; 4] = [
        same_pos_same_chapter,
        same_pos_shared_tag,
        same_pos,
        any_card,
    ];
    for matches in policies {
        if distractors.len() == 3 {
            break;
        }
        let pool: Vec<usize> = eligible
            .iter()
            .copied()
            .filter(|&i| {
                i != target && matches(&cards[i], t) && !used.contains(cards[i].front.as_str())
            })
            .collect();
        for pi in shuffled_indices(pool.len(), rng) {
            if distractors.len() == 3 {
                break;
            }
            let front = cards[pool[pi]].front.as_str();
            if used.insert(front) {
                distractors.push(front.to_string());
            }
        }
    }

    if distractors.len() < 3 {
        return None;
    }

    let correct_index = rng.below(4);
    let mut choices = Vec::with_capacity(4);
    let mut rest = distractors.into_iter();
    for slot in 0..4 {
        if slot == correct_index {
            choices.push(t.front.clone());
        } else {
            choices.push(rest.next().expect("3 distractors fill 3 non-correct slots"));
        }
    }
    Some(Mcq {
        card_id: t.id.clone(),
        prompt_gloss: t.back.clone(),
        choices,
        correct_index,
    })
}

fn same_pos_same_chapter(c: &Card, t: &Card) -> bool {
    c.pos == t.pos && c.chapter == t.chapter
}

/// "Same textbook tag": the cards share at least one tag string (e.g. both
/// tagged "athenaze", or volume-prefixed variants of the same book).
fn same_pos_shared_tag(c: &Card, t: &Card) -> bool {
    c.pos == t.pos && c.tags.iter().any(|tag| t.tags.contains(tag))
}

fn same_pos(c: &Card, t: &Card) -> bool {
    c.pos == t.pos
}

fn any_card(_: &Card, _: &Card) -> bool {
    true
}

// --- Alphabet Check (see DESIGN-alphabet-gate.md §4.3) ---

/// Quiz direction for an Alphabet Check question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlphabetDirection {
    /// Prompt shows the glyph(s), choices are letter names.
    GlyphToName,
    /// Prompt shows the letter name, choices are glyphs.
    NameToGlyph,
}

/// One Alphabet Check question: a glyph↔name multiple-choice item with 4
/// choices, `correct_index` marking the choice belonging to `card_id`.
#[derive(Debug, Clone, PartialEq)]
pub struct AlphabetQuestion {
    pub card_id: String,
    pub direction: AlphabetDirection,
    pub prompt: String,
    pub choices: Vec<String>,
    pub correct_index: usize,
}

/// Extract the short quiz label (letter name / romanization) from a letters
/// card's English `back`, per chapter format:
/// - ch.1–3 (`Β β` / "beta — b; ..."): text before the first " — ";
/// - ch.5 (`αι` / "αι — ai, as in ..."): the romanization token after the
///   dash, cut at ',' or '/'. Seven ch.5 backs carry no romanization token
///   ("ει — a long close e, as in 'vein'"); for those the label falls back
///   to the standard scholarly romanization of the front digraph
///   ([`diphthong_romanization`]), because a name↔glyph quiz needs one.
/// - ch.4 diacritics: no usable label — always returns None.
///
/// Eligibility invariant (checked here, tested against the real deck):
/// non-empty, <= 20 chars, <= 2 words, and (for ch.5) label != front.
pub fn alphabet_check_label(card: &Card) -> Option<String> {
    if card.chapter == 4 {
        return None;
    }
    let dash = card.back.find(" — ")?;
    let label = if card.chapter == 5 {
        let token = card.back[dash + 4..]
            .split([',', '/'])
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        if is_valid_alphabet_label(&token) && token != card.front.trim() {
            token
        } else {
            diphthong_romanization(card.front.trim())?.to_string()
        }
    } else {
        card.back[..dash].trim().to_string()
    };
    if is_valid_alphabet_label(&label) && (card.chapter != 5 || label != card.front.trim()) {
        Some(label)
    } else {
        None
    }
}

/// The eligibility invariant for a quiz label: non-empty, at most 20 chars,
/// at most 2 words.
fn is_valid_alphabet_label(label: &str) -> bool {
    !label.is_empty() && label.chars().count() <= 20 && label.split_whitespace().count() <= 2
}

/// Standard scholarly (LSJ-style) romanizations of the Greek diphthong
/// digraphs. All 11 outputs are distinct, so they can serve as MCQ choices
/// without ambiguity. Returns None for anything else.
fn diphthong_romanization(front: &str) -> Option<&'static str> {
    Some(match front {
        "αι" => "ai",
        "ει" => "ei",
        "οι" => "oi",
        "υι" => "ui",
        "αυ" => "au",
        "ευ" => "eu",
        "ηυ" => "ēu",
        "ου" => "ou",
        "ᾳ" => "āi",
        "ῃ" => "ēi",
        "ῳ" => "ōi",
        _ => return None,
    })
}

/// Cards usable in the Alphabet Check: letters track, well-formed
/// front/back, and an extractable label. Private to this module.
fn eligible_alphabet_cards(cards: &[Card]) -> Vec<(&Card, String)> {
    cards
        .iter()
        .filter(|c| {
            crate::curriculum::card_in_textbook(c, "letters")
                && !c.front.trim().is_empty()
                && !c.back.trim().is_empty()
        })
        .filter_map(|c| alphabet_check_label(c).map(|label| (c, label)))
        .collect()
}

/// Generate an n-question Alphabet Check from the loaded `cards`. Pure: no
/// DB, no clock — the output depends only on the slice and `seed`.
///
/// Stratified by chapter: each represented chapter gets at least 3 questions
/// while the pool is large enough; the rest is drawn round-robin. Question
/// order is mixed (seeded shuffle) and directions alternate
/// GlyphToName/NameToGlyph deterministically (the seeded order's parity
/// decides who goes first). Distractors reuse `build_mcq`'s widening policy
/// (same-chapter → same-tag → same-pos → any) over the eligible pools —
/// names for name-space questions, glyphs for glyph-space questions.
pub fn generate_alphabet_check(
    cards: &[Card],
    n: usize,
    seed: u64,
) -> Vec<AlphabetQuestion> {
    if n == 0 {
        return Vec::new();
    }
    let mut rng = Xorshift64::new(seed);

    let eligible = eligible_alphabet_cards(cards);
    if eligible.len() < 4 {
        return Vec::new();
    }

    // Parallel eligible card slices: one whose fronts are the quiz labels
    // (with label-vs-label comparability encoded in `pos`, so ch.5
    // romanizations only confuse other ch.5 romanizations), and one with the
    // real glyph fronts. Both align index-for-index with `eligible`.
    let label_cards: Vec<Card> = eligible
        .iter()
        .map(|(c, label)| {
            let mut lc = (*c).clone();
            lc.front = label.clone();
            lc.back = c.id.clone();
            lc.pos = format!("letter-label-{}", c.chapter);
            lc
        })
        .collect();
    let glyph_cards: Vec<Card> = eligible.iter().map(|(c, _)| (*c).clone()).collect();
    let all_eligible: Vec<usize> = (0..eligible.len()).collect();

    // Stratified picks: per-chapter shuffled pools, at least 3 per chapter
    // while possible, then round-robin.
    let mut by_chapter: std::collections::BTreeMap<i32, Vec<usize>> =
        std::collections::BTreeMap::new();
    for (i, (c, _)) in eligible.iter().enumerate() {
        by_chapter.entry(c.chapter).or_default().push(i);
    }
    let mut picks: Vec<usize> = Vec::new();
    let mut cursor: std::collections::HashMap<i32, usize> = std::collections::HashMap::new();
    let chapters: Vec<i32> = by_chapter.keys().copied().collect();
    for &ch in &chapters {
        let pool = &by_chapter[&ch];
        for pi in shuffled_indices(pool.len(), &mut rng)
            .into_iter()
            .take(3.min(pool.len()))
        {
            if picks.len() >= n {
                break;
            }
            picks.push(pool[pi]);
            *cursor.entry(ch).or_default() += 1;
        }
    }
    while picks.len() < n {
        let mut progressed = false;
        for &ch in &chapters {
            if picks.len() >= n {
                break;
            }
            if by_chapter[&ch].len() > cursor[&ch] {
                picks.push(by_chapter[&ch][cursor[&ch]]);
                *cursor.entry(ch).or_default() += 1;
                progressed = true;
            }
        }
        if !progressed {
            break; // every eligible card used at most once
        }
    }

    // Mixed final order, then directions alternate along it.
    let order = shuffled_indices(picks.len(), &mut rng);
    let start_glyph_to_name = rng.below(2) == 0;

    let mut out = Vec::new();
    for (pos, oi) in order.into_iter().enumerate() {
        let ti = picks[oi];
        let direction = if (pos % 2 == 0) == start_glyph_to_name {
            AlphabetDirection::GlyphToName
        } else {
            AlphabetDirection::NameToGlyph
        };
        let (card, label) = &eligible[ti];
        let (prompt, pool): (String, &[Card]) = match direction {
            AlphabetDirection::GlyphToName => {
                (card.front.trim().to_string(), &label_cards)
            }
            AlphabetDirection::NameToGlyph => (label.clone(), &glyph_cards),
        };
        let mcq = build_mcq(pool, &all_eligible, ti, &mut rng);
        let Some(mcq) = mcq else {
            continue; // fewer than 3 distinct distractors; skip this pick
        };
        debug_assert_eq!(mcq.card_id, card.id);
        out.push(AlphabetQuestion {
            card_id: card.id.clone(),
            direction,
            prompt,
            choices: mcq.choices,
            correct_index: mcq.correct_index,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: &str, front: &str, back: &str, chapter: i32, pos: &str, tags: &[&str]) -> Card {
        Card {
            id: id.to_string(),
            front: front.to_string(),
            back: back.to_string(),
            chapter,
            card_type: "vocab".to_string(),
            pos: pos.to_string(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
        }
    }

    /// The choice fronts that are NOT the correct answer.
    fn distractor_fronts(mcq: &Mcq) -> Vec<&str> {
        mcq.choices
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != mcq.correct_index)
            .map(|(_, c)| c.as_str())
            .collect()
    }

    #[test]
    fn test_distractors_prefer_same_pos_same_chapter() {
        // Policy-1 pool has 3 peers, so decoys from wider pools (same-tag
        // other chapter, other pos, other tag) must stay unused.
        let cards = vec![
            card("t", "λόγος", "word", 1, "noun", &["athenaze"]),
            card("a", "ἄνθρωπος", "man", 1, "noun", &["athenaze"]),
            card("b", "θεός", "god", 1, "noun", &["athenaze"]),
            card("c", "οἶκος", "house", 1, "noun", &["athenaze"]),
            card("d", "νῆσος", "island", 2, "noun", &["athenaze"]),
            card("e", "λύω", "I loose", 1, "verb", &["athenaze"]),
            card("f", "πόλις", "city", 9, "noun", &["other"]),
        ];
        for seed in [1u64, 7, 99] {
            let set = generate_mcq_set(&cards, 1, seed);
            assert_eq!(set.len(), 1);
            assert_eq!(set[0].card_id, "t");
            assert_eq!(set[0].prompt_gloss, "word");
            let ds = distractor_fronts(&set[0]);
            assert_eq!(ds.len(), 3);
            for d in &ds {
                assert!(
                    ["ἄνθρωπος", "θεός", "οἶκος"].contains(d),
                    "distractor {d} must come from the same-pos same-chapter pool"
                );
            }
        }
    }

    #[test]
    fn test_distractor_fallback_widens_in_strict_order() {
        // Exactly one candidate per policy level 1..3, so the picks are
        // forced regardless of the seed: widening fills 1 -> 2 -> 3.
        let cards = vec![
            card("t", "λόγος", "word", 1, "noun", &["athenaze"]),
            card("p1", "ἄνθρωπος", "man", 1, "noun", &["athenaze"]), // policy 1
            card("p2", "νῆσος", "island", 2, "noun", &["athenaze"]), // policy 2 (shared tag)
            card("p3", "πόλις", "city", 9, "noun", &["other"]),      // policy 3
            card("p4", "λύω", "I loose", 1, "verb", &["athenaze"]),  // policy 4
            card("p5", "γράφω", "I write", 1, "verb", &["athenaze"]), // policy 4
        ];
        for seed in [1u64, 42, 1000] {
            let set = generate_mcq_set(&cards, 1, seed);
            assert_eq!(set.len(), 1);
            let ds = distractor_fronts(&set[0]);
            assert!(ds.contains(&"ἄνθρωπος"), "policy-1 peer first: {ds:?}");
            assert!(ds.contains(&"νῆσος"), "policy-2 peer fills next: {ds:?}");
            assert!(ds.contains(&"πόλις"), "policy-3 peer fills last: {ds:?}");
            assert!(
                !ds.contains(&"λύω") && !ds.contains(&"γράφω"),
                "policy-4 reached only when narrower pools are exhausted: {ds:?}"
            );
        }
    }

    #[test]
    fn test_distractor_fallback_reaches_any_card() {
        // No policy-1/2/3 candidates at all: three verbs must fill in.
        let cards = vec![
            card("t", "λόγος", "word", 1, "noun", &["athenaze"]),
            card("v1", "λύω", "I loose", 2, "verb", &["other"]),
            card("v2", "γράφω", "I write", 2, "verb", &["other"]),
            card("v3", "λέγω", "I say", 2, "verb", &["other"]),
        ];
        let set = generate_mcq_set(&cards, 1, 5);
        assert_eq!(set.len(), 1);
        let ds = distractor_fronts(&set[0]);
        assert_eq!(ds.len(), 3);
        for d in &ds {
            assert!(["λύω", "γράφω", "λέγω"].contains(d), "policy-4 fallback: {d}");
        }
    }

    #[test]
    fn test_card_omitted_when_fewer_than_three_distractors_exist() {
        let cards = vec![
            card("t", "λόγος", "word", 1, "noun", &["athenaze"]),
            card("a", "ἄνθρωπος", "man", 1, "noun", &["athenaze"]),
            card("b", "θεός", "god", 1, "noun", &["athenaze"]),
        ];
        // Even policy 4 yields only 2 distractors -> the card is omitted.
        assert!(generate_mcq_set(&cards, 5, 1).is_empty());
    }

    #[test]
    fn test_choices_deduped_by_front_text() {
        // d1/d2 share a front, and `dup` shares the TARGET's front: at most
        // one of each may appear among the 4 choices.
        let cards = vec![
            card("t", "λόγος", "word", 1, "noun", &["athenaze"]),
            card("dup", "λόγος", "speech", 1, "noun", &["athenaze"]),
            card("d1", "δόξα", "opinion", 1, "noun", &["athenaze"]),
            card("d2", "δόξα", "glory", 1, "noun", &["athenaze"]),
            card("d3", "χάρις", "grace", 1, "noun", &["athenaze"]),
            card("d4", "τάγμα", "rank", 1, "noun", &["athenaze"]),
        ];
        let set = generate_mcq_set(&cards, 1, 3);
        assert_eq!(set.len(), 1);
        let mcq = &set[0];
        assert_eq!(mcq.choices.len(), 4);
        assert_eq!(
            mcq.choices.iter().filter(|c| c.as_str() == "λόγος").count(),
            1,
            "the correct front appears exactly once: {:?}",
            mcq.choices
        );
        let mut uniq = mcq.choices.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(uniq.len(), 4, "choices must be unique: {:?}", mcq.choices);
    }

    #[test]
    fn test_duplicate_fronts_can_starve_distractor_pool() {
        // 3 candidates but only 2 unique fronts -> omission.
        let cards = vec![
            card("t", "λόγος", "word", 1, "noun", &["athenaze"]),
            card("d1", "δόξα", "opinion", 1, "noun", &["athenaze"]),
            card("d2", "δόξα", "glory", 1, "noun", &["athenaze"]),
            card("d3", "χάρις", "grace", 1, "noun", &["athenaze"]),
        ];
        assert!(generate_mcq_set(&cards, 1, 3).is_empty());
    }

    #[test]
    fn test_empty_front_or_back_cards_are_skipped() {
        let mut cards = vec![
            card("bad1", "", "ghost", 1, "noun", &["athenaze"]),
            card("bad2", "φάντασμα", "", 1, "noun", &["athenaze"]),
            card("bad3", "   ", "whitespace", 1, "noun", &["athenaze"]),
        ];
        for i in 0..4 {
            cards.push(card(
                &format!("g{i}"),
                &format!("λέξη{i}"),
                &format!("gloss{i}"),
                1,
                "noun",
                &["athenaze"],
            ));
        }
        let set = generate_mcq_set(&cards, 10, 11);
        assert_eq!(set.len(), 4, "only the 4 well-formed cards can prompt");
        for mcq in &set {
            assert!(!mcq.prompt_gloss.trim().is_empty());
            assert!(
                mcq.choices.iter().all(|c| !c.trim().is_empty()),
                "empty fronts must never surface as choices: {:?}",
                mcq.choices
            );
            assert!(!mcq.card_id.starts_with("bad"));
        }
    }

    /// 24 cards across 3 chapters x 2 parts of speech: each chapter-pos
    /// bucket has exactly 4 cards, so policy 1 always provides exactly 3
    /// distractors and every card is usable as a prompt.
    fn big_deck() -> Vec<Card> {
        (0..24)
            .map(|i| {
                card(
                    &format!("c{i}"),
                    &format!("λέξη{i}"),
                    &format!("gloss{i}"),
                    (i % 3) + 1,
                    if i % 2 == 0 { "noun" } else { "verb" },
                    &["athenaze"],
                )
            })
            .collect()
    }

    #[test]
    fn test_seed_determinism() {
        let cards = big_deck();
        let a = generate_mcq_set(&cards, 12, 42);
        let b = generate_mcq_set(&cards, 12, 42);
        assert_eq!(a, b, "same seed must reproduce the identical set");
        assert_eq!(a.len(), 12);
        for mcq in &a {
            assert_eq!(mcq.choices.len(), 4);
            assert!(mcq.correct_index < 4);
            let target = cards.iter().find(|c| c.id == mcq.card_id).unwrap();
            assert_eq!(mcq.prompt_gloss, target.back);
            assert_eq!(mcq.choices[mcq.correct_index], target.front);
        }

        // Different seeds reshuffle: across several seeds at least one must
        // differ (12! orderings + choice permutations make a universal
        // collision astronomically unlikely; no per-seed identity asserted).
        let differs = (1..=8u64).any(|seed| generate_mcq_set(&cards, 12, seed) != a);
        assert!(differs, "different seeds should reshuffle the set");
    }

    #[test]
    fn test_windowed_sampling_respects_caller_order_priority() {
        // limit 4 over an 8-card deck: the FIRST window (first 4 eligible
        // cards) must supply every prompt; the second window is untouched.
        let cards = big_deck();
        let first_window: Vec<&str> = cards[..4].iter().map(|c| c.id.as_str()).collect();
        for seed in [1u64, 7, 42, 999] {
            let set = generate_mcq_set(&cards, 4, seed);
            assert_eq!(set.len(), 4);
            for mcq in &set {
                assert!(
                    first_window.contains(&mcq.card_id.as_str()),
                    "seed {seed}: {} must come from the priority window",
                    mcq.card_id
                );
            }
        }
    }

    #[test]
    fn test_grade_typed_matrix() {
        let expected = "ἄνθρωπος";
        // Exact: byte-identical after whitespace cleanup.
        assert_eq!(grade_typed(expected, "ἄνθρωπος"), TypedGrade::Exact);
        assert_eq!(grade_typed(expected, "  ἄνθρωπος  "), TypedGrade::Exact);
        assert_eq!(grade_typed(expected, "\tἄνθρωπος\n"), TypedGrade::Exact);
        assert_eq!(grade_typed("ὁ ἄνθρωπος", "ὁ   ἄνθρωπος"), TypedGrade::Exact);
        // Lenient: accent-stripped and/or case-folded.
        assert_eq!(grade_typed(expected, "ανθρωπος"), TypedGrade::Lenient);
        assert_eq!(grade_typed(expected, "ΑΝΘΡΩΠΟΣ"), TypedGrade::Lenient);
        assert_eq!(grade_typed(expected, " ΑΝΘΡΩΠΟΣ  "), TypedGrade::Lenient);
        // Wrong: any other string.
        assert_eq!(grade_typed(expected, "λόγος"), TypedGrade::Wrong);
        assert_eq!(grade_typed(expected, ""), TypedGrade::Wrong);
        assert_eq!(grade_typed(expected, "ἄνθρωπος κακός"), TypedGrade::Wrong);
    }

    // --- Alphabet Check tests (DESIGN-alphabet-gate.md §8) ---

    /// The real letters deck, so data drift (fewer than 30 eligible cards,
    /// shifted back formats) fails this suite in CI.
    fn real_letters_deck() -> Vec<Card> {
        serde_json::from_str(include_str!("../../../../data/cards/letters.json"))
            .expect("letters.json must parse")
    }

    #[test]
    fn alphabet_check_labels_and_eligibility() {
        let cards = real_letters_deck();
        assert_eq!(cards.len(), 43, "the letters track is 43 cards today");

        // Ch.1-3: label is the text before " — " on the back.
        let alpha = cards.iter().find(|c| c.id == "letters-001").unwrap();
        assert_eq!(alphabet_check_label(alpha).as_deref(), Some("alpha"));
        // Ch.5: romanization after the dash, cut at ',' or '/'.
        let ai = cards.iter().find(|c| c.id == "letters-033").unwrap();
        assert_eq!(alphabet_check_label(ai).as_deref(), Some("ai"));
        // letters-034's back has no romanization token ("ει — a long close
        // e, ..."): the label falls back to the front's standard
        // romanization.
        let ei = cards.iter().find(|c| c.id == "letters-034").unwrap();
        assert_eq!(alphabet_check_label(ei).as_deref(), Some("ei"));
        let iota_sub = cards.iter().find(|c| c.id == "letters-041").unwrap();
        assert_eq!(alphabet_check_label(iota_sub).as_deref(), Some("āi"));

        let eligible = eligible_alphabet_cards(&cards);
        assert!(
            eligible.len() >= 30,
            "data drift: only {} check-eligible letters cards (today 35)",
            eligible.len()
        );
        // Today's exact split: ch.1 7, ch.2 9, ch.3 8, ch.5 11; ch.4 has none.
        let mut per_chapter = [0usize; 6];
        for (c, _) in &eligible {
            per_chapter[c.chapter as usize] += 1;
        }
        assert_eq!(per_chapter[4], 0, "ch.4 diacritics are not check-eligible");
        assert_eq!(
            (per_chapter[1], per_chapter[2], per_chapter[3], per_chapter[5]),
            (7, 9, 8, 11),
            "chapter split of the 35 eligible cards"
        );
        // All ch.5 labels are distinct (no ambiguous NameToGlyph prompts).
        let mut ch5_labels: Vec<&str> = eligible
            .iter()
            .filter(|(c, _)| c.chapter == 5)
            .map(|(_, l)| l.as_str())
            .collect();
        ch5_labels.sort_unstable();
        ch5_labels.dedup();
        assert_eq!(ch5_labels.len(), per_chapter[5]);
        // Eligibility invariant on every entry.
        for (_, label) in &eligible {
            assert!(!label.is_empty());
            assert!(label.chars().count() <= 20);
            assert!(label.split_whitespace().count() <= 2);
        }
    }

    #[test]
    fn alphabet_check_generator() {
        let cards = real_letters_deck();
        let eligible = eligible_alphabet_cards(&cards);
        let glyph_of = |id: &str| -> &str {
            cards.iter().find(|c| c.id == id).unwrap().front.trim()
        };
        let label_of = |id: &str| -> &str {
            eligible
                .iter()
                .find(|(c, _)| c.id == id)
                .map(|(_, l)| l.as_str())
                .expect("all prompts must come from the eligible set")
        };

        // Determinism per seed.
        let a = generate_alphabet_check(&cards, 24, 42);
        let b = generate_alphabet_check(&cards, 24, 42);
        assert_eq!(a, b, "same seed must reproduce the identical check");
        assert!(a.len() >= 20, "near-full check expected, got {}", a.len());

        // Both directions present, in strict alternation.
        assert!(a.iter().any(|q| q.direction == AlphabetDirection::GlyphToName));
        assert!(a.iter().any(|q| q.direction == AlphabetDirection::NameToGlyph));
        assert!(
            a.windows(2)
                .all(|w| w[0].direction != w[1].direction),
            "directions must alternate"
        );

        // >= 3 questions per represented chapter (1, 2, 3, 5).
        for chapter in [1, 2, 3, 5] {
            let in_chapter = a
                .iter()
                .filter(|q| {
                    cards.iter().find(|c| c.id == q.card_id).unwrap().chapter == chapter
                })
                .count();
            assert!(in_chapter >= 3, "chapter {chapter} only has {in_chapter} questions");
        }

        for q in &a {
            // Structural: 4 unique choices, in-range correct index.
            assert_eq!(q.choices.len(), 4);
            assert!(q.correct_index < 4);
            let mut uniq = q.choices.clone();
            uniq.sort();
            uniq.dedup();
            assert_eq!(uniq.len(), 4, "choices unique: {:?}", q.choices);
            // Prompt/choice polarity matches the direction.
            match q.direction {
                AlphabetDirection::GlyphToName => {
                    assert_eq!(q.prompt, glyph_of(&q.card_id));
                    assert_eq!(q.choices[q.correct_index], label_of(&q.card_id));
                }
                AlphabetDirection::NameToGlyph => {
                    assert_eq!(q.prompt, label_of(&q.card_id));
                    assert_eq!(q.choices[q.correct_index], glyph_of(&q.card_id));
                }
            }
        }

        // Different seeds reshuffle.
        assert!(generate_alphabet_check(&cards, 24, 1337) != a);
    }

    #[test]
    fn check_misses_recycle() {
        // A missed check answer flows through the practice submission path
        // exactly like a practice-question miss: review_logs gains a row with
        // source='practice' and rating 1, and the miss creates a memory_state
        // on the Again (sub-day) learning step — see
        // srs::tests::test_submit_review_again_schedules_subday_due for the
        // scheduling guarantee — so the letter resurfaces within the session.
        let db = crate::db::Database::open(":memory:", None).unwrap();
        let srs = crate::srs::SrsEngine::new();
        let cards = real_letters_deck();
        let check = generate_alphabet_check(&cards, 24, 7);
        let q = check.first().expect("a generated question");

        srs.submit_review_with_source(&db, &q.card_id, 1, "practice")
            .unwrap();

        let (rating, source): (i32, String) = db
            .conn()
            .query_row(
                "SELECT rating, source FROM review_logs WHERE card_id = ?1",
                [&q.card_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(rating, 1);
        assert_eq!(source, "practice");

        // Again-step memory state: first rep, first lapse, and a sub-day due
        // date (no midnight stamp), i.e. it is about to be due.
        let (reps, lapses, due_date): (i32, i32, String) = db
            .conn()
            .query_row(
                "SELECT reps, lapses, due_date FROM memory_states WHERE card_id = ?1",
                [&q.card_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!((reps, lapses), (1, 1));
        assert!(
            !due_date.ends_with("T00:00:00Z"),
            "Again step must keep the sub-day time component: {due_date}"
        );
        // ...and the due-review path picks it up (nudge it just past due).
        db.conn()
            .execute(
                "UPDATE memory_states SET due_date = '2000-01-01T00:00:00Z' WHERE card_id = ?1",
                [&q.card_id],
            )
            .unwrap();
        let due: Vec<String> = srs
            .get_due_cards(&db, 10)
            .unwrap()
            .iter()
            .map(|cws| cws.card.id.clone())
            .collect();
        assert!(due.contains(&q.card_id));
    }
}
