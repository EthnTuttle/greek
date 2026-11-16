# Pedagogy review — "zero to Greek" for an absolute beginner

- **Reviewer:** Fable (pedagogy-focused review subagent)
- **Date:** 2026-11-14
- **Scope:** pedagogical correctness and learner flow only. No code, git or build changes were made.
- **Product goal under test:** "I want to go from zero to Greek with this application."

## How this was checked

Everything below comes from reading sources and data, plus cheap read-only Python/sqlite scripts. Nothing was built or run on a device.

- **Read:** `crates/core/src/{curriculum,srs,questions,morphology,reader,search}/mod.rs`, `crates/uniffi-bindings/src/lib.rs`, the Android view-models and screens, `desktop/src-ui/{index.html,app.js}`, `desktop/src-tauri/src/commands.rs`, `docs/guide.html`, `DESIGN-alphabet-gate.md`.
- **Data scripts:** counted and scanned `data/cards/*.json`; queried `data/morphology.sqlite` read-only with the same SQL the core uses.
- **Scheduling numbers:** from a Python re-implementation of `fsrs-2.0.4` (`model.rs`, default parameters) plus the app's step ladder. They are a model of the code, not a run of it.
- **Not verified:** Kotlin/Compose behaviour is from code reading only. Two claims rest on my recollection of the textbooks rather than a source in the repo; they are marked where they appear.

## Verdict in one paragraph

The engineering is sound, but the app is not yet a zero-to-Greek path. A novice can get through the alphabet in about a week if they discover two hidden steps. After that, the textbook track is serialized so hard that Athenaze chapter 2 cannot open sooner than about 23 days after vocabulary starts, and typically takes about 7 weeks. Meanwhile the learner is fed a synthesized "DCC" list containing wrong accents, wrong glosses and duplicate words. The reader shows the wrong dictionary entry for some of the commonest words in Greek. Typed practice marks correct answers wrong on roughly a third of vocabulary cards. Most fixes are small: defaults, copy, routing and a data pass.

## What already works for a beginner

- The gate idea is right: script before vocabulary, with a test-out lane, an organic lane and a self-declared lane. Due reviews are never gated (`curriculum/mod.rs:199-215`, `uniffi-bindings/src/lib.rs:492-509`).
- The letters deck content is good. Each of the 43 cards gives name, Erasmian value, IPA, an English anchor and a real example word (`data/cards/letters.json`).
- Interval previews on the rating buttons reflect the real schedule, including the 1m/5m/10m ladder (`srs/mod.rs:200-212`).
- One session draws on one daily new-card budget, due cards first (`lib.rs:481-571`).
- The Alphabet Check gives per-question feedback, alternates directions, is stratified by chapter, and recycles misses into review (`questions/mod.rs:403-506`).
- The in-app polytonic keyboard removes the usual "install a Greek IME" barrier.
- The χρή/δεῖ equivalence filter stops an equally valid answer being shown as wrong (`questions/mod.rs:179-201`).
- Sync, sharing and identities are off the critical path on Android (under Settings, off by default).

---

## 1. Onboarding and the alphabet gate

### What a first-time user sees

- There is no onboarding. Home opens with the heading "Welcome back!" (`HomeScreen.kt:49`), also on desktop (`desktop/src-ui/index.html:34`).
- The gate card says "Read Greek first" and offers **Continue letters** and **Take the Alphabet Check (~3 min)** (`HomeScreen.kt:57-129`, `strings.xml:77-82`).
- Below it are "Due today 0", "New today 0 / 10" and **Start Review**.
- Nothing explains the difference between Review, Practice and Progress across the six tabs (`AppNavigation.kt:70-77`).

### The gate as coded

| Lane | Rule | Source |
|---|---|---|
| Check | at least 22 correct of 24 | `curriculum/mod.rs:25-26, 380-382`; `lib.rs:878-888` |
| Organic | all 43 letters introduced, at least 20 practice-source attempts at 85% or better, activity on 2 or more UTC days | `curriculum/mod.rs:32-34, 344-348` |
| Declared | Settings → "I already know the Greek alphabet" | `lib.rs:892-896`; `SettingsScreen.kt:169-176` |
| Grandfather | any non-letters memory state | `curriculum/mod.rs:318-337` |

Letters chapters unlock on "seen twice" (reps of 2 or more on 80% of the chapter), not on 30-day stability (`curriculum/mod.rs:40, 403-407`).

### Is it a welcome or a wall?

It is passable, but the default path has dead ends before the first vocabulary word.

