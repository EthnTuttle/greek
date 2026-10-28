use greek_core::{
    now_iso8601, Analysis, Card, CardWithState, ChapterInfo, Curriculum, CurriculumInfo, Database,
    MorphologyDb, Reader, SearchEngine, SearchResult, SrsEngine, TextSection, Token,
    AlphabetGateMethod, ALPHABET_CHECK_N,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

uniffi::setup_scaffolding!();

/// Errors surfaced to the Kotlin layer. UniFFI turns these into a
/// checked `GreekException` so callers can recover instead of crashing.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum GreekError {
    #[error("database error: {msg}")]
    Database { msg: String },
    #[error("sync error: {0}")]
    Sync(String),
}

impl From<anyhow::Error> for GreekError {
    fn from(e: anyhow::Error) -> Self {
        GreekError::Database {
            msg: e.to_string(),
        }
    }
}

impl From<rusqlite::Error> for GreekError {
    fn from(e: rusqlite::Error) -> Self {
        GreekError::Database {
            msg: e.to_string(),
        }
    }
}

impl From<serde_json::Error> for GreekError {
    fn from(e: serde_json::Error) -> Self {
        GreekError::Sync(format!("json error: {e}"))
    }
}

// --- Data transfer types for UniFFI ---

#[derive(uniffi::Record)]
pub struct CardView {
    pub id: String,
    pub front: String,
    pub back: String,
    pub chapter: i32,
    pub card_type: String,
    pub pos: String,
    pub tags: Vec<String>,
}

#[derive(uniffi::Record)]
pub struct CardWithStateView {
    pub card: CardView,
    pub stability: f32,
    pub difficulty: f32,
    pub due_date: String,
    pub reps: i32,
    pub lapses: i32,
    pub is_new: bool,
}

#[derive(uniffi::Record)]
pub struct AnalysisView {
    pub lemma: String,
    pub lemma_id: i64,
    pub postag: String,
    pub postag_human: String,
    pub definition: String,
    pub confidence: f32,
}

#[derive(uniffi::Record)]
pub struct SearchResultView {
    pub lemma_id: i64,
    pub headword: String,
    pub definition: String,
    pub pos: String,
}

#[derive(uniffi::Record)]
pub struct CurriculumView {
    pub textbook: String,
    pub display_name: String,
    pub total_chapters: i32,
}

#[derive(uniffi::Record)]
pub struct ChapterView {
    pub chapter: i32,
    pub title: String,
    pub card_count: i32,
    pub mastered_count: i32,
    pub unlocked: bool,
    pub available: bool,
    pub unlock_hint: String,
}

#[derive(uniffi::Record)]
pub struct TextSectionView {
    pub id: String,
    pub title: String,
    pub author: String,
    pub text: String,
    pub tokens: Vec<TokenView>,
    pub difficulty: f32,
}

#[derive(uniffi::Record)]
pub struct TokenView {
    pub surface: String,
    pub whitespace_after: bool,
}

#[derive(uniffi::Record)]
pub struct ReviewStats {
    pub due_count: i32,
    pub reviewed_today: i32,
    pub streak_days: i32,
    pub total_cards: i32,
}

#[derive(uniffi::Record)]
pub struct IntervalPreviewView {
    pub again_secs: u32,
    pub hard_secs: u32,
    pub good_secs: u32,
    pub easy_secs: u32,
}

#[derive(uniffi::Record)]
pub struct McqView {
    pub card_id: String,
    pub prompt_gloss: String,
    pub choices: Vec<String>,
    pub correct_index: u8,
}

#[derive(uniffi::Record)]
pub struct PracticeStatsView {
    pub total: u32,
    pub correct: u32,
}

/// One Alphabet Check question. `glyph_to_name` picks the polarity: when
/// true, `prompt` is the Greek glyph(s) and `choices` are letter names;
/// when false, `prompt` is the letter name and `choices` are glyphs.
#[derive(uniffi::Record)]
pub struct AlphabetQuestionView {
    pub card_id: String,
    pub glyph_to_name: bool,
    pub prompt: String,
    pub choices: Vec<String>,
    pub correct_index: u8,
}

/// Alphabet-gate snapshot for the Home banner/progress card. `method` is
/// the latched/evaluated serde name ("check_passed" | "organic" |
/// "declared_known" | "vocab_history"), or "" when there is no method
/// (unsatisfied, or satisfied fail-open with no letters deck loaded).
#[derive(uniffi::Record)]
pub struct AlphabetGateStateView {
    pub satisfied: bool,
    pub method: String,
    pub letters_introduced: u32,
    pub letters_total: u32,
    pub letters_practice_total: u32,
    pub letters_practice_correct: u32,
    pub letters_active_days: u32,
    pub letters_chapters_met: u32,
}

// --- Conversion helpers ---

fn card_to_view(c: &Card) -> CardView {
    CardView {
        id: c.id.clone(),
        front: c.front.clone(),
        back: c.back.clone(),
        chapter: c.chapter,
        card_type: c.card_type.clone(),
        pos: c.pos.clone(),
        tags: c.tags.clone(),
    }
}

fn card_with_state_to_view(cws: &CardWithState) -> CardWithStateView {
    let card = card_to_view(&cws.card);
    match &cws.memory {
        Some(m) => CardWithStateView {
            card,
            stability: m.stability,
            difficulty: m.difficulty,
            due_date: m.due_date.clone(),
            reps: m.reps,
            lapses: m.lapses,
            is_new: false,
        },
        None => CardWithStateView {
            card,
            stability: 0.0,
            difficulty: 0.0,
            due_date: String::new(),
            reps: 0,
            lapses: 0,
            is_new: true,
        },
    }
}

fn analysis_to_view(a: &Analysis) -> AnalysisView {
    AnalysisView {
        lemma: a.lemma.clone(),
        lemma_id: a.lemma_id,
        postag: a.postag.clone(),
        postag_human: a.postag_human.clone(),
        definition: a.definition.clone(),
        confidence: a.confidence,
    }
}

fn search_result_to_view(r: &SearchResult) -> SearchResultView {
    SearchResultView {
        lemma_id: r.lemma_id,
        headword: r.headword.clone(),
        definition: r.definition.clone(),
        pos: r.pos.clone(),
    }
}

fn curriculum_to_view(c: &CurriculumInfo) -> CurriculumView {
    CurriculumView {
        textbook: c.textbook.clone(),
        display_name: c.display_name.clone(),
        total_chapters: c.total_chapters,
    }
}

fn chapter_to_view(c: &ChapterInfo) -> ChapterView {
    ChapterView {
        chapter: c.chapter,
        title: c.title.clone(),
        card_count: c.card_count,
        mastered_count: c.mastered_count,
        unlocked: c.unlocked,
        available: c.available,
        unlock_hint: c.unlock_hint.clone(),
    }
}

fn gate_state_to_view(s: &greek_core::AlphabetGateState) -> AlphabetGateStateView {
    AlphabetGateStateView {
        satisfied: s.satisfied,
        method: s.method.map(|m| m.name().to_string()).unwrap_or_default(),
        letters_introduced: s.letters_introduced,
        letters_total: s.letters_total,
        letters_practice_total: s.letters_practice_total,
        letters_practice_correct: s.letters_practice_correct,
        letters_active_days: s.letters_active_days,
        letters_chapters_met: s.letters_chapters_met,
    }
}

fn section_to_view(t: &TextSection) -> TextSectionView {
    TextSectionView {
        id: t.id.clone(),
        title: t.title.clone(),
        author: t.author.clone(),
        text: t.text.clone(),
        tokens: t.tokens.iter().map(token_to_view).collect(),
        difficulty: t.difficulty,
    }
}

fn token_to_view(t: &Token) -> TokenView {
    TokenView {
        surface: t.surface.clone(),
        whitespace_after: t.whitespace_after,
    }
}

// --- Main API object (Mutex-wrapped for thread safety) ---

#[derive(uniffi::Object)]
pub struct GreekCore {
    db: Mutex<Database>,
    srs: Mutex<SrsEngine>,
    morphology: MorphologyDb,
    search: SearchEngine,
    cards: Vec<Card>,
    /// Path of the app database; `<dir>/iroh-sync/` holds the iroh store.
    db_path: String,
    /// Lazily-opened iroh sync node (never started unless a sync_iroh_*
    /// method is called, so non-sync users pay nothing).
    iroh: Mutex<Option<Arc<sync_iroh::SyncIrohTransport>>>,
    /// Index of card id -> position in `cards`, built once at construction
    /// so due-card enrichment is O(1) per card instead of O(n).
    card_index: HashMap<String, usize>,
}

/// Build a card id -> index map for O(1) lookups.
fn build_card_index(cards: &[Card]) -> HashMap<String, usize> {
    cards
        .iter()
        .enumerate()
        .map(|(i, c)| (c.id.clone(), i))
        .collect()
}

#[uniffi::export]
impl GreekCore {
    #[uniffi::constructor]
    pub fn new(
        db_path: String,
        morphology_db_path: Option<String>,
    ) -> Result<Arc<Self>, GreekError> {
        let db = Database::open(&db_path, morphology_db_path.as_deref())?;
        let srs = SrsEngine::new();
        let morphology = MorphologyDb::new();
        let search = SearchEngine::new();

        Ok(Arc::new(Self {
            db: Mutex::new(db),
            srs: Mutex::new(srs),
            morphology,
            search,
            cards: Vec::new(),
            card_index: HashMap::new(),
            db_path,
            iroh: Mutex::new(None),
        }))
    }

    #[uniffi::constructor]
    pub fn new_with_cards(
        db_path: String,
        morphology_db_path: Option<String>,
        cards_json: String,
    ) -> Result<Arc<Self>, GreekError> {
        let db = Database::open(&db_path, morphology_db_path.as_deref())?;
        let srs = SrsEngine::new();
        let morphology = MorphologyDb::new();
        let search = SearchEngine::new();
        let cards: Vec<Card> = serde_json::from_str(&cards_json).unwrap_or_default();
        let card_index = build_card_index(&cards);

        Ok(Arc::new(Self {
            db: Mutex::new(db),
            srs: Mutex::new(srs),
            morphology,
            search,
            cards,
            card_index,
            db_path,
            iroh: Mutex::new(None),
        }))
    }

    // --- SRS Operations ---

