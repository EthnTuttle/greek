# In-app Greek keyboard — design & scope (v0 for review)

Status: **proposal approved in principle by owner ("support both"), ready to
build as part of Wave 9 alongside progress sharing**.

## 1. Problem

Typed Greek input currently requires the user to install/enable a Greek IME in
Android system settings. For a learning app whose typed-answer drills are the
hardest evidence of recall, that setup cost is lethal friction. Ancient
(polytonic) Greek makes it worse: most stock Greek keyboards expose modern
monotonic input only.

## 2. Decision

**Support both.** In-app Greek keyboard available everywhere the app asks for
Greek text; if the user already has a system Greek IME focused/selected,
nothing blocks it. Scope is app-internal Compose UI — we do NOT build an
Android InputMethodService (an IME would add background-process privacy review
surface for zero benefit inside our own screens).

## 3. Where it appears (v1)

| Surface | Why |
|---|---|
| Practice typed-answer mode | core motivation — recall evidence |
| Letters track drills (type-the-letter) | alphabet mastery should test output, not just recognition |
| Dictionary search field | looking up inflected forms you just read |
| (future) Share-personal-note field | owner may type Greek suffixes — inherits automatically |

## 4. Design

- **Layout**: 24-letter grid (Α Β Γ …), shift doubles uppercase/lowercase,
  long-press or accents row for acute/grave/circumflex + breathing + diaeresis
  (polytonic set: ά ὰ ᾶ ἀ ἁ ἄ ἅ ἂ ἃ ϊ ϋ ί ΐ etc. via combining marks —
  implementation uses precomposed single-codepoint forms where they exist and
  NFC-normalizes on commit).
- **Behavior contract**: taps insert at cursor into the *currently focused*
  Greek-enabled input field within our app; never replaces system IME; no text
  leaves the process; no clipboard access; haptic on key press (respect system
  setting).
- **Settings**: "Greek input" tri-state under Practice settings:
  `in-app keyboard (default) | prefer system IME | both (in-app shown as
  collapsible strip)`. Persist in user_settings so it syncs across devices.
- **Ergonomic detail**: the strip docks above the system keyboard area so both
  can be co-present; tapping a key does not dismiss the system IME.

## 5. Implementation sketch (Android-first; desktop later)

- `GreekKeyboardPanel` Compose component (~150 lines): owns key grid + variant
  popup; emits commit events into a `GreekInputState` the text fields consume.
- Text fields switch from BasicTextField IME-mode to "manual insertion" mode
  when the panel drives them (still allow system IME editing — union, not xor).
- Letters-track integration: new drill variant "type the letter" reuses
  existing Practice engine with `question.type = typed_letter`.
- Desktop (Tauri): on-screen keyboard follows the same component when the
  desktop Practice screen ships (v1.1); keyboard shortcuts already exist for
  ratings.

## 6. Tests

- Unit: NFC normalization table (typed sequences == deck targets, so "typed
  α + combining acute" matches card text "ά"); variant popup completeness vs
  deck content audit (every codepoint used in letters/vocab decks is
  typeable).
- UI/integration via existing harness: letters drill completed end-to-end
  using ONLY the in-app keyboard on the bench device (E2E-INPUT suite slot
  reserved in scripts/e2e).

## 7. Out of scope

- Full predictive text / autocomplete, swipe typing, custom layouts, IME
  service, hardware-keyboard remapping, transliteration aids (a→α) — all
  v-next candidates.

## Appendix R — Review consolidation (2026-10-19; same panel). Binding decisions.

**Input contract (blocker):** `GreekInputController` interface
(`insertText(text, replaceComposing)`, `deleteBackward()`) exposed via
`CompositionLocal`; a `GreekTextField` wrapper owns `TextFieldValue` and does
selection/composition-preserving surgery (never clobber an in-flight IME
composition). Backspace deletes **grapheme clusters** (`BreakIterator` with
Greek locale) with press-and-hold repeat (500ms delay / 50ms interval).

**Embedding (blocker):** the panel is **screen-level content**, not a
BottomSheet: bottom of the screen Column behind `AnimatedVisibility`, content
padded via `WindowInsets.ime`; fixed 280dp height (capped at 40% in landscape;
side-docked layout on ≥600dp width). Both panel and system IME may coexist
without focus fights; no focus theft.

**Preference locality (blocker):** `greek_input_mode` is **device-local**
(DataStore), NOT synced LWW — phone-without-IME and tablet-with-IME differ by
definition. One-time migration reads the earlier synced value as the local
default.

**Grading policy (pedagogy blocker):** typed-answer correctness uses explicit
equivalence classes — STRICT: rough/smooth breathing (phonemic: rough = /h/),
iota subscript, diaeresis, and final-sigma folding (σ→ς word-final both sides
before compare). LENIENT until formally taught: pitch accents (acute/grave/
circumflex) — Erasmian does not pronounce them. Dictionary search uses a
separate looser normalization (case-fold + accent-insensitive), distinct from
grading normalization on purpose.

**Coverage audit (CI):** a Gradle task decodes every deck + the morphology
surface-form inventory, extracts all Greek codepoints (U+0370–03FF,
U+1F00–1FFF), and fails the build if any NFC form lacks a keyboard mapping —
the variant set must include iota-subscript precomposeds (ᾳ ῃ ῳ) and
uppercase+breathing forms (Ἀ Ἁ), not just the modern monotonic set.

**Drill modality:** `typed_letter` prompts by **Erasmian sound or letter name,
never the target glyph** (audio→type is production; glyph→glyph is copywork).
Single-letter drills use a dedicated auto-submit field (one grapheme
completes), not the vocab typed-answer composable.

**Ordering constraint:** Wave-9 ships keyboard panel → typed_letter drill →
typed-answer remains default-off until the panel exists. Gate criteria are
untouched (typed drills are additive, never retroactive).

**Rendering/a11y/perf:** single shared `Popup` for variant picker (no per-key
DropdownMenu recomposition), haptics via
`VIRTUAL_KEY + ENABLED_IF_USER_SETTING`, every key gets TalkBack semantics
(role/stateDescription; variant popup is Role.Menu with Role.MenuItem rows).