1. **The primary button leads nowhere.**
   - "Continue letters" opens the Progress tab (`AppNavigation.kt:177-181`).
   - Tapping an unlocked chapter there only re-calls `unlockChapter` and shows nothing (`CurriculumViewModel.kt:135-176`).
   - There is no letters lesson screen. Letters are actually learned through **Start Review**, which the card does not mention.
   - "Study letters" on the failed-check screen goes to the same list (`AppNavigation.kt:225-227`).

2. **Chapter unlocking is manual and invisible on Android.**
   - New cards come only from chapters whose stored `unlocked` flag is set (`lib.rs:538-541`).
   - Meeting the requirement does not unlock anything. The user must tap the locked row (`curriculum/mod.rs:511-520`).
   - `ChapterItem` renders locked, ready-to-unlock and unlocked rows identically: no lock icon, no "ready" badge (`CurriculumScreen.kt:212-296`).
   - A successful unlock gives no snackbar and no refresh.
   - Review's empty state says "Unlock a new chapter to keep learning" (`ReviewScreen.kt:51`) without saying where or how.
   - Desktop is better here: it shows a lock, the hint and an "Unlock" button (`app.js` `chapterRow`).

3. **Minute three of day one is a dead end.**
   - The first session is the 7 vowels only, because chapter 2 is locked.
   - Rating them Good schedules them 10 minutes out (`srs/mod.rs:59-61`), so nothing comes back in a 2-minute session.
   - The session ends with "Great work! You've completed all cards for today" (`ReviewScreen.kt:350`), which is false: 7 cards are due in 10 minutes.
   - Home shows "Due today 0". Starting Review again gives "No cards due… Unlock a new chapter", and the chapter hint reads "Master 80% of Chapter 1 to unlock (0/6)".
   - The real next step is not stated: come back in 10 minutes, review again, then tap chapter 2 in Progress.

4. **"Easy" on a new letter stalls the track.**
   - Easy on a new card skips the ladder and takes the FSRS interval, about 15.7 days (`srs/mod.rs:688-699`; default `w[3]` = 15.69).
   - The card then has one rep and is not due for two weeks, so it does not count toward the reps-of-2 unlock rule.
   - A learner who rates the Latin-looking vowels Easy cannot unlock chapter 2 through Review for 16 days. The only ways out (Practice, typing drill, check) are not signposted.

5. **Failing the check gives little help.**
   - The verdict screen shows the score and "The ones you missed are already scheduled for review" (`strings.xml:93`).
   - It does not list the missed letters, and it has no button to review them. They are due in one minute, but "Study letters" goes to the Progress list (`PracticeScreen.kt:377-467`).
   - Each check answer creates a memory state, and `new_cards_today` counts first-ever reviews from any source (`srs/mod.rs:377-391`). A day-one check therefore uses up more than the 10-card allowance, so no further letters can be introduced that day.
   - That behaviour is acceptable as remediation (the misses become the first lesson), but the UI does not say so.

6. **The check tests labels the cards never teach.**
   - A full check draws 6 of its 24 questions from the diphthong chapter (4 eligible chapters, round-robin).
   - 7 of the 11 diphthong labels are fallbacks from `diphthong_romanization` (`ei, ui, eu, ēu, āi, ēi, ōi`; `questions/mod.rs:362-377`). They do not appear on the card backs: letters-034 says only "a long close e, as in 'vein'".
   - They can be deduced, but only 2 misses are allowed, so diphthong transliteration is the likeliest place for a card-taught novice to fail.
   - Chapter 4 (breathings and accents) is not assessed at all (`questions/mod.rs:327-329`).

7. **The organic lane is opaque and its accuracy signal is polluted.**
   - The gate card shows "Letters n/5 stages · Practice accuracy x% (goal 85%)" (`strings.xml:79`). It does not show the 43-introduced, 20-attempt or 2-day requirements, so a learner at 5/5 and 90% cannot tell why vocabulary is still locked.
   - The Practice tab is the obvious way to produce practice-source attempts. Half its questions are typed (`PracticeViewModel.kt:19`), and typed answers are graded against the raw front (`lib.rs:745-753`).
   - For letters the front is `"Α α"`, `"Σ σ/ς"` or `"ζ ξ ψ"`. Typing `α` for "alpha" is graded wrong, because `normalize_form("Α α")` is `"α α"`. 25 of 43 letter fronts have this shape.
   - The multiple-choice half leaks answers. Chapter 5 prompts begin with the answer ("αι — ai, as in 'aisle'"), and chapter 4 prompts contain the glyph.
   - Two paths give a clean signal: the "Type the letters" drill (`LetterDrillViewModel.kt`, reachable only from a button on Progress) and the check itself.

