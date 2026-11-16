# Pedagogy close-out — verification of the remediation wave

- **Reviewer:** Fable · **Date:** 2026-11-14 · **Follows:** `plans/pedagogy-review.md`
- **Scope:** triage of original P0 items 1–4 and P1 items 5–13 against the current working tree. No edits, no builds.
- **Method:** code reading, plus read-only Python replays over the shipped `data/cards/*.json` and `data/morphology.sqlite`. The replays re-implement the dedupe key, the unlock rule and the lookup ranking as written in the Rust source. Nothing was run in the real binary or on a device.

## Verdict

Eleven of the thirteen items are fixed as designed. Items 4 and 11 each work alone but combine into a new deadlock, and the dedupe key also breaks the letters track. Both come from one function and are the only release blockers. Part of this is my own doing: recommendation 11 proposed the headword dedupe and I did not flag how it interacts with the per-card unlock count.

## Release blockers (one root cause)

The dedupe key is `normalized_headword` (`uniffi-bindings/src/lib.rs:389-401`). It strips all diacritics and a leading `ο`/`η`/`το`, and `build_review_session` applies it to every deck, letters included (`lib.rs:739-776`). A suppressed card never gets a memory state. `get_chapters` still counts learned cards by card id against 80% of the full chapter size (`curriculum/mod.rs:511, 525-527`).

**B1. Vocabulary chapters deadlock once duplicates exceed 20% of a chapter.**

- JACT chapter 2 can never open. `καλός`, `καί` and `οὐ` are always taken by Athenaze chapter 1 first, leaving 10 admissible of 13 where 11 are needed.
- Athenaze stops early. In the static replay, a learner whose cards take 5 days to count as learned stalls permanently before chapter 3. `βαίνω`, `πρός`, `ἐν`, `δέ` and `αὐτός` are already held by JACT and the core deck, leaving 17 of 22 where 18 are needed.
- A learner whose cards count after 1 day reaches chapter 7 and stalls before chapter 8 (17 of 22 again: `ναῦς`, `θάλαττα` and three principal-parts twins).
- Principal-parts cards are always suppressed by their base verb, which uses up most of each chapter's slack before any cross-deck duplicate.
- Total cards ever admitted in the replay: about 490–530 of 1,140.
- The existing test (`tests/pacing_dedupe_integration.rs:29`) covers only chapter 1 → 2, where no duplicates exist yet.

**B2. The same key collapses distinct letters and distinct words.**

- Letters chapter 4: `ἁ ἀ ά ὰ ᾶ` all key to `α`, so only the rough breathing is ever shown. Smooth breathing, acute, grave and circumflex are never taught.
- Chapter 4 then has 3 admissible cards of 7 where 6 are needed, so chapter 5 (diphthongs) never unlocks through Review.
- Letters introduced tops out at 28 of 43, so the organic lane can never complete. The Alphabet Check becomes the only exit, with six questions on diphthongs the learner was never shown.
- `Η η` keys to `η` (the article strip fires) and suppresses `ῃ`. `Ο ο` keys to `ο` and suppresses the core deck's `ὁ, ἡ, τό`.
- Homographs are dropped: whichever is admitted first hides the other. Pairs in the shipped decks: `εἰς`/`εἷς`, `τις`/`τίς`, `εἰμί`/`εἶμι`, `ὄρος`/`ὅρος`, `οὐ`/`οὗ`, `ἄρα`/`ἆρα`, `πῶς`/`πως`, `τότε`/`τοτέ`. In the fixed admission order, "one", "who? what?" and "to go" are never taught.

**Smallest fix (three changes at the same sites):**

1. Exempt the letters deck from dedupe.
2. Make the key diacritic-preserving: lowercase, NFC, grave to acute, σ/ς folded, article removed only when a second Greek word follows.
3. Make the unlock arithmetic dedupe-aware. Either drop suppressed cards from the chapter denominator, or count a suppressed card as learned when the card holding its key is learned.

Add a test that walks the shipped decks to the last chapter of each track.

## Item-by-item triage

