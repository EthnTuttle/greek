package com.greekapp

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import uniffi.uniffi_bindings.GreekCore
import uniffi.uniffi_bindings.GreekException

/**
 * Wave-8 cross-device sync E2E — device-side harness, protocol
 * "greek-sync-e2e v1". Every test operates on a SCRATCH [GreekCore] under
 * `filesDir/e2e-scratch/` (wiped at test start); the app's real DB is never
 * touched. Evidence is exchanged with the host orchestrator through
 * `filesDir/e2e/` (reachable via `adb shell run-as com.greekapp` because the
 * debug build is debuggable) using plain text/JSON files and flag files;
 * see the project docs for the exact contract.
 *
 * Confirmed binding surface (android/app/src/main/java/uniffi/uniffi_bindings):
 * suspend `syncIrohTicket()/syncIrohJoin(ticket)/syncIrohNow()`,
 * suspend `syncNostrPush()/syncNostrPull()`,
 * sync `syncSetNostrRelays(csv)`, `syncGenerateNostrIdentity()`,
 * `syncSetNostrNsec(nsec)`, `syncNostrConfig()`, `submitReview`,
 * `recordAlphabetCheckResult`, `getAlphabetGateState`,
 * `syncExportBackupJson()`, `syncImportBackupJson(json)`,
 * `syncPairedDevices()`, and `GreekCore.newWithCards(db, morphOrNull, json)`.
 */
@RunWith(AndroidJUnit4::class)
class CrossDeviceSyncE2E {

    private val context
        get() = InstrumentationRegistry.getInstrumentation().targetContext

    /** Honor the orchestrator's `-e GREEK_IROH_NET loopback` instrumentation
     * arg by pushing it into the process env BEFORE any GreekCore opens its
     * iroh transport (std::env::var in Rust reads the libc environment).
     * Default (unset) = N0, the production path. */
    private fun applyNetModeArg() {
        val mode = InstrumentationRegistry.getArguments().getString("GREEK_IROH_NET")
        if (!mode.isNullOrBlank()) {
            android.system.Os.setenv("GREEK_IROH_NET", mode, true)
        }
    }

    /** Evidence directory shared with the orchestrator. */
    private fun e2eDir(): File = File(context.filesDir, "e2e").apply { mkdirs() }

    /** Wipe + recreate a scratch root for one GreekCore instance. */
    private fun scratchDir(tag: String): File =
        File(context.filesDir, "e2e-scratch/$tag").apply {
            deleteRecursively()
            check(mkdirs()) { "cannot create scratch dir $this" }
        }

    /** The real letters track, mirroring the Rust e2e_two_device_sync fixture
     * (the alphabet gate is fail-open satisfied with method "" when no letters
     * deck is loaded, so a latch assertion is meaningless without it). */
    private fun lettersJson(): String =
        context.assets.open("cards/letters.json").bufferedReader().use { it.readText() }

    private fun coreWithLetters(dir: File): GreekCore =
        GreekCore.newWithCards(File(dir, "greek_app.db").absolutePath, null, lettersJson())

    /** Suspend FFI call with a hard timeout so a wedged network call fails
     * the test instead of hanging the instrumentation session. */
    private fun <T> call(timeoutMs: Long, block: suspend () -> T): T =
        runBlocking { withTimeout(timeoutMs) { block() } }

    /** Protocol seeds: cards "d1".."d5" with ratings [3,4,2,3,1], then latch
     * the alphabet gate through the real check-verdict path (23/24). */
    private fun seedDeviceReviewsAndGate(core: GreekCore) {
        val ratings = listOf(3, 4, 2, 3, 1)
        ratings.forEachIndexed { i, r ->
            assertTrue("seed d${i + 1}", core.submitReview("d${i + 1}", r.toUByte()))
        }
        assertTrue("gate latch", core.recordAlphabetCheckResult(23u, 24u))
        assertTrue("gate satisfied", core.getAlphabetGateState().satisfied)
    }

    private fun reviewUuids(exportJson: String): Set<String> {
        val reviews = JSONObject(exportJson).getJSONArray("reviews")
        return buildSet {
            for (i in 0 until reviews.length()) add(reviews.getJSONObject(i).getString("uuid"))
        }
    }

