# Progress sharing over nostr — design & scope (v0 for review)

Status: **owner decisions received (2026-10-19), ready to build as Wave 9**.
Default posture: **opt-in only, off by default, never retroactive**.
Default posture: **opt-in only, off by default, never retroactive**.

## 1. Goal

Publish coarse, self-flattering learning signals ("7-day streak", "alphabet
mastered") to nostr relays the *learner* trusts — accountability, family study
groups, communities of practice — without exposing what was studied, when
(fine-grained), or anything about specific cards.

## 2. Non-goals (v1)

- No zaps/replies/DMs/repost feeds, no images, no badge issuance (NIP-58)
- No fine-grained telemetry: never per-review, per-card, per-time-of-day data
- No cross-posting to non-nostr networks
- No public-website integration, no profile management (NIP-01 kind 0 stays
  unset: bare keypair, no name/avatar unless the user sets it elsewhere)
- No kind-5 deletion tooling (v1.1 candidate), no chronological history pages

## 3. Privacy model

| Data class | Shared? |
|---|---|
| Milestone kind + coarse number (streak day count, "24/24 check") | yes — this is the feature |
| Totals (lifetime review count bucketed: ≥100, ≥500, ≥1000) | yes, thresholded |
| What's being studied beyond "Greek" | no |
| Review history, due-queue, memory states, HLCs, device id | no |
| The sync spool's ciphertext kind-1078/30078 events | untouched, unlinked |

The single biggest footgun we will NOT build: publishing under kinds that the
sync transport fetches. Sync pulls `{authors:[self], kinds:[1078, 30078]}`
(SYNC-WIRE.md §2); progress events must therefore **never** use kinds 1078 or
30078, or they'd be re-ingested as spool candidates.

## 4. Event taxonomy

Two primitives only:

1. **Milestone notes** — NIP-01 `kind 1` text notes, fired once per milestone
   crossing, recorded so they never repost (incl. across reinstalls and
   devices — see §6).
2. **Milestone-gated personal note (OQ-5, owner decision)**: when a milestone
   fires, the user MAY attach their own sentence before posting ("template,
   then allow users to add text"). The feature stays gated to app-content
   sharing — there is deliberately NO free-standing compose box; every post
   is anchored to a real learning event. Custom suffix is plain text,
   length-capped (420 chars), appended after the template line.

Milestone set for v1 (exactly these, nothing else):

| slug | Fires when | Example content |
|---|---|---|
| `gate.passed` | alphabet gate latches (any lane) | "Can read the Greek alphabet — 24/24 check passed 🦉" |
| `chapter.mastered` | a curriculum chapter reaches masteredCount == total | "Mastered a chapter of Athenaze II" |
| `streak.7` `streak.30` `streak.100` | streak_days threshold crossing | "7-day Greek streak" |
| `reviews.100` `reviews.500` `reviews.1000` | lifetime review count crossing | "100 reviews into learning Greek" |

("mastered" reuses the exact SQL definition that drives the curriculum view's
masteredCount — no new pedagogy logic.)

## 5. Identity & relay choices — OWNER DECISIONS (2026-10-19)

- **Identity (OQ-1): SEPARATE sharing keypair**, stored under
  `nostr_nsec_sharing` (distinct from the sync key at `nostr_nsec`) so the
  two flows can never cross-contaminate: sync pulls filter its own author,
  sharing never touches the sync identity in either direction. The sharing
  key is generated lazily — only when the user first enables the feature.
- **Relays (OQ-2): default to well-known public relays** (publish-friendly
  starter set: relay.damus.io, nos.lol, relay.nostr.band) with a
  user-editable relay list in the Share section. The user's Pyramid relay can
  be added by anyone who wants the members-only semipublic audience; nothing
  is hardcoded to it.
- **OQ-4: no NIP-38 live status** — dropped from v1 entirely (not "later";
  revisit only if someone asks).

## 6. Dedup, rate limit, multi-device

- Outbox table `share_outbox` (slug, dedupe_key, payload_json, state,
  created_at): one row per (slug, subject); flush on app foreground,
  single-flight, failures stay queued with backoff — publishing failures are
  **never** user-visible errors and never block learning flows.
- Posted markers sync across devices as ordinary LWW settings
  (`share.posted.<dedupe_key>`), so two devices rarely double-post; the
  residual race (both devices cross a milestone inside one sync window) is
  documented as acceptable (worst case: one duplicate note).
- Hard rate caps regardless of triggers: max 2 posts/day, min 6h between
  posts; NIP-38 status excluded from caps (it's ephemeral).

## 7. Architecture

- New crate `crates/share-nostr`: milestone detection (pure functions over
  existing stats/curriculum queries), template rendering, outbox, publisher
  over `nostr-sdk` (same stack as sync-nostr).
- Detection is **pull-based on state transitions**: after review submit,
  after alphabet-check verdict, on app foreground for streak/count checks.
  No always-on observers, no background service in v1.
- UniFFI exports: `share_config_get/set`, `share_enable(bool)`,
  `share_pending_count()`, `share_flush_now() -> {posted, deferred}`.
- Android: Settings → "Share progress (nostr)" section with the full
  transparency list of exactly what posts look like (rendered examples);
  OFF until explicitly enabled. Desktop inherits the same section on the
  Sync page later.
- The feature composes with sync but does not require it; if nostr identity
  is missing, enabling sharing offers `sync_generate_nostr_identity()`.

## 8. Test plan (extends wave-8 harness)

- Unit (share-nostr): template rendering, threshold crossing edge cases
  (exactly-7 vs 6→8 jump posts once), outbox dedup, rate-cap math.
- Host integration vs `local_relay`: enable → latch gate → assert exactly one
  kind-1 note with expected template, no repeats on second latch, disabled
  state publishes nothing.
- androidTest: toggle persists; disabling clears outbox; settings surface
  renders examples correctly.
- Reuses scripts/e2e nostr flow; the relay quota (`--once-events`) gains a
  progress-event case.

## 9. Rollout gates

- G-share-1: feature off by default on fresh installs AND upgrades (explicit
  config key absent ⇒ off).
- G-share-2: e2e green: selfcheck + bench device (`all --device`).
- G-share-3: owner field test on the Pyramid relay with their member key.

## 10. Owner decisions — resolved (2026-10-19)

- **OQ-1** → separate sharing keypair (§5).
- **OQ-2** → public relays by default, fully user-configurable list (§5).
- **OQ-3** → ALL of the options: the app ships bilingual (EN + EL) template
  presets AND every template string is user-editable per milestone (edited
  text persists in `user_settings` under `share.template.<slug>`; a
  "reset to default" chip restores the shipped string).
- **OQ-4** → no live status. Feature set is milestones + milestone-anchored
  personal notes only.
- **OQ-5** → anchored custom text allowed (§4). Milestone editor = the only
  place users write free text.

### Resolved v1 milestone editor flow
Settings → Share progress → tap a milestone row → editor sheet:
rendered preview (template live-updates), text field for custom suffix
(cap 420), EN/EL preset toggle, post history for that slug (timestamps +
relays accepted), "test post" button disabled unless sharing is ON
(no shadow-posting from the editor).