### Realistic time to first vocabulary word

- A diligent novice who finds the "return after 10 minutes, then tap the next chapter" rhythm introduces all 43 letters by day 5–6 and can pass the check around day 5–7. This matches the design estimate.
- One who does not find the unlock tap is stuck on the 7 vowels indefinitely, unless they try the check or Practice.

---

## 2. Curriculum shape and pacing

### Deck inventory

| Deck | Cards | Chapters with cards | Notes |
|---|---|---|---|
| Letters | 43 | 5 (7/9/9/7/11) | |
| Athenaze I | 340 | 1–16 (21–23 each) | includes 36 principal-parts cards |
| Athenaze II | 119 | 17–25 (13–15 each) | chapters 26–31 show "Coming soon" |
| JACT | 131 | 1–10 (12–15 each) | sections 11–20 show "Coming soon" |
| "DCC Core" | 550 | one pseudo-chapter | 500 words plus 50 principal-parts cards |

- New cards are served in a fixed order: letters → athenaze → jact → dcc (`lib.rs:532`).
- The cap is 10 new cards per day, hard-coded and not adjustable (`srs/mod.rs:34`; `HomeScreen.kt:163`).
- A chapter unlocks when 80% of the previous chapter has FSRS stability of 30 days or more (`curriculum/mod.rs:8-20`), and then needs the manual tap.

### The 30-day gate stalls the textbook track

Days from the start of vocabulary until 80% of each Athenaze chapter is "mastered", from the FSRS re-implementation (400 simulated learners per row, daily use, reviews done when due):

| Learner behaviour | Ch.2 opens | Ch.3 opens | Ch.4 opens | Ch.5 opens |
|---|---|---|---|---|
| Ideal: always Good, never lapses | 23 | 46 | 69 | 92 |
| 20% first-rated Again, 5% lapses (median) | 24 | 49 | 76 | 100 |
| 50% first-rated Again, 10% lapses (median) | 47 | 88 | 129 | 172 |
| 90% first-rated Again, 10% lapses (median) | 53 | 105 | 156 | 207 |

Per-card time to reach 30-day stability, from the same model:

| Rating history | Days to "mastered" |
|---|---|
| Good every time | 18.7 (4 reviews) |
| Easy first, then Good | 15.7 (2 reviews) |
| Again on first sight, then Good | 35 |
| One lapse after graduating | 45–51 |

What this means for the learner:

- Athenaze chapter 2 cannot open before about day 23 of vocabulary, whatever the learner does.
- All 16 chapters of Athenaze I take about a year at best and about two years at typical accuracy. A class covers them at roughly a chapter a week.
- The rule rewards pressing Easy on a first exposure and penalizes an honest Again. Self-rating then stops being honest.
- Android shows "0% complete (0/23)" on a chapter for the first three weeks or more, however much the learner has reviewed (`CurriculumScreen.kt:289`).

### It is not a deadlock, but it is a drift

- The 10 cards a day never run dry. After Athenaze chapter 1 (23 cards, days 1–3) and JACT chapter 1 (13), everything comes from the "DCC" deck: about 200 cards by the time chapter 2 opens.
- Those are bare function words with no grammar context, for example `ἄν = modal particle (with subj./opt.)`, `ὅστις`, `ἵνα`.
- There is no track choice. A JACT learner gets Athenaze chapter 1 first, and nobody can opt out of DCC.
- Chapters with no cards never block later ones (`curriculum/mod.rs:16-20`). That part is handled correctly.

### Duplicate learning units

- The 1,140 vocabulary cards contain only about 690 distinct headwords. 257 headwords appear on 2–6 cards each (707 cards in total).
- 205 of those headwords are duplicated across decks, for example `καί` ×3, `φέρω` ×6 counting principal-parts cards, `πρός` ×5.
- Each copy is a separate "new" card with its own memory state. Mastering `καλός` in DCC does nothing for Athenaze chapter 1.
- At 10 cards a day, the real rate of new words is well under 10.

---

## 3. Item quality

### Question shapes

| Surface | Direction | Grading |
|---|---|---|
| Review flashcard | Greek → English | self-rated |
| Practice multiple choice | English → Greek | correct maps to Good |
| Practice typed | English → Greek | exact maps to Easy, accent-free maps to Good |
| Letter typing drill | name → glyph | correct and exact maps to Easy |
| Alphabet Check | both directions | correct maps to Good |

There is no measured Greek → English question, although that is the reading direction the app is aiming at.