    /** Failure message contract on timeouts: evidence listing + partial export. */
    private fun dumpWhat(core: GreekCore?): String {
        val listing = e2eDir().list()?.sorted()?.joinToString(", ") ?: "<empty>"
        val partial = try {
            core?.syncExportBackupJson()?.take(500)
        } catch (e: Exception) {
            "<export failed: $e>"
        }
        return "evidence dir: [$listing]; partial export: $partial"
    }

    /** Fail on timeout with the evidence dump (JUnit assertion semantics). */
    private fun timeoutFail(what: String, core: GreekCore?) {
        fail("TIMEOUT: $what; ${dumpWhat(core)}")
    }

    private fun writeEvidence(name: String, content: String) {
        File(e2eDir(), name).writeText(content)
    }

    private fun seedsJson(uuids: Set<String>, hlc: String): String =
        JSONObject()
            .put("uuids", JSONArray(uuids.sorted()))
            .put("hlc", hlc)
            .toString() + "\n"

    private fun statsJson(receivedNonlocal: Int, exportSize: Int): String =
        JSONObject()
            .put("receivedNonlocal", receivedNonlocal)
            .put("exportSize", exportSize)
            .toString() + "\n"

    /**
     * Tier-2 (iroh live pairing). Device seeds 5 reviews + gate latch,
     * publishes its pairing ticket, then polls sync rounds until the host's
     * 3 reviews land (8 uuids total). Keeps the endpoint alive afterwards so
     * the host's final pulls succeed. Evidence: ticket.txt, seeds_a.json,
     * export_a.json, stats_a.json, pull_done.flag.
     */
    @Test
    @LargeTest
    fun irohLivePairingE2E() {
        applyNetModeArg()
        val dir = scratchDir("iroh")
        val e2e = e2eDir()
        listOf("ticket.txt", "seeds_a.json", "pull_done.flag", "export_a.json", "stats_a.json")
            .forEach { File(e2e, it).delete() }

        val core = coreWithLetters(dir)
        try {
            seedDeviceReviewsAndGate(core)

            // Endpoint + doc creation can take 10-20s on a real device, so
            // the first calls get a generous timeout.
            val ticket = call(IROH_FIRST_OP_TIMEOUT_MS) { core.syncIrohTicket() }
            assertTrue("ticket payload shape: $ticket", ticket.startsWith("greeksync://ticket/"))
            // Immediately push local state into the doc so a joining peer
            // sees the device history on its first round.
            call(IROH_FIRST_OP_TIMEOUT_MS) { core.syncIrohNow() }

            val seedExport = core.syncExportBackupJson()
            val seedUuids = reviewUuids(seedExport)
            assertEquals("device must have exactly the 5 seeded reviews", 5, seedUuids.size)
            val seedHlc = JSONObject(seedExport).getString("to_hlc")
            writeEvidence("ticket.txt", ticket + "\n")
            writeEvidence("seeds_a.json", seedsJson(seedUuids, seedHlc))

            val deadlineMs = System.currentTimeMillis() + IROH_POLL_DEADLINE_MS
            var exportJson = seedExport
            var receivedNonlocal = 0
            while (true) {
                try {
                    call(IROH_OP_TIMEOUT_MS) { core.syncIrohNow() }
                } catch (t: TimeoutCancellationException) {
                    timeoutFail("syncIrohNow round wedged >${IROH_OP_TIMEOUT_MS}ms", core)
                }
                exportJson = core.syncExportBackupJson()
                val uuids = reviewUuids(exportJson)
                receivedNonlocal = (uuids - seedUuids).size
                if (receivedNonlocal >= HOST_REVIEW_COUNT && uuids.size == TOTAL_REVIEW_COUNT) break
                if (System.currentTimeMillis() > deadlineMs) {
                    timeoutFail("host reviews not received within ${IROH_POLL_DEADLINE_MS}ms", core)
                }
                Thread.sleep(IROH_POLL_INTERVAL_MS)
            }
            val finalUuids = reviewUuids(exportJson)
            assertEquals("export must hold device(5) + host(3) reviews", TOTAL_REVIEW_COUNT, finalUuids.size)
            assertEquals(HOST_REVIEW_COUNT, receivedNonlocal)
            assertTrue("gate must still be satisfied after merge", core.getAlphabetGateState().satisfied)

            writeEvidence("export_a.json", exportJson)
            writeEvidence("stats_a.json", statsJson(receivedNonlocal, exportJson.length))
            writeEvidence("pull_done.flag", "done\n")

            // Keep the iroh endpoint alive so the host's last pulls succeed.
            Thread.sleep(KEEP_ALIVE_MS)
        } catch (ce: CancellationException) {
            throw ce // respect coroutine cancellation semantics
        } finally {
            runCatching { core.close() }
        }
    }

