use greek_core::{
    Analysis, Card, CardWithState, ChapterInfo, Curriculum, CurriculumInfo, Database, MorphologyDb,
    Reader, SearchEngine, SearchResult, SrsEngine, TextSection, Token,
};
use std::sync::{Arc, Mutex};

uniffi::setup_scaffolding!();

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
}

#[uniffi::export]
impl GreekCore {
    #[uniffi::constructor]
    pub fn new(db_path: String, morphology_db_path: Option<String>) -> Arc<Self> {
        let db =
            Database::open(&db_path, morphology_db_path.as_deref()).expect("Failed to open database");
        let srs = SrsEngine::new();
        let morphology = MorphologyDb::new();
        let search = SearchEngine::new();

        Arc::new(Self {
            db: Mutex::new(db),
            srs: Mutex::new(srs),
            morphology,
            search,
            cards: Vec::new(),
        })
    }

    #[uniffi::constructor]
    pub fn new_with_cards(
        db_path: String,
        morphology_db_path: Option<String>,
        cards_json: String,
    ) -> Arc<Self> {
        let db =
            Database::open(&db_path, morphology_db_path.as_deref()).expect("Failed to open database");
        let srs = SrsEngine::new();
        let morphology = MorphologyDb::new();
        let search = SearchEngine::new();
        let cards: Vec<Card> = serde_json::from_str(&cards_json).unwrap_or_default();

        Arc::new(Self {
            db: Mutex::new(db),
            srs: Mutex::new(srs),
            morphology,
            search,
            cards,
        })
    }

    // --- SRS Operations ---

    pub fn get_due_cards(&self, limit: i32) -> Vec<CardWithStateView> {
        let db = self.db.lock().unwrap();
        let srs = self.srs.lock().unwrap();
        let due = srs.get_due_cards(&db, limit as usize);

        let mut result: Vec<CardWithStateView> = Vec::new();
        for cws in &due {
            let mut view = card_with_state_to_view(cws);
            if let Some(card) = self.cards.iter().find(|c| c.id == cws.card.id) {
                view.card = card_to_view(card);
            }
            result.push(view);
        }

        result
    }

    pub fn get_new_cards(&self, textbook: String, chapter: i32, limit: i32) -> Vec<CardView> {
        let db = self.db.lock().unwrap();

        let chapter_cards: Vec<&Card> = self
            .cards
            .iter()
            .filter(|c| c.chapter == chapter && c.tags.iter().any(|t| t.contains(&textbook)))
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
            .take(limit as usize)
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

        let due_count: i32 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM memory_states WHERE due_date <= datetime('now')",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let reviewed_today: i32 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM review_logs WHERE timestamp >= date('now')",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let total_cards = self.cards.len() as i32;

        ReviewStats {
            due_count,
            reviewed_today,
            streak_days: 0,
            total_cards,
        }
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
        Curriculum::get_chapters(&db, &textbook)
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
}

// --- Standalone functions ---

#[uniffi::export]
pub fn parse_postag(postag: String) -> String {
    MorphologyDb::parse_postag(&postag)
}