### Typed grading produces false "wrong" answers

- `grade_typed` compares against the raw card front: exact, or equal after stripping all diacritics and case (`questions/mod.rs:111-121`; `morphology/mod.rs:11-18`). NFC and accent leniency are fine.
- Practice alternates typed and multiple choice by index, regardless of the card's shape (`PracticeViewModel.kt:19`).
- Vocabulary fronts by shape:

| Front shape | Cards | Share of vocabulary | Typed outcome |
|---|---|---|---|
| Single word | 520 | 46% | fair |
| Article + noun (`ὁ ἀγρός`) | 211 | 18% | wrong unless the article is typed; the prompt never asks for it |
| Punctuation or multi-part (`μακρός, -ά, -όν`, `πρός (+ acc.)`, `οὐ, οὐκ, οὐχ`, `ἐστί(ν)`, principal-parts lists) | 409 | 36% (48% of DCC) | effectively always wrong |

- A wrong answer is submitted as Again (`lib.rs:775-781`). On a graduated card that is an FSRS lapse: stability collapses and the 30-day unlock clock restarts. A learner who knows the word is told they don't, and loses weeks of chapter progress.
- Final sigma is not folded in the core. `λογοσ` is graded wrong for `λόγος`.
- The in-app keyboard appears to make this worse. `resolveSigma` emits `ς` whenever the cursor is at the end of the field after a Greek letter, and I found no correction when another letter follows (`GreekInputController.kt:212-243`). Typing `ἐστί` left to right would give `ἐςτί`, which the core grades wrong. This is from code reading and should be confirmed on a device.
- The Kotlin grader used by the letter drill does fold sigma and is strict on breathings. The two graders disagree.

### Multiple-choice issues

- **Plain verb shown as wrong.** 47 of the 68 Athenaze/JACT principal-parts cards share a chapter with the plain-verb card for the same word. The prompt "flee, escape (principal parts)" can list `φεύγω` as a wrong choice, because the fronts and glosses differ and `answers_equivalent` passes them. With only 8–10 verbs per chapter in the distractor pool, this will be common.
- **Answer in the prompt.** The 50 DCC principal-parts cards are reversed (front `ποιέω`, back the list), so the prompt begins with the answer.
- **In-deck twins.** Pairs such as dcc-078 `χρόνος, ὁ` and dcc-371 `χρόνος` can appear as each other's wrong choice.
- **Otherwise plausible.** Distractors drawn from the same part of speech and chapter are a sound default.
- **Practice ignores chapter locks.** When fewer than 10 reviewed cards exist, Practice fills from all unseen eligible cards (`lib.rs:702-732`). It quizzes never-taught words and creates memory states for them, which uses up the daily allowance.
- **Misleading empty-state copy.** "Practice unlocks after some review" (`strings.xml:53`) does not describe this behaviour.

### Card content errors

A scan found the following. Each is small, but each teaches something false.

| Card | Shows | Problem |
|---|---|---|
| ath1-052 | `ὁ αὐλός` = "the furrow; pipe" | αὐλός is a pipe or flute; furrow is ἡ αὖλαξ |
| dcc-057 | `εἰμί (εἶμι)` = "to go" | leads with εἰμί "to be" |
| dcc-089 | `ἀνθρώπος, ὁ` | accent should be ἄνθρωπος |
| dcc-201, dcc-365 | `μᾶλα` | should be μάλα (circumflex on a short vowel) |
| dcc-264 | `πράγμα` | should be πρᾶγμα |
| dcc-320 | `ὅρος, ὁ` = "boundary, mountain" | mountain is τὸ ὄρος |
| dcc-442 | `βασίλεια, ἡ` = "kingdom, queenship" | βασίλεια is "queen"; kingdom is βασιλεία |
| dcc-227 | `τοτέ` = "at that time, then" | a twin of dcc-120 `τότε`; τοτέ means "at times" |
| dcc-286, 347, 377, 400 | `εἴδω`, `παρεῖναι`, `ζῆν`, `ἄξιον` | pseudo-headwords duplicating real entries |
| dcc-pp-002, dcc-pp-011 | `…/ἐλέχθη`; δύναμαι's aorist in two slots | wrong form (ἐλέχθην) and misplaced parts |
| jact-014 and similar | `τό πλοῖον` | acute where the Athenaze deck writes `τὸ` |

Deck-level problems:

