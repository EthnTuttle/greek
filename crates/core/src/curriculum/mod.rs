use crate::db::Database;
use crate::srs::Card;
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

/// A card counts as mastered once its memory-state stability reaches this
/// many days (i.e. it is well entrenched in long-term memory). A card that
/// has merely been seen once does NOT count.
pub const MASTERY_STABILITY_DAYS: f32 = 30.0;

/// Fraction of a chapter's cards that must be learned (not necessarily
/// mature) before the next chapter's unlock requirement counts as met.
pub const UNLOCK_THRESHOLD: f32 = 0.8;

/// Number of learned cards required in a chapter before the next chapter's
/// unlock requirement counts as met: `ceil(UNLOCK_THRESHOLD * card_count)`.
/// A card_count of 0 yields 0, so the requirement is always met — this
/// prevents deadlocks for chapters past card coverage (e.g. Athenaze ch.17+).
fn needed_for_unlock(card_count: i32) -> i32 {
    (UNLOCK_THRESHOLD * card_count as f32).ceil() as i32
}

// --- Alphabet gate ---

/// Number of questions in the Alphabet Check.
pub const ALPHABET_CHECK_N: u32 = 24;
/// Minimum correct answers needed to pass the Alphabet Check.
pub const ALPHABET_CHECK_PASS_MIN: u32 = 22;

/// Letters-track lane thresholds for the organic (no-button) unlock: the
/// learner must have measured letters-practice accuracy of at least 85%
/// over at least 20 attempts, spread across at least 2 distinct UTC days.
const ORGANIC_MIN_PRACTICE: u32 = 20;
const ORGANIC_MIN_ACCURACY: f64 = 0.85;
const ORGANIC_MIN_DAYS: u32 = 2;

/// A letters-track card counts as "familiar" after one passing or failing
/// introduction. Unlike vocabulary chapters this needs no two-day evidence:
/// a single Easy rating must never strand the alphabet track for weeks.
const LETTERS_FAMILIAR_REPS: i32 = 1;

/// How the alphabet gate was satisfied. Persisted in `user_settings` under
/// its serde snake-case name (e.g. `"check_passed"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlphabetGateMethod {
    CheckPassed,
    Organic,
    DeclaredKnown,
    VocabHistory,
}

impl AlphabetGateMethod {
    /// Serde name as stored in the `user_settings.alphabet_gate` value
    /// (identical to the `rename_all = "snake_case"` serialized form; a unit
    /// test pins the two together).
    pub fn name(&self) -> &'static str {
        match self {
            Self::CheckPassed => "check_passed",
            Self::Organic => "organic",
            Self::DeclaredKnown => "declared_known",
            Self::VocabHistory => "vocab_history",
        }
    }

    /// Inverse of [`AlphabetGateMethod::name`]; unknown strings -> None.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "check_passed" => Some(Self::CheckPassed),
            "organic" => Some(Self::Organic),
            "declared_known" => Some(Self::DeclaredKnown),
            "vocab_history" => Some(Self::VocabHistory),
            _ => None,
        }
    }
}