    pub fn get_due_cards(&self, limit: i32) -> Result<Vec<CardWithStateView>, GreekError> {
        let db = self.db.lock().unwrap();
        let srs = self.srs.lock().unwrap();
        let due = srs.get_due_cards(&db, limit as usize)?;

        let mut result: Vec<CardWithStateView> = Vec::new();
        for cws in &due {
            let mut view = card_with_state_to_view(cws);
            if let Some(&idx) = self.card_index.get(&cws.card.id) {
                view.card = card_to_view(&self.cards[idx]);
            } else if let Ok((front, back)) = db.conn().query_row(
                "SELECT front, back FROM lookup_cards WHERE card_id = ?1",
                [&cws.card.id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            ) {
                // Card added via the reader's "Add to SRS" lookup flow.
                view.card.front = front;
                view.card.back = back;
            }
            result.push(view);
        }

        Ok(result)
    }

    /// DEPRECATED for session construction (C3): each per-chapter call
    /// re-consults the daily new-card budget independently, so a caller
    /// looping chapters can admit several caps' worth of new cards in one
    /// session (observed: 7 Letters + 10 Athenaze + 3 JACT under a 10/day
    /// cap). Session building MUST go through [`Self::build_review_session`];
    /// this entry point stays for chapter-scoped browsing compatibility.
    pub fn get_new_cards(&self, textbook: String, chapter: i32, limit: i32) -> Vec<CardView> {
        let db = self.db.lock().unwrap();

        // Alphabet gate: vocabulary new-card introduction is refused until
        // the learner proves alphabet fluency. (Due reviews via
        // get_due_cards are never gated.) The refusal lives here in the core
        // so every current and future UI path is capped consistently.
        if textbook != "letters" && !Curriculum::alphabet_gate_state(&db, &self.cards).satisfied {
            return Vec::new();
        }

        // Enforce the daily new-card cap at the Rust layer so every UI path
        // is capped consistently.
        let remaining = greek_core::srs::new_cards_remaining_today(&db);
        if remaining == 0 {
            return Vec::new();
        }
        let effective_limit = (limit.max(0) as u32).min(remaining) as usize;

        let chapter_cards: Vec<&Card> = self
            .cards
            .iter()
            .filter(|c| c.chapter == chapter && greek_core::card_in_textbook(c, &textbook))
            .collect();

        let has_state: std::collections::HashSet<String> = db
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

        chapter_cards
            .into_iter()
            .filter(|c| !has_state.contains(&c.id))
            .take(effective_limit)
            .map(|c| card_to_view(c))
            .collect()
    }

    /// Build a full review session in ONE pass (C3 fix).
    ///
    /// Replaces the Android-side `get_due_cards` + per-chapter `get_new_cards`
    /// loop, in which every chapter call re-saw the full unspent daily budget.
    /// UI-visible ordering matches the old semantics: due cards first (most
    /// overdue first, never gated, never counting toward the new-card cap),
    /// then new cards in curriculum order (letters → athenaze → jact → dcc),
    /// chapter order within a curriculum, deck order within a chapter —
    /// consuming a SINGLE daily-budget snapshot ([`greek_core::srs::SessionBudget`]),
    /// so one session introduces at most the day's true remaining allowance.
    pub fn build_review_session(
        &self,
        session_size: u32,
    ) -> Result<Vec<CardWithStateView>, GreekError> {
        let db = self.db.lock().unwrap();
        let srs = self.srs.lock().unwrap();
        let size = session_size as usize;

        let mut result: Vec<CardWithStateView> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

        // 1. Due cards first — never alphabet-gated, never budget-capped.
        //    (Same content enrichment as get_due_cards: in-memory deck first,
        //    then the reader's lookup_cards table.)
        for cws in srs.get_due_cards(&db, size)? {
            let mut view = card_with_state_to_view(&cws);
            if let Some(&idx) = self.card_index.get(&cws.card.id) {
                view.card = card_to_view(&self.cards[idx]);
            } else if let Ok((front, back)) = db.conn().query_row(
                "SELECT front, back FROM lookup_cards WHERE card_id = ?1",
                [&cws.card.id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            ) {
                view.card.front = front;
                view.card.back = back;
            }
            seen.insert(cws.card.id.clone());
            result.push(view);
        }
        if result.len() >= size {
            result.truncate(size);
            return Ok(result);
        }

        // 2. New cards across curricula from ONE budget snapshot (C3): the
        //    daily allowance is consulted once here and decremented per
        //    admitted card, so spanning chapters/curricula cannot re-arm it.
        let mut budget = greek_core::srs::SessionBudget::open(&db);

        let has_state: std::collections::HashSet<String> = db
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

        'chapters: for textbook in Curriculum::get_textbooks() {
            let name = textbook.textbook;
            // Alphabet gate per track (same rule as get_new_cards).
            if name != "letters" && !Curriculum::alphabet_gate_state(&db, &self.cards).satisfied {
                continue;
            }
            for ch in Curriculum::get_chapters(&db, &name, &self.cards) {
                if !ch.unlocked || !ch.available {
                    continue;
                }
                for card in self
                    .cards
                    .iter()
                    .filter(|c| c.chapter == ch.chapter && greek_core::card_in_textbook(c, &name))
                {
                    if result.len() >= size
                        || has_state.contains(&card.id)
                        || seen.contains(&card.id)
                    {
                        if result.len() >= size {
                            break 'chapters;
                        }
                        continue;
                    }
                    if !budget.try_admit() {
                        // Daily new-card budget spent: no later chapter can
                        // admit either — stop the whole build.
                        return Ok(result);
                    }
                    result.push(card_with_state_to_view(&CardWithState {
                        card: card.clone(),
                        memory: None,
                    }));
                    seen.insert(card.id.clone());
                }
            }
        }

        Ok(result)
    }

    pub fn submit_review(&self, card_id: String, rating: u8) -> bool {
        let db = self.db.lock().unwrap();
        let srs = self.srs.lock().unwrap();
        let ok = srs.submit_review(&db, &card_id, rating).is_ok();
        if ok {
            // Progress sharing (W9): reviews.* thresholds were REMOVED from
            // the milestone set (Appendix R); this hook only marks today's
            // streak dirty so the lazy detector (get_review_stats / flush)
            // re-checks it. No-op when sharing is disabled — see
            // share_hook_review_submitted.
            share_hook_review_submitted(&db);
        }
        ok
    }

    pub fn get_review_stats(&self) -> ReviewStats {
        let db = self.db.lock().unwrap();

        // Use the SAME now-string format that get_due_cards compares against
        // (ISO-8601 with a 'T'), so same-day due cards are counted correctly.
        let now = now_iso8601();
        let due_count: i32 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM memory_states WHERE due_date <= ?1",
                [&now],
                |row| row.get(0),
            )
            .unwrap_or(0);

        // Today's date prefix (YYYY-MM-DD) for comparing ISO-8601 timestamps.
        let today = &now[..10.min(now.len())];
        let reviewed_today: i32 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM review_logs WHERE timestamp >= ?1",
                [today],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let streak_days = compute_streak_days(&db);

        // Progress sharing (W9): this is the bindings call the Android home
        // screen makes on foreground — the designated lazy detection point
        // for streak.{7,30,100} and chapter.mastered (Appendix R). Cheap:
        // the engine caches its last detection pass. No-op when disabled.
        share_hook_stats_refresh(&db, &self.cards);

        let total_cards = self.cards.len() as i32;

        ReviewStats {
            due_count,
            reviewed_today,
            streak_days,
            total_cards,
        }
    }

    /// Preview the next-review interval (seconds from now) for each rating
    /// of the given card. Returns None when the card has no scheduling state.
    pub fn preview_intervals(&self, card_id: String) -> Option<IntervalPreviewView> {
        let db = self.db.lock().unwrap();
        let srs = self.srs.lock().unwrap();
        // Core now propagates SQL/decode errors; the UniFFI surface keeps
        // its Option shape, mapping any error to None.
        srs.preview_intervals(&db, &card_id)
            .ok()
            .flatten()
            .map(|p| IntervalPreviewView {
                again_secs: p.again_secs,
                hard_secs: p.hard_secs,
                good_secs: p.good_secs,
                easy_secs: p.easy_secs,
            })
    }

    /// Number of cards introduced (first-ever review) on today's date.
    pub fn new_cards_today(&self) -> u32 {
        let db = self.db.lock().unwrap();
        greek_core::srs::new_cards_today(&db)
    }

    /// Remaining new-card allowance for today under the daily cap.
    pub fn new_cards_remaining_today(&self) -> u32 {
        let db = self.db.lock().unwrap();
        greek_core::srs::new_cards_remaining_today(&db)
    }

    // --- Practice questions (measured assessment) ---

    /// Generate up to `limit` EN→GR multiple-choice questions from the
    /// in-memory deck, honoring the alphabet-gate practice policy (C4):
    /// letters-track cards are always eligible, vocabulary cards only once
    /// the gate is satisfied. Within the eligible pool, cards that already
    /// have a memory state are offered first; each tier is generated by a
    /// SEPARATE seeded call whose shuffle sees the FULL tier (C5), so
    /// question sets are no longer pinned to the first `limit` cards in
    /// deck order. The seed is unix-epoch minutes, so question sets vary
    /// per session.
    pub fn generate_practice_mcq(&self, limit: u32) -> Vec<McqView> {
        let db = self.db.lock().unwrap();
        let limit = limit as usize;
        if limit == 0 {
            return Vec::new();
        }

        // C4: one shared policy for the draw pool — letters always,
        // vocabulary only with the gate satisfied (fails open when the
        // letters track is not loaded). The Alphabet Check keeps its own
        // separate letters-only generator.
        let eligible: Vec<&Card> = self
            .cards
            .iter()
            .filter(|c| greek_core::curriculum::practice_eligible(&db, &self.cards, c))
            .collect();

        let have_state: std::collections::HashSet<String> = db
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

        // Reviewed cards first, then unseen — one generator call per tier
        // (C5) over a SHARED distractor pool: prompts come only from the
        // tier, distractors from the whole eligible set, so a small
        // reviewed tier never starves its own distractors.
        let pool: Vec<Card> = eligible.iter().map(|c| (*c).clone()).collect();
        let reviewed: Vec<Card> = eligible
            .iter()
            .filter(|c| have_state.contains(&c.id))
            .map(|c| (*c).clone())
            .collect();
        let unseen: Vec<Card> = eligible
            .iter()
            .filter(|c| !have_state.contains(&c.id))
            .map(|c| (*c).clone())
            .collect();

        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() / 60)
            .unwrap_or(0);