- **The "DCC Core" deck is not the DCC list.** From about #300 the entries cycle alphabetically α→ω several times (`…ἅρμα, βία, γελάω, δένδρον, ἐπιστολή, ζῷον, ἥλιος, θεραπεύω…`), which no frequency list does. It has 24 in-deck duplicate headwords (`μᾶλλον` ×3, `νέος` ×2, `ῥήτωρ` ×2, `πράττω`/`πράσσω`). The frequency-band tags and the "Frequency List" title therefore overclaim.
- **Athenaze chapter assignment looks approximate.** From my memory of the book, ὁ ἄνθρωπος and ὁ σῖτος are chapter 1 words; the deck has them in chapters 8 and 3. Please check against the book before promising "Athenaze chapter N".
- **Principal parts arrive too early.** Six-part lists with perfect passives appear from chapter 5, before the learner has met a past tense. Card direction also differs: Athenaze/JACT show the list and ask the meaning, DCC shows the verb and asks the list.
- **One letters front has two answers.** `ᾳ` is the front of both letters-032 ("Iota subscript…") and letters-041 ("long a with iota glide"), so a self-rating learner cannot tell which is wanted.

---

## 4. Retention experience (SRS)

### The schedule as coded

- While a card is "learning" (every review so far was Again, or the card is new), the ladder applies: Again 1 minute, Hard 5 minutes, Good 10 minutes. Easy graduates at once (`srs/mod.rs:59-61, 663-700`).
- Any passing rating ends learning. After that everything is FSRS at 90% desired retention.
- A session is up to 20 cards: due cards, most overdue first, then new cards in curriculum order (`lib.rs:481-571`; `ReviewViewModel.kt:348`).
- After each answer the single most-overdue due card is appended if it is not already queued, up to 60 cards (`ReviewViewModel.kt:196-216`).

### Findings

1. **New cards are tested before they are taught.**
   - A never-seen card is shown as front → "Show Answer" → four ratings (`ReviewScreen.kt:98-243`).
   - Android gives no "New" marker (desktop has a chip) and no guidance on what to rate something you have never seen.
   - Easy is on offer for 16 days. An honest Again sends the card down the 35-day path under the chapter gate.

2. **There is no relearning step after the first pass.**
   - One Good graduates the card. Failing it at its 10-minute check sends it 1.6 days out, not 1 minute.
   - A mature card that lapses returns hours to days later (about 2.5 days in my model for a card at 14-day stability).
   - The guide describes this as a deliberate choice. For a beginner it means "you forgot this" is followed by silence, with no short re-exposure, at exactly the moment a relearn step helps most.

3. **Same-session recycling works with gaps.**
   - Again cards are picked up once the rest of the queue is done and their minute has passed.
   - A miss on the last card or two ends the session with "completed all cards for today" while cards are due in under a minute.
   - Good-rated new cards (10 minutes) almost never return in the same session.
   - Desktop has no requeue at all. `review_rate` pops the card and never rebuilds (`commands.rs:143-160`), so misses return only when the Review tab is reopened. This contradicts guide §2.

4. **Due cards first, then new, is a reasonable default.**
   - When 20 or more cards are due, the session contains no new cards and the completion screen does not say that more are waiting.
   - With 10 new cards a day, reviews by week 3–4 will be several sessions a day. An "N more due — continue" button would be more honest than "Great work!".

5. **Drill outcomes over-credit.**
   - Every correct letter-drill keystroke is submitted as correct and exact, which maps to Easy (`LetterDrillViewModel.kt:129-149`; `lib.rs:775-781`).
   - One correct tap on an unseen letter schedules it about 16 days out with a single rep.
   - A lucky multiple-choice guess (25%) on an unseen card introduces it as Good without it having been taught.
   - The design document specifies that a hit should map to Good.

6. **Days are counted in UTC.**
   - The daily cap and the organic lane's "2 distinct days" use UTC dates (`srs/mod.rs:378-390`; `curriculum/mod.rs:247`).
   - For a US evening learner the allowance resets mid-session, and one sitting can count as two days.

---

## 5. Reader and dictionary

### The reader is not beginner input

- It holds three hard-coded passages of unadapted Greek: an Aesop fable, Anabasis 1.1.1 and the opening of Lysias 1 (`reader/mod.rs:23-46`).
- The "easy" fable contains genitive absolutes, aorist middle participles and a relative clause with ἄν and the subjunctive.
- Nothing is graded to the decks, and nothing uses the learner's known vocabulary.
- For a novice the reader is alphabet-decoding practice at best. Word learning in context is not delivered; the app teaches isolated word pairs.

### Tap analysis misleads on the commonest words