/// Snapshot of the alphabet-gate evaluation for progress UIs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlphabetGateState {
    pub satisfied: bool,
    pub method: Option<AlphabetGateMethod>,
    pub letters_introduced: u32,
    pub letters_total: u32,
    pub letters_practice_total: u32,
    pub letters_practice_correct: u32,
    pub letters_active_days: u32,
    pub letters_chapters_met: u32,
    /// Explicit organic-lane progress: current measured accuracy and each
    /// denominator/threshold, so clients need not duplicate gate policy.
    pub letters_practice_accuracy: f64,
    pub letters_practice_required: u32,
    pub letters_accuracy_required: f64,
    pub letters_active_days_required: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurriculumInfo {
    pub textbook: String,
    pub display_name: String,
    pub total_chapters: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterInfo {
    pub chapter: i32,
    pub title: String,
    pub card_count: i32,
    /// Introduced sufficiently for pacing (see the vocabulary/letters rules
    /// below). This is deliberately separate from long-term mastery.
    pub learning_count: i32,
    /// Display-only long-term measure: stability >= 30 days.
    pub mastered_count: i32,
    pub unlocked: bool,
    /// Whether this chapter has loaded card data. Chapters with no cards
    /// (card_count == 0) are marked unavailable so the UI can render them
    /// as "Coming Soon".
    pub available: bool,
    /// Unlock-gating hint shown for locked chapters whose previous chapter
    /// has too few learned cards. Empty when unlocked (including a newly
    /// auto-unlocked chapter) or when an alphabet-gated chapter otherwise
    /// meets its prerequisite.
    pub unlock_hint: String,
}

/// Returns true if a card belongs to the given curriculum textbook.
/// Matches an exact tag (e.g. "jact") or a volume-prefixed tag
/// (e.g. "athenaze-i"/"athenaze-ii" for the "athenaze" textbook,
/// "dcc-core" for the "dcc" textbook). Using an exact/prefix match
/// prevents volumes from cross-contaminating each other.
pub fn card_in_textbook(card: &Card, textbook: &str) -> bool {
    let prefix = format!("{}-", textbook);
    card.tags
        .iter()
        .any(|t| t == textbook || t.starts_with(&prefix))
}

/// Normalize a vocabulary headword without erasing meaning. Unlike
/// `morphology::normalize_form` (used deliberately for accent-lenient search
/// and typed answers), this key only applies NFC + case folding + final-sigma
/// folding. Accents, breathings and iota subscripts remain significant.
pub fn normalized_headword_preserving_diacritics(front: &str) -> String {
    let cut = front
        .char_indices()
        .find_map(|(i, c)| matches!(c, ',' | '(').then_some(i))
        .unwrap_or(front.len());
    let normalized = front[..cut].to_lowercase().nfc().collect::<String>();
    let normalized = normalized.trim().replace('ς', "σ");
    let normalized = normalized.trim();
    let without_article = normalized
        .split_once(char::is_whitespace)
        .and_then(|(first, rest)| {
            let rest = rest.trim();
            let is_article = matches!(
                crate::morphology::normalize_form(first).as_str(),
                "ο" | "η" | "το"
            );
            let rest_is_greek = rest.chars().next().is_some_and(|c| {
                ('\u{0370}'..='\u{03ff}').contains(&c) || ('\u{1f00}'..='\u{1fff}').contains(&c)
            });
            (is_article && rest_is_greek).then_some(rest)
        })
        .unwrap_or(normalized);
    without_article.to_string()
}

/// Principal-parts status is explicit deck metadata, not an ID naming guess:
/// every shipped PP card has this type and/or tag.
pub fn is_principal_parts(card: &Card) -> bool {
    card.card_type == "PrincipalParts" || card.tags.iter().any(|t| t == "principal-parts")
}

/// The session's cross-deck learning-unit key. Letters are entirely exempt.
/// Plain vocabulary shares its diacritic-preserving citation-form key.
/// Principal parts are separate from plain verbs and dedupe only with another
/// PP card carrying the same full list: Athenaze/JACT store that list on the
/// front; the core deck stores it on the back with a bare verb on the front.
pub fn learning_unit_key(card: &Card) -> Option<String> {
    if card_in_textbook(card, "letters") {
        return None;
    }
    if is_principal_parts(card) {
        let list = if card.front.contains(',') {
            &card.front
        } else {
            &card.back
        };
        let normalized = list.to_lowercase().nfc().collect::<String>();
        return Some(format!(
            "principal-parts:{}",
            normalized.trim().replace('ς', "σ")
        ));
    }
    let key = normalized_headword_preserving_diacritics(&card.front);
    (!key.is_empty()).then_some(key)
}

/// Shared practice-eligibility policy (C4): a letters-track card is ALWAYS
/// practice-eligible; a vocabulary card only once the alphabet gate is
/// satisfied. (`Curriculum::alphabet_gate_state` fails open when the letters
/// track is not loaded, so letters-less builds are never bricked.)
///
/// One policy, applied by every practice surface: the bindings'
/// `generate_practice_mcq` filters its draw pool with it, and
/// `submit_practice_result` refuses to create practice-source state for
/// loaded-deck cards failing it — otherwise a stray practice answer's
/// `memory_states` row would fabricate "vocab history" and trip the Lane-C
/// grandfather. The Alphabet Check generator keeps its OWN letters-only
/// eligibility rules and does not route through the gate.
pub fn practice_eligible(db: &Database, deck: &[Card], card: &Card) -> bool {
    card_in_textbook(card, "letters") || Curriculum::alphabet_gate_state(db, deck).satisfied
}

/// Card ids from `memory_states` matching a simple `WHERE <sql> < ?1` filter
/// (e.g. `"stability >= ?1"` for mastery, `"reps >= ?1"` for letters
/// familiarity). Returns an empty set on any query error.
fn memory_ids_where(
    db: &Database,
    where_sql: &str,
    threshold: f64,
) -> std::collections::HashSet<String> {
    db.conn()
        .prepare(&format!(
            "SELECT card_id FROM memory_states WHERE {where_sql}"
        ))
        .ok()
        .and_then(|mut stmt| {
            stmt.query_map([threshold], |row| row.get::<_, String>(0))
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default()
}

/// Vocabulary cards satisfying all three parts of the pacing predicate:
/// graduated (`reps > lapses`, the same boundary as the learning ladder),
/// reviewed on at least two distinct UTC days, and most recently rated Hard
/// or better. `review_logs` supplies both temporal facts so evidence syncs
/// across devices instead of depending on a local-only memory-state column.
fn learned_vocabulary_ids(db: &Database) -> std::collections::HashSet<String> {
    db.conn()
        .prepare(
            "SELECT ms.card_id
             FROM memory_states ms
             JOIN (
                 SELECT card_id
                 FROM review_logs
                 GROUP BY card_id
                 HAVING COUNT(DISTINCT substr(timestamp, 1, 10)) >= 2
             ) days ON days.card_id = ms.card_id
             WHERE ms.reps > ms.lapses
               AND (
                   SELECT rl.rating
                   FROM review_logs rl
                   WHERE rl.card_id = ms.card_id
                   ORDER BY rl.timestamp DESC, rl.hlc DESC, rl.uuid DESC, rl.id DESC
                   LIMIT 1
               ) >= 2",
        )
        .ok()
        .and_then(|mut stmt| {
            stmt.query_map([], |row| row.get::<_, String>(0))
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default()
}

/// All introduced card IDs and their session learning-unit keys. Kept beside
/// chapter arithmetic so denominator suppression cannot drift from session
/// admission. Lookup cards participate just as they do in session dedupe.
fn memory_units(
    db: &Database,
    cards: &[Card],
) -> (
    std::collections::HashSet<String>,
    std::collections::HashSet<String>,
) {
    let state_ids: std::collections::HashSet<String> = db
        .conn()
        .prepare("SELECT card_id FROM memory_states")
        .ok()
        .map(|mut stmt| {
            stmt.query_map([], |row| row.get::<_, String>(0))
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
                .unwrap_or_default()
        })
        .unwrap_or_default();
    let mut keys = std::collections::HashSet::new();
    for card in cards {
        if state_ids.contains(&card.id) {
            if let Some(key) = learning_unit_key(card) {
                keys.insert(key);
            }
        }
    }
    if let Ok(mut stmt) = db.conn().prepare(
        "SELECT lc.card_id, lc.front FROM lookup_cards lc
         JOIN memory_states ms ON ms.card_id = lc.card_id",
    ) {
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }) {
            for (id, front) in rows.flatten() {
                // A loaded deck ID wins over a coincident lookup row, just
                // as session enrichment prefers bundled content.
                if cards.iter().any(|card| card.id == id) {
                    continue;
                }
                let key = id
                    .strip_prefix("lookup-")
                    .map(normalized_headword_preserving_diacritics)
                    .filter(|key| !key.is_empty())
                    .unwrap_or_else(|| normalized_headword_preserving_diacritics(&front));
                if !key.is_empty() {
                    keys.insert(key);
                }
            }
        }
    }
    (state_ids, keys)
}

pub struct Curriculum;

impl Curriculum {
    /// Get available textbooks
    pub fn get_textbooks() -> Vec<CurriculumInfo> {
        vec![
            // Stage 0: alphabet + Erasmian pronunciation. Ordered FIRST so
            // that onboarding/new-card fill starts with letters before any
            // vocabulary textbook.
            CurriculumInfo {
                textbook: "letters".to_string(),
                display_name: "The Alphabet & Pronunciation".to_string(),
                total_chapters: 5,
            },
            CurriculumInfo {
                textbook: "athenaze".to_string(),
                display_name: "Athenaze: An Introduction to Ancient Greek".to_string(),
                total_chapters: 31,
            },
            CurriculumInfo {
                textbook: "jact".to_string(),
                display_name: "Reading Greek (JACT)".to_string(),
                total_chapters: 20,
            },
            CurriculumInfo {
                textbook: "dcc".to_string(),
                display_name: "Core Vocabulary (unofficial)".to_string(),
                // The 550 core-vocabulary cards are one deck (not a
                // frequency-ordered list; the Fable audit found padded
                // alphabetical repetition and spurious frequency claims),
                // exposed as one pseudo-chapter (chapter 0).
                total_chapters: 1,
            },
        ]
    }

    /// Evaluate the alphabet gate: is vocabulary *new-card introduction*
    /// unlocked? (Due reviews are never gated.)
    ///
    /// DEVIATION from DESIGN-alphabet-gate.md §4.1: the signature takes the
    /// full loaded card slice (`cards`) instead of a bare `letters_total`;
    /// `letters_total` is computed internally, because the letters-chapter
    /// exposure counts also come from the cards. Behavior is the design's.
    ///
    /// Evaluation order:
    /// 1. fail-open when the letters track isn't loaded (`letters_total == 0`),
    ///    so the gate can never brick a build;
    /// 2. the persisted latch (`user_settings.alphabet_gate`) — once latched,
    ///    the gate never re-locks;
    /// 3. Lane C grandfather: any non-letters `memory_states` row exists;
    /// 4. Lane B organic: full letters coverage AND >=85% measured letters
    ///    practice accuracy over >=20 attempts AND activity on >=2 distinct
    ///    UTC days.
    pub fn alphabet_gate_state(db: &Database, cards: &[Card]) -> AlphabetGateState {
        let letters_total = cards
            .iter()
            .filter(|c| card_in_textbook(c, "letters"))
            .count() as u32;

        let letters_introduced: u32 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM memory_states WHERE card_id LIKE 'letters-%'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0) as u32;

        let (letters_practice_total, letters_practice_correct): (u32, u32) = db
            .conn()
            .query_row(
                "SELECT COUNT(*),
                        COALESCE(SUM(CASE WHEN rating >= 3 THEN 1 ELSE 0 END), 0)
                 FROM review_logs
                 WHERE source = 'practice' AND card_id LIKE 'letters-%'",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .map(|(total, correct)| (total as u32, correct as u32))
            .unwrap_or((0, 0));

        let letters_active_days: u32 = db
            .conn()
            .query_row(
                "SELECT COUNT(DISTINCT substr(timestamp, 1, 10))
                 FROM review_logs WHERE card_id LIKE 'letters-%'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0) as u32;

        // Letters chapters where >=80% of the seeded cards have had one
        // introduction (reps >= 1, any source): the exposure progress bar.
        let letters_chapters_met = {
            let familiar_ids = memory_ids_where(db, "reps >= ?1", LETTERS_FAMILIAR_REPS as f64);
            cards
                .iter()
                .filter(|c| card_in_textbook(c, "letters"))
                .fold(
                    std::collections::BTreeMap::<i32, (u32, u32)>::new(),
                    |mut acc, c| {
                        let entry = acc.entry(c.chapter).or_default();
                        entry.0 += 1;
                        if familiar_ids.contains(&c.id) {
                            entry.1 += 1;
                        }
                        acc
                    },
                )
                .values()
                .filter(|&&(total, met)| {
                    total > 0 && (met as i32) >= needed_for_unlock(total as i32)
                })
                .count() as u32
        };

        // 1. Fail-open when the letters deck isn't loaded.
        if letters_total == 0 {
            return AlphabetGateState {
                satisfied: true,
                method: None,
                letters_introduced,
                letters_total,
                letters_practice_total,
                letters_practice_correct,
                letters_active_days,
                letters_chapters_met,
                letters_practice_accuracy: if letters_practice_total == 0 {
                    0.0
                } else {
                    letters_practice_correct as f64 / letters_practice_total as f64
                },
                letters_practice_required: ORGANIC_MIN_PRACTICE,
                letters_accuracy_required: ORGANIC_MIN_ACCURACY,
                letters_active_days_required: ORGANIC_MIN_DAYS,
            };
        }

        // 2. The latch: once written, the gate stays satisfied forever.
        let latched: Option<AlphabetGateMethod> = db
            .conn()
            .query_row(
                "SELECT value FROM user_settings WHERE key = 'alphabet_gate'",
                [],
                |row| row.get::<_, String>(0),
            )
            .ok()
            .and_then(|v| AlphabetGateMethod::from_name(&v));
        if let Some(method) = latched {
            return AlphabetGateState {
                satisfied: true,
                method: Some(method),
                letters_introduced,
                letters_total,
                letters_practice_total,
                letters_practice_correct,
                letters_active_days,
                letters_chapters_met,
                letters_practice_accuracy: if letters_practice_total == 0 {
                    0.0
                } else {
                    letters_practice_correct as f64 / letters_practice_total as f64
                },
                letters_practice_required: ORGANIC_MIN_PRACTICE,
                letters_accuracy_required: ORGANIC_MIN_ACCURACY,
                letters_active_days_required: ORGANIC_MIN_DAYS,
            };
        }

        // 3. Lane C grandfather: any vocabulary/lookup memory state means the
        //    learner had a pre-gate vocabulary habit; auto-satisfy.
        let has_vocab_history: bool = db
            .conn()
            .query_row(
                "SELECT 1 FROM memory_states WHERE card_id NOT LIKE 'letters-%' LIMIT 1",
                [],
                |_| Ok(true),
            )
            .unwrap_or(false);
        if has_vocab_history {
            return AlphabetGateState {
                satisfied: true,
                method: Some(AlphabetGateMethod::VocabHistory),
                letters_introduced,
                letters_total,
                letters_practice_total,
                letters_practice_correct,
                letters_active_days,
                letters_chapters_met,
                letters_practice_accuracy: if letters_practice_total == 0 {
                    0.0
                } else {
                    letters_practice_correct as f64 / letters_practice_total as f64
                },
                letters_practice_required: ORGANIC_MIN_PRACTICE,
                letters_accuracy_required: ORGANIC_MIN_ACCURACY,
                letters_active_days_required: ORGANIC_MIN_DAYS,
            };
        }

        // 4. Lane B organic. Reached only when NO latch row exists; when the
        //    lane recomputes satisfied, persist the latch immediately (C12) so
        //    every reader sees a latched gate and later accuracy dips below
        //    the bar can never flip it back off. Best-effort: a failed write
        //    must not flip this evaluation.
        let organic = letters_introduced >= letters_total
            && letters_practice_total >= ORGANIC_MIN_PRACTICE
            && (letters_practice_correct as f64)
                >= ORGANIC_MIN_ACCURACY * letters_practice_total as f64
            && letters_active_days >= ORGANIC_MIN_DAYS;
        if organic {
            let _ = Self::record_alphabet_gate(db, AlphabetGateMethod::Organic);
        }
        let method = organic.then_some(AlphabetGateMethod::Organic);

        AlphabetGateState {
            satisfied: organic,
            method,
            letters_introduced,
            letters_total,
            letters_practice_total,
            letters_practice_correct,
            letters_active_days,
            letters_chapters_met,
            letters_practice_accuracy: if letters_practice_total == 0 {
                0.0
            } else {
                letters_practice_correct as f64 / letters_practice_total as f64
            },
            letters_practice_required: ORGANIC_MIN_PRACTICE,
            letters_accuracy_required: ORGANIC_MIN_ACCURACY,
            letters_active_days_required: ORGANIC_MIN_DAYS,
        }
    }

    /// Persist the alphabet-gate latch: `user_settings.alphabet_gate` = the
    /// method's serde snake-case name, `alphabet_gate_at` = now (ISO-8601).
    /// Once written it is never re-locked.
    pub fn record_alphabet_gate(db: &Database, method: AlphabetGateMethod) -> anyhow::Result<()> {
        let now = crate::srs::now_iso8601();
        // Routed through setting_put so the latch is HLC-stamped and dual-
        // written to settings_sync + user_settings (participates in sync;
        // local readers of user_settings are unaffected).
        crate::sync::setting_put(db, "alphabet_gate", method.name())?;
        crate::sync::setting_put(db, "alphabet_gate_at", &now)?;
        Ok(())
    }

    /// Alphabet Check verdict: pass at >=22 of >=24.
    pub fn evaluate_alphabet_check(correct: u32, total: u32) -> bool {
        correct >= ALPHABET_CHECK_PASS_MIN && total >= ALPHABET_CHECK_N
    }

    /// Return chapter pacing and display statistics.
    ///
    /// Vocabulary `learning_count` uses the synced-log predicate documented
    /// by [`learned_vocabulary_ids`]. Letters use single-pass familiarity
    /// (`reps >= 1`). `mastered_count` is always the independent display-only
    /// 30-day-stability measure. Meeting 80% of the preceding chapter's
    /// learning count atomically admits and latches the next chapter here;
    /// empty preceding chapters satisfy the rule and never cause deadlock.
    /// The 80% denominator uses only units not currently key-suppressed by an
    /// existing memory state; owned states always count, and latches never
    /// relock if later evidence changes the live fraction.
    pub fn get_chapters(db: &Database, textbook: &str, cards: &[Card]) -> Vec<ChapterInfo> {
        let total_chapters = Self::get_textbooks()
            .iter()
            .find(|t| t.textbook == textbook)
            .map(|t| t.total_chapters)
            .unwrap_or(0);

        let learning_ids = if textbook == "letters" {
            memory_ids_where(db, "reps >= ?1", LETTERS_FAMILIAR_REPS as f64)
        } else {
            learned_vocabulary_ids(db)
        };
        let mastered_ids = memory_ids_where(db, "stability >= ?1", MASTERY_STABILITY_DAYS as f64);
        let (state_ids, memory_keys) = memory_units(db, cards);
        let gate_locked = textbook != "letters" && !Self::alphabet_gate_state(db, cards).satisfied;
        let textbook_cards: Vec<&Card> = cards
            .iter()
            .filter(|c| card_in_textbook(c, textbook))
            .collect();
        let chapter_range: Vec<i32> = if textbook == "dcc" {
            vec![0]
        } else {
            (1..=total_chapters).collect()
        };
        let default_chapter = if textbook == "dcc" { 0 } else { 1 };
        let mut chapters = Vec::new();

        for chapter in chapter_range {
            let stored_unlocked: bool = db
                .conn()
                .query_row(
                    "SELECT unlocked FROM curriculum_progress WHERE textbook = ?1 AND chapter = ?2",
                    rusqlite::params![textbook, chapter],
                    |row| row.get::<_, i32>(0).map(|v| v != 0),
                )
                .unwrap_or(chapter == default_chapter);
            let chapter_cards: Vec<&&Card> = textbook_cards
                .iter()
                .filter(|c| c.chapter == chapter)
                .collect();
            let card_count = chapter_cards.len() as i32;
            let learning_count = chapter_cards
                .iter()
                .filter(|c| learning_ids.contains(&c.id))
                .count() as i32;
            let mastered_count = chapter_cards
                .iter()
                .filter(|c| mastered_ids.contains(&c.id))
                .count() as i32;

            let previous = chapter - 1;
            // Threshold over currently ADMISSIBLE learning units: an
            // unreviewed card whose key is already represented elsewhere is
            // excluded. Cards with their own state always remain included,
            // preventing history/order changes from invalidating learned
            // evidence. With no duplicates this is the original denominator.
            let previous_admissible: Vec<&&Card> = textbook_cards
                .iter()
                .filter(|c| {
                    c.chapter == previous
                        && (state_ids.contains(&c.id)
                            || learning_unit_key(c)
                                .map(|key| !memory_keys.contains(&key))
                                .unwrap_or(true))
                })
                .collect();
            let previous_count = previous_admissible.len() as i32;
            let previous_learning = previous_admissible
                .iter()
                .filter(|c| learning_ids.contains(&c.id))
                .count() as i32;
            let needed = needed_for_unlock(previous_count);
            let prerequisite_met = chapter == default_chapter || previous_learning >= needed;

            // Automatic unlock is a durable latch. Existing unlocked rows
            // are never revoked by a later Again or deleted review history.
            if chapter != default_chapter && !stored_unlocked && !gate_locked && prerequisite_met {
                let _ = Self::unlock_chapter(db, textbook, chapter);
            }
            let unlocked = (stored_unlocked || prerequisite_met) && !gate_locked;
            let unlock_hint = if unlocked {
                String::new()
            } else if gate_locked && chapter == default_chapter {
                "Finish 'The Alphabet & Pronunciation' — or pass the Alphabet Check.".to_string()
            } else if prerequisite_met {
                String::new()
            } else {
                format!(
                    "Learn 80% of Chapter {} to unlock ({}/{})",
                    previous, previous_learning, needed
                )
            };

            chapters.push(ChapterInfo {
                chapter,
                title: chapter_title(textbook, chapter),
                card_count,
                learning_count,
                mastered_count,
                unlocked,
                available: card_count > 0,
                unlock_hint,
            });
        }

        chapters
    }

    /// Whether an API caller may unlock this chapter now.
    ///
    /// This deliberately consumes `get_chapters`' public state rather than
    /// reimplementing learning arithmetic: an already-unlocked chapter is
    /// idempotently accepted; a locked chapter must be available and have an
    /// empty hint (the previous chapter's threshold is met). Vocabulary also
    /// requires the alphabet gate explicitly, because only the first chapter
    /// carries the gate hint while later chapters show prerequisite hints.
    pub fn can_unlock_chapter(db: &Database, textbook: &str, chapter: i32, cards: &[Card]) -> bool {
        if textbook != "letters" && !Self::alphabet_gate_state(db, cards).satisfied {
            return false;
        }
        Self::get_chapters(db, textbook, cards)
            .into_iter()
            .find(|c| c.chapter == chapter)
            .map(|c| c.unlocked || (c.available && c.unlock_hint.is_empty()))
            .unwrap_or(false)
    }

    /// Persist an already-authorized chapter unlock. Callers at public API
    /// boundaries must check `can_unlock_chapter` first.
    pub fn unlock_chapter(db: &Database, textbook: &str, chapter: i32) -> anyhow::Result<()> {
        db.conn().execute(
            "INSERT INTO curriculum_progress (textbook, chapter, unlocked)
             VALUES (?1, ?2, 1)
             ON CONFLICT(textbook, chapter) DO UPDATE SET unlocked = 1",
            rusqlite::params![textbook, chapter],
        )?;
        Ok(())
    }
}

/// Get the title for a given chapter. Returns placeholder titles for now.
fn chapter_title(textbook: &str, chapter: i32) -> String {
    match textbook {
        "letters" => match chapter {
            1 => "The Vowels".to_string(),
            2 => "The Stops".to_string(),
            3 => "The Continuants".to_string(),
            4 => "Breathings & Accents".to_string(),
            5 => "Diphthongs".to_string(),
            _ => format!("Chapter {}", chapter),
        },
        "athenaze" => match chapter {
            1 => "Ο ΔΙΚΑΙΟΠΟΛΙΣ (Dicaeopolis)".to_string(),
            2 => "Ο ΞΑΝΘΙΑΣ (Xanthias)".to_string(),
            3 => "Ο ΑΡΟΤΟΣ (The Plowing)".to_string(),
            4 => "ΠΡΟΣ ΤΗΙ ΚΡΗΝΗΙ (At the Spring)".to_string(),
            5 => "Ο ΛΥΚΟΣ (The Wolf)".to_string(),
            6 => "Ο ΜΥΘΟΣ (The Story)".to_string(),
            7 => "Ο ΚΥΚΛΩΨ (The Cyclops)".to_string(),
            8 => "ΠΡΟΣ ΤΟ ΑΣΤΥ (To the City)".to_string(),
            9 => "Η ΠΑΝΗΓΥΡΙΣ (The Festival)".to_string(),
            10 => "Η ΣΥΜΦΟΡΑ (The Misfortune)".to_string(),
            _ => format!("Chapter {}", chapter),
        },
        "jact" => match chapter {
            1 => "The Insurance Fraud".to_string(),
            2 => "The Insurance Fraud (continued)".to_string(),
            3 => "Aristophanes' Acharnians".to_string(),
            _ => format!("Section {}", chapter),
        },
        "dcc" => "Core Vocabulary (unofficial)".to_string(),
        _ => format!("Chapter {}", chapter),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_card(id: &str, chapter: i32, textbook_tag: &str) -> Card {
        Card {
            id: id.to_string(),
            front: String::new(),
            back: String::new(),
            chapter,
            card_type: "Vocab".to_string(),
            pos: "noun".to_string(),
            tags: vec![textbook_tag.to_string(), format!("chapter-{}", chapter)],
        }
    }

    #[test]
    fn test_get_textbooks() {
        let textbooks = Curriculum::get_textbooks();
        assert_eq!(textbooks.len(), 4);
        // Stage 0 letters track comes first (onboarding priority).
        assert_eq!(textbooks[0].textbook, "letters");
        assert_eq!(textbooks[0].total_chapters, 5);
        assert_eq!(textbooks[1].textbook, "athenaze");
        assert_eq!(textbooks[2].textbook, "jact");
        assert_eq!(textbooks[3].textbook, "dcc");
    }

    #[test]
    fn test_get_chapters() {
        let db = Database::open(":memory:", None).unwrap();
        let chapters = Curriculum::get_chapters(&db, "athenaze", &[]);
        assert_eq!(chapters.len(), 31);
        // Chapter 1 should be unlocked by default
        assert!(chapters[0].unlocked);
        assert_eq!(chapters[0].title, "Ο ΔΙΚΑΙΟΠΟΛΙΣ (Dicaeopolis)");
    }

    #[test]
    fn test_get_chapters_card_counts_and_availability() {
        let db = Database::open(":memory:", None).unwrap();
        let cards = vec![
            sample_card("ath1-001", 1, "athenaze-i"),
            sample_card("ath1-002", 1, "athenaze-i"),
            sample_card("ath17-001", 17, "athenaze-ii"),
        ];
        let chapters = Curriculum::get_chapters(&db, "athenaze", &cards);
        // Chapter 1 has 2 cards, is available; chapter 26 has none, not available.
        assert_eq!(chapters[0].card_count, 2);
        assert!(chapters[0].available);
        assert_eq!(chapters[16].card_count, 1); // chapter 17
        assert!(chapters[16].available);
        assert_eq!(chapters[25].card_count, 0); // chapter 26
        assert!(!chapters[25].available);
    }

    #[test]
    fn test_get_chapters_mastered_count() {
        let db = Database::open(":memory:", None).unwrap();
        // ath1-001 is mastered (stability >= 30); ath1-002 has been reviewed
        // (reps >= 1) but at low stability, so it must NOT count as mastered.
        db.conn()
            .execute(
                "INSERT INTO memory_states (card_id, stability, difficulty, due_date, reps, lapses)
                 VALUES ('ath1-001', 30.0, 5.0, '2024-01-01T00:00:00Z', 3, 0),
                        ('ath1-002', 5.0, 5.0, '2024-01-01T00:00:00Z', 3, 0)",
                [],
            )
            .unwrap();
        let cards = vec![
            sample_card("ath1-001", 1, "athenaze-i"),
            sample_card("ath1-002", 1, "athenaze-i"),
        ];
        let chapters = Curriculum::get_chapters(&db, "athenaze", &cards);
        assert_eq!(chapters[0].card_count, 2);
        assert_eq!(chapters[0].mastered_count, 1);
    }

    #[test]
    fn test_mastery_stability_threshold() {
        let db = Database::open(":memory:", None).unwrap();
        // 29.9 days: below threshold -> not mastered.
        // 30.0 days: at threshold -> mastered.
        db.conn()
            .execute(
                "INSERT INTO memory_states (card_id, stability, difficulty, due_date, reps, lapses)
                 VALUES ('ath1-low', 29.9, 5.0, '2024-01-01T00:00:00Z', 5, 0),
                        ('ath1-high', 30.0, 5.0, '2024-01-01T00:00:00Z', 5, 0)",
                [],
            )
            .unwrap();
        let cards = vec![
            sample_card("ath1-low", 1, "athenaze-i"),
            sample_card("ath1-high", 1, "athenaze-i"),
        ];
        let chapters = Curriculum::get_chapters(&db, "athenaze", &cards);
        assert_eq!(chapters[0].card_count, 2);
        assert_eq!(chapters[0].mastered_count, 1);
    }

    #[test]
    fn test_needed_for_unlock_ceil() {
        // 80% of 5 rounds up to 4.
        assert_eq!(needed_for_unlock(5), 4);
        assert_eq!(needed_for_unlock(4), 4); // ceil(3.2)
        assert_eq!(needed_for_unlock(10), 8);
        assert_eq!(needed_for_unlock(1), 1);
        // 0 cards -> 0 needed, i.e. requirement always met (no deadlock).
        assert_eq!(needed_for_unlock(0), 0);
    }

    #[test]
    fn test_unlock_hint_locked_chapter() {
        let db = Database::open(":memory:", None).unwrap();
        // Chapter 1 has 5 cards, 2 mastered; need ceil(0.8 * 5) = 4.
        db.conn()
            .execute(
                "INSERT INTO memory_states (card_id, stability, difficulty, due_date, reps, lapses)
                 VALUES ('ath1-001', 30.0, 5.0, '2024-01-01T00:00:00Z', 5, 0),
                        ('ath1-002', 30.0, 5.0, '2024-01-01T00:00:00Z', 5, 0)",
                [],
            )
            .unwrap();
        let cards = vec![
            sample_card("ath1-001", 1, "athenaze-i"),
            sample_card("ath1-002", 1, "athenaze-i"),
            sample_card("ath1-003", 1, "athenaze-i"),
            sample_card("ath1-004", 1, "athenaze-i"),
            sample_card("ath1-005", 1, "athenaze-i"),
        ];
        let chapters = Curriculum::get_chapters(&db, "athenaze", &cards);
        // Chapter 2 is locked and the requirement is not met.
        assert!(!chapters[1].unlocked);
        assert_eq!(
            chapters[1].unlock_hint,
            "Learn 80% of Chapter 1 to unlock (0/4)"
        );
        assert!(!Curriculum::can_unlock_chapter(&db, "athenaze", 2, &cards));

        // Once unlocked, the hint clears even though mastery is unchanged.
        Curriculum::unlock_chapter(&db, "athenaze", 2).unwrap();
        let chapters = Curriculum::get_chapters(&db, "athenaze", &cards);
        assert!(chapters[1].unlocked);
        assert_eq!(chapters[1].unlock_hint, "");
        assert!(Curriculum::can_unlock_chapter(&db, "athenaze", 2, &cards));
    }

    #[test]
    fn test_unlock_hint_empty_when_requirement_met() {
        let db = Database::open(":memory:", None).unwrap();
        // Four graduated cards, each passing on two days, meet the learned
        // requirement. Stability is deliberately low: maturity is not a gate.
        for id in ["ath1-001", "ath1-002", "ath1-003", "ath1-004"] {
            db.conn()
                .execute(
                    "INSERT INTO memory_states
                         (card_id, stability, difficulty, last_review, due_date, reps, lapses)
                     VALUES (?1, 2.0, 5.0, '2024-01-02T00:00:00Z',
                             '2024-01-03T00:00:00Z', 5, 0)",
                    [id],
                )
                .unwrap();
            for day in ["2024-01-01", "2024-01-02"] {
                db.conn()
                    .execute(
                        "INSERT INTO review_logs (card_id, timestamp, rating, elapsed_days, scheduled_days)
                         VALUES (?1, ?2, 3, 0.0, 0.0)",
                        rusqlite::params![id, format!("{day}T12:00:00Z")],
                    )
                    .unwrap();
            }
        }
        let cards = vec![
            sample_card("ath1-001", 1, "athenaze-i"),
            sample_card("ath1-002", 1, "athenaze-i"),
            sample_card("ath1-003", 1, "athenaze-i"),
            sample_card("ath1-004", 1, "athenaze-i"),
            sample_card("ath1-005", 1, "athenaze-i"),
            sample_card("ath2-001", 2, "athenaze-i"),
        ];
        let chapters = Curriculum::get_chapters(&db, "athenaze", &cards);
        // First satisfied read admits and durably latches chapter 2.
        assert!(chapters[1].unlocked);
        assert_eq!(chapters[1].unlock_hint, "");
        assert!(Curriculum::can_unlock_chapter(&db, "athenaze", 2, &cards));
    }

    #[test]
    fn test_unlock_hint_empty_for_default_chapters() {
        let db = Database::open(":memory:", None).unwrap();
        // Athenaze chapter 1 is unlocked by default: hint always empty.
        let chapters = Curriculum::get_chapters(&db, "athenaze", &[]);
        assert!(chapters[0].unlocked);
        assert_eq!(chapters[0].unlock_hint, "");
        // JACT chapter 1 likewise.
        let chapters = Curriculum::get_chapters(&db, "jact", &[]);
        assert!(chapters[0].unlocked);
        assert_eq!(chapters[0].unlock_hint, "");
        // DCC chapter 0 is always unlocked: hint always empty.
        let cards = vec![sample_card("dcc-001", 0, "dcc-core")];
        let chapters = Curriculum::get_chapters(&db, "dcc", &cards);
        assert!(chapters[0].unlocked);
        assert_eq!(chapters[0].unlock_hint, "");
    }

    #[test]
    fn test_unlock_hint_empty_when_previous_chapter_has_no_cards() {
        let db = Database::open(":memory:", None).unwrap();
        // Athenaze chapter 16 has no cards (past card coverage), so chapter
        // 17's requirement counts as met and must not deadlock.
        let cards = vec![sample_card("ath17-001", 17, "athenaze-ii")];
        let chapters = Curriculum::get_chapters(&db, "athenaze", &cards);
        assert!(chapters[16].unlocked); // chapter 17: empty prerequisite never deadlocks
        assert_eq!(chapters[16].unlock_hint, "");
    }

    #[test]
    fn test_get_chapters_dcc() {
        let db = Database::open(":memory:", None).unwrap();
        let cards = vec![sample_card("dcc-001", 0, "dcc-core")];
        let chapters = Curriculum::get_chapters(&db, "dcc", &cards);
        assert_eq!(chapters.len(), 1);
        assert_eq!(chapters[0].chapter, 0);
        assert_eq!(chapters[0].card_count, 1);
        assert!(chapters[0].available);
        assert!(chapters[0].unlocked);
    }

    #[test]
    fn test_unlock_chapter() {
        let db = Database::open(":memory:", None).unwrap();
        // Nonempty, unlearned chapter 1 keeps chapter 2 closed. (With no
        // loaded cards at all, the no-deadlock rule intentionally opens it.)
        let cards = vec![sample_card("ath1-001", 1, "athenaze-i")];
        let chapters = Curriculum::get_chapters(&db, "athenaze", &cards);
        assert!(!chapters[1].unlocked);

        Curriculum::unlock_chapter(&db, "athenaze", 2).unwrap();

        let chapters = Curriculum::get_chapters(&db, "athenaze", &cards);
        assert!(chapters[1].unlocked);
    }

    /// A Stage-0 letters card: real cards are tagged ["letters", "chapter-N"].
    fn letters_card(id: &str, chapter: i32) -> Card {
        Card {
            id: id.to_string(),
            front: String::new(),
            back: String::new(),
            chapter,
            card_type: "Letter".to_string(),
            pos: "letter".to_string(),
            tags: vec!["letters".to_string(), format!("chapter-{}", chapter)],
        }
    }

    #[test]
    fn test_card_in_textbook_letters() {
        let card = letters_card("letters-010", 2);
        assert!(card_in_textbook(&card, "letters"));
        // ...and it does not leak into the vocabulary textbooks.
        assert!(!card_in_textbook(&card, "athenaze"));
        assert!(!card_in_textbook(&card, "jact"));
        assert!(!card_in_textbook(&card, "dcc"));
        // Vocabulary cards do not leak into the letters track either.
        assert!(!card_in_textbook(
            &sample_card("ath1-001", 1, "athenaze-i"),
            "letters"
        ));
    }

    #[test]
    fn test_get_chapters_letters() {
        let db = Database::open(":memory:", None).unwrap();
        // Mirror data/cards/letters.json shape: 7 ch.1 cards + 1 per later
        // chapter so every chapter is available.
        let mut cards: Vec<Card> = (1..=7)
            .map(|i| letters_card(&format!("letters-{i:03}"), 1))
            .collect();
        cards.push(letters_card("letters-020", 2));
        cards.push(letters_card("letters-026", 3));
        cards.push(letters_card("letters-030", 4));
        cards.push(letters_card("letters-040", 5));

        let chapters = Curriculum::get_chapters(&db, "letters", &cards);
        assert_eq!(chapters.len(), 5);
        assert!(chapters.iter().all(|c| c.available));
        let titles: Vec<&str> = chapters.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "The Vowels",
                "The Stops",
                "The Continuants",
                "Breathings & Accents",
                "Diphthongs"
            ]
        );
        // Chapter 1 is unlocked by default, with no hint.
        assert!(chapters[0].unlocked);
        assert_eq!(chapters[0].unlock_hint, "");
        // Chapter 2 is locked; none of the 7 ch.1 cards is mastered, and
        // ceil(0.8 * 7) = 6 are needed.
        assert!(!chapters[1].unlocked);
        assert!(chapters[1].unlock_hint.contains("Chapter 1"));
        assert_eq!(
            chapters[1].unlock_hint,
            "Learn 80% of Chapter 1 to unlock (0/6)"
        );
    }

    #[test]
    fn test_letters_unlock_chain_with_mastered_chapter_1() {
        let db = Database::open(":memory:", None).unwrap();
        // Master all 7 chapter-1 letters (stability >= MASTERY_STABILITY_DAYS).
        for i in 1..=7 {
            db.conn()
                .execute(
                    "INSERT INTO memory_states (card_id, stability, difficulty, due_date, reps, lapses)
                     VALUES (?1, 30.0, 5.0, '2024-01-01T00:00:00Z', 5, 0)",
                    [format!("letters-{i:03}")],
                )
                .unwrap();
        }
        let mut cards: Vec<Card> = (1..=7)
            .map(|i| letters_card(&format!("letters-{i:03}"), 1))
            .collect();
        cards.push(letters_card("letters-008", 2));

        let chapters = Curriculum::get_chapters(&db, "letters", &cards);
        assert_eq!(chapters[0].card_count, 7);
        assert_eq!(chapters[0].mastered_count, 7);
        // The requirement is met (7 >= 6), so chapter 2 auto-unlocks;
        // manual unlock remains idempotent.
        assert!(chapters[1].unlocked);
        assert_eq!(chapters[1].unlock_hint, "");

        Curriculum::unlock_chapter(&db, "letters", 2).unwrap();
        let chapters = Curriculum::get_chapters(&db, "letters", &cards);
        assert!(chapters[1].unlocked);
    }

    #[test]
    fn vocabulary_learning_requires_graduation_two_days_and_latest_pass() {
        let db = Database::open(":memory:", None).unwrap();
        for (id, reps, lapses) in [
            ("ath1-good", 3, 1),
            ("ath1-one-day", 3, 0),
            ("ath1-latest-again", 2, 1),
            ("ath1-still-learning", 1, 1),
        ] {
            db.conn()
                .execute(
                    "INSERT INTO memory_states
                         (card_id, stability, difficulty, last_review, due_date, reps, lapses)
                     VALUES (?1, 2.0, 5.0, '2024-01-02T12:00:00Z',
                             '2024-01-03T12:00:00Z', ?2, ?3)",
                    rusqlite::params![id, reps, lapses],
                )
                .unwrap();
        }
        for (id, day, rating) in [
            ("ath1-good", "2024-01-01", 3),
            ("ath1-good", "2024-01-02", 3),
            ("ath1-one-day", "2024-01-02", 3),
            ("ath1-latest-again", "2024-01-01", 3),
            ("ath1-latest-again", "2024-01-02", 1),
            ("ath1-still-learning", "2024-01-01", 3),
            ("ath1-still-learning", "2024-01-02", 3),
        ] {
            db.conn()
                .execute(
                    "INSERT INTO review_logs (card_id, timestamp, rating, elapsed_days, scheduled_days)
                     VALUES (?1, ?2, ?3, 0.0, 0.0)",
                    rusqlite::params![id, format!("{day}T12:00:00Z"), rating],
                )
                .unwrap();
        }
        assert_eq!(
            learned_vocabulary_ids(&db),
            std::collections::HashSet::from(["ath1-good".to_string()])
        );
    }

    #[test]
    fn automatic_unlock_latches_without_30_day_mastery() {
        let db = Database::open(":memory:", None).unwrap();
        let cards = vec![
            sample_card("ath1-001", 1, "athenaze-i"),
            sample_card("ath2-001", 2, "athenaze-i"),
        ];
        db.conn()
            .execute(
                "INSERT INTO memory_states
                     (card_id, stability, difficulty, last_review, due_date, reps, lapses)
                 VALUES ('ath1-001', 1.0, 5.0, '2024-01-02T12:00:00Z',
                         '2024-01-03T12:00:00Z', 2, 0)",
                [],
            )
            .unwrap();
        for day in ["2024-01-01", "2024-01-02"] {
            db.conn()
                .execute(
                    "INSERT INTO review_logs (card_id, timestamp, rating, elapsed_days, scheduled_days)
                     VALUES ('ath1-001', ?1, 3, 0.0, 0.0)",
                    [format!("{day}T12:00:00Z")],
                )
                .unwrap();
        }
        let chapters = Curriculum::get_chapters(&db, "athenaze", &cards);
        assert_eq!(chapters[0].learning_count, 1);
        assert_eq!(chapters[0].mastered_count, 0);
        assert!(chapters[1].unlocked);

        // Once latched, even deleting all evidence cannot auto-relock it.
        db.conn()
            .execute_batch("DELETE FROM memory_states; DELETE FROM review_logs;")
            .unwrap();
        assert!(Curriculum::get_chapters(&db, "athenaze", &cards)[1].unlocked);
        let persisted: i32 = db
            .conn()
            .query_row(
                "SELECT unlocked FROM curriculum_progress
                 WHERE textbook = 'athenaze' AND chapter = 2",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(persisted, 1);
    }

    #[test]
    fn one_easy_letter_satisfies_single_pass_familiarity() {
        let db = Database::open(":memory:", None).unwrap();
        let mut cards: Vec<Card> = (1..=7)
            .map(|i| letters_card(&format!("letters-{i:03}"), 1))
            .collect();
        cards.push(letters_card("letters-008", 2));
        for i in 1..=6 {
            db.conn()
                .execute(
                    "INSERT INTO memory_states
                         (card_id, stability, difficulty, due_date, reps, lapses)
                     VALUES (?1, 15.69, 5.0, '2099-01-01T00:00:00Z', 1, 0)",
                    [format!("letters-{i:03}")],
                )
                .unwrap();
        }
        let chapters = Curriculum::get_chapters(&db, "letters", &cards);
        assert_eq!(chapters[0].learning_count, 6);
        assert_eq!(chapters[0].mastered_count, 0);
        assert!(chapters[1].unlocked, "an Easy first exposure cannot stall");
    }

    // --- Alphabet gate tests (DESIGN-alphabet-gate.md §8) ---

    /// The 43 letters cards mirroring `data/cards/letters.json` chapter
    /// layout (7 vowels, 9 stops, 9 continuants, 7 diacritics, 11 diphthongs).
    fn letters_deck() -> Vec<Card> {
        (1..=43)
            .map(|i| {
                let chapter = match i {
                    1..=7 => 1,
                    8..=16 => 2,
                    17..=25 => 3,
                    26..=32 => 4,
                    _ => 5,
                };
                letters_card(&format!("letters-{i:03}"), chapter)
            })
            .collect()
    }

    /// Full letters coverage (43 memory_states rows) plus `correct + wrong`
    /// letters practice rows spread across `dates`.
    fn seed_organic_lane(db: &Database, dates: &[&str], correct: usize, wrong: usize) {
        for i in 1..=43 {
            db.conn()
                .execute(
                    "INSERT INTO memory_states (card_id, stability, difficulty, due_date, reps, lapses)
                     VALUES (?1, 30.0, 5.0, '2024-01-01T00:00:00Z', 1, 0)",
                    [format!("letters-{i:03}")],
                )
                .unwrap();
        }
        for k in 0..(correct + wrong) {
            let rating = if k < wrong { 1 } else { 4 };
            let ts = format!("{}T10:{:02}:00Z", dates[k % dates.len()], k % 60);
            db.conn()
                .execute(
                    "INSERT INTO review_logs (card_id, timestamp, rating, elapsed_days, scheduled_days, source, uuid, device_id, hlc)
                     VALUES (?1, ?2, ?3, 0.0, 0.0, 'practice', ?4, ?5, ?6)",
                    rusqlite::params![
                        format!("letters-{:03}", (k % 43) + 1),
                        ts,
                        rating,
                        uuid::Uuid::new_v4().to_string(),
                        crate::sync::device_id(db),
                        crate::sync::hlc_now(db).to_string(),
                    ],
                )
                .unwrap();
        }
    }

    #[test]
    fn gate_fresh_db_unsatisfied() {
        let db = Database::open(":memory:", None).unwrap();
        let state = Curriculum::alphabet_gate_state(&db, &letters_deck());
        assert!(!state.satisfied);
        assert_eq!(state.method, None);
        assert_eq!(state.letters_total, 43);
        assert_eq!(state.letters_introduced, 0);
        assert_eq!(state.letters_practice_total, 0);
        assert_eq!(state.letters_practice_correct, 0);
        assert_eq!(state.letters_active_days, 0);
        assert_eq!(state.letters_chapters_met, 0);
    }

    #[test]
    fn gate_fail_open_without_letters_deck() {
        // letters_total == 0 (deck not loaded): the gate must never brick.
        let db = Database::open(":memory:", None).unwrap();
        let state = Curriculum::alphabet_gate_state(&db, &[]);
        assert!(state.satisfied);
        assert_eq!(state.method, None);
        assert_eq!(state.letters_total, 0);
    }

    #[test]
    fn gate_grandfathered_by_vocab_history() {
        let db = Database::open(":memory:", None).unwrap();
        // One non-letters memory state (the owner's old vocab data) suffices.
        db.conn()
            .execute(
                "INSERT INTO memory_states (card_id, stability, difficulty, due_date, reps, lapses)
                 VALUES ('ath1-001', 30.0, 5.0, '2024-01-01T00:00:00Z', 5, 0)",
                [],
            )
            .unwrap();
        let state = Curriculum::alphabet_gate_state(&db, &letters_deck());
        assert!(state.satisfied);
        assert_eq!(state.method, Some(AlphabetGateMethod::VocabHistory));
        // Letters-only rows must NOT trigger the lane.
        let db2 = Database::open(":memory:", None).unwrap();
        db2.conn()
            .execute(
                "INSERT INTO memory_states (card_id, stability, difficulty, due_date, reps, lapses)
                 VALUES ('letters-001', 30.0, 5.0, '2024-01-01T00:00:00Z', 5, 0)",
                [],
            )
            .unwrap();
        assert!(!Curriculum::alphabet_gate_state(&db2, &letters_deck()).satisfied);
    }

    #[test]
    fn gate_organic_lane() {
        // 43 letters states + 20 practice attempts at 90% across 2 UTC days.
        let db = Database::open(":memory:", None).unwrap();
        seed_organic_lane(&db, &["2026-10-15", "2026-10-16"], 18, 2);
        let state = Curriculum::alphabet_gate_state(&db, &letters_deck());
        assert!(state.satisfied, "organic lane: {:?}", state);
        assert_eq!(state.method, Some(AlphabetGateMethod::Organic));
        assert_eq!(state.letters_introduced, 43);
        assert_eq!(state.letters_practice_total, 20);
        assert_eq!(state.letters_practice_correct, 18);
        assert_eq!(state.letters_active_days, 2);

        // Same accuracy on ONE day only: the spacing hedge fails.
        let one_day = Database::open(":memory:", None).unwrap();
        seed_organic_lane(&one_day, &["2026-10-15"], 18, 2);
        assert!(!Curriculum::alphabet_gate_state(&one_day, &letters_deck()).satisfied);

        // 80% accuracy (16/20) over 2 days: below the 85% bar.
        let sloppy = Database::open(":memory:", None).unwrap();
        seed_organic_lane(&sloppy, &["2026-10-15", "2026-10-16"], 16, 4);
        assert!(!Curriculum::alphabet_gate_state(&sloppy, &letters_deck()).satisfied);

        // Under-20 attempts, even at 100%, is not enough.
        let thin = Database::open(":memory:", None).unwrap();
        seed_organic_lane(&thin, &["2026-10-15", "2026-10-16"], 19, 0);
        assert!(!Curriculum::alphabet_gate_state(&thin, &letters_deck()).satisfied);
    }

    #[test]
    fn gate_organic_lane_latches_on_first_satisfied_read() {
        // C12: reaching the organic thresholds must persist the latch on the
        // evaluating read itself — later accuracy dips can never un-satisfy.
        let db = Database::open(":memory:", None).unwrap();
        seed_organic_lane(&db, &["2026-10-15", "2026-10-16"], 18, 2);

        // No latch row before the first evaluation.
        let before: Option<String> = db
            .conn()
            .query_row(
                "SELECT value FROM user_settings WHERE key = 'alphabet_gate'",
                [],
                |row| row.get(0),
            )
            .ok();
        assert_eq!(before, None);

        let state = Curriculum::alphabet_gate_state(&db, &letters_deck());
        assert!(state.satisfied);
        assert_eq!(state.method, Some(AlphabetGateMethod::Organic));

        // The latch row now exists, written by the organic lane itself.
        let latched: String = db
            .conn()
            .query_row(
                "SELECT value FROM user_settings WHERE key = 'alphabet_gate'",
                [],
                |row| row.get(0),
            )
            .expect("organic lane must latch on first satisfied read");
        assert_eq!(latched, "organic");

        // Degrade accuracy far below the 85% bar (18 correct of 40 = 45%)
        // with 20 later wrong practice attempts: the gate STILL reads
        // satisfied, still via the Organic latch.
        for k in 0..20 {
            let ts = format!("2026-10-{:02}T11:{:02}:00Z", 17 + (k % 2), k % 60);
            db.conn()
                .execute(
                    "INSERT INTO review_logs (card_id, timestamp, rating, elapsed_days, scheduled_days, source, uuid, device_id, hlc)
                     VALUES (?1, ?2, 1, 0.0, 0.0, 'practice', ?3, ?4, ?5)",
                    rusqlite::params![
                        format!("letters-{:03}", (k % 43) + 1),
                        ts,
                        uuid::Uuid::new_v4().to_string(),
                        crate::sync::device_id(&db),
                        crate::sync::hlc_now(&db).to_string(),
                    ],
                )
                .unwrap();
        }
        let state = Curriculum::alphabet_gate_state(&db, &letters_deck());
        assert!(
            state.satisfied,
            "the organic latch must survive later accuracy dips: {state:?}"
        );
        assert_eq!(state.method, Some(AlphabetGateMethod::Organic));
        assert_eq!(state.letters_practice_total, 40);
        assert_eq!(state.letters_practice_correct, 18);
    }

    #[test]
    fn practice_eligible_policy() {
        // Fresh core with the letters deck loaded, gate unsatisfied:
        // letters eligible, vocabulary not.
        let db = Database::open(":memory:", None).unwrap();
        let mut deck = letters_deck();
        let vocab = sample_card("ath1-001", 1, "athenaze-i");
        deck.push(vocab.clone());
        assert!(practice_eligible(&db, &deck, &deck[0]));
        assert!(!practice_eligible(&db, &deck, &vocab));

        // After the latch, vocabulary becomes eligible too.
        Curriculum::record_alphabet_gate(&db, AlphabetGateMethod::DeclaredKnown).unwrap();
        assert!(practice_eligible(&db, &deck, &vocab));

        // No letters deck loaded at all: the gate fails open, so vocabulary
        // is eligible (letters-less builds must never be bricked).
        let bare = Database::open(":memory:", None).unwrap();
        assert!(practice_eligible(&bare, &[vocab.clone()], &vocab));
    }

    #[test]
    fn gate_latch_persists() {
        let db = Database::open(":memory:", None).unwrap();
        // Some letters activity first, then the check-pass latch.
        seed_organic_lane(&db, &["2026-10-15"], 3, 1);
        Curriculum::record_alphabet_gate(&db, AlphabetGateMethod::CheckPassed).unwrap();
        // Wipe all underlying evidence: the latch still holds.
        db.conn()
            .execute_batch("DELETE FROM memory_states; DELETE FROM review_logs;")
            .unwrap();
        let state = Curriculum::alphabet_gate_state(&db, &letters_deck());
        assert!(state.satisfied);
        assert_eq!(state.method, Some(AlphabetGateMethod::CheckPassed));
        assert_eq!(state.letters_introduced, 0);
        let at: String = db
            .conn()
            .query_row(
                "SELECT value FROM user_settings WHERE key = 'alphabet_gate_at'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            at.len() >= 20 && at.contains('T'),
            "ISO-8601 timestamp: {at}"
        );
    }

    #[test]
    fn evaluate_alphabet_check_bounds() {
        assert!(Curriculum::evaluate_alphabet_check(22, 24));
        assert!(Curriculum::evaluate_alphabet_check(24, 24));
        assert!(Curriculum::evaluate_alphabet_check(23, 30));
        assert!(!Curriculum::evaluate_alphabet_check(21, 24));
        // A short check can never pass, however accurate.
        assert!(!Curriculum::evaluate_alphabet_check(23, 23));
    }

    #[test]
    fn alphabet_gate_method_names_match_serde() {
        for (method, name) in [
            (AlphabetGateMethod::CheckPassed, "check_passed"),
            (AlphabetGateMethod::Organic, "organic"),
            (AlphabetGateMethod::DeclaredKnown, "declared_known"),
            (AlphabetGateMethod::VocabHistory, "vocab_history"),
        ] {
            assert_eq!(method.name(), name);
            // The persisted name is exactly the serde snake-case form.
            assert_eq!(
                serde_json::to_string(&method).unwrap(),
                format!("\"{name}\"")
            );
            assert_eq!(AlphabetGateMethod::from_name(name), Some(method));
        }
        assert_eq!(AlphabetGateMethod::from_name("nonsense"), None);
    }

    #[test]
    fn letters_exposure_rule_vs_vocab_30d() {
        let db = Database::open(":memory:", None).unwrap();
        // Identical low-stability rows (stability = 2 days, reps = 2) on
        // letters ch.1 AND athenaze ch.1.
        for i in 1..=7 {
            db.conn()
                .execute(
                    "INSERT INTO memory_states (card_id, stability, difficulty, due_date, reps, lapses)
                     VALUES (?1, 2.0, 5.0, '2024-01-01T00:00:00Z', 2, 0)",
                    [format!("letters-{i:03}")],
                )
                .unwrap();
        }
        for i in 1..=5 {
            db.conn()
                .execute(
                    "INSERT INTO memory_states (card_id, stability, difficulty, due_date, reps, lapses)
                     VALUES (?1, 2.0, 5.0, '2024-01-01T00:00:00Z', 2, 0)",
                    [format!("ath1-{i:03}")],
                )
                .unwrap();
        }
        let mut letters: Vec<Card> = (1..=7)
            .map(|i| letters_card(&format!("letters-{i:03}"), 1))
            .collect();
        letters.push(letters_card("letters-008", 2));
        let vocab: Vec<Card> = (1..=10)
            .map(|i| {
                sample_card(
                    &format!("ath1-{i:03}"),
                    if i <= 5 { 1 } else { 2 },
                    "athenaze-i",
                )
            })
            .collect();

        // Letters: one introduction is enough for learning; all seven meet
        // the chapter, while low stability keeps long-term mastery at zero.
        let chapters = Curriculum::get_chapters(&db, "letters", &letters);
        assert_eq!(chapters[0].learning_count, 7);
        assert_eq!(chapters[0].mastered_count, 0);
        assert!(chapters[1].unlocked);
        assert_eq!(chapters[1].unlock_hint, "");
        let mut all_cards = letters.clone();
        all_cards.extend(vocab.iter().cloned());
        let state = Curriculum::alphabet_gate_state(&db, &all_cards);
        assert_eq!(state.letters_chapters_met, 1);

        // Athenaze: the 30-day stability rule still applies, so the identical
        // rows count for nothing. (The vocab rows also grandfather the gate,
        // so the normal prev-chapter hint shows rather than the gate hint.)
        let chapters = Curriculum::get_chapters(&db, "athenaze", &vocab);
        assert_eq!(chapters[0].mastered_count, 0);
        assert_eq!(
            chapters[1].unlock_hint,
            "Learn 80% of Chapter 1 to unlock (0/4)"
        );
    }

    #[test]
    fn gate_locks_vocab_chapters_with_hint() {
        let db = Database::open(":memory:", None).unwrap();
        let vocab: Vec<Card> = (1..=10)
            .map(|i| {
                sample_card(
                    &format!("ath1-{i:03}"),
                    if i <= 5 { 1 } else { 2 },
                    "athenaze-i",
                )
            })
            .collect();
        // Letters deck loaded (so the gate is live) but no progress at all.
        let mut cards = letters_deck();
        cards.extend(vocab.iter().cloned());

        let chapters = Curriculum::get_chapters(&db, "athenaze", &cards);
        // While the gate is unmet, even the default-unlocked ch.1 reports
        // locked and carries the gate hint; later chapters keep their
        // existing prev-chapter hints.
        assert!(chapters.iter().all(|c| !c.unlocked));
        assert_eq!(
            chapters[0].unlock_hint,
            "Finish 'The Alphabet & Pronunciation' — or pass the Alphabet Check."
        );
        assert_eq!(
            chapters[1].unlock_hint,
            "Learn 80% of Chapter 1 to unlock (0/4)"
        );
        // After passing the check, ch.1 is back to default-unlocked.
        Curriculum::record_alphabet_gate(&db, AlphabetGateMethod::CheckPassed).unwrap();
        let chapters = Curriculum::get_chapters(&db, "athenaze", &cards);
        assert!(chapters[0].unlocked);
        assert_eq!(chapters[0].unlock_hint, "");
        assert!(!chapters[1].unlocked);
        assert_eq!(
            chapters[1].unlock_hint,
            "Learn 80% of Chapter 1 to unlock (0/4)"
        );
        // The letters track itself is never gate-locked.
        let chapters = Curriculum::get_chapters(&db, "letters", &cards);
        assert!(chapters[0].unlocked);
    }
}