        let mut mcqs =
            greek_core::questions::generate_mcq_set_with_pool(&reviewed, &pool, limit, seed);
        if mcqs.len() < limit {
            mcqs.extend(greek_core::questions::generate_mcq_set_with_pool(
                &unseen,
                &pool,
                limit - mcqs.len(),
                seed,
            ));
        }
        mcqs.into_iter()
            .map(|m| McqView {
                card_id: m.card_id,
                prompt_gloss: m.prompt_gloss,
                choices: m.choices,
                correct_index: m.correct_index as u8,
            })
            .collect()
    }

    /// Accent-tolerant typed-answer check: true when the answer grades Exact
    /// or Lenient against the card's Greek front. Unknown card -> false.
    pub fn check_typed_answer(&self, card_id: String, answer: String) -> bool {
        match self.card_index.get(&card_id) {
            Some(&i) => {
                greek_core::questions::grade_typed(&self.cards[i].front, &answer)
                    != greek_core::questions::TypedGrade::Wrong
            }
            None => false,
        }
    }

    /// Strict typed-answer check: true only on an Exact grade (accents must
    /// match). Unknown card -> false.
    pub fn typed_answer_exact(&self, card_id: String, answer: String) -> bool {
        match self.card_index.get(&card_id) {
            Some(&i) => {
                greek_core::questions::grade_typed(&self.cards[i].front, &answer)
                    == greek_core::questions::TypedGrade::Exact
            }
            None => false,
        }
    }

    /// Record a practice outcome as an SRS review tagged source="practice":
    /// wrong -> 1 (Again), correct -> 3 (Good), correct + exact -> 4 (Easy).
    /// Returns false on error, or when the card fails the alphabet-gate
    /// practice policy (C4 belt): a practice-source memory state for an
    /// ineligible vocabulary card would fabricate "vocab history" and
    /// falsely trip the gate's Lane-C grandfather. Card ids outside the
    /// loaded deck keep legacy behavior (no policy data to judge them).
    pub fn submit_practice_result(&self, card_id: String, correct: bool, exact: bool) -> bool {
        let rating: u8 = if !correct {
            1
        } else if exact {
            4
        } else {
            3
        };
        let db = self.db.lock().unwrap();
        let srs = self.srs.lock().unwrap();
        if let Some(&i) = self.card_index.get(&card_id) {
            if !greek_core::curriculum::practice_eligible(&db, &self.cards, &self.cards[i]) {
                return false;
            }
        }
        srs.submit_review_with_source(&db, &card_id, rating, "practice")
            .is_ok()
    }

    /// Lifetime measured-assessment totals: (total attempts, correct ones).
    pub fn get_practice_stats(&self) -> PracticeStatsView {
        let db = self.db.lock().unwrap();
        let (total, correct) = greek_core::srs::practice_stats(&db);
        PracticeStatsView { total, correct }
    }

    // --- Morphology Operations ---

    pub fn lookup_word(&self, form: String) -> Vec<AnalysisView> {
        let db = self.db.lock().unwrap();
        self.morphology
            .lookup(db.conn(), &form)
            .iter()
            .map(analysis_to_view)
            .collect()
    }

    // --- Search Operations ---

    pub fn search_dictionary(&self, query: String, limit: i32) -> Vec<SearchResultView> {
        let db = self.db.lock().unwrap();
        self.search
            .search(db.conn(), &query, limit as usize)
            .iter()
            .map(search_result_to_view)
            .collect()
    }

    // --- Curriculum Operations ---

    pub fn get_curricula(&self) -> Vec<CurriculumView> {
        Curriculum::get_textbooks()
            .iter()
            .map(curriculum_to_view)
            .collect()
    }

    pub fn get_chapters(&self, textbook: String) -> Vec<ChapterView> {
        let db = self.db.lock().unwrap();
        Curriculum::get_chapters(&db, &textbook, &self.cards)
            .iter()
            .map(chapter_to_view)
            .collect()
    }

    pub fn unlock_chapter(&self, textbook: String, chapter: i32) -> bool {
        let db = self.db.lock().unwrap();
        // Alphabet gate: manual unlocks on vocabulary tracks are refused
        // while the gate is unmet.
        if textbook != "letters" && !Curriculum::alphabet_gate_state(&db, &self.cards).satisfied {
            return false;
        }
        Curriculum::unlock_chapter(&db, &textbook, chapter).is_ok()
    }

    // --- Alphabet gate (DESIGN-alphabet-gate.md) ---

    /// Generate the Alphabet Check: 24 stratified glyph↔name questions over
    /// the letters track. The seed is unix-epoch minutes, so each attempt
    /// shuffles a fresh sample. Each answer should be reported through
    /// `submit_practice_result` (misses become FSRS-Again items), and the
    /// verdict through `record_alphabet_check_result`.
    pub fn generate_alphabet_check(&self) -> Vec<AlphabetQuestionView> {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() / 60)
            .unwrap_or(0);
        greek_core::questions::generate_alphabet_check(&self.cards, ALPHABET_CHECK_N as usize, seed)
            .into_iter()
            .map(|q| AlphabetQuestionView {
                card_id: q.card_id,
                glyph_to_name: q.direction == greek_core::questions::AlphabetDirection::GlyphToName,
                prompt: q.prompt,
                choices: q.choices,
                correct_index: q.correct_index as u8,
            })
            .collect()
    }

    /// Record an Alphabet Check verdict: on a pass (>=22 of >=24) the gate
    /// latches as check_passed. Returns the gate's satisfied flag afterward.
    pub fn record_alphabet_check_result(&self, correct: u32, total: u32) -> bool {
        let db = self.db.lock().unwrap();
        let passed = Curriculum::evaluate_alphabet_check(correct, total);
        if passed {
            let _ = Curriculum::record_alphabet_gate(&db, AlphabetGateMethod::CheckPassed);
            // Progress sharing (W9): the latch transition just above is
            // `gate.passed`'s only trigger. No-op when sharing is disabled.
            share_hook_gate_latched(&db);
        }
        Curriculum::alphabet_gate_state(&db, &self.cards).satisfied
    }

    /// Lane C skip affordance ("I already know the Greek alphabet"): latches
    /// the gate as declared_known. Returns the satisfied flag afterward.
    pub fn declare_alphabet_known(&self) -> bool {
        let db = self.db.lock().unwrap();
        let _ = Curriculum::record_alphabet_gate(&db, AlphabetGateMethod::DeclaredKnown);
        Curriculum::alphabet_gate_state(&db, &self.cards).satisfied
    }

    /// Gate snapshot for the Home banner / progress card.
    pub fn get_alphabet_gate_state(&self) -> AlphabetGateStateView {
        let db = self.db.lock().unwrap();
        gate_state_to_view(&Curriculum::alphabet_gate_state(&db, &self.cards))
    }

    // --- Reader Operations ---

    pub fn get_text_sections(&self) -> Vec<TextSectionView> {
        Reader::get_sections()
            .iter()
            .map(section_to_view)
            .collect()
    }

    // --- Known-words tracking (reader) ---

    /// Mark a word (lemma or surface form) as known/unknown for the user.
    pub fn mark_word_known(&self, word: String, known: bool) -> Result<(), GreekError> {
        let db = self.db.lock().unwrap();
        db.conn().execute(
            "INSERT INTO known_words (word, known, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(word) DO UPDATE SET known = excluded.known, updated_at = excluded.updated_at",
            rusqlite::params![word, known as i32, now_iso8601()],
        )?;
        Ok(())
    }

    /// Returns true if the word has been explicitly marked known.
    pub fn is_word_known(&self, word: String) -> bool {
        let db = self.db.lock().unwrap();
        db.conn()
            .query_row(
                "SELECT known FROM known_words WHERE word = ?1",
                [&word],
                |row| row.get::<_, i32>(0).map(|v| v != 0),
            )
            .unwrap_or(false)
    }

    /// Return all words the user has marked as known.
    pub fn get_known_words(&self) -> Vec<String> {
        let db = self.db.lock().unwrap();
        db.conn()
            .prepare("SELECT word FROM known_words WHERE known = 1")
            .ok()
            .and_then(|mut stmt| {
                stmt.query_map([], |row| row.get::<_, String>(0))
                    .ok()
                    .map(|rows| rows.filter_map(|r| r.ok()).collect())
            })
            .unwrap_or_default()
    }

    // --- Add card from reader lookup ---

    /// Create a new SRS card for a word looked up in the reader. The card id
    /// is derived from the lemma so repeated lookups of the same word are
    /// idempotent (the content row upserts, nothing else is written).
    ///
    /// C2: NO `memory_states` row is created here. A row's *presence* means
    /// "this card has a learned FSRS state replayable from `review_logs`";
    /// the old zero-state row (stability/difficulty 0.0, reps 0) made every
    /// initial preview a bogus fixed interval and made the first real review
    /// diverge from `sync::rebuild_memory_states` replay. Scheduling state
    /// appears with the first review (`SrsEngine::submit_review` upserts it);
    /// until then the card behaves exactly like any never-reviewed new card.
    pub fn add_card_from_lookup(
        &self,
        lemma: String,
        form: String,
        definition: String,
    ) -> Result<String, GreekError> {
        let card_id = format!("lookup-{}", lemma);
        let db = self.db.lock().unwrap();

        // Persist the card content (front = the Greek word, back = definition)
        // so it survives restarts and can be shown when the card is studied.
        // The lemma headword is preferred for the front; fall back to the form.
        let front = if form.is_empty() { lemma.clone() } else { form };
        db.conn().execute(
            "INSERT INTO lookup_cards (card_id, front, back)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(card_id) DO UPDATE SET front = excluded.front, back = excluded.back",
            rusqlite::params![card_id, front, definition],
        )?;

        Ok(card_id)
    }
}

// --- Device sync (DESIGN-device-sync.md §9, SYNC-WIRE.md) ---

/// Stats returned by every sync operation.
#[derive(Debug, Clone, uniffi::Record)]
pub struct SyncStatsView {
    pub reviews_sent: u32,
    pub reviews_received: u32,
    pub settings_applied: u32,
}

/// One entry of the paired-device registry (`sync_devices` table).
#[derive(Debug, Clone, uniffi::Record)]
pub struct PairedDeviceView {
    pub device_id: String,
    pub label: String,
    pub paired_at: String,
    pub last_sync_hlc: String,
}

/// Nostr spool configuration snapshot for the Settings screen.
#[derive(Debug, Clone, uniffi::Record)]
pub struct NostrConfigView {
    pub relays_csv: String,
    pub has_identity: bool,
}

/// Default nostr relay seed when the user has not configured any.
const DEFAULT_NOSTR_RELAYS: &str = "wss://chat.virginiafreedom.tech";

/// Local-only settings helpers. Sync cursors are per-device high-water marks
/// and must NOT ride settings_sync (LWW across devices with drifting clocks
/// could rewind a cursor so far that events are skipped forever, or advance
/// it past unsent local events), so they upsert `user_settings` directly —
/// mirroring how `hlc_now` persists its own state. Push cursors are
/// additionally DESTINATION-scoped as `{prefix}@{fingerprint}` keys managed
/// by `greek_core::sync::push_cursor_*` (S7); only the pull cursor
/// (`nostr_last_pull_hlc`) still uses these helpers directly.
fn user_setting_get(db: &Database, key: &str) -> Option<String> {
    db.conn()
        .query_row(
            "SELECT value FROM user_settings WHERE key = ?1",
            [key],
            |row| row.get::<_, String>(0),
        )
        .ok()
}

