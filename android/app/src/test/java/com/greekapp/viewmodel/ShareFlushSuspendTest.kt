package com.greekapp.viewmodel

import com.greekapp.core.ShareFlushReport
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

/**
 * Regression test for the wave-9 suspend-signature P2: the regenerated
 * UniFFI binding declares `shareFlushNow` as a *suspend* fun (JVM
 * signature gains a hidden Continuation, which is why the old zero-arg
 * reflective lookup could never find it). The seam interface and
 * [FakeGreekCore] therefore expose `suspend fun shareFlushNow`; these
 * tests drive that suspend path from a host-JVM coroutine context and
 * prove the handler's result surfaces to the caller unchanged.
 */
class ShareFlushSuspendTest {

    @Test
    fun shareFlushNowHandlerResult_surfacesToCaller() = runBlocking {
        var calls = 0
        val expected = ShareFlushReport(
            posted = 2u,
            deferred = 1u,
            duplicates = 0u,
            failed = listOf("wss://r.example: timeout")
        )
        val core = FakeGreekCore(onShareFlushNow = { calls++; expected })

        val got = core.shareFlushNow()

        assertEquals(1, calls)
        assertEquals(expected, got)
    }

    @Test
    fun shareFlushNowCallSite_inCoroutineContext() {
        // Shape of the ShareViewModel.flushNow call site: launched coroutine,
        // result consumed after suspension. Using the fake keeps it host-JVM.
        val core = FakeGreekCore(onShareFlushNow = { ShareFlushReport.EMPTY })
        val got = runBlocking { core.shareFlushNow() }
        assertEquals(ShareFlushReport.EMPTY, got)
    }

    @Test
    fun shareFlushNowWithoutHandler_throwsNotImplementedError() {
        // FakeGreekCore's house style: unstubbed members throw.
        val core = FakeGreekCore()
        try {
            runBlocking<Unit> { core.shareFlushNow() }
            fail("expected NotImplementedError")
        } catch (e: NotImplementedError) {
            assertTrue(true)
        }
    }
}
