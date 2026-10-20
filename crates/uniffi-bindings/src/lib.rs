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

    pub fn get_new_cards(&self, textbook: String, chapter: i32, limit: i32) -> Vec<CardView> {
        let db = self.db.lock().unwrap();

        // Alphabet gate: vocabulary new-card introduction is refused until
        // the learner proves alphabet fluency. (Due reviews via
        // get_due_cards are never gated.) The refusal lives here in the core
        // so every current and future UI path is capped consistently.
        if textbook != "letters"
            && !Curriculum::alphabet_gate_state(&db, &self.cards).satisfied
        {
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
            .filter(|c| {
                c.chapter == chapter && greek_core::card_in_textbook(c, &textbook)
            })
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
        srs.preview_intervals(&db, &card_id)
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
    /// in-memory deck. Cards that already have a memory state are offered
    /// first (the memory-state ids are queried once and the slice is ordered
    /// reviewed-then-unseen); within those tiers the seeded shuffle decides.
    /// The seed is unix-epoch minutes, so question sets vary per session.
    pub fn generate_practice_mcq(&self, limit: u32) -> Vec<McqView> {
        let db = self.db.lock().unwrap();

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

        // Reviewed cards first (deck order preserved within each tier).
        let mut ordered: Vec<Card> = Vec::with_capacity(self.cards.len());
        ordered.extend(
            self.cards
                .iter()
                .filter(|c| have_state.contains(&c.id))
                .cloned(),
        );
        ordered.extend(
            self.cards
                .iter()
                .filter(|c| !have_state.contains(&c.id))
                .cloned(),
        );

        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() / 60)
            .unwrap_or(0);

        greek_core::questions::generate_mcq_set(&ordered, limit as usize, seed)
            .into_iter()
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
    /// Returns false on error.
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
        if textbook != "letters"
            && !Curriculum::alphabet_gate_state(&db, &self.cards).satisfied
        {
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
        greek_core::questions::generate_alphabet_check(
            &self.cards,
            ALPHABET_CHECK_N as usize,
            seed,
        )
        .into_iter()
        .map(|q| AlphabetQuestionView {
            card_id: q.card_id,
            glyph_to_name: q.direction
                == greek_core::questions::AlphabetDirection::GlyphToName,
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

    /// Create a new SRS card + initial (immediately-due) memory_state for a word
    /// looked up in the reader. The card id is derived from the lemma so repeated
    /// lookups of the same word are idempotent (no-op if it already exists).
    pub fn add_card_from_lookup(
        &self,
        lemma: String,
        form: String,
        definition: String,
    ) -> Result<String, GreekError> {
        let card_id = format!("lookup-{}", lemma);
        let db = self.db.lock().unwrap();

        // If a memory_state already exists for this card, treat as a no-op.
        let exists: bool = db
            .conn()
            .query_row(
                "SELECT 1 FROM memory_states WHERE card_id = ?1",
                [&card_id],
                |_| Ok(true),
            )
            .unwrap_or(false);

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

        if !exists {
            let now = now_iso8601();
            // Initial state: neutral FSRS-ish values, due immediately.
            db.conn().execute(
                "INSERT INTO memory_states (card_id, stability, difficulty, last_review, due_date, reps, lapses)
                 VALUES (?1, ?2, ?3, NULL, ?4, 0, 0)",
                rusqlite::params![card_id, 0.0_f32, 0.0_f32, now],
            )?;
        }

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
/// mirroring how `hlc_now` persists its own state.
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

/// Build a v1 SyncBatch from local state after `since` (None = everything).
fn build_batch(
    db: &Database,
    since: Option<&str>,
) -> Result<greek_core::SyncBatch, GreekError> {
    let device_id = greek_core::device_id(db);
    let reviews = greek_core::events_since(db, since)?;
    let settings = greek_core::settings_since(db, since)?;
    Ok(greek_core::SyncBatch {
        v: 1,
        device_id,
        from_hlc: since.unwrap_or_default().to_string(),
        to_hlc: greek_core::hlc_now(db).to_string(),
        reviews,
        settings,
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

    /// Full backup: a complete v1 SyncBatch (`v`, all events, all settings)
    /// as JSON — the "export backup" / paired-file payload.
    pub fn sync_export_backup_json(&self) -> String {
        let db = self.db.lock().unwrap();
        build_batch(&db, None)
            .and_then(|b| serde_json::to_string(&b).map_err(GreekError::from))
            .unwrap_or_else(|e| format!("{{\"error\":\"{}\"}}", e))
    }

    /// Import a backup produced by `sync_export_backup_json`. STRICT: only
    /// wire version v==1 is accepted. Applies review events (uuid-dedup)
    /// and settings (HLC-LWW), then folds memory state.
    pub fn sync_import_backup_json(&self, json: String) -> Result<SyncStatsView, GreekError> {
        let batch: greek_core::SyncBatch =
            serde_json::from_str(&json).map_err(|e| sync_err(format!("bad backup JSON: {e}")))?;
        if batch.v != 1 {
            return Err(GreekError::Sync(format!(
                "unsupported sync wire version {} (this app speaks v1)",
                batch.v
            )));
        }
        let db = self.db.lock().unwrap();
        let reviews_received = greek_core::apply_review_events(&db, &batch.reviews)?;
        let settings_applied = greek_core::apply_settings(&db, &batch.settings)?;
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
            // Identity now lives in the secrets vault; the legacy
            // user_settings check only exists for not-yet-migrated reads.
            has_identity: greek_core::secret_has(&db, "nostr_nsec")
                || user_setting_get(&db, "nostr_nsec").is_some(),
        }
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
    pub async fn sync_nostr_push(&self) -> Result<SyncStatsView, GreekError> {
        let (keys, relays, batch) = {
            let db = self.db.lock().unwrap();
            let keys = nostr_keys(&db)?;
            let cursor = user_setting_get(&db, "nostr_last_push_hlc");
            let batch = build_batch(&db, cursor.as_deref())?;
            (keys, nostr_relays(&db), batch)
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
        user_setting_put_local(&db, "nostr_last_push_hlc", &to_hlc)?;
        Ok(SyncStatsView {
            reviews_sent,
            reviews_received: 0,
            settings_applied: 0,
        })
    }

    /// Pull all batches after the pull cursor from the nostr relays, apply
    /// them, and advance the (device-local) cursor.
    pub async fn sync_nostr_pull(&self) -> Result<SyncStatsView, GreekError> {
        let (keys, relays, cursor) = {
            let db = self.db.lock().unwrap();
            (
                nostr_keys(&db)?,
                nostr_relays(&db),
                user_setting_get(&db, "nostr_last_pull_hlc"),
            )
        };
        let relay_refs: Vec<&str> = relays.iter().map(|s| s.as_str()).collect();
        let transport = sync_nostr::NostrSpoolTransport::connect(keys, &relay_refs)
            .await
            .map_err(sync_err)?;
        let batches = transport
            .fetch_since(cursor.as_deref())
            .await
            .map_err(sync_err)?;
        let mut stats = SyncStatsView {
            reviews_sent: 0,
            reviews_received: 0,
            settings_applied: 0,
        };
        let db = self.db.lock().unwrap();
        let mut max_hlc: Option<String> = None;
        for batch in &batches {
            stats.reviews_received += greek_core::apply_review_events(&db, &batch.reviews)?;
            stats.settings_applied += greek_core::apply_settings(&db, &batch.settings)?;
            if max_hlc.as_deref() < Some(batch.to_hlc.as_str()) {
                max_hlc = Some(batch.to_hlc.clone());
            }
        }
        if let Some(h) = max_hlc {
            user_setting_put_local(&db, "nostr_last_pull_hlc", &h)?;
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
    /// peers (tolerates everyone being offline), pull every doc entry back,
    /// and merge (reviews uuid-dedup, settings HLC-LWW, then the FSRS fold).
    pub async fn sync_iroh_now(&self) -> Result<SyncStatsView, GreekError> {
        let t = self.open_iroh().await?;
        let (reviews, settings, new_cursor) = {
            let db = self.db.lock().unwrap();
            let cursor = user_setting_get(&db, "iroh_last_push_hlc");
            let reviews = greek_core::events_since(&db, cursor.as_deref())?;
            let settings = greek_core::settings_since(&db, cursor.as_deref())?;
            let new_cursor = greek_core::hlc_now(&db).to_string();
            (reviews, settings, new_cursor)
        };
        let reviews_sent = reviews.len() as u32;
        t.push_local(&reviews, &settings).await.map_err(sync_err)?;
        {
            let db = self.db.lock().unwrap();
            user_setting_put_local(&db, "iroh_last_push_hlc", &new_cursor)?;
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
}
