/// LLM integration for grammar tutoring (stub for Phase 1).
/// The tutor will use Claude API with constraints:
/// - Never generate Greek text (unreliable)
/// - Only explain, quiz, and check user answers
/// - Guided by chapter-aligned grammar topics

pub struct LlmTutor {
    _private: (),
}

impl LlmTutor {
    pub fn new() -> Self {
        Self { _private: () }
    }
}

impl Default for LlmTutor {
    fn default() -> Self {
        Self::new()
    }
}
