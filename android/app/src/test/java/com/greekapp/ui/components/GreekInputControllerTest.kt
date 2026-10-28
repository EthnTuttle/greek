package com.greekapp.ui.components

import androidx.compose.runtime.MutableState
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import org.junit.Assert.assertEquals
import org.junit.Assert.fail
import org.junit.Test

/**
 * Regression tests for production audits A4 (panel commits mutated only the
 * internal state and never reached the screen's onValueChange) and the
 * reverse-selection P2 (a TextRange with start > end threw
 * IndexOutOfBoundsException inside the range surgery).
 *
 * Host-JVM: MutableStateGreekInputController only needs Compose runtime
 * state + TextFieldValue, both plain JVM classes — no Robolectric.
 */
class GreekInputControllerTest {

    private fun controllerWith(
        text: String,
        selection: TextRange,
        onCommit: (String) -> Unit = {}
    ): Pair<MutableStateGreekInputController, MutableState<TextFieldValue>> {
        val state = mutableStateOf(TextFieldValue(text, selection))
        return MutableStateGreekInputController(state, onCommit) to state
    }

    // ---- Reverse-selection P2 ----

    @Test
    fun `insertText with reversed selection replaces the same range`() {
        // Reversed (3,1) must behave identically to (1,3).
        val (c, state) = controllerWith("λογος", TextRange(3, 1))
        c.insertText("AA")
        assertEquals("λAAος", state.value.text)
        assertEquals(TextRange(3), state.value.selection)
    }

    @Test
    fun `deleteBackward with reversed selection deletes the same range`() {
        val (c, state) = controllerWith("λογος", TextRange(3, 1))
        c.deleteBackward()
        assertEquals("λος", state.value.text)
        assertEquals(TextRange(1), state.value.selection)
    }

    @Test
    fun `reversed selection never throws`() {
        val state = mutableStateOf(TextFieldValue("αβγδ", TextRange(3, 1)))
        val c = MutableStateGreekInputController(state)
        try {
            c.insertText("X")
            c.deleteBackward()
        } catch (e: IndexOutOfBoundsException) {
            fail("reversed selection must not reach replaceRange/removeRange raw: $e")
        }
    }

    @Test
    fun `collapsed selection backward delete still removes one grapheme`() {
        val (c, state) = controllerWith("λογος", TextRange(2))
        c.deleteBackward()
        assertEquals("λγος", state.value.text)
        assertEquals(TextRange(1), state.value.selection)
    }

    // ---- A4: panel commits propagate ----

    @Test
    fun `insertText commit reaches onValueChange`() {
        val committed = mutableListOf<String>()
        val (c, state) = controllerWith("λ", TextRange(1)) { committed += it }
        c.insertText("ογος")
        assertEquals("λογος", state.value.text)
        assertEquals(listOf("λογος"), committed)
    }

    @Test
    fun `deleteBackward commit reaches onValueChange`() {
        val committed = mutableListOf<String>()
        val (c, _) = controllerWith("λογος", TextRange(5)) { committed += it }
        c.deleteBackward() // deletes final ς grapheme
        assertEquals(listOf("λογο"), committed)
    }
}