    /**
     * Tier-2 (nostr over a local relay via `adb reverse tcp:7777`).
     * Identity flow is pre-seed host->device: the orchestrator writes
     * `nsec_in.hex` (lowercase hex, 64 chars) FIRST; every participant
     * imports the same identity, so no nsec getter is needed on device.
     * Device echoes the imported identity to `nsec.hex` for the record.
     * Evidence: nsec.hex, push_done.flag, export_a.json, stats_a.json,
     * pull_done.flag. Reads: b_pushed.flag (orchestrator go-ahead).
     */
    @Test
    @LargeTest
    fun nostrLocalRelayE2E() {
        val dir = scratchDir("nostr")
        val e2e = e2eDir()
        // Stale-output hygiene; nsec_in.hex + b_pushed.flag are orchestrator
        // inputs, but b_pushed.flag from a PREVIOUS run must not be honored.
        listOf("nsec.hex", "push_done.flag", "pull_done.flag", "export_a.json",
               "stats_a.json", "seeds_a.json", "b_pushed.flag").forEach { File(e2e, it).delete() }

        val core = coreWithLetters(dir)
        try {
            core.syncSetNostrRelays("ws://127.0.0.1:7777")

            // Wait for the host-pre-seeded identity (orchestrator writes it
            // after `adb reverse` is up).
            val nsecIn = File(e2e, "nsec_in.hex")
            val nsecDeadline = System.currentTimeMillis() + FLAG_DEADLINE_MS
            while (!nsecIn.exists()) {
                if (System.currentTimeMillis() > nsecDeadline) {
                    timeoutFail("orchestrator did not write nsec_in.hex", core)
                }
                Thread.sleep(FLAG_POLL_INTERVAL_MS)
            }
            val nsec = nsecIn.readText().trim()
            assertTrue("nsec_in.hex must be 64 lowercase hex, got '$nsec'",
                nsec.matches(Regex("[0-9a-f]{64}")))
            core.syncSetNostrNsec(nsec)
            assertTrue("identity imported", core.syncNostrConfig().hasIdentity)
            writeEvidence("nsec.hex", nsec + "\n")

            seedDeviceReviewsAndGate(core)
            // Contract: publish seeds_a.json BEFORE push_done.flag — host B
            // compares its pulled uuid set against these.
            val seeds = reviewUuids(core.syncExportBackupJson())
            val seedArr = seeds.joinToString(",") { "\"$it\"" }
            writeEvidence("seeds_a.json", "{\"uuids\":[$seedArr],\"hlc\":\"\"}")
            call(NOSTR_OP_TIMEOUT_MS) { core.syncNostrPush() }
            writeEvidence("push_done.flag", "done\n")

            // Wait for the host's push before pulling.
            val bFlag = File(e2e, "b_pushed.flag")
            val bDeadline = System.currentTimeMillis() + FLAG_DEADLINE_MS
            while (!bFlag.exists()) {
                if (System.currentTimeMillis() > bDeadline) {
                    timeoutFail("orchestrator did not write b_pushed.flag", core)
                }
                Thread.sleep(FLAG_POLL_INTERVAL_MS)
            }

            // Pull until the host's 3 reviews land (8 uuids total).
            val seedUuids = reviewUuids(core.syncExportBackupJson())
            assertEquals(5, seedUuids.size)
            val pullDeadline = System.currentTimeMillis() + NOSTR_PULL_DEADLINE_MS
            var exportJson: String
            var receivedNonlocal: Int
            while (true) {
                exportJson = core.syncExportBackupJson()
                receivedNonlocal = (reviewUuids(exportJson) - seedUuids).size
                if (receivedNonlocal >= HOST_REVIEW_COUNT && reviewUuids(exportJson).size == TOTAL_REVIEW_COUNT) break
                if (System.currentTimeMillis() > pullDeadline) {
                    timeoutFail("host reviews not received via nostr within ${NOSTR_PULL_DEADLINE_MS}ms", core)
                }
                try {
                    call(NOSTR_OP_TIMEOUT_MS) { core.syncNostrPull() }
                } catch (t: TimeoutCancellationException) {
                    timeoutFail("syncNostrPull wedged >${NOSTR_OP_TIMEOUT_MS}ms", core)
                }
                Thread.sleep(NOSTR_POLL_INTERVAL_MS)
            }
            val finalUuids = reviewUuids(exportJson)
            assertEquals(TOTAL_REVIEW_COUNT, finalUuids.size)
            assertEquals(HOST_REVIEW_COUNT, receivedNonlocal)
            assertTrue("gate must still be satisfied after merge", core.getAlphabetGateState().satisfied)

            writeEvidence("export_a.json", exportJson)
            writeEvidence("stats_a.json", statsJson(receivedNonlocal, exportJson.length))
            writeEvidence("pull_done.flag", "done\n")
        } finally {
            runCatching { core.close() }
        }
    }