- `MorphologyDb::lookup` tries an exact match, then an accent-stripped match, with no ordering (`morphology/mod.rs:39-105`).
- The sheet shows only the first row's lemma and gloss. Other parses are listed as bare tag strings without their lemma (`ReaderViewModel.kt:135-147`).
- Results from running the same SQL against the shipped database:

| Tapped | Shown first | Should be |
|---|---|---|
| `τὴν` | ἄτη, "bewilderment, infatuation…" (1 of 50 rows) | ὁ, "the" |
| `τὼ` | ἄατος, "insatiate" | ὁ |
| `ὃς` | ὅσος, "quantus" (a Latin gloss) | ὅς, "who, which" |
| `λέγει` | λέγω, "to lay asleep, lull to sleep" | λέγω, "say" |
| `ἐν` | εἰμί, "to be" (3rd plural imperfect) | ἐν, "in" |
| `ἦν` | ἐάν, "if haply" | εἰμί, "was" |
| `οἰκεῖ` | οἰκεύς, "an inmate of one's house" | οἰκέω |
| `ἂν`, `μᾶλλον` | blank gloss | — |
| `ὑμῖν`, `σύ` | "Not found in dictionary" | — |

- The grave accent is the main cause. Running text writes final acutes as graves, the forms table stores acutes, and so the lookup falls through to the accent-stripped match, which merges homographs.
- In my replay, 17 word tokens across the three passages fell through to the accent-stripped match. Converting grave to acute first turns 16 of them into exact matches, and the first row is then the right lemma for every function word (`τὴν`, `τὼ`, `ὃς`, `περὶ`, `μὲν`, `ἐμοὶ`). It does not fix ranking on its own: `τελευτὴν` still lists the verb τελευτάω before the noun, and `ἂν` still has a blank gloss.
- In the "easy" fable 5 of 75 word tokens are unresolvable: `ἐκδύσῃ`, `ἀντεχομένου`, `καταπονούμενος`, `ἀποκαμὼν`, `προσέλαμψε`.
- Participle labelling is correct after the recent fix ("participle singular aorist middle…").

### "Add to SRS" cards never reappear

- After the C2 fix, `add_card_from_lookup` writes only a `lookup_cards` row and no memory state (`lib.rs:967-989`). That is correct for scheduling.
- `build_review_session` offers new cards only from the bundled decks (`lib.rs:532-567`). Lookup cards are loaded only when they are already due (`lib.rs:499-506`).
- So a looked-up word gets the snackbar "Added … to your review deck" (`ReaderViewModel.kt:201`) and then can never appear in a session on either platform.
- The comment at `HomeScreen.kt:33-36` still assumes the old behaviour.
- The lookup flow correctly gives no gate credit.
- If surfaced, the card would be the tapped inflected form (possibly with a grave accent) against the first row's gloss, with no lemma, parse or sentence. Combined with the mis-ranking above, that can produce cards like `τὴν` → "bewilderment".

### Dictionary

- Search normalization is good. `λογος`, `λογοσ`, `ΛΟΓΟΣ` and the prefix `λογ` all find λόγος (`search/mod.rs:70-115`).
- The subtitle honestly says "Morpheus glosses (LSJ pending)".
- 2,055 of 36,165 lemmas have a blank gloss. Among card headwords these include `μᾶλλον`, `μάλιστα`, `ἄν`, `μέντοι`, `ταχέως`, `τρεῖς` and `ὀκτώ`.
- 51 card headwords have no lemma row. These include the Attic `-ττ-` spellings the course uses (`θάλαττα`, `πράττω`, `τάττω`, `γλῶττα`), pronouns (`σύ`, `ἡμεῖς`, `ὑμεῖς`) and deponents (`πορεύομαι`, `ἀποκρίνομαι`, `ψεύδομαι`).
- A beginner who searches for a word from their own flashcard can get "No results found".

### Misleading Home statistic

"Words learned" counts words marked known in the reader (`HomeViewModel.kt:76-81`). A learner with 200 flashcards under review sees "Words learned: 0".

---

## 6. Cross-platform flow

| Capability | Android | Desktop |
|---|---|---|
| Review flashcards | yes; in-session requeue; session survives process death | yes; "New card" chip; no requeue |
| Alphabet Check | yes (Home, Progress, Settings) | yes (Curriculum tab) |
| "I already know the alphabet" | Settings | absent |
| Practice (multiple choice and typed) | yes | absent |
| Letter typing drill | yes (button on Progress) | absent |
| Reader | yes | absent |
| Dictionary | yes, with in-app keyboard | yes |
| Chapter lock, hint and unlock button | none visible; snackbar on tap | lock icon, hint, "Unlock" button |
| Sync | Settings → Device sync | top-level tab |

