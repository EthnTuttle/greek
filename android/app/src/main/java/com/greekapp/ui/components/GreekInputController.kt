package com.greekapp.ui.components

import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.OutlinedTextField
import androidx.compose.runtime.Composable
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import java.text.BreakIterator
import java.text.Normalizer
import java.util.Locale

/**
 * Wave-9 in-app Greek keyboard input contract (DESIGN-greek-keyboard.md,
 * Appendix R — binding).
 *
 * The keyboard panel commits into whatever [GreekTextField] is currently
 * focused. Fields register their controller with the [GreekInputDispatcher]
 * provided via [LocalGreekInputDispatcher] at the app root while they hold
 * focus; the panel reads the active controller at tap time. No focus theft:
 * keys never request focus and taps never dismiss the system IME.
 */
interface GreekInputController {
    /**
     * Insert [text] (already NFC-normalized by the panel) at the selection.
     * With [replaceComposing] = false an in-flight IME composition is
     * preserved (never clobbered): insertion lands at the selection and the
     * composition range is adjusted. With true, an active composition is
     * committed-replaced by [text].
     */
    fun insertText(text: String, replaceComposing: Boolean = false)

    /** Delete the grapheme cluster immediately before the cursor
     *  ([java.text.BreakIterator], Greek locale) or the selection. */
    fun deleteBackward()

    /**
     * Panel context hook (binding-spec extension): the σ/ς key needs the
     * current field text and cursor to decide mid-word vs final sigma
     * ([GreekTextOps.resolveSigma]).
     */
    val currentValue: TextFieldValue?
}

/**
 * Registry for "which Greek-enabled field is focused right now", provided
 * once at the app root. Plain holder — the panel reads [active] at tap
 * time, so no recomposition contract is needed.
 */
class GreekInputDispatcher {
    @Volatile
    var active: GreekInputController? = null
}

val LocalGreekInputDispatcher = compositionLocalOf { GreekInputDispatcher() }

/**
 * Pure text surgery for [GreekInputController] implementations, expressed on
 * plain range primitives so it is JVM-unit-testable without Compose classes.
 */
object GreekTextOps {

    private val graphemeIterator: BreakIterator
        get() = BreakIterator.getCharacterInstance(Locale("el"))

    /**
     * Insert [insert] replacing [selStart, selEnd); an in-flight composition
     * [compStart, compEnd) (use -1, -1 for none) is preserved and remapped
     * unless [replaceComposing], in which case the composition range itself
     * is the replaced span. Returns (newText, newCursor, compStart, compEnd).
     */
    fun insert(
        text: String,
        selStart: Int,
        selEnd: Int,
        compStart: Int,
        compEnd: Int,
        insert: String,
        replaceComposing: Boolean
    ): TextEditResult {
        val hasComp = compStart in 0..compEnd && compEnd <= text.length
        return if (hasComp && replaceComposing) {
            val newText = text.replaceRange(compStart, compEnd, insert)
            TextEditResult(newText, compStart + insert.length, -1, -1)
        } else {
            val newText = text.replaceRange(selStart, selEnd, insert)
            val cursor = selStart + insert.length
            var cs = compStart
            var ce = compEnd
            if (hasComp) {
                val delta = insert.length - (selEnd - selStart)
                when {
                    // Insertion exactly at the composition end appends to it
                    // (ongoing typing inside a composition).
                    selStart == selEnd && selStart == ce -> { ce += insert.length }
                    selEnd <= cs -> { cs += delta; ce += delta }
                    selStart > ce -> { /* untouched */ }
                    else -> {
                        // Insertion overlaps the composition: keep the span
                        // covering the union (never clobber mid-composition).
                        cs = minOf(cs, selStart)
                        ce = maxOf(ce, selEnd) + delta
                    }
                }
                if (cs >= ce || ce > newText.length) { cs = -1; ce = -1 }
            }
            TextEditResult(newText, cursor, cs, ce)
        }
    }

    /** Offset of the start of the grapheme cluster ending at [offset]. */
    fun previousGraphemeBoundary(text: String, offset: Int): Int {
        if (offset <= 0) return 0
        val it = graphemeIterator
        it.setText(text)
        val prev = it.preceding(offset)
        return if (prev == BreakIterator.DONE) 0 else prev
    }

