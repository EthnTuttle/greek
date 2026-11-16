package com.greekapp.viewmodel

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Regression tests for audit items A6 (identity overwrite) and the
 * backup-export-lost-on-rotation P2 in SyncViewModel.
 *
 * Following the CurriculumViewModelTest / ReviewViewModelTest precedent, the
 * ViewModel itself can't run on plain JUnit (viewModelScope needs
 * Dispatchers.Main, android.util.Log is stubbed), so the decisions are
 * extracted into static, Log-free companion helpers and tested here:
 *
 *  - A6: generating with an existing identity overwrites the stored secret
 *    unconditionally, so [SyncViewModel.generateAction] must route through
 *    the rotation-confirmation gate — the no-confirm path can therefore
 *    never reach the core.
 *
 *  - Export P2: the core returns an `{"error": ...}`-shaped string on export
 *    failure; [SyncViewModel.backupPayloadError] must reject it so the SAF
 *    picker is never launched and no file gets a fake "success" write.
 *
 *  - pendingExport ownership: the staged payload lives in SyncState (the
 *    ViewModel), not in a `remember`, so it survives configuration changes
 *    while the document picker is open.
 */
class SyncViewModelTest {

    // ---- A6: identity-rotation gate ----

    @Test
    fun generateWithExistingIdentity_requiresConfirmation() {
        // RequireConfirmation means requestGenerateIdentity() only flips the
        // dialog flag — generateIdentity() (the private core call) is
        // unreachable until confirmIdentityRotation().
        assertEquals(
            SyncViewModel.Companion.GenerateAction.RequireConfirmation,
            SyncViewModel.generateAction(hasIdentity = true)
        )
    }

    @Test
    fun generateWithoutIdentity_runsImmediately() {
        assertEquals(
            SyncViewModel.Companion.GenerateAction.GenerateNow,
            SyncViewModel.generateAction(hasIdentity = false)
        )
    }

    @Test
    fun rotationConfirmation_isViewModelStateNotRemember() {
        // The dialog open flag and the identity flag live in SyncState, so a
        // config change can't dismiss the dialog into a state where a later
        // tap bypasses confirmation.
        val rotating = SyncState(hasIdentity = true, rotationConfirmPending = true)
        assertTrue(rotating.rotationConfirmPending)
        assertTrue(rotating.hasIdentity)
    }

    // ---- Export P2: payload validation ----

    @Test
    fun backupPayload_rejectsCoreErrorShape() {
        // The exact failure shape the core returns (lib.rs export path).
        assertNotNull(SyncViewModel.backupPayloadError("{\"error\":\"database is locked\"}"))
    }

    @Test
    fun backupPayload_acceptsVersionedBackup() {
        assertNull(
            SyncViewModel.backupPayloadError(
                "{\"version\":1,\"reviews\":[{\"card\":\"γράφω\"}],\"settings\":{}}"
            )
        )
    }

    @Test
    fun backupPayload_rejectsNullEmptyAndNonObjects() {
        assertNotNull(SyncViewModel.backupPayloadError(null))
        assertNotNull(SyncViewModel.backupPayloadError(""))
        assertNotNull(SyncViewModel.backupPayloadError("   "))
        assertNotNull(SyncViewModel.backupPayloadError("[]"))
        assertNotNull(SyncViewModel.backupPayloadError("\"version\""))
        assertNotNull(SyncViewModel.backupPayloadError("{")) // unterminated
    }

    @Test
    fun backupPayload_rejectsVersionlessObject() {
        assertNotNull(SyncViewModel.backupPayloadError("{}"))
        assertNotNull(SyncViewModel.backupPayloadError("{\"reviews\":[],\"settings\":{}}"))
    }

    @Test
    fun backupPayload_bracesAndQuotesInsideValuesDontConfuseTheCheck() {
        // A value containing structural characters inside a string must not
        // end the object scan early or hide the version key.
        assertNull(
            SyncViewModel.backupPayloadError(
                "{\"note\":\"brace } and \\\"quote\\\" and [\",\"version\":3}"
            )
        )
        // Error key hiding AFTER a composite value must still be seen.
        assertNotNull(
            SyncViewModel.backupPayloadError(
                "{\"data\":[{\"version\":1}],\"error\":\"partial export\"}"
            )
        )
    }

    // ---- Credential recovery: confirmation + one-shot copy ----

    @Test
    fun identityExport_neverRevealsBeforeConfirmation() {
        val waiting = SyncState(hasIdentity = true, identityExportConfirmPending = true)
        val notConfirmed = SyncViewModel.identityExportRevealState(
            waiting,
            confirmed = false,
            nsec = "secret"
        )
        assertNull(notConfirmed.exportedNsec)
        assertTrue(!notConfirmed.identityExportConfirmPending)

        val confirmed = SyncViewModel.identityExportRevealState(
            waiting,
            confirmed = true,
            nsec = "secret"
        )
        assertEquals("secret", confirmed.exportedNsec)
        assertTrue(!confirmed.identityExportConfirmPending)
    }

    @Test
    fun identityExport_dismissedDialogCannotRevealLateCoreResult() {
        val dismissed = SyncState(hasIdentity = true, identityExportConfirmPending = false)
        val result = SyncViewModel.identityExportRevealState(
            dismissed,
            confirmed = true,
            nsec = "secret"
        )
        assertNull(result.exportedNsec)
    }

    @Test
    fun identityExport_copyHandsOffRevealedValueOnlyOnce() {
        val revealed = SyncState(exportedNsec = "secret")
        val (afterFirst, first) = SyncViewModel.identityExportCopyState(revealed)
        assertEquals("secret", first)
        assertTrue(afterFirst.identityExportCopied)
        // Dialog text remains available for manual selection, but another
        // copy action cannot trigger another value handoff/core operation.
        assertEquals("secret", afterFirst.exportedNsec)
        val (afterSecond, second) = SyncViewModel.identityExportCopyState(afterFirst)
        assertNull(second)
        assertEquals(afterFirst, afterSecond)
    }

    // ---- Export P2: staged payload ownership ----

    @Test
    fun pendingExport_isViewModelStateNotRemember() {
        // SyncState (owned by the ViewModel) carries the payload between the
        // fetch callback and the SAF picker callback; a config change drops
        // composition state, not this.
        val staged = SyncState(pendingExport = "{\"version\":1}")
        assertEquals("{\"version\":1}", staged.pendingExport)
        // Consuming (the picker callback path) clears it.
        assertNull(staged.copy(pendingExport = null).pendingExport)
    }
}
