# Workstream plan — Android release blockers (A1–A6 + coupled P2s)

Source revision verified 2026-10-21 against report `production-readiness-2026-10-20.md` (0b766e7).
All findings below were re-verified in current source; none appear stale. Notes per item.

## A1 — P1 — Keyboard class-init StackOverflowError
- Root cause: `ui/components/GreekKeyboard.kt:75-78` — local `fun put(ch, name)` shadows `MutableMap.put` and calls itself at :77 (infinite recursion during `LETTER_NAMES` init). Verified.
- Fix: delete the local helper (or rename, e.g. `putEntry`) and call the map `put` directly; map upper/lowercase explicitly.
- Files: `GreekKeyboard.kt`.
- Test: `GreekKeyboardInitTest` (host JVM or Robolectric) asserting `Class.forName("com.greekapp.ui.components.GreekKeyboardKt")` + a Compose test opening Dictionary/Practice screens.
- Effort: S. Risk: none.
- Deps: none. Blocks meaningful validation of A4/P2-reverse-selection (screens must open first).

## A2 — P1 — Failed review saves acknowledged as success
- Root cause: `viewmodel/ReviewViewModel.kt:222-233` — exception caught, but the `Boolean` from `core.submitReview` (line 224) is discarded; Rust converts storage errors to `false` (`crates/uniffi-bindings/src/lib.rs:439-452`). Verified.
- Fix: assign and check the return; on `false` treat exactly like the catch path (keep card, `isSubmitting=false`, retry error message).
- Coupled: `PracticeViewModel.kt:258` checks the bool but only logs a warning (acceptable per comment); `recordVerdict` at :303-313 degrades to a locally-computed verdict on core failure — surface a "verdict not saved" state and allow retry instead of silent local success.
- Files: `ReviewViewModel.kt`, `PracticeViewModel.kt` (+ strings).
- Test: `ReviewViewModelTest.submitReviewReturnsFalse_keepsCard` with a fake `GreekCoreApi` returning `false`; `PracticeViewModelTest.verdictPersistFailure_retryable`.
- Effort: S–M. Risk: low.
- Deps: compounds C1 (non-atomic review write) — Android retry is only safe once C1 lands; also gate on C9 (panic risk while mutex held).

## A3 — P1 — Chapter taps target the wrong textbook
- Root cause: `ui/screens/CurriculumScreen.kt:102` passes only `chapter.number`; `viewmodel/CurriculumViewModel.kt:135-141` and `:169-176` take `firstOrNull` section containing that number (Letters is first), ignoring the tapped section. Verified.
- Fix: carry the textbook through: `onChapterClick = { ch -> viewModel.onChapterTapped(section.name /* or textbook */, ch.number) }`; look up the section by textbook instead of "first containing number".
- Files: `CurriculumScreen.kt`, `CurriculumViewModel.kt`.
- Test: `CurriculumViewModelTest.chapterTapped_resolvesInOwnTextbook` with overlapping chapter numbers (Letters ch.2 vs Athenaze ch.2).
- Effort: S. Risk: low.
- Deps: none. (C11 separately relaxes unlock enforcement in core.)

## A4 — P1 — Built-in keyboard edits discarded
- Root cause: `ui/components/GreekInputController.kt:264-293` — panel commits mutate only the internal `tfvState`; `onValueChange` fires exclusively from the system-field handler at :338-340; the external-push sync at :329-331 overwrites panel edits on the next recomposition. Verified.
- Fix: make panel commits propagate: pass `onValueChange` into `MutableStateGreekInputController` (or wrap the controller in `GreekTextField`) so `insertText`/`deleteBackward` call `onValueChange(result.text)` after mutating `tfvState`; make the :329-331 external push skip self-originated values (track last-committed text).
- Files: `GreekInputController.kt`, call-site wiring stays unchanged.
- Test: Compose UI test `GreekKeyboardCommitTest` — focus a `GreekTextField`, tap panel keys, assert screen state `String` updated and text survives recomposition.
- Effort: M. Risk: medium (recomposition loop if external-push guard is wrong).
- Deps: A1 (screens must initialize); test under letter-typing + dictionary routes.