### P0

1. **Reader wrong lemma — FIXED.** `grave_to_acute` runs before the exact match, candidates are ranked closed-class first and deduped (`morphology/mod.rs:37, 73, 106`), then deck lemmas are promoted (`lib.rs:259`). The sheet lists every candidate (`ReaderViewModel.kt:67`). Replay: `τὴν`→ὁ, `λέγει`→"say, speak", `ἐν`→ἐν, `ἦν`→εἰμί, `ὃς`→ὅς, `οἰκεῖ`→οἰκέω; `ἄν` and `μᾶλλον` now have glosses. The asset version is 3 and both database copies match `data/MANIFEST`. Residual, polish: `σύ` resolves to ὗς "wild swine", and `ὑμῖν` plus five fable forms are still not found.

2. **Card data and deck naming — FIXED.** Every card I listed is corrected (`ath1-052` "pipe, flute", `dcc-057` `εἶμι`, `dcc-089` `ἄνθρωπος`, `μάλα`, `πρᾶγμα`, `ὅρος` "boundary", `βασίλεια` "queen", principal-parts slots, `τὸ πλοῖον`). The deck is titled "Core Vocabulary (unofficial)" and the frequency-band tags are gone. In-deck duplicates keep their IDs and are handled by dedupe. Residual, polish: Athenaze chapter assignment is still unverified against the book.

3. **Typed practice false wrongs — FIXED.** `is_typeable_front` limits typed questions to single Greek words and letter pairs (`questions/mod.rs:117`). `grade_typed` uses derived answer keys and folds sigma (`:161-262`). Android honours the flag (`PracticeViewModel.kt:22`) and repairs medial sigma (`GreekInputController.kt:213`). Hits rate Good (`lib.rs:1017-1019`). A practice miss on a graduated card keeps its FSRS memory (`srs/mod.rs:690`).

4. **30-day gate — PARTIAL (the rule is fixed, the outcome is blocked by B1).** The unlock rule is now "learned": graduated, reviewed on two UTC days, last rating Hard or better (`curriculum/mod.rs:179-207`). Unlocks are automatic and latched (`:533`), and 30-day stability is display-only. The pacing is right, but the track still cannot be completed.

### P1

5. **Primary button teaches — FIXED.** "Continue letters" and the check's "Review these now" route to Review (`AppNavigation.kt`). Home shows "Start here: the alphabet" and "Learn letters (N new)", with N taken from a real session build (`HomeViewModel.kt:162-171`).

6. **Automatic unlock — FIXED.** The core latches the unlock on read. Android shows a lock icon, the hint, an Unlock button and a confirmation snackbar (`CurriculumScreen.kt:264-306`, `CurriculumViewModel.kt:197-228`).

7. **New cards as lessons — FIXED.** A first exposure shows both sides with a "New" badge and two buttons, "Show again soon (1m)" and "Got it (10m)" (`ReviewScreen.kt:106-206`). Letters count after one rep (`curriculum/mod.rs:39`), so an Easy can no longer stall the track.

8. **Honest completion — FIXED.** `completionStatus` reports "N more due — continue" or "N cards come back in about M minutes" (`ReviewViewModel.kt:485`). The false "all cards for today" line is gone.

9. **Lookup cards resurface — FIXED.** Never-reviewed `lookup_cards` rows are admitted after due cards and before deck cards (`lib.rs:690-737`), and the back now carries lemma plus gloss. Residual, polish: Android still calls the three-argument API, so no sentence context is stored, and a lookup whose lemma already has state is skipped silently after the "Added" snackbar.

10. **Failed check as lesson — FIXED.** Misses are listed, the button starts a review, and the daily-allowance note is shown (`PracticeViewModel.kt:602`, strings). Seven diphthong backs now carry their label (`letters-034` "ει — ei, …"). The gate card shows the organic checklist with explicit counts. B2 undercuts this in practice, because the diphthong chapter never opens.

11. **Track choice and paying twice — PARTIAL.** By owner decision there is no selector. Dedupe works as written, and multiple-choice distractors now exclude headword twins (`questions/mod.rs:333-352`). It is the source of B1 and B2.