fn user_setting_put_local(db: &Database, key: &str, value: &str) -> Result<(), GreekError> {
    db.conn().execute(
        "INSERT INTO user_settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )?;
    Ok(())
}

/// Build a v2 SyncBatch from local state after `since` (None = everything)
/// for the HLC-tracked collections (`reviews`, `settings`). The v2 content
/// collections (`known_words`, `lookup_cards`, `chapter_unlocks`) have no
/// HLC columns, so EVERY batch ships their full row sets — the tables are
/// small and ingress is dedup'ing (known-words LWW on updated_at, PK-dedup
/// inserts for the other two), so convergence needs no cursor (S1).
fn build_batch(db: &Database, since: Option<&str>) -> Result<greek_core::SyncBatch, GreekError> {
    let device_id = greek_core::device_id(db);
    let reviews = greek_core::events_since(db, since)?;
    let settings = greek_core::settings_since(db, since)?;
    let known_words = greek_core::sync::known_words_all(db)?;
    let lookup_cards = greek_core::sync::lookup_cards_all(db)?;
    let chapter_unlocks = greek_core::sync::chapter_unlocks_all(db)?;
    Ok(greek_core::SyncBatch {
        v: greek_core::sync::SYNC_WIRE_VERSION_MAX,
        device_id,
        from_hlc: since.unwrap_or_default().to_string(),
        to_hlc: greek_core::hlc_now(db).to_string(),
        reviews,
        settings,
        known_words,
        lookup_cards,
        chapter_unlocks,
    })
}

/// Load the nostr identity secret from the `secrets` vault (schema v4).
/// Falls back to the legacy `user_settings.nostr_nsec` location (pre-v4
/// databases that somehow skipped the migration) — and MIGRATES it on
/// read: into `secrets`, out of both synced/plaintext settings tables.
fn load_nostr_nsec(db: &Database) -> Option<String> {
    if let Ok(Some(v)) = greek_core::secret_get(db, "nostr_nsec") {
        return Some(v);
    }
    let legacy = user_setting_get(db, "nostr_nsec")?;
    let _ = greek_core::secret_put(db, "nostr_nsec", &legacy);
    let _ = db
        .conn()
        .execute("DELETE FROM user_settings WHERE key = 'nostr_nsec'", []);
    let _ = db
        .conn()
        .execute("DELETE FROM settings_sync WHERE key = 'nostr_nsec'", []);
    Some(legacy)
}

/// Parse the stored nostr identity, or explain how to configure one.
fn nostr_keys(db: &Database) -> Result<nostr_sdk::prelude::Keys, GreekError> {
    let nsec = load_nostr_nsec(db).ok_or_else(|| {
        GreekError::Sync(
            "nostr sync identity not configured; call sync_generate_nostr_identity \
             or sync_set_nostr_nsec first"
                .to_string(),
        )
    })?;
    nostr_sdk::prelude::Keys::parse(&nsec)
        .map_err(|e| GreekError::Sync(format!("stored nostr key is invalid: {e}")))
}

fn nostr_relays(db: &Database) -> Vec<String> {
    user_setting_get(db, "nostr_relays")
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_NOSTR_RELAYS.to_string())
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// S7 destination fingerprint for the nostr spool: FNV-1a over (1) the
/// NORMALIZED relay set — lowercased, trimmed, sorted, deduped, so order or
/// case/whitespace variance in the stored CSV doesn't split the scope — and
/// (2) the identity pubkey hex. Changing either yields a new cursor scope,
/// hence a full push to the new destination (the old scope's cursor is kept
/// for a possible switch-back; see `greek_core::sync::push_cursor_get`).
fn nostr_destination_fingerprint(keys: &nostr_sdk::prelude::Keys, relays: &[String]) -> String {
    let mut normalized: Vec<String> = relays
        .iter()
        .map(|r| r.trim().to_lowercase())
        .filter(|r| !r.is_empty())
        .collect();
    normalized.sort();
    normalized.dedup();
    greek_core::sync::destination_fingerprint(&[
        &normalized.join(","),
        &keys.public_key().to_hex(),
    ])
}

fn sync_err(e: impl std::fmt::Display) -> GreekError {
    GreekError::Sync(e.to_string())
}

#[uniffi::export]
impl GreekCore {
    /// This device's stable sync id (bootstrapped once into the database).
    pub fn sync_device_id(&self) -> String {
        let db = self.db.lock().unwrap();
        greek_core::device_id(&db)
    }

    /// All paired devices (from iroh sync rounds), most recent first.
    pub fn sync_paired_devices(&self) -> Vec<PairedDeviceView> {
        let db = self.db.lock().unwrap();
        db.conn()
            .prepare(
                "SELECT device_id, label, paired_at, COALESCE(last_sync_hlc, '')
                 FROM sync_devices ORDER BY paired_at DESC",
            )
            .ok()
            .and_then(|mut stmt| {
                stmt.query_map([], |row| {
                    Ok(PairedDeviceView {
                        device_id: row.get(0)?,
                        label: row.get(1)?,
                        paired_at: row.get(2)?,
                        last_sync_hlc: row.get(3)?,
                    })
                })
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
            })
            .unwrap_or_default()
    }

    /// Full backup: a complete v2 SyncBatch (`v`, all events, all settings,
    /// all known-words, lookup-card content rows, and chapter unlocks) as
    /// JSON — the "export backup" / paired-file payload.
    pub fn sync_export_backup_json(&self) -> String {
        let db = self.db.lock().unwrap();
        build_batch(&db, None)
            .and_then(|b| serde_json::to_string(&b).map_err(GreekError::from))
            .unwrap_or_else(|e| format!("{{\"error\":\"{}\"}}", e))
    }

    /// Import a backup produced by `sync_export_backup_json`. Wire versions
    /// 1 (reviews+settings only) and 2 (+ known_words / lookup_cards /
    /// chapter_unlocks) are accepted through ONE shared gate
    /// (`greek_core::sync::validate_batch`); anything newer is rejected
    /// whole. Applies review events (uuid-dedup, Unicode/NFC card-id gate),
    /// settings (HLC-LWW), and the v2 content collections (dedup/LWW), then
    /// folds memory state.
    ///
    /// ATOMIC (S5): ALL collections apply inside ONE `with_tx` transaction
    /// on the shared connection — a late error (a setting row with a
    /// malformed HLC, a fold failure anywhere in the batch) rolls back the
    /// ENTIRE import, never leaving earlier inserts committed behind an
    /// `Err` return. On `Err` the database holds nothing from this payload.
    pub fn sync_import_backup_json(&self, json: String) -> Result<SyncStatsView, GreekError> {
        let batch: greek_core::SyncBatch =
            serde_json::from_str(&json).map_err(|e| sync_err(format!("bad backup JSON: {e}")))?;
        greek_core::sync::validate_batch(&batch).map_err(|e| sync_err(e.to_string()))?;
        let db = self.db.lock().unwrap();
        // All five collections, same envelope: `with_tx` is nesting-aware,
        // so each applier's own internal transaction joins this one and any
        // error discards the whole import.
        let (reviews_received, settings_applied) = db.with_tx(|_| {
            let reviews_received = greek_core::apply_review_events(&db, &batch.reviews)?;
            let settings_applied = greek_core::apply_settings(&db, &batch.settings)?;
            // v2 content collections — EMPTY on a v1 payload (serde
            // defaults), so a v1 import is exactly the legacy behavior.
            greek_core::sync::apply_lookup_cards(&db, &batch.lookup_cards)?;
            greek_core::sync::apply_known_words(&db, &batch.known_words)?;
            greek_core::sync::apply_chapter_unlocks(&db, &batch.chapter_unlocks)?;
            // S7: rows merged from a backup can PREDATE a transport's push
            // cursor — that destination never saw them (they arrived via
            // file, not the wire). Rewind any cursor at-or-above the merged
            // minimum HLC so the next push to it re-ships this history;
            // receivers uuid-dedup, so resend is safe. Pull cursors and
            // non-merged (empty) payloads are untouched.
            let merged_min = batch
                .reviews
                .iter()
                .map(|r| r.hlc.as_str())
                .chain(batch.settings.iter().map(|(_, _, h)| h.as_str()))
                .filter(|h| !h.is_empty())
                .min()
                .map(str::to_string);
            if let Some(min_hlc) = merged_min {
                greek_core::sync::rewind_push_cursors_below(&db, &min_hlc)?;
            }
            Ok((reviews_received, settings_applied))
        })?;
        Ok(SyncStatsView {
            reviews_sent: 0,
            reviews_received,
            settings_applied,
        })
    }

    /// Snapshot of the nostr spool config for the Settings screen.
    pub fn sync_nostr_config(&self) -> NostrConfigView {
        let db = self.db.lock().unwrap();
        NostrConfigView {
            relays_csv: user_setting_get(&db, "nostr_relays")
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_NOSTR_RELAYS.to_string()),
            // Routing through `load_nostr_nsec` (the same legacy-migration-
            // aware accessor as the push/pull paths) means this snapshot
            // also MIGRATES a pre-v4 `user_settings.nostr_nsec` into the
            // vault on first contact instead of leaving it orphaned.
            has_identity: load_nostr_nsec(&db).is_some(),
        }
    }

    /// Public key (lowercase hex) of the sync nostr identity; `None` when no
    /// identity is configured yet or the stored secret is invalid.
    ///
    /// Derived through the SAME code path as identity load (`nostr_keys` →
    /// `load_nostr_nsec`), so this getter is legacy-migration-aware: calling
    /// it on a pre-v4 database migrates `user_settings.nostr_nsec` into the
    /// secrets vault (it does NOT generate a fresh identity — generation
    /// stays an explicit `sync_generate_nostr_identity` decision). Desktop
    /// uses this to display/verify the identity without linking nostr-sdk
    /// itself (previously it had to `Keys::parse` the secret).
    pub fn sync_nostr_pubkey(&self) -> Option<String> {
        let db = self.db.lock().unwrap();
        nostr_keys(&db).ok().map(|k| k.public_key().to_hex())
    }

    /// Set the relay list (comma-separated `wss://…` URLs).
    pub fn sync_set_nostr_relays(&self, relays_csv: String) {
        let db = self.db.lock().unwrap();
        // Propagates via settings sync: the whole wallet of devices shares
        // one relay config.
        let _ = greek_core::setting_put(&db, "nostr_relays", relays_csv.trim());
    }

    /// Generate a fresh nostr identity and store it; returns the public key
    /// (hex) for display. Any previous identity is REPLACED — old batches
    /// encrypted to it become unreadable, so the UI must confirm first.
    pub fn sync_generate_nostr_identity(&self) -> String {
        let keys = nostr_sdk::prelude::Keys::generate();
        let nsec_hex = keys.secret_key().to_secret_hex();
        let db = self.db.lock().unwrap();
        // Secrets live in the vault (never synced, never exported).
        // Keystore-backed wrapping stays a rollout-gate item — ROLLOUT.md.
        let _ = greek_core::secret_put(&db, "nostr_nsec", &nsec_hex);
        keys.public_key().to_hex()
    }

    /// Import a nostr identity (hex or `nsec1…` bech32); validated before
    /// storing.
    pub fn sync_set_nostr_nsec(&self, nsec: String) -> Result<(), GreekError> {
        let keys = nostr_sdk::prelude::Keys::parse(nsec.trim())
            .map_err(|e| sync_err(format!("not a valid nostr secret key (hex or nsec1…): {e}")))?;
        let db = self.db.lock().unwrap();
        // Secrets live in the vault — see sync_generate_nostr_identity.
        greek_core::secret_put(&db, "nostr_nsec", &keys.secret_key().to_secret_hex())?;
        Ok(())
    }
}

/// Secrets vault accessors (schema v4). Values stored here are device-local
/// ONLY: they never ride settings sync and never appear in
/// `sync_export_backup_json` (DESIGN-progress-sharing.md, Appendix R
/// "Secrets"). Writing a secret through the settings API is refused.
#[uniffi::export]
impl GreekCore {
    /// Read a secret (e.g. `nostr_nsec`); `None` when absent.
    pub fn secret_get(&self, key: String) -> Option<String> {
        let db = self.db.lock().unwrap();
        greek_core::secret_get(&db, &key).ok().flatten()
    }

    /// Store a secret (upsert).
    pub fn secret_put(&self, key: String, value: String) -> Result<(), GreekError> {
        let db = self.db.lock().unwrap();
        greek_core::secret_put(&db, &key, &value)?;
        Ok(())
    }

    /// Remove a secret; no-op when absent.
    pub fn secret_delete(&self, key: String) {
        let db = self.db.lock().unwrap();
        let _ = greek_core::secret_delete(&db, &key);
    }

    /// `true` when the secret exists (no value read-out).
    pub fn secret_has(&self, key: String) -> bool {
        let db = self.db.lock().unwrap();
        greek_core::secret_has(&db, &key)
    }
}

/// Internal iroh lazy-open — deliberately OUTSIDE the exported impl blocks
/// (uniffi tries to export every method of an annotated impl, and the
/// transport type is not a uniffi type).
impl GreekCore {
    async fn open_iroh(&self) -> Result<Arc<sync_iroh::SyncIrohTransport>, GreekError> {
        {
            let g = self.iroh.lock().unwrap();
            if let Some(t) = &*g {
                return Ok(t.clone());
            }
        }
        let dir = std::path::Path::new(&self.db_path)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("iroh-sync");
        // GREEK_IROH_NET=loopback selects the hermetic no-relay/no-DNS
        // mode (E2E harnesses, hosts with broken/filtered DNS). Default
        // stays N0 — production behavior on devices is unchanged.
        let loopback = std::env::var("GREEK_IROH_NET").ok().as_deref() == Some("loopback");
        let t = Arc::new(
            if loopback {
                sync_iroh::SyncIrohTransport::open_loopback(&dir).await
            } else {
                sync_iroh::SyncIrohTransport::open(&dir).await
            }
            .map_err(|e| sync_err(format!("iroh sync open failed: {e:#}")))?,
        );
        *self.iroh.lock().unwrap() = Some(t.clone());
        Ok(t)
    }
}

/// Async sync operations (nostr spool + iroh). Futures run on uniffi's
/// tokio async runtime (`async_compat` schedules a runtime on first poll).
#[uniffi::export(async_runtime = "tokio")]
impl GreekCore {
    /// Push all local events/settings since the last push cursor to the
    /// configured nostr relays, then advance the (device-local) cursor.
    ///
    /// The cursor is scoped to the CURRENT destination (relays + identity
    /// fingerprint — S7): relay/identity changes therefore start the new
    /// destination with a FULL push, and switching back resumes the old
    /// destination's cursor instead of resending.
    pub async fn sync_nostr_push(&self) -> Result<SyncStatsView, GreekError> {
        let (keys, relays, batch, cursor_fp) = {
            let db = self.db.lock().unwrap();
            let keys = nostr_keys(&db)?;
            let relays = nostr_relays(&db);
            let fp = nostr_destination_fingerprint(&keys, &relays);
            let cursor = greek_core::sync::push_cursor_get(
                &db,
                greek_core::sync::NOSTR_PUSH_CURSOR_PREFIX,
                &fp,
            );
            let batch = build_batch(&db, cursor.as_deref())?;
            (keys, relays, batch, fp)
        };
        let to_hlc = batch.to_hlc.clone();
        if batch.reviews.is_empty() && batch.settings.is_empty() {
            // Nothing to spool: don't burn a relay publish (and rate limits
            // on public relays) on an empty checkpoint round. Cursor stays.
            return Ok(SyncStatsView {
                reviews_sent: 0,
                reviews_received: 0,
                settings_applied: 0,
            });
        }
        let reviews_sent = batch.reviews.len() as u32;
        let relay_refs: Vec<&str> = relays.iter().map(|s| s.as_str()).collect();
        let transport = sync_nostr::NostrSpoolTransport::connect(keys, &relay_refs)
            .await
            .map_err(sync_err)?;
        transport.push(&batch).await.map_err(sync_err)?;
        let db = self.db.lock().unwrap();
        greek_core::sync::push_cursor_put(
            &db,
            greek_core::sync::NOSTR_PUSH_CURSOR_PREFIX,
            &cursor_fp,
            &to_hlc,
        )?;
        Ok(SyncStatsView {
            reviews_sent,
            reviews_received: 0,
            settings_applied: 0,
        })
    }