## A5 — P1 — APK ABI metadata allows 32-bit installs
- Root cause: `android/app/build.gradle.kts:23-34` has no `ndk.abiFilters`; dependency libs ship armeabi-v7a/x86 but the Rust lib is only arm64-v8a/x86_64 → 32-bit devices install then crash at `System.loadLibrary` (`GreekApplication.kt:171`). Verified.
- Fix: `defaultConfig { ndk { abiFilters += setOf("arm64-v8a", "x86_64") } }` (also strips 32-bit JNA libs, partially addressing the 16 KB-alignment note); optionally wrap `System.loadLibrary` in a try/catch with a hard-error state as defense-in-depth.
- Files: `android/app/build.gradle.kts` (+ optionally `GreekApplication.kt`).
- Test: APK-inspection check (script or Gradle verification task) asserting `lib/` contains only the two ABIs; Play Console install-eligibility sanity check.
- Effort: S. Risk: low.
- Deps: none.

## A6 — P1 — Identity re-generation silently replaces decryption key
- Root cause: `ui/screens/SyncScreen.kt:258-262` Generate button is always enabled (no `hasIdentity` guard); `crates/uniffi-bindings/src/lib.rs:1036-1043` overwrites `secrets.nostr_nsec` unconditionally. Verified (doc comment already warns — UI never enforces it).
- Fix (minimal, no core change): when `state.hasIdentity`, hide Replace behind an explicit "Rotate identity" flow: confirmation dialog warning that old-relay history becomes unreadable, plus a "copy/show current nsec first" step. Rename snackbar string accordingly.
- Files: `SyncScreen.kt`, `SyncViewModel.kt` (`generateIdentity(confirmed: Boolean)`), `res/values/strings.xml`.
- Test: `SyncScreenTest.existingIdentity_requiresRotationConfirm` (Compose); ViewModel test that `generateIdentity` without confirm does not call core.
- Effort: S–M. Risk: low.
- Deps: full recovery needs core work — S1 (backup schema covers identity/history) and the credential-recovery P2 (nsec export path); this item ships the UI guard, core rotation semantics stay a core-workstream follow-up.

## P2 — Completed-session restoration re-offers the last card
- Root cause: `ReviewViewModel.kt:262-280` — at completion the batch stays in `SavedStateHandle` and `KEY_INDEX` is saved as the *unchanged* last index (:268, `cards.size-1`); restore prefers `SavedStateHandle` (:50-56) and `restoreFrom` (:86-95) computes `isComplete=false`. Verified. Also :312-345 persist/clear run as separate unawaited coroutines (race).
- Fix: on `complete`, remove `KEY_CARDS/KEY_INDEX/KEY_REVEALED` from `savedStateHandle` synchronously in the same block as `clearDurableSession()` (or store `index = cards.size`).
- Test: `ReviewViewModelTest.completion_doesNotRestoreFromSavedState` (simulate SavedStateHandle restore).
- Effort: S. Risk: low. Deps: A2 (same function—do together).

## P2 — Reverse selections crash editing
- Root cause: `GreekInputController.kt:270-273,285-288` forward raw `selection.start/end`; helpers use them directly in `replaceRange` (:92) / `removeRange` (:136) — `start > end` throws `IndexOutOfBoundsException`. Verified (helpers at :67-146 confirm direct use).
- Fix: normalize endpoints (`minOf/maxOf`) at controller entry points before calling `GreekTextOps` (fix at the trust boundary, not inside ops).
- Test: `GreekTextOpsControllerTest.reversedSelection_noCrash` (unit, host JVM).
- Effort: S. Risk: none. Deps: none (independent of A4 but same file—batch edits).

