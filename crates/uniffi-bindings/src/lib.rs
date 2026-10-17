use greek_core::{
    now_iso8601, Analysis, Card, CardWithState, ChapterInfo, Curriculum, CurriculumInfo, Database,
    MorphologyDb, Reader, SearchEngine, SearchResult, SrsEngine, TextSection, Token,
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
        srs.submit_review(&db, &card_id, rating).is_ok()
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
        Curriculum::unlock_chapter(&db, &textbook, chapter).is_ok()
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