    /// Pull every complete batch from the nostr relays and apply the ones
    /// whose wire batch UUID is not yet in the `sync_applied_batches`
    /// ledger (S6). Admission is per-batch UUID at ANY HLC position — the
    /// `nostr_last_pull_hlc` cursor no longer filters (it is updated as an
    /// observability-only high-water mark afterwards) — so a batch whose
    /// chunks completed late, or which was pushed from a lagging clock,
    /// still merges instead of being permanently skipped. Each admitted
    /// batch applies and marks in ONE transaction, so a crash mid-apply
    /// leaves the UUID unmarked and the whole batch safely retry-able.
    /// Incomplete batches are simply not returned by the transport
    /// (unmarked — a later fetch completes them); complete-but-corrupt
    /// ones are never surfaced either, so they can never be marked.
    pub async fn sync_nostr_pull(&self) -> Result<SyncStatsView, GreekError> {
        let (keys, relays) = {
            let db = self.db.lock().unwrap();
            (nostr_keys(&db)?, nostr_relays(&db))
        };
        let relay_refs: Vec<&str> = relays.iter().map(|s| s.as_str()).collect();
        let transport = sync_nostr::NostrSpoolTransport::connect(keys, &relay_refs)
            .await
            .map_err(sync_err)?;
        let fetched = transport.fetch_all().await.map_err(sync_err)?;
        let mut stats = SyncStatsView {
            reviews_sent: 0,
            reviews_received: 0,
            settings_applied: 0,
        };
        let db = self.db.lock().unwrap();
        // Observability high-water mark is decided over ALL fetched
        // batches (applied or already-marked): it is metadata reporting
        // what the device has physically SEEN, never an admission gate.
        let mut max_hlc: Option<String> = None;
        for fb in &fetched {
            if max_hlc.as_deref() < Some(fb.batch.to_hlc.as_str()) {
                max_hlc = Some(fb.batch.to_hlc.clone());
            }
        }
        for fb in &fetched {
            let batch = &fb.batch;
            // S6 admission: batch identity, not HLC position. Replay of an
            // applied batch is also uuid-safe inside apply_review_events,
            // but skipping here avoids a pointless full re-fold.
            if greek_core::sync::batch_is_applied(&db, &fb.batch_uuid) {
                continue;
            }
            // Apply + mark in ONE transaction: both, or neither.
            let (rev, set) = db.with_tx(|_conn| -> anyhow::Result<(u32, u32)> {
                let rev = greek_core::apply_review_events(&db, &batch.reviews)?;
                let set = greek_core::apply_settings(&db, &batch.settings)?;
                // v2 content collections (empty for v1 batches — no-op).
                // Codec-level reassembly already rejected unknown versions,
                // so every batch here passed the shared version gate.
                greek_core::sync::apply_lookup_cards(&db, &batch.lookup_cards)?;
                greek_core::sync::apply_known_words(&db, &batch.known_words)?;
                greek_core::sync::apply_chapter_unlocks(&db, &batch.chapter_unlocks)?;
                // S4: `to_hlc` is the SENDER's clock stamp — it is not
                // routed through apply_review_events/apply_settings (an
                // empty batch carries no rows at all), so observe it here
                // or a later local review could sort before everything
                // this batch reports.
                greek_core::sync::hlc_observe(&db, &batch.to_hlc)?;
                greek_core::sync::mark_batch_applied(&db, &fb.batch_uuid)?;
                Ok((rev, set))
            })?;
            stats.reviews_received += rev;
            stats.settings_applied += set;
        }
        if let Some(h) = max_hlc {
            // Advance-only: a re-imported/old batch must never rewind the
            // mark below what an earlier pull already reported.
            let cur = user_setting_get(&db, "nostr_last_pull_hlc");
            if cur.as_deref().map_or(true, |c| h.as_str() > c) {
                user_setting_put_local(&db, "nostr_last_pull_hlc", &h)?;
            }
        }
        Ok(stats)
    }

    /// Share payload for pairing a new device (render into a QR code).
    /// Contains the write-capable namespace secret: it is a credential —
    /// never send it over a network channel.
    pub async fn sync_iroh_ticket(&self) -> Result<String, GreekError> {
        let t = self.open_iroh().await?;
        let ticket = t.create_or_get_ticket().await.map_err(sync_err)?;
        Ok(format!("{}{}", sync_iroh::TICKET_SCHEME_PREFIX, ticket))
    }

    /// Join an existing notebook from a scanned ticket (raw string or the
    /// `greeksync://ticket/…` payload). Devices seen via the doc are recorded
    /// in the paired-device registry on the next `sync_iroh_now`.
    pub async fn sync_iroh_join(&self, ticket: String) -> Result<(), GreekError> {
        let t = self.open_iroh().await?;
        t.join_from_ticket(&ticket).await.map_err(sync_err)
    }

    /// One full sync round over iroh: push the local delta, sync with known
    /// peers, pull every doc entry back, and merge (reviews uuid-dedup,
    /// settings HLC-LWW, then the FSRS fold).
    ///
    /// Round semantics (S8): a fresh device that never paired is a clean
    /// no-op. Once paired, a round that FAILS against every known peer (or
    /// finds none to dial) is surfaced as an error — degraded, not silently
    /// "synced". A round that produced no peer signal at all (deadline) stays
    /// tolerated as Ok; sync is opportunistic.
    ///
    /// The push cursor is scoped to the CURRENT notebook namespace (S7):
    /// creating or joining a different notebook starts it with a FULL push
    /// while the previous namespace's cursor is preserved.
    pub async fn sync_iroh_now(&self) -> Result<SyncStatsView, GreekError> {
        let t = self.open_iroh().await?;
        let namespace = t.doc_namespace_id().await.map_err(sync_err)?;
        let (reviews, settings, new_cursor) = {
            let db = self.db.lock().unwrap();
            let cursor = greek_core::sync::push_cursor_get(
                &db,
                greek_core::sync::IROH_PUSH_CURSOR_PREFIX,
                &namespace,
            );
            let reviews = greek_core::events_since(&db, cursor.as_deref())?;
            let settings = greek_core::settings_since(&db, cursor.as_deref())?;
            let new_cursor = greek_core::hlc_now(&db).to_string();
            (reviews, settings, new_cursor)
        };
        let reviews_sent = reviews.len() as u32;
        t.push_local(&reviews, &settings).await.map_err(sync_err)?;
        {
            let db = self.db.lock().unwrap();
            greek_core::sync::push_cursor_put(
                &db,
                greek_core::sync::IROH_PUSH_CURSOR_PREFIX,
                &namespace,
                &new_cursor,
            )?;
        }
        t.sync_once().await.map_err(sync_err)?;
        let (remote_reviews, remote_settings) = t.pull_remote().await.map_err(sync_err)?;
        let db = self.db.lock().unwrap();
        let reviews_received = greek_core::apply_review_events(&db, &remote_reviews)?;
        let settings_applied = greek_core::apply_settings(&db, &remote_settings)?;
        // Any device id appearing in the doc that isn't ours is a paired
        // peer; record it, and remember the newest hlc we saw from it.
        let local_device = greek_core::device_id(&db);
        let mut seen: HashMap<String, String> = HashMap::new();
        for ev in &remote_reviews {
            if ev.device_id != local_device {
                let entry = seen.entry(ev.device_id.clone()).or_default();
                if ev.hlc > *entry {
                    *entry = ev.hlc.clone();
                }
            }
        }
        for (dev, max_hlc) in seen {
            greek_core::pair_device(&db, &dev, "")?;
            greek_core::note_last_sync(&db, &dev, &max_hlc)?;
        }
        Ok(SyncStatsView {
            reviews_sent,
            reviews_received,
            settings_applied,
        })
    }
}

// --- Progress sharing over nostr (DESIGN-progress-sharing.md, Appendix R) ---
//
// Thin UniFFI surface over the `share-nostr` engine crate (W9-B). ALL logic —
// milestone detection, template validation (caps / NFC / no-URL / no-mention),
// outbox, rate caps, per-relay delivery state, HKDF key derivation — lives in
// the engine; this region only maps types, enforces the disabled fast-path,
// and returns errors.
//
// REQ-B (assumed engine API; if final share_nostr names/types differ at merge
// time, adapt in THIS region only — the frozen UniFFI surface below does not
// change):
//   REQ-B1: `share_nostr::is_enabled(db: &Database) -> bool`
//   REQ-B2: `share_nostr::relays_get(db: &Database) -> Vec<String>`
//           (defaults from the Appendix-R curated suggestions when unset)
//   REQ-B3: `share_nostr::share_pubkey_hex(db: &Database) -> Option<String>`
//           (stateless HKDF-SHA256(sync_secret, "greek/share-nostr/v1");
//            engine reads the sync secret via the W9-A `secret_get` helper —
//            "nostr_nsec" — from the secrets table; if the helper is absent
//            at merge time the merger wires it. Returns None when sharing
//            has never been enabled or no sync secret exists.)
//   REQ-B4: `share_nostr::enable(db: &Database, relays: &[String]) -> Result<(), E>`
//           (derives the identity key per REQ-B3 — derivation is STATELESS,
//            nothing is persisted beyond the enable flag and markers; seeds
//            `share.posted.*` markers for all already-crossed thresholds so
//            enabling is NEVER retroactive; validates relays per policy
//            (wss:// only, <=8, dedupe — see share_relays_set) then stores
//            `share.relays`.)
//   REQ-B5: `share_nostr::disable(db: &Database, forget_identity: bool) -> Result<(), E>`
//           (stops + clears queued outbox rows. `forget_identity` purges any
//            derived-key materialization cache; derivation being stateless,
//            this is expected to be a documented no-op — see surface docs on
//            share_disable.)
//   REQ-B6: `share_nostr::relays_set(db: &Database, relays: &[String]) -> Result<(), E>`
//   REQ-B7: `share_nostr::milestones(db: &Database) -> Vec<share_nostr::MilestoneInfo>`
//           where MilestoneInfo has slug/title_en/title_el/template_en/
//           template_el/custom_template_en: Option<String>/
//           custom_template_el: Option<String>/enabled: bool/
//           last_posted_ms: Option<i64>/last_state: String
//   REQ-B8: `share_nostr::set_template(db, slug: &str, locale: &str, text: &str) -> Result<(), E>`
//           (locale validated "en"|"el"; engine validates caps/NFC/no-URL/
//            no-mention/non-empty and rejects unknown slugs)
//   REQ-B9: `share_nostr::reset_template(db, slug: &str, locale: &str) -> Result<(), E>`
//   REQ-B10: `share_nostr::set_milestone_enabled(db, slug: &str, enabled: bool) -> Result<(), E>`
//   REQ-B11: `share_nostr::preview(db, slug: &str, suffix: &str) -> Result<String, E>`
//           (render only, no publish; runs ALL publish-time validation)
//   REQ-B12: `async share_nostr::flush_now(db: &Mutex<Database>) -> Result<share_nostr::FlushReport, E>`
//           where FlushReport { posted: u32, deferred: u32, duplicates: u32,
//           failed: Vec<String> /* "relay: reason" */ } — single-flight,
//           bypasses the jitter window on user request. Engine locks the
//           mutex only around short db-critical sections, never across I/O
//           awaits (uniffi exported futures must be Send).
//   REQ-B13: `share_nostr::pending_count(db: &Database) -> u32`
//   REQ-B14: `share_nostr::history(db: &Database, slug: &str) -> Vec<share_nostr::HistoryEntry>`
//           where HistoryEntry { event_id: String, created_ms: i64,
//           state: String, per_relay_json: String }
//   REQ-B15 (where `E` appears above): any `impl std::fmt::Display` error;
//           bindings map every engine error through `share_err` (below) into
//           GreekError. NOTE: engine errors currently land in the
//           `GreekError::Sync` variant — a dedicated `Share` variant would be
//           cleaner but GreekError is joint-edited territory in Wave 9
//           (W9-A secrets), so the mapping stays reuse-based to avoid an
//           edit collision; merger MAY introduce `GreekError::Share`.
//   REQ-B16 (hooks): `share_nostr::mark_streak_dirty(db)`,
//           `share_nostr::detect_gate_latch(db) -> bool`,
//           `share_nostr::detect_streak_chapter(db)`.
//
// Error-mapping sink for the whole share region (see REQ-B15).
fn share_err(e: impl std::fmt::Display) -> GreekError {
    GreekError::Sync(format!("sharing: {e}"))
}