- A desktop-only novice has a simpler path that works: flashcards, visible unlocks and the check. They have no Practice and no reader, and they reach the organic lane only through repeated failed checks.
- The desktop Sync tab puts "iroh ticket", "nostr relays" and "Ensure identity" one click from Home. It is not on the critical path, but it is the only place where a beginner meets unexplained jargon at the top level.
- On Android, sync and sharing are properly behind Settings with consent dialogs.
- `docs/guide.html` matches the code on the gate, cap, lanes and gloss caveats. It differs in four places:
  - "Misses come back the same session" is untrue on desktop and partial on Android.
  - It says Practice draws "from your own deck"; Practice also draws unseen cards.
  - It presents typed grading as reliable, without the front-shape problem.
  - It does not mention that unlocking a chapter needs a tap.
- No flow requires technical knowledge to reach vocabulary. The hidden steps are pedagogical, not technical: the ten-minute return, the unlock tap, and knowing that "Start Review" means "learn letters".

---

## 7. Ranked recommendations

Each item gives the smallest change that would resolve it.

### PEDAGOGY-P0 — incorrect teaching or harmful progression

1. **Reader shows the wrong lemma for common words (§5).**
   - Convert grave to acute before the exact match in `MorphologyDb::lookup`.
   - Show every distinct lemma and gloss in the sheet, not just the first row.
   - Order candidates: closed-class lemmas (article, pronoun, preposition, particle, εἰμί) and lemmas present in the decks first.
   - Add override glosses for `ἄν`, `μᾶλλον`, `μάλιστα`, `μέντοι`, `ταχέως`, and the right `λέγω` sense.

2. **Card data errors and the mislabelled "DCC" deck (§3).**
   - Fix the listed cards in `data/cards/*.json`, keeping IDs so history survives.
   - Either import the real DCC core list (I believe it is published under a Creative Commons licence; check the terms) or rename the deck "Core vocabulary (unofficial)" and drop the frequency claim.
   - Retire in-deck duplicates and add a lint (duplicate headword, impossible accent) to the data pipeline, which already validates inputs.

3. **Typed practice marks correct answers wrong and lapses known cards (§3).**
   - Interim: serve a typed question only when the front is a single Greek word, and multiple choice otherwise.
   - Then: grade against an answer key derived from the front (headword before the first comma or parenthesis, article optional, `(ν)` optional).
   - Fold σ/ς in `grade_typed`.
   - Confirm the keyboard's medial-sigma behaviour on a device.
   - Consider never mapping a practice miss on a graduated card to a full Again. The measured-accuracy log can record it without resetting the schedule.

4. **The 30-day chapter gate over-serializes the course (§2).**
   - Unlock on "learned", not "mature": for example, 80% of the chapter graduated, reviewed on 2 or more distinct days, and last rated Hard or better. That takes about 3–5 days per chapter.
   - Keep 30-day stability as a separate "mastered" count for display.
   - This is a change to one predicate in `get_chapters`. It also removes the incentive to press Easy and shortens the weeks of 0% progress.
   - The 10-a-day cap already protects against overload. The gate does not need to do that job twice.

### P1 — beginner-blocking flow

5. **Make the primary button teach.** Route "Continue letters" and "Study letters" to a Review session, not the Progress list. Before the gate, relabel Start Review as "Learn letters (7 new)". A first-run heading such as "Start here: the alphabet" should replace "Welcome back!".

6. **Unlock chapters automatically.** When a chapter's requirement is met, treat it as unlocked in `build_review_session` and `get_chapters`, and persist the row. If the tap is kept, Android needs the desktop treatment: lock icon, visible hint, "Unlock" button, refresh and confirmation.

7. **Present new cards as lessons.** For new cards show both sides at once, labelled "New", with two buttons: "Show again soon (1m)" and "Got it (10m)". Hide Easy and Hard on first exposure. For letters, count a card as familiar after one rep (or drop intra-letters gating) so an Easy cannot stall the track.

8. **Make completion honest.** When the queue ends with cards due within about 15 minutes, say "7 letters come back in about 8 minutes — return then to unlock the next stage". When more cards are due, offer "N more due — continue".

9. **Surface reader lookup cards.** Include never-reviewed `lookup_cards` rows as new cards in `build_review_session`, ahead of deck cards. Store the lemma and gloss on the back and the sentence as context. Until then, change the snackbar so it does not promise a review.