12. **Beginner-facing numbers — FIXED.** Home shows "Cards introduced". Chapters show "x/y learning · z mastered" on both platforms.

13. **Desktop role — FIXED.** Desktop is a companion with same-session requeue (`commands.rs:172-188`), a declare-known action (`:325`), Sync off the top tab row, and learning/mastered reporting.

## Semantic changes worth knowing

- **The 10-minute return slows unlocking.** A card reviewed twice on its first day is next due about 4.5 days later, so it counts as learned around day 5. A card reviewed once a day counts on day 2. The app's own "come back in 10 minutes" prompt therefore delays chapter unlocks by about four days per card. This is harmless once B1 is fixed (about a chapter a week against a chapter every 2–3 days), but it is the reverse of what the prompt implies.
- **`learning_count` means two things.** In review stats it is every introduced card (`lib.rs:831-840`). In chapter views it is cards meeting the learned rule. The Home label "Cards introduced" is accurate.
- **A practice miss drops a card from the learned count until it is re-passed** (its last rating is Again). Unlocks are latched, so nothing re-locks.
- **Every Again now returns in one minute**, including lapses on graduated cards (`srs/mod.rs:723`). The card is shown once more, then FSRS resumes.
- **Principal-parts cards are in effect retired from Review**, since the base verb always holds their key. That matches my earlier advice, but they still count in chapter sizes (see B1).

## Interview-flow results, recomputed

No new FSRS modelling; these follow from the rules above and the earlier numbers.

- **(a) Day 1 to 10 letters: yes.** The first lesson is the 7 vowels. One rep each unlocks chapter 2 automatically, and the next "Learn letters" tap gives 3 more under the daily cap.
- **(b) Week 1 to 50 words: no, for two reasons.**
  - As shipped, the letters lesson runs dry at 28 cards on day 3 (B2). A learner who then finds and passes the check reaches roughly 30–40 words by day 7, but only because 15 letters cards were skipped.
  - With B2 fixed, 43 letters take 5 days under the cap, giving about 27 words by day 7 and 50 by day 10. Fifty in week one needs letters exempt from the cap, which is a policy choice, not a defect.
- **(c) Month 1 to Athenaze chapter 3–4: unreliable as shipped; yes once B1 is fixed.**
  - As shipped, a learner who follows the 10-minute prompt never gets chapter 3. A once-a-day learner gets chapters 3–4 within a week of starting vocabulary and stalls before chapter 8.
  - With B1 fixed, the two-sitting learner opens chapter 3 about 13 days into vocabulary and chapter 4 about 20 days in, which is roughly calendar day 25. The once-a-day learner is well past chapter 4.
  - Retention is unchanged: FSRS at 90%, and practice misses no longer damage it.

## Polish (everything else)

- Reader data gaps: `σύ`, `ὑμῖν`, a handful of unresolved forms, and one parse shown per lemma.
- Store the sentence when adding a reader word; tell the user when a lookup is skipped as already known.
- Breathings and accents are still not assessed by the Alphabet Check.
- Letters practice prompts still contain the answer for diphthongs.
- The core deck's progress can never reach 100% because of suppressed twins.
- Verify Athenaze chapter assignment against the book.
- Daily cap and day counts still use UTC.
- `docs/guide.html` needs updating for the new unlock rule and practice behaviour.
- Longer-term content, unchanged from the first review: graded reading texts, Greek → English measured questions, audio.

---

## Resolution addendum (final spot verification)

**Verdict: no release blockers remain.** B1 and B2 are fixed at the root, and the fix holds under a schedule the repo's own test does not cover.

**How this was checked.** I read the implementation and `tests/pacing_dedupe_integration.rs`. I did not run the tests (no builds). I re-implemented the new key, principal-parts rule and admissible-denominator rule in Python and replayed the shipped decks.