    /** Delete selection, or the grapheme cluster before the cursor. */
    fun deleteBackward(
        text: String,
        selStart: Int,
        selEnd: Int,
        compStart: Int,
        compEnd: Int
    ): TextEditResult {
        val hasComp = compStart in 0..compEnd && compEnd <= text.length
        if (selStart != selEnd) {
            val newText = text.removeRange(selStart, selEnd)
            val removed = selEnd - selStart
            var cs = compStart
            var ce = compEnd
            if (hasComp) {
                when {
                    selEnd <= cs -> { cs -= removed; ce -= removed }
                    selStart >= ce -> {}
                    else -> {
                        cs = minOf(cs, selStart)
                        ce = maxOf(cs, ce - removed)
                        if (cs >= ce) { cs = -1; ce = -1 }
                    }
                }
                if (cs >= 0 && ce > newText.length) { cs = -1; ce = -1 }
            }
            return TextEditResult(newText, selStart, cs, ce)
        }
        if (selStart == 0) return TextEditResult(text, 0, compStart, compEnd)
        val boundary = previousGraphemeBoundary(text, selStart)
        val newText = text.removeRange(boundary, selStart)
        val removed = selStart - boundary
        var cs = compStart
        var ce = compEnd
        if (hasComp) {
            when {
                selStart <= cs -> { cs -= removed; ce -= removed }
                boundary >= ce -> {}
                else -> {
                    cs = minOf(cs, boundary)
                    ce -= removed
                    if (cs >= ce) { cs = -1; ce = -1 }
                }
            }
            if (cs >= 0 && ce > newText.length) { cs = -1; ce = -1 }
        }
        return TextEditResult(newText, boundary, cs, ce)
    }

    /** Number of grapheme clusters in [s] (1 = single grapheme typed). */
    fun graphemeCount(s: String): Int {
        if (s.isEmpty()) return 0
        val it = graphemeIterator
        it.setText(s)
        var count = 0
        it.first()
        while (it.next() != BreakIterator.DONE) count++
        return count
    }

    @Suppress("DEPRECATION")
    private fun Int.isGreekLetter(): Boolean {
        if (this !in 0x0370..0x03FF && this !in 0x1F00..0x1FFF) return false
        val ch = this.toChar()
        return Character.isLetter(ch) &&
            Character.UnicodeBlock.of(ch).let {
                it == Character.UnicodeBlock.GREEK ||
                    it == Character.UnicodeBlock.GREEK_EXTENDED
            }
    }

    private fun Int.isCombiningGreekMark(): Boolean =
        Character.getType(this) == Character.NON_SPACING_MARK.toInt() ||
            this == 0x0313 || this == 0x0314 || this == 0x0342 || this == 0x0345 ||
            this == 0x0308 || this == 0x0300 || this == 0x0301 || this == 0x0304 ||
            this == 0x0306

    /**
     * Context-appropriate sigma for an insertion at ([selStart]..[selEnd]) in
     * [text] (per binding spec: σ mid-word, ς automatically when the next
     * char is none/space/punctuation — computed from the live field text).
     * Emits σ at word start (no preceding Greek letter).
     */
    fun resolveSigma(text: String, selStart: Int, selEnd: Int): String {
        // Walk back past combining marks to find the base char before cursor.
        var i = selStart
        while (i > 0) {
            val cp = text.codePointBefore(i)
            if (cp.isCombiningGreekMark()) {
                i -= Character.charCount(cp)
            } else {
                break
            }
        }
        val prev = if (i > 0) text.codePointBefore(i) else null
        val precededByGreekLetter = prev != null && prev.isGreekLetter()

        // A selection that continues with a Greek letter mid-word stays σ.
        var j = selEnd
        val next: Int? = run {
            while (j < text.length) {
                val cp = text.codePointAt(j)
                if (cp.isCombiningGreekMark()) {
                    j += Character.charCount(cp)
                } else {
                    return@run cp
                }
            }
            null
        }

        val nextEndsWord = next == null ||
            next.toChar().isWhitespace() ||
            !next.isGreekLetter()
        return if (precededByGreekLetter && nextEndsWord) "ς" else "σ"
    }