    /**
     * Tier-4 sync edge cases — all local, all fast, no network expectations.
     */
    @Test
    @LargeTest
    fun syncEdgeCases() {
        // (a) Truncated/corrupted backup JSON -> typed Sync error, no crash,
        // instance stays usable afterwards.
        scratchCoreRun("edge-a") { core ->
            val ex = expectSyncError {
                core.syncImportBackupJson("""{"v":1,"device_id":"x","reviews":[""")
            }
            assertTrue("truncated import must mention bad JSON, got: $ex",
                ex.v1.contains("bad backup JSON"))
            assertTrue("core still healthy after rejected import",
                core.submitReview("d1", 3.toUByte()))
        }

        // (b) Well-formed batch carrying a hostile card_id
        // ("x'; DROP TABLE memory_states;--") is dropped silently per the
        // core's apply_review_events validation (card-id charset/rating/HLC
        // gate); the DB keeps answering queries afterwards.
        scratchCoreRun("edge-b") { core ->
            val hostile = """x'; DROP TABLE memory_states;--"""
            val batch = wireBatch(
                uuid = "e2e-hostile-1", cardId = hostile, rating = 3,
                hlc = "1749039000000-0000-aabbccdd", deviceId = "attacker")
            val stats = core.syncImportBackupJson(batch)
            assertEquals("hostile event must be dropped", 0u, stats.reviewsReceived)
            assertTrue("nothing applied", reviewUuids(core.syncExportBackupJson()).isEmpty())
            // Integrity: DB still answers queries; SQL injection had no effect.
            assertTrue(core.submitReview("d1", 4.toUByte()))
            assertEquals(1, reviewUuids(core.syncExportBackupJson()).size)
        }

        // (c) Malformed iroh ticket -> clean typed Sync error, no panic.
        scratchCoreRun("edge-c") { core ->
            val ex = expectSyncErrorSuspend {
                core.syncIrohJoin("greeksync://ticket/GARBAGE")
            }
            assertTrue("ticket error should be descriptive: $ex", ex.v1.isNotEmpty())
        }

        // (d) syncNostrPull with no identity/relay configured -> clean Sync
        // error explaining the missing identity (mirrors the Rust e2e test).
        scratchCoreRun("edge-d") { core ->
            val ex = expectSyncErrorSuspend { core.syncNostrPull() }
            assertTrue("unconfigured pull must mention identity: $ex",
                ex.v1.contains("identity"))
        }

        // (e) Empty-delta: fresh instance, no reviews, no peers ->
        // syncIrohNow returns quickly with zero counts (no hang).
        scratchCoreRun("edge-e") { core ->
            val t0 = System.currentTimeMillis()
            val stats = call(CALL_TIMEOUT_MS) { core.syncIrohNow() }
            val elapsed = System.currentTimeMillis() - t0
            assertEquals("no reviews to send", 0u, stats.reviewsSent)
            assertEquals("no peers to receive from", 0u, stats.reviewsReceived)
            assertTrue("empty-delta round must be quick (${elapsed}ms)",
                elapsed < CALL_TIMEOUT_MS)
        }

        // (f) Idempotent re-import: export X, import into Y twice; the
        // second import inserts 0 reviews (uuid-dedup) and applies 0 settings
        // (HLC-LWW same-HLC loses).
        run {
            val dirX = scratchDir("edge-f-x")
            val dirY = scratchDir("edge-f-y")
            val x = GreekCore(File(dirX, "greek_app.db").absolutePath, null)
            val y = GreekCore(File(dirY, "greek_app.db").absolutePath, null)
            try {
                assertTrue(x.submitReview("d1", 3.toUByte()))
                assertTrue(x.submitReview("d2", 4.toUByte()))
                val json = x.syncExportBackupJson()
                val first = y.syncImportBackupJson(json)
                assertEquals(2u, first.reviewsReceived)
                val second = y.syncImportBackupJson(json)
                assertEquals("replay must insert nothing", 0u, second.reviewsReceived)
                assertEquals("same-HLC settings must not re-apply", 0u, second.settingsApplied)
                assertEquals(2, reviewUuids(y.syncExportBackupJson()).size)
            } finally {
                runCatching { x.close() }
                runCatching { y.close() }
            }
        }
    }