10. **Turn the failed check into a lesson.**
    - List the missed letters with names on the verdict screen and add "Review these now".
    - Put the taught label on the diphthong card backs (for example "ει — ei: a long close e…"), or cut the diphthong share of the check.
    - Show the organic lane as a checklist on the gate card: "43/43 introduced · 14/20 practice answers · 1/2 days · 82% of 85%".
    - Do not count check answers on unseen cards against the daily allowance, or say that they do.

11. **Let the learner pick a track, and stop paying twice for a word.** Add a "My textbook" setting (Athenaze, JACT, frequency list) that sets the order for new cards. When a new card's headword already has a memory state under another ID, skip it. An alias table keyed on normalized headword is enough.

12. **Fix beginner-facing numbers.** "Words learned" should count cards in review, not reader marks. Chapter progress should show two figures ("18/23 learning · 4 mastered") in place of "0% complete".

13. **Decide the desktop role.** Either describe desktop as a review and dictionary companion in the UI and the site, or add the "already know the alphabet" action and a requeue for missed cards. Move Sync out of the top-level tab row.

### P2 — quality upgrades

14. **Add a relearning step.** After any Again, bring the card back within 1–10 minutes until it is passed once, while keeping its FSRS state. The current rule graduates on a single Good.

15. **Map practice and drill hits to Good.** This is what the design specifies. Reserve Easy for explicit self-rating.

16. **Add measured Greek → English questions.** Recognition multiple choice (Greek prompt, English choices) matches the reading goal. It also suits the multi-part fronts that cannot be typed.

17. **Separate principal parts.** Keep them out of each chapter's 80% count, introduce them no earlier than the chapter that teaches the aorist, and make them one consistent card type (verb → list). Exclude a principal-parts card's plain-verb twin from its distractors, and the reverse.

18. **Add graded reading.** Ship a few very easy texts per chapter band. Public-domain beginner readers are the obvious source, or short sentences built only from words in the learner's review set. Highlight words that are in the learner's deck. This is the biggest gap between "flashcard app" and "zero to Greek", but it is content work, not a defect.

19. **Fill dictionary gaps for course words.** Add Attic `-ττ-` aliases, pronoun headwords and deponents, and override the blank glosses of deck headwords. `data/overrides/lemma_glosses.tsv` already exists for this.

20. **Letters polish.**
    - Exempt letters from the 10-a-day cap, or raise it for them.
    - Author check questions for breathings.
    - Give the two `ᾳ` cards distinct fronts.
    - Use bare glyph and short-name prompts for letters practice, so prompts stop containing the answer.

21. **Use the device's local day** for the daily cap and the "2 days" rule.

22. **Keep Practice inside unlocked chapters** when filling from unseen cards, and correct the empty-state copy.

23. **Update `docs/guide.html`** for same-session misses on each platform, the unlock tap or its removal, and the limits of typed grading.

24. **Say that there is no audio.** The track is titled "The Alphabet & Pronunciation", but it is text-only. Either say so on the track or add recordings later.

---

## INTERVIEW-FLOW RESULT

Could a motivated total novice realistically hit the three milestones? **(a) Day 1 to the first 10 letters: partly.** The default path gives exactly 7 letters, the vowels. Reaching 10 on day one needs a return after 10 minutes and the invisible unlock tap in Progress, or the check or Practice, which expose 24 letters at once as a test the learner will fail. Without those discoveries the learner stays at 7. **(b) Week 1 to 50 words read and answered: no.** The gate opens around day 5–7 for a diligent learner who finds the unlock rhythm and gets past the diphthong-label questions. At 10 new cards a day that leaves 10–20 words by day 7, and 50 by about day 10–11. Declaring the alphabet known gives about 70 by day 7. The "read" half is not met in any case, because the reader has no beginner-level text and misidentifies words like τὴν, ἐν and λέγει. **(c) Month 1 to Athenaze chapter 3–4 with retention: no, by construction.** The 30-day rule means chapter 2 opens no sooner than about day 23 of vocabulary and typically around week 7. Chapter 3 is at least day 46 and chapter 4 at least day 69. At day 30 the learner would have Athenaze chapter 1 (23 words), JACT chapter 1 (13) and about 200 cards from the synthesized "DCC" list, duplicates and a few wrong entries included. FSRS itself would retain those cards well. The limit is the progression rule and the content, not the memory model. Unlocking on "learned" (item 4), fixing the two hidden steps (items 5–7) and the data pass (items 2–3) would make (a) easy, (b) reachable by about day 9–10, and (c) plausible for the vocabulary of chapters 3–4. Reading at that level would still need graded texts (item 18).
