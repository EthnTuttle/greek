# Release gate — manual/physical drills

Everything here requires devices/emulators and cannot be automated in this
environment. Run AFTER `scripts/ci/run-local.sh` (R4) reports `RESULT: PASS`.
Source: audit §"Recommended release gate" + accumulated manual items (see
`plans/followups.md`).

Setup: build a fresh SIGNED release APK from current source; run
`scripts/ci/run-local.sh --quick --release-check` on that APK to verify `lib/`
contains only arm64-v8a/x86_64 (both present), verify 16KB LOAD alignment, and
confirm the staging script has re-verified the morphology hash.

## 1. Smoke & navigation (was impossible pre-A1)
- [ ] Open Dictionary, Practice/Alphabet Check, Letter Typing — each screen
      keyboard-inits cleanly (A1).
- [ ] In a typing screen: type via system IME, then via in-app polytonic panel
      (long-press for diacritic variants) — committed text survives rotate (A4).
- [ ] Drag-select text right-to-left, delete — no crash (reverse-selection P2).

## 2. Save-failure / retry
- [ ] Fill a review session; with storage faulted (test build w/ failing
      adapter, or follow A2 unit coverage), failed save shows retry; card not
      skipped; retry saves exactly once (A2).
- [ ] Practice verdict persistence failure → "Retry saving result" button works
      (Wave-2 practice UI).
- [ ] Alphabet check under airplane mode → error + retry affordance; after
      restore, verdict saves once (A2-coupled).

## 3. Process death & lifecycle
- [ ] Mid-session: swipe-kill the app → relaunch → in-progress session resumes
      at the right card WITHOUT re-offering completed ones (session P2).
- [ ] On a completed session: kill → relaunch → no ghost card.
- [ ] Export backup, rotate while SAF picker open → full JSON written,
      "Backup written" snackbar (backup-rotation P2).
- [ ] Export-failure injection (stub `{"error"…}` → picker must not open (A6-set).

## 4. Identity & sync
- [ ] With identity configured: 'Rotate identity' shows dialog; Cancel leaves
      identity; copy warns about history loss; Confirm rotates and shows new
      npub (A6).
- [ ] Dialog survives device rotation.
- [ ] Two devices: sync reviews across; then DELAY one device offline for an
      hour, add more reviews on both, reconnect → convergence incl. delayed
      batch recovery (S6/S7).
- [ ] Upgrade-in-place from the last release APK (data kept: reviews, known
      words, unlocks — v5/v6 migrations).
- [ ] Fresh-install restore from a v2 backup taken on device A: Greek lookup
      cards incl. `lookup-λόγος`, known words, chapter unlocks all present (S1).
- [ ] Sync-screen "sync now" on a dead relay: partial-failure surfaces, doesn't
      say clean success (D7 parity on Android side).

## 5. Desktop (if shipping the .deb)
- [ ] Fresh-profile .deb install: dictionary pre-bundled (`dict_status`
      installed=true); morphology queries return results (D2).
- [ ] Full beginner path: vowels ×2 → alphabet check → Letters ch.2 unlock →
      non-vowel cards arrive (D1).
- [ ] Import Android v2 backup → session cache invalidated, no re-offered
      completed cards (D8).
- [ ] Rating buttons hidden pre-reveal; completed cards don't leak through
      `[hidden]` (D4 — check with big fonts / small window too, per the
      accessibility P2).

## 6. iroh pair drill (S8/S9)
- [ ] Creator restart: pairing persists; creator syncs WITHOUT re-sharing the
      ticket (S8).
- [ ] Creator makes notebook A, joins B, restarts → B is active (S9).
- [ ] Failed sync shows failure, not green (S8 event-crediting).

## 7. Reminders & accessibility (finalization wave)
- [ ] API 33+: enable reminders → permission prompt; deny → toggle stays off
      with snackbar; allow → notification actually posts; system-level revoke
      → receiver exits gracefully.
- [ ] Reboot with reminders enabled → alarms re-arm (boot receiver); disabled →
      none scheduled.
- [ ] TalkBack on Reader: per-word accessibility action "Look up …" opens the
      analysis sheet without gestures.
- [ ] Largest font + short screen: Home scrolls to Start Review and stats.

## Sign-off rule
All boxes + `scripts/ci/run-local.sh` reporting `RESULT: PASS` + the R5 OSV
check on the final lockfile → tag release; upload pinned morphology.sqlite +
APK as release assets (fixes the empty GitHub Releases page the audit flagged).