    // ---- helpers ----

    /** Minimal well-formed v1 SyncBatch JSON with a single review event.
     * `hlc` uses the frozen wire format `"{ms:013}-{counter:04}-{node:8}"`. */
    private fun wireBatch(
        uuid: String, cardId: String, rating: Int, hlc: String, deviceId: String,
    ): String = JSONObject()
        .put("v", 1)
        .put("device_id", deviceId)
        .put("from_hlc", hlc)
        .put("to_hlc", hlc)
        .put("reviews", JSONArray().put(
            JSONObject()
                .put("uuid", uuid)
                .put("card_id", cardId)
                .put("timestamp", "2026-06-09T00:00:00Z")
                .put("rating", rating)
                .put("elapsed_days", 0.0)
                .put("scheduled_days", 0.0)
                .put("source", "review")
                .put("device_id", deviceId)
                .put("hlc", hlc)))
        .put("settings", JSONArray())
        .toString()

    private fun scratchCoreRun(tag: String, block: (GreekCore) -> Unit) {
        val dir = scratchDir(tag)
        val core = GreekCore(File(dir, "greek_app.db").absolutePath, null)
        try {
            block(core)
        } finally {
            runCatching { core.close() }
        }
    }

    private fun expectSyncError(block: () -> Unit): GreekException.Sync =
        try {
            block()
            throw AssertionError("expected GreekException.Sync (clean, typed error — no crash)")
        } catch (e: GreekException.Sync) {
            e
        }

    private fun expectSyncErrorSuspend(block: suspend () -> Unit): GreekException.Sync =
        try {
            call(CALL_TIMEOUT_MS) { block() }
            throw AssertionError("expected GreekException.Sync (clean, typed error — no crash)")
        } catch (e: GreekException.Sync) {
            e
        }

    /** greek-sync-e2e v1 protocol constants (timings frozen with the host
     * harness — do not change without coordinating). */
    private companion object {
        const val HOST_REVIEW_COUNT = 3
        const val TOTAL_REVIEW_COUNT = 8

        // iroh tier
        const val IROH_OP_TIMEOUT_MS = 90_000L
        const val IROH_FIRST_OP_TIMEOUT_MS = 120_000L
        const val IROH_POLL_INTERVAL_MS = 5_000L
        const val IROH_POLL_DEADLINE_MS = 150_000L
        const val KEEP_ALIVE_MS = 30_000L

        // nostr tier
        const val NOSTR_OP_TIMEOUT_MS = 60_000L
        const val NOSTR_POLL_INTERVAL_MS = 3_000L
        const val NOSTR_PULL_DEADLINE_MS = 90_000L
        const val FLAG_POLL_INTERVAL_MS = 1_000L
        const val FLAG_DEADLINE_MS = 90_000L

        // edge cases
        const val CALL_TIMEOUT_MS = 60_000L
    }
}
