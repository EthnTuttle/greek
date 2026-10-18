#!/usr/bin/env node
/**
 * Headless smoke harness for the Greek desktop UI (zero-build, no jsdom).
 *
 * Stubs window.__TAURI__.core.invoke with canned contract responses and a
 * minimal DOM, then drives every screen's load/render path. Exits non-zero
 * on any thrown error or failed assertion.
 *
 * Run from desktop/src-ui:   node dev/test-ui.mjs
 */
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import vm from "node:vm";

const here = dirname(fileURLToPath(import.meta.url));
const src = readFileSync(join(here, "..", "app.js"), "utf8");

/* ---------------- assertions ---------------- */

let failures = 0;
let checks = 0;
function assert(cond, label) {
  checks++;
  if (cond) {
    console.log("  ok  " + label);
  } else {
    failures++;
    console.error("FAIL  " + label);
  }
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/* ---------------- fake DOM ---------------- */

function makeEl(tag) {
  const attrs = {};
  return {
    tagName: String(tag || "div").toUpperCase(),
    hidden: false,
    textContent: "",
    innerHTML: "",
    value: "",
    disabled: false,
    title: "",
    href: "",
    download: "",
    style: {},
    files: [],
    classList: {
      add() {},
      remove() {},
      toggle() {},
      contains: () => false,
    },
    setAttribute(k, v) {
      attrs[k] = String(v);
    },
    getAttribute(k) {
      return k in attrs ? attrs[k] : null;
    },
    removeAttribute(k) {
      delete attrs[k];
    },
    addEventListener() {},
    removeEventListener() {},
    appendChild() {},
    removeChild() {},
    click() {},
    focus() {},
    querySelector: () => makeEl(),
    querySelectorAll: () => [],
  };
}

const registry = Object.create(null);
const el = (id) => registry[id] || (registry[id] = makeEl());

const tabBtns = ["home", "review", "curriculum", "dictionary", "sync"].map(
  (t) => {
    const b = makeEl("button");
    b.setAttribute("data-tab", t);
    return b;
  }
);
const rateBtns = [1, 2, 3, 4].map((r) => {
  const b = makeEl("button");
  b.setAttribute("data-rating", String(r));
  b.querySelector = () => makeEl();
  return b;
});

const documentStub = {
  readyState: "complete",
  getElementById: el,
  querySelectorAll(sel) {
    if (sel === "#tab-nav .tab-btn") return tabBtns;
    if (sel === "#rating-row .rate-btn") return rateBtns;
    return [];
  },
  querySelector: () => makeEl(),
  createElement: (tag) => {
    const e = makeEl(tag);
    if (tag === "a") {
      // record anchor downloads for the export assertion
      e.click = function () {
        exportClick = { href: e.href, download: e.download };
      };
    }
    return e;
  },
  addEventListener() {},
  removeEventListener() {},
  body: makeEl("body"),
};

let exportClick = null;
let clipboardText = null;

/* ---------------- canned backend ---------------- */

const ALLOWED = new Set([
  "home_stats",
  "review_current",
  "review_rate",
  "curriculum",
  "dict_search",
  "sync_status",
  "sync_ticket",
  "sync_join",
  "sync_now",
  "nostr_identity_ensure",
  "nostr_relays_set",
  "backup_export",
  "backup_import",
]);
const calls = [];

const REVIEWS = [
  {
    card: { cardId: "c1", front: "Β β", back: "beta — b as in book" },
    remaining: 1,
    isNew: true,
    previews: ["10m", "1d", "3d", "7d"],
  },
  {
    card: { cardId: "c2", front: "Γ γ", back: "gamma — g as in go" },
    remaining: 0,
    isNew: false,
    previews: ["1d", "3d", "6d", "12d"],
  },
  { card: null, remaining: 0, isNew: false, previews: ["10m", "1d", "3d", "7d"] },
];
let reviewIdx = 0;

const PUBKEY = "a".repeat(64);
let dictFailNext = false;

const CANNED = {
  home_stats: () =>
    JSON.stringify({
      dueToday: 3,
      newToday: 2,
      newCap: 10,
      streakDays: 4,
      practiceCorrect: 8,
      practiceTotal: 10,
      gateSatisfied: false,
      lettersChaptersMet: 2,
    }),
  review_current: () => JSON.stringify(REVIEWS[reviewIdx]),
  review_rate: (args) => {
    if (!Number.isInteger(args.rating) || args.rating < 1 || args.rating > 4) {
      return Promise.reject("rating must be 1..4");
    }
    reviewIdx = Math.min(reviewIdx + 1, REVIEWS.length - 1);
    return JSON.stringify(REVIEWS[reviewIdx]);
  },
  curriculum: () =>
    JSON.stringify({
      gateNotMet: true,
      textbooks: [
        {
          id: "letters",
          title: "The Alphabet & Pronunciation",
          chapters: [
            { index: 1, title: "The Vowels", totalCards: 7, masteredCount: 4, unlocked: true, hint: "" },
            { index: 2, title: "Stop Consonants", totalCards: 9, masteredCount: 0, unlocked: false, hint: "Master chapter 1 to unlock" },
          ],
        },
        {
          id: "core",
          title: "Core Vocabulary",
          chapters: [
            { index: 1, title: "Everyday Words", totalCards: 20, masteredCount: 0, unlocked: false, hint: "" },
          ],
        },
      ],
    }),
  dict_search: (args) => {
    if (dictFailNext) {
      dictFailNext = false;
      return Promise.reject("index not loaded");
    }
    if (typeof args.q !== "string") return Promise.reject("q must be a string");
    return JSON.stringify([
      { lemma: "ἄνθρωπος", pos: "noun", gloss: "man, human being" },
      { lemma: "λόγος", pos: "noun", gloss: "word, reason" },
    ]);
  },
  sync_status: () =>
    JSON.stringify({
      deviceId: "abc123def4567890fedcba",
      irohConfigured: true,
      nostrConfigured: false,
      relays: "wss://relay.damus.io",
      paired: [
        { deviceId: "phone01234567890abcdef", label: "Pixel", lastSyncHlc: "1730000000-0" },
        { deviceId: "tablet0123456789abcdef", label: "", lastSyncHlc: "" },
      ],
    }),
  sync_ticket: () =>
    sleep(30).then(() => "greeksync://ticket/ak9f0exampleticketpayload"),
  sync_join: (args) => {
    if (!String(args.ticket || "").startsWith("greeksync://")) {
      return Promise.reject("invalid ticket");
    }
    return "ok";
  },
  sync_now: () =>
    JSON.stringify({
      reviewsSent: 2,
      reviewsReceived: 5,
      settingsApplied: 1,
      viaIroh: true,
      viaNostr: false,
    }),
  nostr_identity_ensure: () => PUBKEY,
  nostr_relays_set: (args) => {
    if (typeof args.csv !== "string") return Promise.reject("csv must be a string");
    return "ok";
  },
  backup_export: () => JSON.stringify({ v: 1, reviews: [], settings: {} }),
  backup_import: (args) => {
    JSON.parse(args.json); // reject early on malformed payloads
    return JSON.stringify({ reviewsReceived: 7, settingsApplied: 2 });
  },
};

async function invokeStub(name, args) {
  if (!ALLOWED.has(name)) {
    throw new Error("UI invoked a command outside the frozen contract: " + name);
  }
  calls.push(name);
  return CANNED[name](args || {});
}

/* ---------------- sandbox + load app.js ---------------- */

const sandbox = {
  window: { __TAURI__: { core: { invoke: invokeStub } } },
  document: documentStub,
  navigator: {
    clipboard: {
      writeText: (t) => {
        clipboardText = t;
        return Promise.resolve();
      },
    },
  },
  setTimeout,
  clearTimeout,
  Blob: class Blob {
    constructor(parts, opts) {
      this.parts = parts;
      this.type = opts && opts.type;
    }
  },
  URL: { createObjectURL: () => "blob:fake", revokeObjectURL: () => {} },
  console,
};
vm.createContext(sandbox);
vm.runInContext(src, sandbox, { filename: "app.js" });

const App = sandbox.window.__APP__;
if (!App) {
  console.error("FAIL  window.__APP__ was not exposed by app.js");
  process.exit(1);
}

/* ---------------- scenarios ---------------- */

async function main() {
  console.log("== Home ==");
  await App.showTab("home");
  assert(el("stat-due-num").textContent === "3", "home: due count rendered");
  assert(el("stat-new").textContent === "2 / 10", "home: new X/cap rendered");
  assert(el("stat-streak").textContent === "4 days", "home: streak rendered");
  assert(el("stat-accuracy").hidden === false, "home: accuracy visible when total>0");
  assert(el("stat-accuracy-num").textContent === "80%", "home: accuracy pct");
  assert(el("gate-banner").hidden === false, "home: gate banner shown when !gateSatisfied");
  assert(
    el("gate-progress").textContent.includes("2/5"),
    "home: gate progress X/5 rendered"
  );

  console.log("== Review ==");
  await App.showTab("review");
  assert(App._review.card && App._review.card.front === "Β β", "review: first card loaded");
  assert(el("review-front").innerHTML.includes("Β β"), "review: front rendered to DOM");
  assert(el("review-new-badge").hidden === false, "review: new badge for isNew card");
  assert(el("review-remaining").textContent === "1 left", "review: remaining chip");
  App.revealAnswer();
  assert(App._review.revealed === true, "review: reveal sets state");
  assert(el("review-back").textContent === "beta — b as in book", "review: back revealed");
  assert(el("rating-row").hidden === false, "review: rating row visible after reveal");
  await App.rate(1);
  App.revealAnswer(); // next card needs its answer revealed before rating
  await App.rate(2);
  assert(App._review.card === null, "review: queue drains to empty");
  assert(el("review-empty").hidden === false, "review: empty state shown");
  // idempotence guards: rating with no card / without reveal must be a no-op
  const before = calls.length;
  await App.rate(3);
  await App.rate(4);
  assert(calls.length === before, "review: rate ignored when queue empty");
  // contract: only ratings actually sent were 1 and 2
  const rateCalls = calls.filter((c) => c === "review_rate").length;
  assert(rateCalls === 2, "review: review_rate invoked exactly for answered cards");

  console.log("== Curriculum ==");
  await App.showTab("curriculum");
  assert(el("curr-gate").hidden === false, "curriculum: gate card when gateNotMet");
  const currHtml = el("curr-list").innerHTML;
  assert(currHtml.includes("The Alphabet &amp; Pronunciation"), "curriculum: textbook title");
  assert(currHtml.includes("🔒"), "curriculum: lock icon on locked chapter");
  assert(currHtml.includes("Master chapter 1 to unlock"), "curriculum: locked hint shown");
  assert(currHtml.includes("4 / 7 mastered"), "curriculum: mastered/total row");

  console.log("== Dictionary ==");
  el("dict-input").value = "man";
  await App.runDictSearch();
  const dictHtml = el("dict-results").innerHTML;
  assert(dictHtml.includes("ἄνθρωπος"), "dictionary: lemma rendered");
  assert(dictHtml.includes("noun"), "dictionary: pos rendered");
  assert(dictHtml.includes("man, human being"), "dictionary: gloss rendered");
  assert(el("dict-status").hidden === true, "dictionary: no status on hits");
  // (debounce timer itself is exercised live via the input listener in the webview;
  //  here we drive runDictSearch directly, including the error path)
  const searchCallsBefore = calls.filter((c) => c === "dict_search").length;
  dictFailNext = true;
  await App.runDictSearch();
  assert(
    el("dict-status").hidden === false &&
      el("dict-status").textContent.includes("Search failed"),
    "dictionary: error state rendered"
  );
  assert(
    calls.filter((c) => c === "dict_search").length >= searchCallsBefore,
    "dictionary: searches recorded"
  );

  console.log("== Sync ==");
  await App.showTab("sync");
  assert(el("sync-device-id").textContent === "abc123def456", "sync: device id 12ch mono");
  assert(el("sync-relays-view").textContent === "wss://relay.damus.io", "sync: relays shown");
  const pairedHtml = el("paired-list").innerHTML;
  assert(pairedHtml.includes("phone0123456"), "sync: paired device listed (12ch)");
  assert(pairedHtml.includes("Pixel"), "sync: paired label");
  assert(pairedHtml.includes("never"), "sync: empty lastSyncHlc → never");
  assert(el("paired-empty").hidden === true, "sync: empty hint hidden with devices");

  await App.showTicket();
  assert(
    el("ticket-text").textContent.startsWith("greeksync://ticket/"),
    "sync: ticket rendered"
  );
  assert(el("ticket-wrap").hidden === false, "sync: ticket block visible");
  await App.copyTicket();
  assert(clipboardText === el("ticket-text").textContent, "sync: ticket copied to clipboard");

  el("join-ticket").value = "greeksync://ticket/intest";
  await App.joinTicket();
  assert(el("join-ticket").value === "", "sync: join clears textarea");
  const joined = calls.filter((c) => c === "sync_join").length === 1;
  assert(joined, "sync: sync_join invoked once");

  el("relays-input").value = "wss://a.example, wss://b.example";
  await App.saveRelays();
  assert(calls.includes("nostr_relays_set"), "sync: relays saved");

  await App.ensureIdentity();
  assert(
    /^[0-9a-f]{64}$/.test(el("pubkey-out").textContent),
    "sync: 64-hex pubkey displayed"
  );

  await App.syncNow();
  const resLine = el("sync-result").textContent;
  assert(resLine.includes("Sent 2"), "sync: result line sent count");
  assert(resLine.includes("received 5"), "sync: result line received count");
  assert(resLine.includes("iroh"), "sync: transport noted");
  assert(el("sync-result").hidden === false, "sync: result visible");

  console.log("== Backup ==");
  await App.exportBackup();
  assert(exportClick && /^greek-backup-\d{8}\.json$/.test(exportClick.download),
    "backup: export downloads greek-backup-<date>.json");
  assert(calls.includes("backup_export"), "backup: backup_export invoked");

  await App.importBackupFile({
    text: () => Promise.resolve('{"v":1,"reviews":[1]}'),
  });
  assert(calls.includes("backup_import"), "backup: backup_import invoked");

  console.log("== Contract ==");
  const outside = calls.filter((c) => !ALLOWED.has(c));
  assert(outside.length === 0, "contract: no invoke outside the frozen list");
  const used = new Set(calls);
  const unused = [...ALLOWED].filter((c) => !used.has(c));
  assert(unused.length === 0, "contract: every command exercised (" + unused.join(",") + ")");
}

main()
  .then(() => {
    console.log(
      `\n${checks - failures}/${checks} checks passed — ${calls.length} invocations`
    );
    if (failures > 0) process.exit(1);
  })
  .catch((err) => {
    console.error("HARNESS ERROR:", err);
    process.exit(1);
  });
