package com.greekapp.core

import org.junit.Assert.assertEquals
import org.junit.Test
import uniffi.uniffi_bindings.ShareConfigView
import uniffi.uniffi_bindings.ShareFlushReportView

/**
 * Regression test for the wave-9 config-mapping P2: the generated
 * [ShareConfigView] exposes the pubkey as `sharePubkeyHex`, but the old
 * reflective adapter read `pubkeyHex` — always missing, silently serving
 * an empty key and latching the share surface off.
 *
 * Constructing the generated record with NAMED ARGS here makes the field
 * name compile-checked against the checked-in bindings: a regen that
 * renames `sharePubkeyHex` fails compilation in this test, not at runtime
 * inside a reflection blind spot.
 */
class ShareRecordMappingTest {

    @Test
    fun shareConfigViewMapsSharePubkeyHex() {
        val pubkey = "ab".repeat(32)
        val view = ShareConfigView(
            enabled = true,
            relays = listOf("wss://relay-a.example", "wss://relay-b.example"),
            sharePubkeyHex = pubkey
        )
        val config = view.toShareConfig()
        assertEquals(true, config.enabled)
        assertEquals(pubkey, config.pubkeyHex)
        assertEquals(listOf("wss://relay-a.example", "wss://relay-b.example"), config.relays)
    }

    @Test
    fun shareConfigViewMapsEmptyIdentityWhenNeverEnabled() {
        val config = ShareConfigView(
            enabled = false,
            relays = emptyList(),
            sharePubkeyHex = ""
        ).toShareConfig()
        assertEquals(ShareConfig.DISABLED, config)
    }

    @Test
    fun shareFlushReportViewMapsCountsAndFailures() {
        val view = ShareFlushReportView(
            posted = 2u,
            deferred = 1u,
            duplicates = 0u,
            failed = listOf("wss://r.example: timeout")
        )
        assertEquals(
            ShareFlushReport(2u, 1u, 0u, listOf("wss://r.example: timeout")),
            view.toShareFlushReport()
        )
    }
}
