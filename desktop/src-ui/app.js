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
      case "sync":
        return loadSyncStatus();
      default:
        return Promise.resolve(); // dictionary is input-driven
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

  function loadCurriculum() {
    return call("curriculum").then(function (data) {
      renderCurriculum(data);
    });
  }

  function renderCurriculum(data) {
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
            var pct =
              ch.totalCards > 0
                ? Math.round((ch.masteredCount * 100) / ch.totalCards)
                : 0;
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
            return (
              '<div class="chapter-row' +
              (ch.unlocked ? "" : " locked") +
              '">' +
              lock +
              '<div class="chapter-main">' +
              '<div class="chapter-title">' +
              ch.index +
              ". " +
              esc(ch.title) +
              "</div>" +
              hint +
              "</div>" +
              progress +
              "</div>"
            );
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
    list.innerHTML = html;
  }

  /* ================= DICTIONARY ================= */

  var dictTimer = null;
  var dictSeq = 0; // drop stale responses when the user types quickly

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
      status.textContent = 'No matches for "' + q + '".';
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