## P2 — Backup export lost on rotation
- Root cause: `ui/screens/SyncScreen.kt:60-66` — `pendingExport` held in `remember`; process/config change while the SAF picker is open nulls it; callback at :63-67 then skips the write, leaving an empty document. Verified.
- Fix: hoist `pendingExport` into `SyncViewModel` state (or `rememberSaveable`), so the picker callback always has the payload. Coupled: core returns an `error`-keyed JSON string on export failure (`lib.rs:982-986`) and `SyncViewModel.kt:82,88-96` will write it and announce success — validate the payload (starts with `{` and has a `version` field, not `error`) in `exportBackupJson` before handing to the picker.
- Files: `SyncScreen.kt`, `SyncViewModel.kt`.
- Test: `SyncViewModelTest.exportError_notSavedAsBackup`; rotation regression covered by ViewModel-state test.
- Effort: S. Risk: low. Deps: proper typed error is a core (uniffi) follow-up; Android-side validation ships independently.

## P2 — Manual sharing flush fails reflection
- Root cause: `core/RustCoreAdapter.kt:414-422` reflects zero-arg `shareFlushNow`, but the generated method is `suspend` with a hidden `Continuation` (`uniffi_bindings.kt:2688`) → lookup falls into the "mangled" retry at :322-324, fails, latches sharing unavailable (:448-450). Second, independent mismatch: `shareConfigGet` (:354) reads property `pubkeyHex` but the record exposes `sharePubkeyHex` (`uniffi_bindings.kt:3834`). Both verified.
- Fix: make `GreekCoreApi.shareFlushNow` a `suspend fun` and call the binding natively (`runBlocking`-free), or add the `Continuation` param to the reflection lookup and drive it; fix the property name to `sharePubkeyHex`. Prefer removing reflection for these two paths once the stale-lib story is confirmed obsolete.
- Files: `RustCoreAdapter.kt`, `GreekCoreApi` interface (+stub), callers of `shareFlushNow`.
- Test: `RustCoreAdapterTest.shareConfigMapping_usesSharePubkeyHex` using a fake object mirroring the generated record; flush path tested via a suspend fake.
- Effort: M. Risk: medium (coroutine/JNI interaction; reflection removal changes stale-lib tolerance).
- Deps: correctness of the underlying engine call depends on core P2s — sharing identity location, mutex-across-network (facade.kt:218-219). UI changes independent.

## Recommended fix order
1. **A1** — zero-risk one-liner unblocking every keyboard screen; nothing can be validated without it.
2. **A5** — one-line Gradle change, independent; land and verify APK contents immediately. (parallel with 1)
3. **A3** — isolated ViewModel/Screen signature change; trivial, no overlap with anything. (parallel with 1–2)
4. **A2 + completed-session P2** — same function/file (`ReviewViewModel`); do as one change; retry path assumes C1 (core) for true safety but the Boolean check is correct and shippable alone.
   - **Practice/Alphabet-check persistence paths (A2-coupled)** — same PR or immediately after; independent file.
5. **A4 + reverse-selection P2** — both in `GreekInputController.kt`; one PR; A1 must be merged first for UI tests to run.
6. **Backup-on-rotation P2 (+ export-validation)** — independent `Sync*` files; any time in parallel with 4–5.
7. **A6** — dialog copy should reference the identity-export path; best after or alongside 6 (same screen). Flag as interim until core rotation/recovery (S1-linked) lands.
8. **Sharing-flush P2** — last: riskiest change (suspend/JNI/reflection), benefits from core sharing-fixes settling first.

Parallelizable groups: {A1, A5, A3} · {A2/P2-session, backup-P2} · {A4/P2-selection, backup-P2} · A6 after 6 · sharing P2 last.
Cross-workstream deps: A2↔C1/C9 (core); A6↔S1 + credential-recovery P2 (core); sharing-P2↔sharing identity/mutex P2s (core). None are hard blockers for shipping the Android-side fixes except noting A2's retry safety caveat until C1 lands.
