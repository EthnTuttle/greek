# PAIRING-REHEARSAL — Tier-3 manual two-real-app rehearsal

Real **release** phone app (Pixel, sideloaded release APK) + real **Tauri
desktop** binary, real pairing over the local network. No orchestrator, no
run-as (release builds deny it). One page; ~15–20 min including screenshots.

Bench device = Pixel 7a (`35291JEHN11734`). Never rehearse on the owner's
daily-driver Pixel 6 (`1B151FDF600826`).

## 0. Setup (5 min)

1. Phone: install the release APK under test; make sure it has ≥ 1 review
   history (review ~5 cards so there is state worth syncing).
2. Desktop: scratch profile, no real data:
   ```
   GREEK_DATA_DIR=$(mktemp -d)/greek-rehearsal \
     desktop/src-tauri/target/release/greek-desktop &
   ```
   Add a few cards or import a backup so desktop ≠ empty.
3. Both devices on the same LAN. Phone battery > 30%, screen set to stay awake.

## 1. Rehearsal A — desktop is the invite side (5 min)

1. Desktop → **Sync** tab → *Pair new device* → shows the `sync_ticket`
   (QR + copyable text).
2. Phone → Sync screen → paste ticket (QR scan if implemented, else type via
   "Join with ticket") → **Join** → then **Sync now**.
3. Wait for both UIs to report success. Note wall-clock.

## 2. Rehearsal B — phone is the invite side (5 min)

1. Phone → Sync → *Pair* → shows its QR/ticket.
2. Desktop → Sync tab → paste the phone's ticket → join → sync.
3. Wait for both UIs to report success.

(B-direction matters: relay/discovery reachability is asymmetric on real NATs.)

## 3. Evidence checklist (5 min)

- [ ] Screenshot: desktop Sync tab showing success + its card/due counts.
- [ ] Screenshot: phone Sync screen showing success.
- [ ] Phone: Settings → **Export backup** → `backup-phone-after.zip`
      (timestamped; file lands in app storage/Downloads — pull via
      `adb pull /sdcard/Download/...` or share-sheet to the host).
- [ ] Desktop: **Export backup** from its Sync/Settings UI → `backup-desktop-after.zip`.
      (Both exports *timed after* the syncs; if either side had no before-export,
      note what the pre-sync counts were from the UI.)
- [ ] Compare due-card counts **in the UIs** (phone Review header vs desktop
      Review header): counts must match exactly after both syncs.
- [ ] Optional strong check: unzip both backups on the host, `python3` — compare
      the set of review-event card IDs (same shape as `scripts/e2e` exports).

## 4. If it goes wrong — rollback / known-good state

- Desktop: `kill` the process; the scratch profile is disposable —
  `rm -rf $GREEK_DATA_DIR` and relaunch. No user data can exist there.
- Phone: review history is append-only CRDT state; a bad rehearsal cannot
  corrupt cards, but to reset: Settings → Import backup (your pre-rehearsal
  backup — take one in step 0 if the bench phone's state matters), else
  clear app data + reinstall and reseed from `raw` content.
- Relay trouble mid-rehearsal: stop both apps, re-export sanity backup counts,
  restart from the previous successful step. Do **not** re-pair over a half-synced
  state — wipe the desktop scratch profile and rehearse clean.
- Record outcomes (PASS/FAIL + screenshots + durations) in the release tracking
  issue; a failed Tier-3 blocks tagging.
