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
const htmlRaw = readFileSync(join(here, "..", "index.html"), "utf8");

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

/* ---------------- stylesheet model (D4) ----------------
 * The mock DOM has no CSS engine, so boolean `hidden` alone does NOT tell
 * whether an element is rendered: in a real browser any author `display`
 * rule on the element's classes overrides the UA sheet's
 * `[hidden] { display: none }` — this exact bug (D4) made hidden rating
 * buttons, badges and the idle spinner visible. Model "rendered visible"
 * as: not hidden, OR hidden but overridden by a class display rule AND the
 * stylesheet is missing the `[hidden] { display: none !important }` guard.
 */
const cssRaw = readFileSync(join(here, "..", "style.css"), "utf8");
const cssCollapsed = cssRaw
  .replace(/\/\*[\s\S]*?\*\//g, "")
  .replace(/\s+/g, "")
  .replace(/;/g, "");
const HIDDEN_GUARD = "[hidden]{display:none!important}";
const cssHasHiddenGuard = cssCollapsed.includes(HIDDEN_GUARD);

// Ids the app toggles via the `hidden` attribute whose classes carry a
// `display` rule in style.css (the D4 victim set). Self-checked below so
// the map rots loudly instead of silently.
const DISPLAY_RULE_BY_ID = {
  "rating-row": ".rating-row",
  "btn-show-answer": ".btn",
  "review-new-badge": ".chip",
  "review-card": ".review-card",
  "sync-busy": ".spinner",
};

/** Rendered-visibility truth for the mock DOM (see header comment). */
function renderedVisible(id) {
  const e = el(id);
  if (!e.hidden) return true;
  return !cssHasHiddenGuard && Boolean(DISPLAY_RULE_BY_ID[id]);
}

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

const tabBtns = ["home", "review", "curriculum", "dictionary"].map(
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
  "alphabet_check_generate",
  "alphabet_check_answer",
  "alphabet_check_verdict",
  "alphabet_declare_known",
  "chapter_unlock",
  "dict_search",
  "dict_status",
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
  { card: null, remaining: 0, isNew: false, previews: [], returningSoonCount: 1, minutesUntilNext: 1 },
];
let reviewIdx = 0;

const PUBKEY = "a".repeat(64);
let dictFailNext = false;
let dictInstalledState = true;
let ch3Unlocked = false; // curriculum canned state: chapter_unlock flips it
let alphabetKnown = false;

const CHECK_QUESTIONS = [
  { cardId: "letters-002", glyphToName: true, prompt: "Β β", choices: ["beta", "alpha", "gamma", "delta"], correctIndex: 0 },
  { cardId: "letters-001", glyphToName: false, prompt: "alpha", choices: ["Η η", "Α α", "Ω ω", "Ε ε"], correctIndex: 1 },
  { cardId: "letters-010", glyphToName: true, prompt: "Γ γ", choices: ["pi", "rho", "gamma", "tau"], correctIndex: 2 },
];

const CANNED = {
  home_stats: () =>
    JSON.stringify({
      dueToday: 3,
      newToday: 2,
      newCap: 10,
      streakDays: 4,
      practiceCorrect: 8,
      practiceTotal: 10,
      wordsLearned: 17,
      gateSatisfied: alphabetKnown,
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
      gateNotMet: !alphabetKnown,
      textbooks: [
        {
          id: "letters",
          title: "The Alphabet & Pronunciation",
          chapters: [
            { index: 1, title: "The Vowels", totalCards: 7, learningCount: 4, masteredCount: 2, unlocked: true, hint: "" },
            { index: 2, title: "Stop Consonants", totalCards: 9, learningCount: 0, masteredCount: 0, unlocked: false, hint: "Learn chapter 1 to unlock" },
            // hint "" on a locked row = requirement met → tappable (D1)
            { index: 3, title: "Continuants", totalCards: 9, learningCount: 0, masteredCount: 0, unlocked: ch3Unlocked, hint: "" },
          ],
        },
        {
          id: "core",
          title: "Core Vocabulary",
          chapters: [
            { index: 1, title: "Everyday Words", totalCards: 20, learningCount: 0, masteredCount: 0, unlocked: false, hint: "" },
          ],
        },
      ],
    }),
  alphabet_check_generate: () => JSON.stringify(CHECK_QUESTIONS),
  alphabet_check_answer: (args) => {
    if (typeof args.cardId !== "string" || typeof args.correct !== "boolean") {
      return Promise.reject("cardId must be a string, correct a boolean");
    }
    return "ok";
  },
  alphabet_check_verdict: (args) => {
    if (!Number.isInteger(args.correct) || !Number.isInteger(args.total)) {
      return Promise.reject("correct/total must be integers");
    }
    // mirror the core contract loosely: mock passes at ≥90%
    return JSON.stringify({ gateSatisfied: args.total > 0 && args.correct * 10 >= args.total * 9 });
  },
  alphabet_declare_known: () => {
    alphabetKnown = true;
    return JSON.stringify({ gateSatisfied: true });
  },
  chapter_unlock: (args) => {
    if (typeof args.textbook !== "string" || !Number.isInteger(args.chapter)) {
      return Promise.reject("textbook must be a string, chapter an integer");
    }
    if (args.textbook === "letters" && args.chapter === 3) {
      ch3Unlocked = true;
      return JSON.stringify({ unlocked: true, hint: "" });
    }
    return JSON.stringify({ unlocked: false, hint: "Locked by the alphabet gate" });
  },
  dict_search: (args) => {
    if (dictFailNext) {
      dictFailNext = false;
      return Promise.reject("index not loaded");
    }
    if (typeof args.q !== "string") return Promise.reject("q must be a string");
    if (!dictInstalledState) return JSON.stringify([]); // no lexicon → zero rows (D2 precondition)
    return JSON.stringify([
      { lemma: "ἄνθρωπος", pos: "noun", gloss: "man, human being" },
      { lemma: "λόγος", pos: "noun", gloss: "word, reason" },
    ]);
  },
  dict_status: () => JSON.stringify({ installed: dictInstalledState, path: dictInstalledState ? "/fake/resource/morphology.sqlite" : null }),
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
  console.log("== Stylesheet (D4) ==");
  assert(
    cssHasHiddenGuard,
    "css: [hidden] { display: none !important } guard present"
  );
  const btnRuleAt = cssCollapsed.indexOf(".btn{");
  assert(
    cssHasHiddenGuard &&
      btnRuleAt > -1 &&
      cssCollapsed.indexOf(HIDDEN_GUARD) < btnRuleAt,
    "css: [hidden] guard declared earlier than the first author display rule"
  );
  for (const id of Object.keys(DISPLAY_RULE_BY_ID)) {
    const sel = DISPLAY_RULE_BY_ID[id];
    const m = new RegExp(sel.replace(/\./g, "\\.") + "\\{[^}]*display:").exec(
      cssCollapsed
    );
    assert(Boolean(m), "css self-check: " + sel + " really sets display (D4 map not stale)");
  }

  console.log("== Markup / navigation ==");
  const navMarkup = /<nav[^>]*id="tab-nav"[\s\S]*?<\/nav>/.exec(htmlRaw);
  assert(Boolean(navMarkup), "markup: primary nav exists");
  assert(navMarkup && !navMarkup[0].includes('data-tab="sync"'), "markup: sync absent from primary tabs");
  assert(htmlRaw.includes('id="btn-open-sync"'), "markup: secondary sync entry present");
  assert(htmlRaw.includes('href="guide.html"'), "markup: desktop-scoped guide linked");
  assert(htmlRaw.includes("review and dictionary companion to the Android app"), "markup: companion scope stated");

  console.log("== Home ==");
  await App.showTab("home");
  assert(el("stat-due-num").textContent === "3", "home: due count rendered");
  assert(el("stat-new").textContent === "2 / 10", "home: new X/cap rendered");
  assert(el("stat-streak").textContent === "4 days", "home: streak rendered");
  assert(el("stat-learned-num").textContent === "17", "home: cards in review rendered as words learned");
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
  assert(renderedVisible("review-new-badge"), "review: new badge rendered visible (D4)");
  assert(el("review-remaining").textContent === "1 left", "review: remaining chip");
  // D4: pre-answer state — rendered visibility, not just the boolean.
  // rating-row carries `display: grid` and btn-show-answer `display:
  // (inline-)block`, so in a real browser `hidden` only wins via the CSS
  // guard; asserting .hidden here used to pass while both were painted.
  assert(el("rating-row").hidden === true, "review: rating row hidden before reveal");
  assert(
    renderedVisible("rating-row") === false,
    "review: rating buttons NOT rendered before the answer is revealed (D4)"
  );
  assert(
    renderedVisible("btn-show-answer") === true,
    "review: show-answer button rendered before reveal (D4)"
  );
  App.revealAnswer();
  assert(App._review.revealed === true, "review: reveal sets state");
  assert(el("review-back").textContent === "beta — b as in book", "review: back revealed");
  assert(el("rating-row").hidden === false, "review: rating row visible after reveal");
  assert(renderedVisible("rating-row") === true, "review: rating row rendered after reveal (D4)");
  assert(
    renderedVisible("btn-show-answer") === false,
    "review: show-answer button hidden after reveal (D4)"
  );
  await App.rate(1);
  App.revealAnswer(); // next card needs its answer revealed before rating
  await App.rate(2);
  assert(App._review.card === null, "review: queue drains to empty");
  assert(el("review-empty").hidden === false, "review: empty state shown");
  assert(el("review-empty-title").textContent === "Cards come back soon", "review: pending return is not called completed");
  assert(el("review-empty-detail").textContent.includes("about 1 minute"), "review: return timing rendered");
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
  assert(currHtml.includes("Learn chapter 1 to unlock"), "curriculum: locked hint shown");
  assert(currHtml.includes("4 / 7 learning · 2 mastered"), "curriculum: learning and mastery shown separately");
  assert(!currHtml.includes("0% complete"), "curriculum: no misleading percentage-only empty progress");
  // D1: interactivity
  assert(currHtml.includes('data-action="check-start"'), "curriculum: check CTA while gate unmet");
  assert(
    currHtml.includes('data-action="chapter-unlock" data-textbook="letters" data-chapter="3"'),
    "curriculum: hint-empty locked row is a tappable unlock button"
  );
  assert(
    !currHtml.includes('data-chapter="2"'),
    "curriculum: hinted locked row stays non-interactive"
  );

  // Chapter-row tap → chapter_unlock, list re-renders with the row unlocked.
  await App.unlockChapterRow("letters", 3);
  assert(calls.includes("chapter_unlock"), "curriculum: chapter_unlock invoked on tap");
  assert(
    el("curr-list").innerHTML.includes("0 / 9 learning · 0 mastered"),
    "curriculum: unlocked ch.3 shows separate progress figures after unlock"
  );

  // Alphabet check loop: generate → answer each → verdict → back.
  await App.startAlphabetCheck();
  assert(calls.includes("alphabet_check_generate"), "check: generate invoked");
  assert(
    el("curr-list").innerHTML.includes("Question 1 of 3"),
    "check: progress label rendered"
  );
  assert(
    el("curr-list").innerHTML.includes('data-action="check-answer"'),
    "check: choice buttons rendered"
  );
  for (let i = 0; i < CHECK_QUESTIONS.length; i++) {
    assert(App._check.index === i, "check: question index " + i);
    await App.answerCheck(CHECK_QUESTIONS[i].correctIndex);
    assert(App._check.answered === true, "check: answer recorded locally");
    await App.nextCheck();
  }
  assert(
    calls.filter((c) => c === "alphabet_check_answer").length === 3,
    "check: per-question answers reported"
  );
  assert(
    calls.filter((c) => c === "alphabet_check_verdict").length === 1,
    "check: verdict persisted exactly once"
  );
  assert(
    el("curr-list").innerHTML.includes("Alphabet check passed"),
    "check: passing verdict rendered"
  );
  assert(el("curr-list").innerHTML.includes("3 / 3"), "check: score rendered");
  await App.exitCheck();
  assert(el("curr-list").innerHTML.includes("The Alphabet"), "check: exit restores curriculum");

  await App.declareAlphabetKnown();
  assert(calls.includes("alphabet_declare_known"), "gate: declared-known command invoked");
  assert(el("curr-gate").hidden === true, "gate: declaration refreshes curriculum gate");
  assert(!el("curr-list").innerHTML.includes('data-action="alphabet-known"'), "gate: declaration CTA removed once satisfied");

  console.log("== Dictionary ==");
  await App.showTab("dictionary");
  assert(calls.includes("dict_status"), "dictionary: tab load checks installation (D2)");
  assert(el("dict-status").hidden === true, "dictionary: installed → no notice on tab load");
  // Not-installed path: D2 distinguishes it from a plain "No matches".
  dictInstalledState = false;
  await App.showTab("home");
  await App.showTab("dictionary");
  assert(
    el("dict-status").hidden === false &&
      el("dict-status").textContent.includes("Dictionary not installed"),
    "dictionary: not-installed message rendered (D2)"
  );
  el("dict-input").value = "zzzqqq"; // zero-hit query while not installed
  await App.runDictSearch();
  assert(
    el("dict-status").hidden === false &&
      el("dict-status").textContent.includes("Dictionary not installed"),
    "dictionary: zero hits under missing lexicon still says not installed (D2)"
  );
  dictInstalledState = true;
  await App.showTab("home");
  await App.showTab("dictionary");
  assert(el("dict-status").hidden === true, "dictionary: installed again → notice clears");
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
  // Mock fidelity: index.html mounts #sync-busy with the `hidden` attribute
  // (idle spinner); the mock DOM can't parse markup, so seed it here.
  el("sync-busy").hidden = true;
  await App.showTab("sync");
  assert(el("sync-device-id").textContent === "abc123def456", "sync: device id 12ch mono");
  assert(el("sync-relays-view").textContent === "wss://relay.damus.io", "sync: relays shown");
  const pairedHtml = el("paired-list").innerHTML;
  assert(pairedHtml.includes("phone0123456"), "sync: paired device listed (12ch)");
  assert(pairedHtml.includes("Pixel"), "sync: paired label");
  assert(pairedHtml.includes("never"), "sync: empty lastSyncHlc → never");
  assert(el("paired-empty").hidden === true, "sync: empty hint hidden with devices");
  assert(renderedVisible("sync-busy") === false, "sync: idle spinner NOT rendered (D4)");

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
