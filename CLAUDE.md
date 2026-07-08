# Greek Learning App

## Project Goal
Ship a complete Ancient Greek learning application (mobile + desktop) that combines:
- Textbook-chapter-aligned SRS (Athenaze vocabulary, FSRS algorithm)
- Tap-to-parse reader (Perseus morphological data, LSJ dictionary)
- LLM grammar tutor (Claude API with guiding-phrase constraints)
- Reading readiness progression (Steadman reader path)

## Architecture
- **Rust core**: morphological parser, SRS scheduler, text corpus + FTS5 index, vocabulary tracker, LLM integration
- **Desktop**: Tauri 2 (webview renders Greek text)
- **Web**: responsive SPA (ship first, fastest path to users)
- **Mobile (later)**: native shells (SwiftUI / Jetpack Compose) over UniFFI-wrapped Rust core

## Key Research (wiki)
- Wiki location: `~/.wiki/topics/ancient-greek-learning/`
- Architecture cross-ref: `compiled/references/app-architecture-crossref.md`
- LLM playbook: `compiled/concepts/llm-enhanced-learning.md`
- Reader comparison: `compiled/references/reader-apps-comparison.md`
- SRS tools: `compiled/references/srs-tools-for-greek.md`
- Sibling architecture: `~/.wiki/topics/rust-multi-platform/` and `~/.wiki/topics/open-source-logos-suite/`

## Data Sources (open, no copyright issues)
- Perseus Digital Library: 13.5M words + morphology (CC BY-SA)
- wmotte/llm_tool_greek_lexicon: 40K lemmas (CC0)
- Greek Learner Texts Project: annotated texts (MIT)
- LSJ dictionary: public domain (1940)
- Alpheios: embeddable reader JS library (OSS)

## Phases
1. **MVP (Phase 1)**: Web app — SRS flashcards + tap-to-parse reader + LLM grammar chat
2. **Phase 2**: Tauri desktop app, offline mode, full text corpus
3. **Phase 3**: Mobile (iOS/Android), sync, social features

## Constraints
- Never use copyrighted Athenaze text content — only vocabulary lists (factual data) and chapter ordering
- LLM must NEVER generate Greek text for the user (unreliable) — only explain, quiz, and check
- Prefer open-source dependencies; MIT/Apache-2.0 licensing for our code
- Offline-first architecture (SQLite + local corpus)
