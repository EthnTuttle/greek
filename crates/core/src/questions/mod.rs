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
}
