/* Greek desktop UI — zero-build single-page app.
 * Talks to the Tauri 2 backend via window.__TAURI__.core.invoke
 * (tauri.conf.json enables withGlobalTauri).
 *
 * FROZEN CONTRACT: all object-returning commands resolve with a JSON *string*
 * that we JSON.parse. Scalar commands (sync_ticket, sync_join,
 * nostr_identity_ensure, nostr_relays_set, backup_export) resolve with a raw
 * string; parseMaybe() tolerates either encoding. Rejections carry string
 * error messages.
 */
(function () {
  "use strict";

  var TABS = ["home", "review", "curriculum", "dictionary", "sync"];

  /* ---------------- invoke helpers ---------------- */

  function tauri() {
    if (typeof window === "undefined" || !window.__TAURI__ || !window.__TAURI__.core) {
      throw new Error("Tauri backend unavailable");
    }
    return window.__TAURI__.core;
  }

  function parseMaybe(raw) {
    if (typeof raw !== "string") return raw;
    try {
      return JSON.parse(raw);
    } catch (_e) {
      return raw; // scalar command payloads (ticket strings, "ok", …)
    }
  }

  /** Invoke a command; resolve with JSON.parsed or raw result; reject err as string. */
  function call(name, args) {
    return Promise.resolve()
      .then(function () {
        return tauri().invoke(name, args || {});
      })
      .then(parseMaybe);
  }

  /* ---------------- dom helpers ---------------- */

  function $(id) {
    return document.getElementById(id);
  }

  function esc(s) {
    return String(s == null ? "" : s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function show(el, visible) {
    el.hidden = !visible;
  }

  var toastTimer = null;
  function toast(msg) {
    var el = $("toast");
    el.textContent = msg;
    show(el, true);
    if (toastTimer) clearTimeout(toastTimer);
    toastTimer = setTimeout(function () {
      show(el, false);
    }, 3200);
  }

  /* ---------------- tabs ---------------- */

  var activeTab = "home";

  function showTab(name) {
    if (TABS.indexOf(name) < 0) return;
    activeTab = name;
    TABS.forEach(function (t) {
      show($("screen-" + t), t === name);
    });
    Array.prototype.forEach.call(
      document.querySelectorAll("#tab-nav .tab-btn"),
      function (btn) {
        if (btn.getAttribute("data-tab") === name) {
          btn.setAttribute("aria-current", "page");
        } else {
          btn.removeAttribute("aria-current");
        }
      }
    );
    return loadTab(name).catch(function (err) {
      toast(String(err));
    });
  }

  function loadTab(name) {
    switch (name) {
      case "home":
        return loadHome();
      case "review":
        return loadReview();
      case "curriculum":
        return loadCurriculum();
      case "dictionary":
        return loadDictStatus();
      case "sync":
        return loadSyncStatus();
      default:
        return Promise.resolve();
    }
  }

  /* ================= HOME ================= */

  function loadHome() {
    return call("home_stats").then(function (s) {
      renderHome(s);
    });
  }

  function renderHome(s) {
    $("stat-due-num").textContent = String(s.dueToday);
    $("stat-new").textContent = s.newToday + " / " + s.newCap;
    $("stat-streak").textContent =
      s.streakDays === 1 ? "1 day" : s.streakDays + " days";

    var accWrap = $("stat-accuracy");
    if (s.practiceTotal > 0) {
      var pct = Math.round((s.practiceCorrect * 100) / s.practiceTotal);
      $("stat-accuracy-num").textContent = pct + "%";
      show(accWrap, true);
    } else {
      show(accWrap, false);
    }

    var banner = $("gate-banner");
    if (!s.gateSatisfied) {
      $("gate-progress").textContent =
        "(Letters chapters " + s.lettersChaptersMet + "/5.)";
      show(banner, true);
    } else {
      show(banner, false);
    }
  }

  /* ================= REVIEW ================= */

  var review = {
    card: null, // current payload's card (null = queue empty)
    previews: [],
    revealed: false,
    busy: false,
  };

  function loadReview() {
    return call("review_current").then(function (payload) {
      renderReviewState(payload);
      if (payload.card) {
        renderCard();
      } else {
        showReviewEmpty();
      }
    });
  }

  function renderReviewState(payload) {
    review.card = payload.card || null;
    review.previews = payload.previews || [];
    review.revealed = false;

    $("review-remaining").textContent = payload.remaining + " left";
    var newBadge = $("review-new-badge");
    show(newBadge, !!(payload.card && payload.isNew));

    var previews = review.previews;
    Array.prototype.forEach.call(
      document.querySelectorAll("#rating-row .rate-btn"),
      function (btn, i) {
        var pv = btn.querySelector(".rate-preview");
        if (pv) pv.textContent = previews[i] != null ? previews[i] : "…";
        btn.disabled = false;
      }
    );
  }

  function renderCard() {
    var card = review.card;
    $("review-front").innerHTML =
      '<span class="greek">' + esc(card.front) + "</span>";
    $("review-front").className =
      "greek review-front" + (String(card.front).length > 14 ? " long" : "");
    show($("review-answer-wrap"), false);
    show($("btn-show-answer"), true);
    show($("rating-row"), false);
    show($("review-empty"), false);
    show($("review-card"), true);
    show($("review-remaining"), true);
  }

  function revealAnswer() {
    if (!review.card || review.revealed) return;
    review.revealed = true;
    $("review-back").textContent = review.card.back;
    show($("review-answer-wrap"), true);
    show($("btn-show-answer"), false);
    show($("rating-row"), true);
  }

  function rate(rating) {
    if (!review.card || !review.revealed || review.busy) return Promise.resolve();
    review.busy = true;
    setRatingButtonsDisabled(true);
    // review_rate is answer+rate in one step and returns the next card payload.
    return call("review_rate", { rating: rating })
      .then(function (payload) {
        renderReviewState(payload);
        if (payload.card) {
          renderCard();
        } else {
          showReviewEmpty();
        }
      })
      .catch(function (err) {
        toast(String(err));
      })
      .then(function () {
        review.busy = false;
        setRatingButtonsDisabled(false);
      });
  }

  function setRatingButtonsDisabled(disabled) {
    Array.prototype.forEach.call(
      document.querySelectorAll("#rating-row .rate-btn"),
      function (btn) {
        btn.disabled = disabled;
      }
    );
  }

  function showReviewEmpty() {
    review.card = null;
    show($("review-card"), false);
    show($("btn-show-answer"), false);
    show($("rating-row"), false);
    show($("review-new-badge"), false);
    show($("review-empty"), true);
  }

  /* ================= CURRICULUM ================= */

  var currData = null; // last curriculum() payload, for re-renders

  function loadCurriculum() {
    return call("curriculum").then(function (data) {
      renderCurriculum(data);
    });
  }

  function renderCurriculum(data) {
    currData = data;
    show($("curr-gate"), !!data.gateNotMet);
    var list = $("curr-list");
    var books = data.textbooks || [];
    if (books.length === 0) {
      list.innerHTML = '<p class="text-dim">No curriculum loaded.</p>';
      return;
    }
    var html = books
      .map(function (book, bi) {
        var chapters = book.chapters || [];
        var rows = chapters
          .map(function (ch) {
            return chapterRow(book.id, ch);
          })
          .join("");
        return (
          '<details class="textbook"' +
          (bi === 0 ? " open" : "") +
          ">" +
          "<summary>" +
          esc(book.title) +
          '<span class="details-count">' +
          chapters.length +
          " chapter" +
          (chapters.length === 1 ? "" : "s") +
          "</span></summary>" +
          rows +
          "</details>"
        );
      })
      .join("");
    // D1: while the alphabet gate is unmet the check is the on-ramp; the
    // button opens the in-app check panel (mirrors Android's Curriculum
    // gate row → AlphabetCheck screen).
    var gateCta = data.gateNotMet
      ? '<div class="check-cta">' +
        '<button class="btn" type="button" data-action="check-start">' +
        "Take the Alphabet Check</button>" +
        '<span class="text-dim">24 quick questions on letter names &amp; shapes.</span>' +
        "</div>"
      : "";
    list.innerHTML = gateCta + html;
  }

  /** One chapter row. Mirrors Android CurriculumViewModel.onChapterTapped:
   *  unlocked rows are informational; locked rows WITH a core hint are
   *  non-interactive (the hint states the requirement); locked rows with an
   *  EMPTY hint have a met requirement and are tappable to unlock. */
  function chapterRow(textbookId, ch) {
    var pct =
      ch.totalCards > 0 ? Math.round((ch.masteredCount * 100) / ch.totalCards) : 0;
    var lock = ch.unlocked
      ? ""
      : '<span class="chapter-lock" aria-label="Locked">🔒</span>';
    var hint =
      !ch.unlocked && ch.hint
        ? '<div class="chapter-hint">' + esc(ch.hint) + "</div>"
        : "";
    var progress = ch.unlocked
      ? '<div class="chapter-progress">' +
        '<div class="chapter-bar"><span style="width:' +
        pct +
        '%"></span></div>' +
        '<span class="chapter-count">' +
        ch.masteredCount +
        " / " +
        ch.totalCards +
        " mastered</span></div>"
      : '<div class="chapter-progress"><span class="chapter-count">Locked</span></div>';
    var inner =
      lock +
      '<div class="chapter-main">' +
      '<div class="chapter-title">' +
      ch.index +
      ". " +
      esc(ch.title) +
      "</div>" +
      hint +
      "</div>" +
      progress;
    if (!ch.unlocked && !ch.hint) {
      // Requirement met per the core (unlock_hint empty): tappable.
      return (
        '<button type="button" class="chapter-row chapter-row-action"' +
        ' data-action="chapter-unlock" data-textbook="' +
        esc(textbookId) +
        '" data-chapter="' +
        ch.index +
        '" aria-label="Unlock chapter ' +
        ch.index +
        ": " +
        esc(ch.title) +
        '">' +
        inner +
        '<span class="chapter-unlock-cta">Unlock</span>' +
        "</button>"
      );
    }
    return (
      '<div class="chapter-row' + (ch.unlocked ? "" : " locked") + '">' + inner + "</div>"
    );
  }

  /** Tap on an unlockable chapter row (hint-empty locked row). */
  function unlockChapterRow(textbook, chapter) {
    if (unlockBusy) return Promise.resolve();
    unlockBusy = true;
    return call("chapter_unlock", { textbook: textbook, chapter: chapter })
      .then(function (r) {
        if (r && r.unlocked) {
          toast("Chapter unlocked.");
        } else {
          // Refused (e.g. alphabet gate on a vocabulary track): say why.
          toast((r && r.hint) || "Chapter could not be unlocked yet.");
        }
      })
      .catch(function (err) {
        toast(String(err));
      })
      .then(function () {
        unlockBusy = false;
        return loadCurriculum();
      });
  }
  var unlockBusy = false;

  /* ---------------- alphabet check ----------------
   * Mirrors Android PracticeViewModel check mode + AlphabetCheckVerdictScreen:
   * one generated 24-question batch; each pick is scored locally against
   * correctIndex AND reported via alphabet_check_answer; after the last
   * question the verdict (correct, total) is persisted exactly once via
   * alphabet_check_verdict, which latches the alphabet gate on a pass. */

  var check = {
    active: false,
    questions: [],
    index: 0,
    correctCount: 0,
    answered: false, // current question has a pick
    picked: null,
    busy: false,
    saveFailed: false,
    verdict: null, // null | {gateSatisfied:bool}
  };

  function checkReset() {
    check.active = false;
    check.questions = [];
    check.index = 0;
    check.correctCount = 0;
    check.answered = false;
    check.picked = null;
    check.busy = false;
    check.saveFailed = false;
    check.verdict = null;
  }

  function startAlphabetCheck() {
    if (check.busy) return Promise.resolve();
    check.busy = true;
    return call("alphabet_check_generate")
      .then(function (questions) {
        if (!questions || questions.length === 0) {
          toast("The Alphabet Check is unavailable right now.");
          return;
        }
        checkReset();
        check.active = true;
        check.questions = questions;
        renderCheck();
      })
      .catch(function (err) {
        toast(String(err));
      })
      .then(function () {
        check.busy = false;
      });
  }

  /** Local scoring for the current question; reports the answer to the core. */
  function answerCheck(choiceIndex) {
    if (!check.active || check.answered || check.busy) return Promise.resolve();
    var q = check.questions[check.index];
    if (!q || choiceIndex < 0 || choiceIndex >= q.choices.length) {
      return Promise.resolve();
    }
    var correct = choiceIndex === q.correctIndex;
    check.answered = true;
    check.picked = choiceIndex;
    if (correct) check.correctCount += 1;
    renderCheck();
    // Per-question recycling hook (misses become FSRS-Again items). Like
    // Android, a persistence hiccup does not block the attempt itself.
    return call("alphabet_check_answer", { cardId: q.cardId, correct: correct }).catch(
      function () {
        toast("Answer not saved — continuing anyway.");
      }
    );
  }

  /** Advance; after the last question, persist the verdict exactly once. */
  function nextCheck() {
    if (!check.active || !check.answered) return Promise.resolve();
    check.index += 1;
    check.answered = false;
    check.picked = null;
    if (check.index < check.questions.length) {
      renderCheck();
      return Promise.resolve();
    }
    return recordCheckVerdict();
  }

  function recordCheckVerdict() {
    if (check.busy) return Promise.resolve();
    check.busy = true;
    check.saveFailed = false;
    renderCheck();
    return call("alphabet_check_verdict", {
      correct: check.correctCount,
      total: check.questions.length,
    })
      .then(function (r) {
        check.verdict = r;
      })
      .catch(function (err) {
        // Locally computed result still shows (Android behaviour); a retry
        // button re-fires this same call.
        check.saveFailed = true;
        toast("Couldn't save the check result: " + String(err));
      })
      .then(function () {
        check.busy = false;
        renderCheck();
      });
  }

  /** Leave the check (verdict screen "Back to curriculum" or abort). */
  function exitCheck() {
    checkReset();
    return loadCurriculum();
  }

  function renderCheck() {
    var list = $("curr-list");
    var total = check.questions.length;
    var passMark = Math.min(22, total); // ALPHABET_CHECK pass bar (22 of 24)
    if (check.index >= total) {
      // Verdict screen.
      var passed = !!(check.verdict && check.verdict.gateSatisfied);
      var locallyPassed = check.correctCount >= 22 && total >= 24;
      // Android: verdict = locallyPassed || core outcome (an already-latched
      // gate still shows the success state).
      var shown = locallyPassed || passed;
      list.innerHTML =
        '<div class="card check-panel" id="check-verdict-panel">' +
        '<h2 id="check-verdict-title">' +
        (shown ? "Alphabet check passed 🎉" : "Not quite") +
        "</h2>" +
        '<p id="check-verdict-score" class="check-score">' +
        check.correctCount +
        " / " +
        total +
        "</p>" +
        '<p class="text-dim">' +
        (shown
          ? "Vocabulary tracks are now unlocked."
          : "You need " +
            passMark +
            " or more to pass. Review the letters and try again.") +
        "</p>" +
        (check.saveFailed
          ? '<p class="chapter-hint">Result not saved yet — it will count ' +
            "once saved.</p>" +
            '<button class="btn secondary" type="button" data-action="check-retry">' +
            "Save verdict</button> "
          : "") +
        (check.verdict && !passed
          ? '<button class="btn secondary" type="button" data-action="check-retry" data-restart="1">' +
            "Try again</button> "
          : "") +
        '<button class="btn" type="button" data-action="check-exit">' +
        "Back to curriculum</button>" +
        "</div>";
      return;
    }
    var q = check.questions[check.index];
    var choices = q.choices
      .map(function (c, i) {
        var cls = "check-choice";
        if (check.answered) {
          if (i === q.correctIndex) cls += " correct";
          else if (i === check.picked) cls += " incorrect";
        }
        return (
          '<button type="button" class="' +
          cls +
          '" data-action="check-answer" data-choice="' +
          i +
          '"' +
          (check.answered ? " disabled" : "") +
          '><span class="check-choice-label">' +
          esc(c) +
          "</span></button>"
        );
      })
      .join("");
    list.innerHTML =
      '<div class="card check-panel">' +
      '<div class="check-topline">' +
      '<span class="chip" id="check-progress">Question ' +
      (check.index + 1) +
      " of " +
      total +
      "</span>" +
      '<span class="chapter-count" id="check-score">' +
      check.correctCount +
      " correct</span>" +
      "</div>" +
      '<p class="check-prompt' +
      (q.glyphToName ? " greek" : "") +
      '" id="check-prompt">' +
      esc(q.prompt) +
      "</p>" +
      '<p class="text-dim check-direction">' +
      (q.glyphToName ? "Which letter is this?" : "Which glyph matches this name?") +
      "</p>" +
      '<div class="check-choices">' +
      choices +
      "</div>" +
      '<p class="check-feedback" id="check-feedback" aria-live="polite">' +
      (check.answered
        ? check.picked === q.correctIndex
          ? "Correct"
          : "Not quite — the answer is “" + esc(q.choices[q.correctIndex]) + "”"
        : "") +
      "</p>" +
      (check.answered
        ? '<button class="btn btn-block" type="button" data-action="check-next">' +
          (check.index + 1 === total ? "Finish" : "Next") +
          "</button>"
        : '<button class="btn btn-block" type="button" data-action="check-exit">' +
          "Abort check</button>") +
      (check.busy ? '<p class="text-dim">Saving…</p>' : "") +
      "</div>";
  }

  /* ================= DICTIONARY ================= */

  var dictTimer = null;
  var dictSeq = 0; // drop stale responses when the user types quickly
  var dictInstalled = null; // null = unknown; dict_status on tab load sets it

  /* D2: a fresh install with no bundled/pinned morphology.sqlite used to be
   * indistinguishable from a zero-hit query. Ask the backend once per tab
   * visit so the panel can say "Dictionary not installed" instead of
   * "No matches". */
  function loadDictStatus() {
    return call("dict_status").then(function (s) {
      var status = $("dict-status");
      dictInstalled = !!s.installed;
      if (!dictInstalled && !$("dict-input").value.trim()) {
        status.textContent =
          "Dictionary not installed — the morphology lexicon is missing, " +
          "so lookups return nothing. Reinstall the app (or drop " +
          "morphology.sqlite into the app data directory) and restart.";
        show(status, true);
      } else {
        show(status, false);
      }
    });
  }

  function onDictInput() {
    if (dictTimer) clearTimeout(dictTimer);
    dictTimer = setTimeout(runDictSearch, 200);
  }

  function runDictSearch() {
    var q = $("dict-input").value.trim();
    var results = $("dict-results");
    var status = $("dict-status");
    if (!q) {
      results.innerHTML = "";
      show(status, false);
      return;
    }
    var seq = ++dictSeq;
    return call("dict_search", { q: q })
      .then(function (hits) {
        if (seq !== dictSeq) return; // superseded by a newer query
        renderDictResults(hits, q);
      })
      .catch(function (err) {
        if (seq !== dictSeq) return;
        results.innerHTML = "";
        status.textContent = "Search failed: " + String(err);
        show(status, true);
      });
  }

  function renderDictResults(hits, q) {
    var results = $("dict-results");
    var status = $("dict-status");
    if (!hits || hits.length === 0) {
      results.innerHTML = "";
      status.textContent =
        dictInstalled === false
          ? "Dictionary not installed — see the notice on this tab."
          : 'No matches for "' + q + '".';
      show(status, true);
      return;
    }
    show(status, false);
    results.innerHTML = hits
      .map(function (h) {
        return (
          "<li>" +
          '<span class="greek dict-lemma">' +
          esc(h.lemma) +
          "</span>" +
          '<span class="dict-pos">' +
          esc(h.pos) +
          "</span>" +
          '<span class="dict-gloss">' +
          esc(h.gloss) +
          "</span>" +
          "</li>"
        );
      })
      .join("");
  }

  /* ================= SYNC ================= */

  function loadSyncStatus() {
    return call("sync_status").then(function (s) {
      renderSyncStatus(s);
    });
  }

  function renderSyncStatus(s) {
    $("sync-device-id").textContent = String(s.deviceId || "").slice(0, 12);
    setDot($("sync-dot-iroh"), !!s.irohConfigured);
    setDot($("sync-dot-nostr"), !!s.nostrConfigured);
    $("sync-relays-view").textContent = s.relays || "—";
    if (s.relays && !$("relays-input").value) {
      $("relays-input").value = s.relays;
    }
    renderPaired(s.paired || []);
  }

  function setDot(el, on) {
    if (on) {
      el.classList.add("on");
      el.title = "Configured";
    } else {
      el.classList.remove("on");
      el.title = "Not configured";
    }
  }

  function renderPaired(devices) {
    var list = $("paired-list");
    show($("paired-empty"), devices.length === 0);
    list.innerHTML = devices
      .map(function (d) {
        return (
          "<li>" +
          '<code class="mono paired-id">' +
          esc(String(d.deviceId || "").slice(0, 12)) +
          "</code>" +
          '<span class="paired-label">' +
          esc(d.label || "(unlabeled)") +
          "</span>" +
          '<span class="paired-hlc">last sync: ' +
          esc(d.lastSyncHlc || "never") +
          "</span>" +
          "</li>"
        );
      })
      .join("");
  }

  /* iroh pairing — QR intentionally omitted for v1: desktop→phone pairing
   * happens by copying the ticket string (paste into the phone, or send it to
   * yourself via a nostr DM). A hand-rolled QR encoder was judged too
   * error-prone for zero-build delivery. */
  function showTicket() {
    var btn = $("btn-ticket");
    btn.disabled = true;
    btn.textContent = "Generating ticket…";
    // May take seconds: the backend binds the iroh endpoint and discovers relays.
    return call("sync_ticket")
      .then(function (ticket) {
        $("ticket-text").textContent = String(ticket);
        show($("ticket-wrap"), true);
      })
      .catch(function (err) {
        toast("Ticket failed: " + String(err));
      })
      .then(function () {
        btn.disabled = false;
        btn.textContent = "Show pairing ticket";
      });
  }

  function copyTicket() {
    var text = $("ticket-text").textContent;
    if (!text) return Promise.resolve();
    if (navigator.clipboard && navigator.clipboard.writeText) {
      return navigator.clipboard
        .writeText(text)
        .then(function () {
          toast("Ticket copied.");
        })
        .catch(function () {
          toast("Copy failed — select the text manually.");
        });
    }
    toast("Clipboard unavailable — select the text manually.");
    return Promise.resolve();
  }

  function joinTicket() {
    var ticket = $("join-ticket").value.trim();
    if (!ticket) {
      toast("Paste a ticket first.");
      return Promise.resolve();
    }
    var btn = $("btn-join");
    btn.disabled = true;
    return call("sync_join", { ticket: ticket })
      .then(function () {
        toast("Joined sync group.");
        $("join-ticket").value = "";
        return loadSyncStatus();
      })
      .catch(function (err) {
        toast("Join failed: " + String(err));
      })
      .then(function () {
        btn.disabled = false;
      });
  }

  function saveRelays() {
    var csv = $("relays-input").value.trim();
    return call("nostr_relays_set", { csv: csv })
      .then(function () {
        toast("Relays saved.");
        return loadSyncStatus();
      })
      .catch(function (err) {
        toast("Save failed: " + String(err));
      });
  }

  function ensureIdentity() {
    var btn = $("btn-ensure-identity");
    btn.disabled = true;
    return call("nostr_identity_ensure")
      .then(function (pubkey) {
        var out = $("pubkey-out");
        out.textContent = String(pubkey);
        show(out, true);
        toast("Identity ready.");
      })
      .catch(function (err) {
        toast("Identity failed: " + String(err));
      })
      .then(function () {
        btn.disabled = false;
      });
  }

  function syncNow() {
    var btn = $("btn-sync-now");
    btn.disabled = true;
    show($("sync-busy"), true);
    show($("sync-result"), false);
    return call("sync_now")
      .then(function (r) {
        var via = [];
        if (r.viaIroh) via.push("iroh");
        if (r.viaNostr) via.push("nostr");
        var line =
          "Sent " +
          r.reviewsSent +
          " · received " +
          r.reviewsReceived +
          " · settings applied " +
          r.settingsApplied +
          (via.length ? " via " + via.join(" + ") : " (no transport)");
        // Partial failure (e.g. nostr pull failed, push succeeded) is NOT a
        // clean success: surface the per-operation errors sync_now reports.
        if (r.errors && r.errors.length) {
          line += " — partial failure: " + r.errors.join("; ");
        }
        var el = $("sync-result");
        el.textContent = line;
        show(el, true);
        return loadSyncStatus();
      })
      .catch(function (err) {
        toast("Sync failed: " + String(err));
      })
      .then(function () {
        btn.disabled = false;
        show($("sync-busy"), false);
      });
  }

  /* ================= BACKUP ================= */

  function exportBackup() {
    var btn = $("btn-export");
    btn.disabled = true;
    return call("backup_export")
      .then(function (json) {
        var payload = typeof json === "string" ? json : JSON.stringify(json);
        var stamp = new Date().toISOString().slice(0, 10).replace(/-/g, "");
        var blob = new Blob([payload], { type: "application/json" });
        var url = URL.createObjectURL(blob);
        var a = document.createElement("a");
        a.href = url;
        a.download = "greek-backup-" + stamp + ".json";
        document.body.appendChild(a);
        a.click();
        document.body.removeChild(a);
        setTimeout(function () {
          URL.revokeObjectURL(url);
        }, 1000);
        toast("Backup exported.");
      })
      .catch(function (err) {
        toast("Export failed: " + String(err));
      })
      .then(function () {
        btn.disabled = false;
      });
  }

  function importBackupFile(file) {
    if (!file) return Promise.resolve();
    return file
      .text()
      .then(function (text) {
        return call("backup_import", { json: text });
      })
      .then(function (r) {
        toast(
          "Backup imported: " +
            r.reviewsReceived +
            " reviews, " +
            r.settingsApplied +
            " settings applied."
        );
        return loadSyncStatus();
      })
      .catch(function (err) {
        toast("Import failed: " + String(err));
      });
  }

  /* ================= wiring ================= */

  function bind() {
    Array.prototype.forEach.call(
      document.querySelectorAll("#tab-nav .tab-btn"),
      function (btn) {
        btn.addEventListener("click", function () {
          showTab(btn.getAttribute("data-tab"));
        });
      }
    );

    // Home
    $("stat-due").addEventListener("click", function () {
      showTab("review");
    });
    $("btn-start-review").addEventListener("click", function () {
      showTab("review");
    });
    $("btn-open-curriculum").addEventListener("click", function () {
      showTab("curriculum");
    });
    $("btn-open-dictionary").addEventListener("click", function () {
      showTab("dictionary");
    });

    // Review
    $("btn-show-answer").addEventListener("click", revealAnswer);
    Array.prototype.forEach.call(
      document.querySelectorAll("#rating-row .rate-btn"),
      function (btn) {
        btn.addEventListener("click", function () {
          rate(parseInt(btn.getAttribute("data-rating"), 10));
        });
      }
    );
    document.addEventListener("keydown", function (ev) {
      if (activeTab !== "review" || ev.repeat) return;
      var tag = ev.target && ev.target.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;
      if (ev.key === " " || ev.code === "Space") {
        ev.preventDefault();
        revealAnswer();
      } else if (/^[1-4]$/.test(ev.key) && review.revealed) {
        rate(parseInt(ev.key, 10));
      }
    });

    // Curriculum (event delegation over chapter rows + the alphabet check)
    $("curr-list").addEventListener("click", function (ev) {
      var el = ev.target;
      while (el && el !== ev.currentTarget && !(el.getAttribute && el.getAttribute("data-action"))) {
        el = el.parentNode;
      }
      if (!el || !el.getAttribute) return;
      var action = el.getAttribute("data-action");
      if (!action) return;
      if (action === "check-start") {
        startAlphabetCheck();
      } else if (action === "check-answer") {
        answerCheck(parseInt(el.getAttribute("data-choice"), 10));
      } else if (action === "check-next") {
        nextCheck();
      } else if (action === "check-retry") {
        if (el.getAttribute("data-restart")) {
          startAlphabetCheck();
        } else {
          recordCheckVerdict();
        }
      } else if (action === "check-exit") {
        exitCheck();
      } else if (action === "chapter-unlock") {
        unlockChapterRow(
          el.getAttribute("data-textbook"),
          parseInt(el.getAttribute("data-chapter"), 10)
        );
      }
    });

    // Dictionary
    $("dict-input").addEventListener("input", onDictInput);

    // Sync
    $("btn-ticket").addEventListener("click", function () {
      showTicket();
    });
    $("btn-copy-ticket").addEventListener("click", function () {
      copyTicket();
    });
    $("btn-join").addEventListener("click", function () {
      joinTicket();
    });
    $("btn-save-relays").addEventListener("click", function () {
      saveRelays();
    });
    $("btn-ensure-identity").addEventListener("click", function () {
      ensureIdentity();
    });
    $("btn-sync-now").addEventListener("click", function () {
      syncNow();
    });

    // Backup
    $("btn-export").addEventListener("click", function () {
      exportBackup();
    });
    $("btn-import").addEventListener("click", function () {
      $("import-file").click();
    });
    $("import-file").addEventListener("change", function (ev) {
      var file = ev.target.files && ev.target.files[0];
      importBackupFile(file).then(function () {
        ev.target.value = ""; // allow re-importing the same file
      });
    });
  }

  function boot() {
    bind();
    loadTab("home").catch(function (err) {
      show($("backend-error"), true);
      toast(String(err));
    });
  }

  /* Expose an API surface for the headless smoke harness (dev/test-ui.mjs). */
  if (typeof window !== "undefined") {
    window.__APP__ = {
      showTab: showTab,
      renderHome: renderHome,
      loadHome: loadHome,
      loadReview: loadReview,
      renderCard: renderCard,
      revealAnswer: revealAnswer,
      rate: rate,
      showReviewEmpty: showReviewEmpty,
      renderCurriculum: renderCurriculum,
      loadCurriculum: loadCurriculum,
      startAlphabetCheck: startAlphabetCheck,
      answerCheck: answerCheck,
      nextCheck: nextCheck,
      recordCheckVerdict: recordCheckVerdict,
      exitCheck: exitCheck,
      unlockChapterRow: unlockChapterRow,
      renderDictResults: renderDictResults,
      runDictSearch: runDictSearch,
      renderSyncStatus: renderSyncStatus,
      loadSyncStatus: loadSyncStatus,
      showTicket: showTicket,
      copyTicket: copyTicket,
      joinTicket: joinTicket,
      saveRelays: saveRelays,
      ensureIdentity: ensureIdentity,
      syncNow: syncNow,
      exportBackup: exportBackup,
      importBackupFile: importBackupFile,
      _review: review, // test-only introspection
      _check: check, // test-only introspection
      _call: call,
    };
  }

  if (typeof document !== "undefined" && document.readyState !== undefined) {
    if (document.readyState === "loading") {
      document.addEventListener("DOMContentLoaded", boot);
    } else {
      boot();
    }
  }
})();