/// Sharing-config snapshot for the Settings screen. Field names camelCase in
/// Kotlin via UniFFI.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ShareConfigView {
    pub enabled: bool,
    pub relays: Vec<String>,
    /// Empty string when sharing identity does not exist yet (never enabled).
    pub share_pubkey_hex: String,
}

/// One milestone's editor/row state (see DESIGN-progress-sharing.md §10/OQ-3).
#[derive(Debug, Clone, uniffi::Record)]
pub struct ShareMilestoneView {
    pub slug: String,
    pub title_en: String,
    pub title_el: String,
    pub template_en: String,
    pub template_el: String,
    pub custom_template_en: Option<String>,
    pub custom_template_el: Option<String>,
    pub enabled: bool,
    pub last_posted_ms: Option<i64>,
    /// Engine outbox/posted state for the last attempt ("", "queued",
    /// "posted", "deferred", "failed:<reason>", ...).
    pub last_state: String,
}

/// Result of a manual flush (Settings "retry now"). `failed` entries are
/// pre-formatted "<relay>: <reason>" strings for direct display.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ShareFlushReportView {
    pub posted: u32,
    pub deferred: u32,
    pub duplicates: u32,
    pub failed: Vec<String>,
}

/// One post-history row for a milestone's editor sheet. `per_relay_json` is a
/// serialized `{ "<relay>": {"state": ..., "reason": ...}, ... }` map — kept
/// opaque across the FFI so the per-relay schema can evolve engine-side
/// without a bindings regen.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ShareHistoryEntryView {
    pub event_id: String,
    pub created_ms: i64,
    pub state: String,
    pub per_relay_json: String,
}

/// Fast-path gate for every detection hook: one settings read through the
/// engine's enabled flag. Detection is ALWAYS a no-op while disabled, and
/// hook helpers additionally swallow engine errors by design — sharing
/// failures must never surface in, or block, learning flows
/// (DESIGN §6: "publishing failures are never user-visible errors").
fn share_enabled_fast(db: &Database) -> bool {
    // REQ-B1
    share_nostr::is_enabled(db)
}

/// Hook: after a successful review submit. reviews.* threshold milestones
/// were removed (Appendix R), so this only dirties today's streak for the
/// lazy streak detector (evaluated at flush / next hook_stats_refresh).
fn share_hook_review_submitted(db: &Database) {
    if share_enabled_fast(db) {
        // REQ-B16
        share_nostr::mark_streak_dirty(db);
    }
}

/// Hook: after the alphabet gate latches (record_alphabet_check_result pass
/// lane). The engine re-reads the gate state and is lane-aware (the
/// declared_known lane never posts).
fn share_hook_gate_latched(db: &Database) {
    if share_enabled_fast(db) {
        // REQ-B16
        let _ = share_nostr::detect_gate_latch(db);
    }
}

/// Hook: on the home-screen stats refresh. Engine runs streak.{7,30,100} +
/// chapter.mastered detection; internally cheap-cached so a UITab storm
/// costs at most one detection pass per cache window.
fn share_hook_stats_refresh(db: &Database, cards: &[greek_core::srs::Card]) {
    if share_enabled_fast(db) {
        // REQ-B16
        share_nostr::facade::detect_streak_chapter(db, cards);
    }
}

/// Progress-sharing operations. Kotlin-side names camelCase via UniFFI:
/// share_config_get -> shareConfigGet, share_enable -> shareEnable, ...
#[uniffi::export]
impl GreekCore {
    /// Sharing config snapshot for the Settings screen.
    pub fn share_config_get(&self) -> ShareConfigView {
        let db = self.db.lock().unwrap();
        ShareConfigView {
            enabled: share_nostr::is_enabled(&db),      // REQ-B1
            relays: share_nostr::relays_get(&db),       // REQ-B2
            share_pubkey_hex: share_nostr::share_pubkey_hex(&db) // REQ-B3
                .unwrap_or_default(),
        }
    }

    /// Enable sharing: validates + stores the relay list, derives the sharing
    /// identity, and seeds posted markers for already-crossed thresholds so
    /// NOTHING already achieved post-dates enable (zero retroactivity).
    /// The UI must show the Appendix-R disclosure screen before calling this.
    pub fn share_enable(&self, relays: Vec<String>) -> Result<(), GreekError> {
        let db = self.db.lock().unwrap();
        share_nostr::facade::enable(&db, &relays, &self.cards).map_err(share_err) // REQ-B4 (facade, cards for seeding)
    }

    /// Disable sharing: stops the publisher and clears queued outbox rows.
    /// When `forget_identity` is set the engine also drops any derived-key
    /// materialization cache it holds; HKDF derivation is stateless, so the
    /// next enable derives byte-identical keys again (i.e. "forget" does not
    /// and cannot retire the derived pubkey — deletion of already-posted
    /// notes is off-table for v1; see DESIGN Appendix R).
    pub fn share_disable(&self, forget_identity: bool) {
        let db = self.db.lock().unwrap();
        // Errors are swallowed: disable must always succeed UI-side (worst
        // case leftover queue rows, which the next enable clears anyway).
        let _ = share_nostr::disable(&db, forget_identity); // REQ-B5
    }

    /// Replace the sharing relay list. Policy (engine-enforced): `wss://`
    /// only, at most 8, deduped. Loopback affordance used by the e2e harness:
    /// `ws://127.0.0.1…` / `ws://localhost…` IS accepted; plain `http(s)://`
    /// and non-loopback `ws://` are refused.
    pub fn share_relays_set(&self, relays: Vec<String>) -> Result<(), GreekError> {
        let db = self.db.lock().unwrap();
        share_nostr::relays_set(&db, &relays).map_err(share_err) // REQ-B6
    }

    /// All milestones with titles/templates (shipped defaults + any custom
    /// overrides) and per-slug enable/state, for the Share settings section
    /// and the milestone editor sheet.
    pub fn share_milestones(&self) -> Vec<ShareMilestoneView> {
        let db = self.db.lock().unwrap();
        // REQ-B7 — if the engine returns a differently-shaped row, adapt the
        // field mapping here only.
        share_nostr::milestones(&db)
            .into_iter()
            .map(|m| ShareMilestoneView {
                slug: m.slug,
                title_en: m.title_en,
                title_el: m.title_el,
                template_en: m.template_en,
                template_el: m.template_el,
                custom_template_en: m.custom_template_en,
                custom_template_el: m.custom_template_el,
                enabled: m.enabled,
                last_posted_ms: m.last_posted_ms,
                last_state: m.last_state,
            })
            .collect()
    }

    /// Set a custom template override for one milestone + locale ("en"|"el").
    /// Engine validates: non-empty, <=1024 UTF-8 bytes + grapheme cap, NFC
    /// normalization, no URLs / image extensions / nostr:or-@ mentions /
    /// key material patterns. Empty text is REFUSED (use
    /// share_reset_template to restore the preset instead).
    pub fn share_set_template(
        &self,
        slug: String,
        locale: String,
        text: String,
    ) -> Result<(), GreekError> {
        let db = self.db.lock().unwrap();
        share_nostr::set_template(&db, &slug, &locale, &text).map_err(share_err) // REQ-B8
    }

    /// Restore the shipped preset for one milestone + locale ("reset to
    /// default" chip). Unknown slug/locale are ignored — reset is idempotent
    /// UI-side.
    pub fn share_reset_template(&self, slug: String, locale: String) {
        let db = self.db.lock().unwrap();
        let _ = share_nostr::reset_template(&db, &slug, &locale); // REQ-B9
    }

    /// Per-milestone on/off toggle (independent of the global enable).
    pub fn share_set_milestone_enabled(&self, slug: String, enabled: bool) {
        let db = self.db.lock().unwrap();
        let _ = share_nostr::set_milestone_enabled(&db, &slug, enabled); // REQ-B10
    }

    /// Render the full post text for the milestone editor's live preview.
    /// Render ONLY — never publishes — but runs all publish-time validation
    /// so exactly what the user sees is what would post. `suffix` is the
    /// milestone-gated personal note (OQ-5); pass "" when there is none.
    pub fn share_preview(&self, slug: String, suffix: String) -> Result<String, GreekError> {
        let db = self.db.lock().unwrap();
        share_nostr::preview(&db, &slug, &suffix).map_err(share_err) // REQ-B11
    }

    /// Outbox rows waiting for publish: source for the quiet persistent
    /// "deferred/failed" badge on the Share settings section.
    pub fn share_pending_count(&self) -> u32 {
        let db = self.db.lock().unwrap();
        share_nostr::pending_count(&db) // REQ-B13
    }

    /// Post history for one milestone (editor sheet): each posted/attempted
    /// event with its per-relay delivery state.
    pub fn share_history(&self, slug: String) -> Vec<ShareHistoryEntryView> {
        let db = self.db.lock().unwrap();
        // REQ-B14 — same "adapt mapping here only" rule as share_milestones.
        share_nostr::history(&db, &slug)
            .into_iter()
            .map(|h| ShareHistoryEntryView {
                event_id: h.event_id,
                created_ms: h.created_ms,
                state: h.state,
                per_relay_json: h.per_relay_json,
            })
            .collect()
    }
}

/// Async progress-sharing operation. Kept in its own async impl block so the
/// sync fns above stay lock-cheap.
#[uniffi::export(async_runtime = "tokio")]
impl GreekCore {
    /// Manually flush the outbox NOW (Settings "retry"), bypassing the
    /// jittered daily window. REQ-B12 (integration constraint): the engine
    /// takes the SHARED `Mutex<Database>` and must lock it only for short
    /// db-critical sections — never across network awaits (a std MutexGuard
    /// is !Send, and uniffi requires exported futures to be Send; this is
    /// the same snapshot/relock discipline as sync_nostr_push). Engine flush
    /// is single-flight and rate-cap bounded; while in flight other bindings
    /// calls may block briefly on individual engine lock sections.
    pub async fn share_flush_now(&self) -> Result<ShareFlushReportView, GreekError> {
        let r = share_nostr::flush_now(&self.db).await.map_err(share_err)?; // REQ-B12
        Ok(ShareFlushReportView {
            posted: r.posted,
            deferred: r.deferred,
            duplicates: r.duplicates,
            failed: r
                .failed
                .into_iter()
                .map(|(relay, reason)| format!("{relay}: {reason}"))
                .collect(),
        })
    }
}