- **Totals match.** My replay admits 798 vocabulary cards plus 43 letters, which is the agent's reported 841.
- **Two schedules, no deadlock.** The repo test learns every admitted card instantly, so Athenaze runs ahead of the other decks. I also ran the opposite case, where cards take 5 days to count and the core deck fills in first. Every populated chapter unlocks in both.

### Audit of the five points

- **Admissible denominator (d).** `get_chapters` drops an unreviewed card from the previous chapter's count when its key is already held elsewhere, and always keeps cards with their own state (`curriculum/mod.rs:645-661`). For a fresh student with no duplicates it is the original denominator. The requirement can only shrink over time, an empty admissible set unlocks, and latches never re-lock. Session admission and the denominator share `learning_unit_key`, so they cannot drift.
  - One visible side effect: `card_count` and `learning_count` are still raw. In the slow schedule later Athenaze chapters have as few as 6–8 admissible cards of 21, so a finished chapter shows "6/21 learning" indefinitely. Exposing the admissible count would fix the display.
- **Principal-parts predicate.** `is_principal_parts` (card type or tag) matches all 118 shipped cards, each of which carries both markers, and nothing else. The list is read from the front for Athenaze/JACT and the back for the core deck, as intended. Dedupe is exact-string, so formatting differences (trailing `—, —`, alternate order) let 19 lists repeat across decks.
- **Vocabulary-only cap vs gate metrics.** The organic gate reads only letters rows in `review_logs` and never the daily counter, so its numbers are unchanged. The two-day rule still stops a same-day pass, which matters more now that all 43 letters can be introduced in one day. Two strings are now wrong: `alphabet_check_daily_cap_note` still says letter answers count toward the allowance, and Home shows "New today: 0 / 10" throughout the alphabet phase.
- **Key collisions.** Over 1,022 plain vocabulary cards there are 698 keys.
  - All nine homograph families are distinct (`εἰς`/`εἷς`, `τις`/`τίς`, `εἰμί`/`εἶμι`, `ὄρος`/`ὅρος`, `οὐ`/`οὗ`, `ἄρα`/`ἆρα`, `πῶς`/`πως`, `τότε`/`τοτέ`, `ποτε`/`ποτέ`).
  - No two different words share a key. The only mixed-gloss group is `εἰμί` "I am" / "to be".
  - The article strip fires only on `ὁ`/`ἡ`/`τό`/`τὸ` followed by a Greek word.
  - Identically accented headwords dedupe across decks as intended. Two spelling variants do not: `οὕτω(ς)`/`οὕτως` and `ζάω`/`ζάω/ζῶ`.
- **Docs.** `docs/guide.html` now describes the learned rule, the letters exemption and same-session misses correctly. Two lines are stale: "unlocks vocabulary on day 4–6", and "80% of the current chapter's cards", which should say cards not already known from another deck.

### Interview-flow results, final

- **(a) Day 1 to 10 letters: yes.** The first lesson is 7 vowels and the second reaches 16. Letters are uncapped, so nothing but the learner's judgement now stops all 43 in one day.
- **(b) Week 1 to 50 words answered: yes.** Letters take 1–3 days, the organic gate can latch on day 2 and the check at any time. Ten words a day from day 3 gives 50 by day 7, all distinct. "Read" still means the three authentic reader passages, where tap analysis is now correct for common words.
- **(c) Month 1 to Athenaze chapter 3–4: yes.** Days are counted from the start of vocabulary, around calendar day 3:

  | Study habit | Ch. 3 opens | Ch. 4 opens | Ch. 5 opens |
  |---|---|---|---|
  | Two sittings a day | day 12 | day 18 | day 24 |
  | One sitting a day | day 5 | day 7 | day 9 |

  The daily cap of 10 words is now the real governor, as intended. Retention is FSRS at 90%, and "learned" means passed on a second day, not long-term.

### Remaining polish from this pass

- Show admissible, not raw, chapter totals.
- Correct the two stale strings and two guide lines above.
- Normalise principal-parts lists and the two headword variants so they dedupe.
- Consider a gentle "that's enough new letters for today" line, since the cap no longer paces the alphabet.
