# Greek desktop webview UI (`desktop/src-ui/`)

Zero-build single-page app: plain `index.html` + `style.css` + `app.js`.
No npm, no bundler, no frameworks, no external fonts/CDNs.

## Serving

Point the Tauri webview at this directory (devUrl / frontendDist in
`tauri.conf.json`, owned by the Rust-side agent). `withGlobalTauri: true`
must be enabled so `window.__TAURI__.core.invoke` exists.

For a quick static preview:

    python3 -m http.server 8000   # from this directory → http://localhost:8000

Outside Tauri, invokes fail and the page shows a "backend unavailable" note.

## Backend contract

All commands go through `window.__TAURI__.core.invoke(name, args)`.
Object-returning commands resolve with a JSON **string** that the UI
`JSON.parse`s (`parseMaybe` tolerates raw scalars too); rejections carry a
string error message. Commands used (frozen set):

`home_stats`, `review_current`, `review_rate` (args `{rating: 1..4}`),
`curriculum`, `dict_search` (`{q}`), `sync_status`, `sync_ticket`,
`sync_join` (`{ticket}`), `sync_now`, `nostr_identity_ensure`,
`nostr_relays_set` (`{csv}`), `backup_export`, `backup_import` (`{json}`).

Review flow: `review_current` on entering the Review tab; clicking a rating
calls `review_rate` immediately (answer+rate in one step) and renders the
returned next-card payload. `card: null` → "All caught up" empty state.

### Element expectations (ids the UI drives)

- Tabs: `#tab-nav .tab-btn[data-tab]` ∈ home/review/curriculum/dictionary/sync,
  sections `#screen-<tab>`.
- Home: `#stat-due-num`, `#stat-new`, `#stat-streak`, `#stat-accuracy(-num)`,
  `#gate-banner` + `#gate-progress`.
- Review: `#review-front`, `#review-back`, `#review-remaining`,
  `#review-new-badge`, `#btn-show-answer`, `#rating-row .rate-btn[data-rating=1..4]`
  with `.rate-preview` children, `#review-empty`.
  Keyboard `Space` = show answer, `1..4` = rate.
- Curriculum: `#curr-gate`, `#curr-list` (accordion rendered into innerHTML).
- Dictionary: `#dict-input` (200 ms debounce), `#dict-results`, `#dict-status`.
- Sync: `#sync-device-id`, `#sync-dot-iroh`, `#sync-dot-nostr`,
  `#sync-relays-view`, `#btn-ticket`/`#ticket-wrap`/`#ticket-text`/
  `#btn-copy-ticket`, `#join-ticket`/`#btn-join`, `#relays-input`/
  `#btn-save-relays`, `#btn-ensure-identity`/`#pubkey-out`, `#btn-sync-now`/
  `#sync-busy`/`#sync-result`, `#paired-list`/`#paired-empty`,
  `#btn-export`, `#btn-import`, `#import-file`, `#toast`.

`window.__APP__` exposes the screen functions for the headless harness.

## QR decision

**No QR code in v1.** A hand-rolled QR encoder was judged too error-prone for
zero-build delivery; the pairing ticket renders as a copyable string
(`#ticket-text`) — desktop→phone pairing happens by pasting (directly or via a
nostr DM to yourself). Revisit in v1.1 by embedding a single-file
public-domain QR encoder.

## Dev harness

    node dev/test-ui.mjs

Stubs `window.__TAURI__.core.invoke` and a minimal DOM, drives all five
screens (incl. error paths), asserts contract adherence, and exits non-zero
on any failure.