    /** (newText, cursor, compositionStart, compositionEnd); -1s = no composition. */
    data class TextEditResult(
        val text: String,
        val cursor: Int,
        val compStart: Int,
        val compEnd: Int
    )
}

/**
 * Default [GreekInputController]: performs selection/composition-preserving
 * surgery on a [MutableState] of [TextFieldValue]. Created once per
 * [GreekTextField] and exposed to the panel via [GreekInputDispatcher].
 */
class MutableStateGreekInputController(
    private val state: MutableState<TextFieldValue>
) : GreekInputController {

    override val currentValue: TextFieldValue
        get() = state.value

    override fun insertText(text: String, replaceComposing: Boolean) {
        // Panel commits are NFC-normalized (design §4: precomposed forms,
        // NFC on commit) so deck text always matches byte-for-byte.
        val normalized = Normalizer.normalize(text, Normalizer.Form.NFC)
        val v = state.value
        val comp = v.composition
        val r = GreekTextOps.insert(
            v.text, v.selection.start, v.selection.end,
            comp?.start ?: -1, comp?.end ?: -1,
            normalized, replaceComposing
        )
        state.value = v.copy(
            text = r.text,
            selection = TextRange(r.cursor),
            composition = if (r.compStart >= 0) TextRange(r.compStart, r.compEnd) else null
        )
    }

    override fun deleteBackward() {
        val v = state.value
        val comp = v.composition
        val r = GreekTextOps.deleteBackward(
            v.text, v.selection.start, v.selection.end,
            comp?.start ?: -1, comp?.end ?: -1
        )
        state.value = v.copy(
            text = r.text,
            selection = TextRange(r.cursor),
            composition = if (r.compStart >= 0) TextRange(r.compStart, r.compEnd) else null
        )
    }
}

/**
 * A Greek-enabled text field: visually a Material3 [OutlinedTextField], but
 * it owns a [TextFieldValue] (selection + IME composition survive panel
 * commits) and registers its [GreekInputController] with the dispatcher
 * while focused, so the on-screen keyboard types into the focused field.
 *
 * The external contract stays `String`-based (callers keep their existing
 * `String` state); external resets (cleared query, next question) replace
 * the field content and move the cursor to the end. System IME input is
 * untouched in every input mode — the panel is a union, never an xor.
 */
@Composable
fun GreekTextField(
    value: String,
    onValueChange: (String) -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    singleLine: Boolean = true,
    label: @Composable (() -> Unit)? = null,
    placeholder: @Composable (() -> Unit)? = null,
    supportingText: @Composable (() -> Unit)? = null,
    leadingIcon: @Composable (() -> Unit)? = null,
    trailingIcon: @Composable (() -> Unit)? = null,
    keyboardOptions: KeyboardOptions = KeyboardOptions.Default,
    keyboardActions: KeyboardActions = KeyboardActions.Default
) {
    val dispatcher = LocalGreekInputDispatcher.current
    val tfvState = remember { mutableStateOf(TextFieldValue(value, TextRange(value.length))) }

    // External source-of-truth push (question advanced / query cleared):
    // replace content when it differs from what the field shows. Panel and
    // IME edits always flow through here first, so this never fights them.
    val tfv = tfvState.value
    if (tfv.text != value) {
        tfvState.value = TextFieldValue(value, TextRange(value.length))
    }

    val controller = remember { MutableStateGreekInputController(tfvState) }

    OutlinedTextField(
        value = tfvState.value,
        onValueChange = { newValue ->
            tfvState.value = newValue
            onValueChange(newValue.text)
        },
        modifier = modifier.onFocusChanged { focus ->
            if (focus.isFocused) {
                dispatcher.active = controller
            } else if (dispatcher.active === controller) {
                dispatcher.active = null
            }
        },
        enabled = enabled,
        singleLine = singleLine,
        label = label,
        placeholder = placeholder,
        supportingText = supportingText,
        leadingIcon = leadingIcon,
        trailingIcon = trailingIcon,
        keyboardOptions = keyboardOptions,
        keyboardActions = keyboardActions
    )
}