/// Count consecutive calendar days (working backward from today) that have
/// at least one review in `review_logs`. Timestamps are ISO-8601 (with a 'T');
/// we compare on the YYYY-MM-DD date prefix.
fn compute_streak_days(db: &Database) -> i32 {
    // Distinct review days, most recent first.
    let days: Vec<String> = db
        .conn()
        .prepare("SELECT DISTINCT substr(timestamp, 1, 10) AS d FROM review_logs ORDER BY d DESC")
        .ok()
        .and_then(|mut stmt| {
            stmt.query_map([], |row| row.get::<_, String>(0))
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default();

    if days.is_empty() {
        return 0;
    }

    let now = now_iso8601();
    let today = &now[..10.min(now.len())];
    let today_days = date_to_epoch_days(today);
    let yesterday_days = today_days.map(|d| d - 1);

    // The streak is only "current" if the most recent review was today or
    // yesterday; otherwise it has been broken.
    let most_recent = date_to_epoch_days(&days[0]);
    match (most_recent, today_days) {
        (Some(mr), Some(_)) => {
            if Some(mr) != today_days && Some(mr) != yesterday_days {
                return 0;
            }
        }
        _ => return 0,
    }

    let mut streak = 0i32;
    let mut expected = most_recent;
    for day in &days {
        let d = date_to_epoch_days(day);
        if d == expected {
            streak += 1;
            expected = expected.map(|e| e - 1);
        } else {
            break;
        }
    }

    streak
}

/// Parse a "YYYY-MM-DD" date into days since a fixed epoch (proleptic Gregorian).
fn date_to_epoch_days(date: &str) -> Option<i64> {
    if date.len() < 10 {
        return None;
    }
    let year: i64 = date.get(0..4)?.parse().ok()?;
    let month: i64 = date.get(5..7)?.parse().ok()?;
    let day: i64 = date.get(8..10)?.parse().ok()?;

    // Howard Hinnant's days_from_civil algorithm.
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146097 + doe - 719468)
}

// --- Standalone functions ---

#[uniffi::export]
pub fn parse_postag(postag: String) -> String {
    MorphologyDb::parse_postag(&postag)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_db_path(tag: &str) -> (std::path::PathBuf, String) {
        let dir = std::env::temp_dir().join(format!("greek-w9-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("app.db");
        (dir, path.to_string_lossy().into_owned())
    }

    // --- C2: lookup cards carry NO memory_states row until first review ---

    /// The full stored row, for exact incremental-vs-replay comparison.
    fn memory_state_snapshot(
        db: &Database,
        card_id: &str,
    ) -> Option<(f32, f32, Option<String>, String, i32, i32)> {
        db.conn()
            .query_row(
                "SELECT stability, difficulty, last_review, due_date, reps, lapses
                 FROM memory_states WHERE card_id = ?1",
                [card_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .ok()
    }

    // C2 regression: adding a lookup card must NOT create a zero-state
    // memory_states row, and its rating previews must be EXACTLY those of a
    // never-present new card (the old row gave a bogus fixed 864s-ish).
    #[test]
    fn test_lookup_card_preview_matches_never_present_new_card() {
        let (dir, path) = tmp_db_path("c2-preview");
        let core = GreekCore::new(path, None).unwrap();

        let lookup_id = core
            .add_card_from_lookup("λόγος".to_string(), "".to_string(), "word".to_string())
            .unwrap();
        assert_eq!(lookup_id, "lookup-λόγος");

        // No memory_states row until the first review.
        {
            let db = core.db.lock().unwrap();
            assert_eq!(
                memory_state_snapshot(&db, &lookup_id),
                None,
                "an unreviewed lookup card must have no memory_states row"
            );
        }

        let lookup_preview = core
            .preview_intervals(lookup_id.clone())
            .expect("a lookup card must yield a new-card preview");
        let fresh_preview = core
            .preview_intervals("never-present-anywhere".to_string())
            .expect("a never-present card must yield a new-card preview");
        assert_eq!(lookup_preview.again_secs, fresh_preview.again_secs);
        assert_eq!(lookup_preview.hard_secs, fresh_preview.hard_secs);
        assert_eq!(lookup_preview.good_secs, fresh_preview.good_secs);
        assert_eq!(lookup_preview.easy_secs, fresh_preview.easy_secs);
        // Sanity: the C6 learning-step ladder applies (a brand-new card is
        // in learning, so Good shows the 10-minute step, never the old
        // bogus fixed ~864s of the zero-state row).
        assert_eq!(fresh_preview.again_secs, 60);
        assert_eq!(fresh_preview.hard_secs, 300);
        assert_eq!(fresh_preview.good_secs, 600);

        drop(core);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // C2 regression: after one Good review, the incrementally-upserted row
    // must be byte-identical to replay-from-history (`rebuild_memory_states`
    // recomputes every card's state from its review_logs). With the old
    // zero-state row this diverged (stability 0.01 incremental vs ~3.17
    // replay) without any network fault.
    #[test]
    fn test_lookup_card_incremental_state_matches_replay() {
        let (dir, path) = tmp_db_path("c2-replay");
        let core = GreekCore::new(path, None).unwrap();

        let lookup_id = core
            .add_card_from_lookup("λόγος".to_string(), "".to_string(), "word".to_string())
            .unwrap();
        assert!(
            core.submit_review(lookup_id.clone(), 3),
            "Good review lands"
        );

        let db = core.db.lock().unwrap();
        let incremental = memory_state_snapshot(&db, &lookup_id)
            .expect("first review must create the memory_states row");
        assert_eq!(incremental.4, 1, "exactly one repetition");

        let rewritten = greek_core::sync::rebuild_memory_states(&db).unwrap();
        assert_eq!(rewritten, 1, "exactly the one reviewed card is folded");

        let replayed = memory_state_snapshot(&db, &lookup_id).unwrap();
        assert_eq!(
            incremental, replayed,
            "incremental submit state MUST equal replay-from-history: {incremental:?} vs {replayed:?}"
        );
        drop(db);
        drop(core);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // C2 migration: a database carrying the legacy zero-state rows is
    // upgraded to schema v5, which purges them — but ONLY those rows. `loose
    // ends` guard: a lookup row with review history (torn-write protection)
    // and any non-lookup row must survive, and the card itself must stay
    // schedulable (new-card preview + first review land normally).
    #[test]
    fn test_v5_migration_purges_zero_state_lookup_rows() {
        let (dir, path) = tmp_db_path("c2-migrate");

        // Hand-build a v4-era database: memory_states/review_logs/lookup_cards
        // at their current shape, stamped at user_version 4 so the v5
        // contents cleanup runs on open.
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE memory_states(
                     card_id TEXT PRIMARY KEY,
                     stability REAL NOT NULL,
                     difficulty REAL NOT NULL,
                     last_review TEXT,
                     due_date TEXT NOT NULL,
                     reps INTEGER NOT NULL DEFAULT 0,
                     lapses INTEGER NOT NULL DEFAULT 0
                 );
                 CREATE TABLE review_logs(
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     card_id TEXT NOT NULL,
                     timestamp TEXT NOT NULL,
                     rating INTEGER NOT NULL,
                     elapsed_days REAL NOT NULL,
                     scheduled_days REAL NOT NULL,
                     source TEXT NOT NULL DEFAULT 'flashcard',
                     uuid TEXT,
                     device_id TEXT,
                     hlc TEXT
                 );
                 CREATE TABLE lookup_cards(
                     card_id TEXT PRIMARY KEY,
                     front TEXT NOT NULL,
                     back TEXT NOT NULL
                 );
                 -- The legacy bug row: never reviewed, all zeros.
                 INSERT INTO memory_states VALUES
                     ('lookup-legacy', 0.0, 0.0, NULL, '2026-01-01T00:00:00Z', 0, 0);
                 INSERT INTO lookup_cards VALUES ('lookup-legacy', 'λόγος', 'word');
                 -- Torn-write guard: zero-shaped row but WITH a review log;
                 -- must survive (its history is real).
                 INSERT INTO memory_states VALUES
                     ('lookup-torn', 0.0, 0.0, NULL, '2026-01-01T00:00:00Z', 0, 0);
                 INSERT INTO review_logs (card_id, timestamp, rating, elapsed_days, scheduled_days)
                     VALUES ('lookup-torn', '2026-01-01T00:00:00Z', 3, 0.0, 1.0);
                 -- Prefix guard: a non-lookup zero-shaped row must survive.
                 INSERT INTO memory_states VALUES
                     ('ath1-001', 0.0, 0.0, NULL, '2026-01-01T00:00:00Z', 0, 0);
                 PRAGMA user_version = 4;",
            )
            .unwrap();
        }

        // Opening upgrades through v5/v6 (the purge runs at v5, inside the
        // migration tx).
        let db = Database::open(&path, None).unwrap();
        let version: i64 = db
            .conn()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 6, "schema must advance to v6");

        assert_eq!(
            memory_state_snapshot(&db, "lookup-legacy"),
            None,
            "the legacy zero-state row must be purged"
        );
        assert!(
            memory_state_snapshot(&db, "lookup-torn").is_some(),
            "a lookup row WITH review history must survive"
        );
        assert!(
            memory_state_snapshot(&db, "ath1-001").is_some(),
            "non-lookup rows must never be touched by the lookup purge"
        );
        let card_present: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM lookup_cards WHERE card_id = 'lookup-legacy'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(card_present, 1, "the card content row must survive");

        // The purged card schedules like any brand-new card: preview byte-
        // identical to a never-reviewed card, and a normal first review that
        // creates a genuine row.
        let engine = SrsEngine::new();
        let preview = engine
            .preview_intervals(&db, "lookup-legacy")
            .unwrap()
            .expect("purged lookup card must yield a new-card preview");
        let fresh = engine
            .preview_intervals(&db, "never-reviewed-at-all")
            .unwrap()
            .expect("a never-reviewed card must yield a new-card preview");
        assert_eq!(
            (
                preview.again_secs,
                preview.hard_secs,
                preview.good_secs,
                preview.easy_secs
            ),
            (
                fresh.again_secs,
                fresh.hard_secs,
                fresh.good_secs,
                fresh.easy_secs
            ),
            "purged lookup card must preview exactly like a never-present new card"
        );
        engine.submit_review(&db, "lookup-legacy", 3).unwrap();
        let state = memory_state_snapshot(&db, "lookup-legacy").unwrap();
        assert_eq!(state.4, 1);
        assert!(state.0 > 0.0);

        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Backup export must contain neither the secret's key nor its value,
    // even when one is stashed (secrets live outside the sync projection).
    #[test]
    fn test_backup_export_scrubs_secrets() {
        let (dir, path) = tmp_db_path("export");
        let core = GreekCore::new(path, None).unwrap();
        let secret = format!("nsec1{}", "q".repeat(58));
        core.secret_put("nostr_nsec".to_string(), secret.clone()).unwrap();
        core.sync_set_nostr_relays("wss://relay.example.com".to_string());

        let json = core.sync_export_backup_json();
        assert!(!json.contains("nostr_nsec"), "export leaked the secret KEY");
        assert!(!json.contains(&secret), "export leaked the secret VALUE");
        // (and confirm the export isn't just empty — relays must be there)
        assert!(json.contains("relay.example.com"), "sanity: {json:?}");

        drop(core);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Identity generation routes to the vault; the synced settings tables
    // stay clean, and sync round-trips still parse the key.
    #[test]
    fn test_generate_identity_stores_in_vault_not_settings() {
        let (dir, path) = tmp_db_path("ident");
        let core = GreekCore::new(path, None).unwrap();
        let pubkey = core.sync_generate_nostr_identity();
        assert_eq!(pubkey.len(), 64);
        assert!(core.secret_has("nostr_nsec".to_string()));

        let json = core.sync_export_backup_json();
        assert!(!json.contains("nostr_nsec"));
        let cfg = core.sync_nostr_config();
        assert!(cfg.has_identity);

        drop(core);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Legacy pre-v4 location: the reader falls back, then migrates the
    // value into the vault and purges both synced/plaintext copies.
    #[test]
    fn test_load_nostr_nsec_migrates_legacy_setting_on_read() {
        let (dir, path) = tmp_db_path("legacy");
        let hex_nsec = "ab".repeat(32);
        {
            let db = Database::open(&path, None).unwrap();
            db.conn()
                .execute(
                    "INSERT INTO user_settings (key, value) VALUES ('nostr_nsec', ?1)",
                    [&hex_nsec],
                )
                .unwrap();
        }
        let db = Database::open(&path, None).unwrap();
        assert_eq!(load_nostr_nsec(&db).as_deref(), Some(hex_nsec.as_str()));
        assert_eq!(
            greek_core::secret_get(&db, "nostr_nsec").unwrap().as_deref(),
            Some(hex_nsec.as_str()),
            "value must land in the secrets vault"
        );
        for table in ["user_settings", "settings_sync"] {
            let n: i64 = db
                .conn()
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE key = 'nostr_nsec'"),
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 0, "{table} copy must be purged");
        }
        nostr_keys(&db).expect("identity must parse after migration");

        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // S5 REGRESSION: a backup import that fails LATE — the review applies
    // cleanly but a settings row carries a malformed HLC — returns Err AND
    // rolls back the WHOLE import: no review_logs row, no folded
    // memory_state, no `theme` in either settings table (the probe case was
    // exactly "error returned, yet review + theme were both retained").
    #[test]
    fn test_import_backup_error_rolls_back_reviews_and_settings() {
        let (dir, path) = tmp_db_path("import-rollback");
        let core = GreekCore::new(path.clone(), None).unwrap();

        let json = serde_json::json!({
            "v": 1,
            "device_id": "feedface-0000-0000-0000-000000000000",
            "from_hlc": "0000000000000-0000-feedface",
            "to_hlc": "1762000000000-0000-feedface",
            "reviews": [{
                "uuid": "11111111-2222-3333-4444-555555555555",
                "card_id": "card-rollback-1",
                "timestamp": "2025-10-09T12:00:00Z",
                "rating": 3,
                "elapsed_days": 0.0,
                "scheduled_days": 1.0,
                "source": "flashcard",
                "device_id": "feedface-0000-0000-0000-000000000000",
                "hlc": "1760000000000-0000-feedface"
            }],
            // Late failure: the review inserts fine, THEN this setting's
            // malformed HLC aborts the import.
            "settings": [["theme", "dark", "not-an-hlc"]]
        })
        .to_string();

        let err = core
            .sync_import_backup_json(json)
            .expect_err("the malformed settings HLC must fail the import");

        match err {
            // anyhow-sourced core errors surface as Database; either variant
            // proves the import failed — the ASSERTIONS below are the point.
            GreekError::Sync(_) | GreekError::Database { .. } => {}
            other => panic!("expected a Sync/Database error, got {other:?}"),
        }

        // Nothing survived: inspect the same database file directly.
        drop(core);
        let db = Database::open(&path, None).unwrap();
        let reviews: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM review_logs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(reviews, 0, "the failed import must not retain reviews");
        let folded: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM memory_states", [], |r| r.get(0))
            .unwrap();
        assert_eq!(folded, 0, "the aborted fold must leave no memory state");
        for table in ["user_settings", "settings_sync"] {
            let n: i64 = db
                .conn()
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE key = 'theme'"),
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 0, "{table} must not retain 'theme' from a failed import");
        }

        // A later GOOD import of the same payload (fixed HLC) must apply
        // cleanly: the failed attempt left no uuid-dedup or LWW residue.
        let fixed = serde_json::json!({
            "v": 1,
            "device_id": "feedface-0000-0000-0000-000000000000",
            "from_hlc": "0000000000000-0000-feedface",
            "to_hlc": "1762000000000-0000-feedface",
            "reviews": [{
                "uuid": "11111111-2222-3333-4444-555555555555",
                "card_id": "card-rollback-1",
                "timestamp": "2025-10-09T12:00:00Z",
                "rating": 3,
                "elapsed_days": 0.0,
                "scheduled_days": 1.0,
                "source": "flashcard",
                "device_id": "feedface-0000-0000-0000-000000000000",
                "hlc": "1760000000000-0000-feedface"
            }],
            "settings": [["theme", "dark", "1760000000001-0000-feedface"]]
        })
        .to_string();
        drop(db);
        let core = GreekCore::new(path, None).unwrap();
        let stats = core.sync_import_backup_json(fixed).unwrap();
        assert_eq!(
            (stats.reviews_received, stats.settings_applied),
            (1, 1),
            "retry after a rolled-back import must apply in full"
        );

        drop(core);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- S1: SyncBatch v2 — the app round-trips its OWN created data ---

    /// S1 HEADLINE (the audit probe as a regression test): fresh core A
    /// adds a real Greek lookup card (id `lookup-λόγος`), reviews it once,
    /// marks the lemma known, and manually unlocks a chapter; export →
    /// import into fresh core B must restore ALL of it: the review (counted
    /// + folded to the byte-identical memory_state), the `lookup_cards`
    /// content row, the known word, and the chapter unlock.
    #[test]
    fn test_s1_greek_lookup_backup_roundtrip() {
        let (dir_a, path_a) = tmp_db_path("s1-a");
        let (dir_b, path_b) = tmp_db_path("s1-b");
        let letters = include_str!("../../../data/cards/letters.json").to_string();

        let a = GreekCore::new_with_cards(path_a, None, letters.clone()).unwrap();
        let card_id = a
            .add_card_from_lookup(
                "λόγος".to_string(),
                "".to_string(),
                "word; reason".to_string(),
            )
            .unwrap();
        assert_eq!(card_id, "lookup-λόγος");
        assert!(a.submit_review(card_id.clone(), 3), "one review lands");
        a.mark_word_known("λόγος".to_string(), true).unwrap();
        assert!(a.unlock_chapter("letters".to_string(), 2), "manual unlock");

        let state_a = {
            let db = a.db.lock().unwrap();
            memory_state_snapshot(&db, &card_id).expect("one review must fold a memory row")
        };

        let json = a.sync_export_backup_json();
        assert!(json.contains("\"v\":2"), "export must be wire v2: {json}");
        assert!(
            json.contains("lookup-λόγος"),
            "export must carry the Greek id"
        );

        let b = GreekCore::new_with_cards(path_b, None, letters).unwrap();
        let stats = b.sync_import_backup_json(json).unwrap();
        assert_eq!(
            stats.reviews_received, 1,
            "the Greek lookup review must APPLY (the S1 bug dropped it): {stats:?}"
        );

        // 1. Review counted AND folded byte-identically to A.
        let state_b = {
            let db = b.db.lock().unwrap();
            memory_state_snapshot(&db, &card_id).expect("imported review must fold a memory row")
        };
        assert_eq!(state_a, state_b, "memory_states must converge");

        // 2. lookup_cards content row restored (an unreviewed lookup card
        //    needs nothing else — schema v5 stores no memory row for it).
        {
            let db = b.db.lock().unwrap();
            let (front, back): (String, String) = db
                .conn()
                .query_row(
                    "SELECT front, back FROM lookup_cards WHERE card_id = ?1",
                    [&card_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .expect("lookup_cards content row must be restored");
            assert_eq!(
                (front, back),
                ("λόγος".to_string(), "word; reason".to_string())
            );
        }

        // 3. Known word restored.
        assert!(b.is_word_known("λόγος".to_string()));
        assert!(b.get_known_words().iter().any(|w| w == "λόγος"));

        // 4. Manual chapter unlock restored (letters ch.2 is locked by
        //    default on a fresh install and the alphabet gate does not
        //    guard the letters track).
        let ch2 = b
            .get_chapters("letters".to_string())
            .into_iter()
            .find(|c| c.chapter == 2)
            .expect("letters ch.2 exists");
        assert!(ch2.unlocked, "manual chapter unlock must be restored");

        // Idempotent replay of the whole v2 payload.
        let stats = b
            .sync_import_backup_json(b.sync_export_backup_json())
            .unwrap();
        assert_eq!(stats.reviews_received, 0);

        drop(a);
        drop(b);
        let _ = std::fs::remove_dir_all(&dir_a);
        let _ = std::fs::remove_dir_all(&dir_b);
    }

    // S1 NFC: a card created from an NFD lemma spelling exports with that
    // id, and the import normalizes BOTH the review's card_id and the
    // lookup_cards key to the NFC form — so the restored content row and
    // the folded memory state key off the SAME id.
    #[test]
    fn test_s1_nfc_variant_roundtrips_through_id_gate() {
        let (dir_a, path_a) = tmp_db_path("s1-nfd-a");
        let (dir_b, path_b) = tmp_db_path("s1-nfd-b");

        // NFD spelling of λόγος: bare omicron + COMBINING TONOS (U+0301).
        let lemma_nfd = "λο\u{0301}γος".to_string();
        assert_ne!(lemma_nfd, "λόγος", "fixture must be NFD");
        let a = GreekCore::new(path_a, None).unwrap();
        let id_nfd = a
            .add_card_from_lookup(lemma_nfd, String::new(), "word".to_string())
            .unwrap();
        assert!(a.submit_review(id_nfd.clone(), 4));

        let b = GreekCore::new(path_b, None).unwrap();
        let stats = b
            .sync_import_backup_json(a.sync_export_backup_json())
            .unwrap();
        assert_eq!(stats.reviews_received, 1);

        let nfc_id = "lookup-λόγος";
        let db = b.db.lock().unwrap();
        let review_id: String = db
            .conn()
            .query_row("SELECT card_id FROM review_logs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(review_id, nfc_id, "review ingress must NFC-normalize");
        let content_id: String = db
            .conn()
            .query_row("SELECT card_id FROM lookup_cards", [], |r| r.get(0))
            .unwrap();
        assert_eq!(content_id, nfc_id, "content ingress must NFC-normalize");
        // And the folded state hangs off the same single id.
        assert!(memory_state_snapshot(&db, nfc_id).is_some());
        let distinct: i64 = db
            .conn()
            .query_row("SELECT COUNT(DISTINCT card_id) FROM review_logs", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(distinct, 1, "NFC/NFD spellings converge to one card");
        drop(db);

        drop(a);
        drop(b);
        let _ = std::fs::remove_dir_all(&dir_a);
        let _ = std::fs::remove_dir_all(&dir_b);
    }

    // S1 VERSION POLICY (file path): a v1-shaped payload (reviews+settings
    // only — and its review targets a GREEK lookup id, the S1 poison case
    // replayed as legacy data) imports cleanly; a future version is
    // rejected whole, identically to the codec-level gate.
    #[test]
    fn test_import_v1_shape_ok_v99_rejected() {
        let (dir, path) = tmp_db_path("s1-versions");
        let core = GreekCore::new(path, None).unwrap();

        let v1_json = serde_json::json!({
            "v": 1,
            "device_id": "feedface-0000-0000-0000-000000000000",
            "from_hlc": "0000000000000-0000-feedface",
            "to_hlc": "1762000000000-0000-feedface",
            "reviews": [{
                "uuid": "99999999-2222-3333-4444-555555555555",
                "card_id": "lookup-λόγος",
                "timestamp": "2025-10-09T12:00:00Z",
                "rating": 3,
                "elapsed_days": 0.0,
                "scheduled_days": 1.0,
                "source": "flashcard",
                "device_id": "feedface-0000-0000-0000-000000000000",
                "hlc": "1760000000000-0000-feedface"
            }],
            "settings": [["theme", "dark", "1760000000001-0000-feedface"]]
        })
        .to_string();
        let stats = core.sync_import_backup_json(v1_json).unwrap();
        assert_eq!(
            (stats.reviews_received, stats.settings_applied),
            (1, 1),
            "a v1 batch with a Greek lookup id must import cleanly: {stats:?}"
        );
        // And it folded (the v1 payload carried no v2 collections and no
        // content row — the memory row alone proves the review landed).
        {
            let db = core.db.lock().unwrap();
            assert!(memory_state_snapshot(&db, "lookup-λόγος").is_some());
        }

        // version 99: rejected whole, nothing applied.
        let mut v99: serde_json::Value =
            serde_json::from_str(&core.sync_export_backup_json()).unwrap();
        *v99.get_mut("v").unwrap() = serde_json::json!(99);
        let err = core
            .sync_import_backup_json(v99.to_string())
            .expect_err("wire version 99 must be rejected");
        match err {
            GreekError::Sync(msg) => assert!(
                msg.contains("unsupported sync wire version 99"),
                "message must name the version: {msg}"
            ),
            other => panic!("expected GreekError::Sync, got {other:?}"),
        }

        drop(core);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
