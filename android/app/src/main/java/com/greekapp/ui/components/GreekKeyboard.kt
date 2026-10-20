package com.greekapp.ui.components

import android.view.HapticFeedbackConstants
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.interaction.Interaction
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.PressInteraction
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ripple
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.paneTitle
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Popup
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.greekapp.data.GreekInputMode
import com.greekapp.data.GreekInputRepository
import com.greekapp.ui.theme.GreekFontFamily
import kotlin.math.roundToInt
import kotlinx.coroutines.delay

/**
 * Wave-9 in-app polytonic Greek keyboard (DESIGN-greek-keyboard.md +
 * Appendix R — binding decisions).
 *
 * - [GreekKeyboardPanel]: the 24-letter key grid (shift flips case) + a
 *   single shared variant [Popup] opened by long-press; commits through the
 *   [GreekInputController] of the currently focused [GreekTextField]
 *   (looked up in [LocalGreekInputDispatcher]). Panel taps never take focus
 *   and never dismiss the system IME.
 * - [GreekKeyboardContainer]: screen-level mount — wraps a screen's content
 *   and docks the panel per the device-local input mode preference.
 */

/** Display names for TalkBack + variant descriptions. */
private val LETTER_NAMES: Map<Char, String> = mutableMapOf<Char, String>().apply {
    fun put(ch: Char, name: String) {
        put(ch, name)
        put(ch.uppercaseChar(), name)
    }
    put('α', "alpha"); put('β', "beta"); put('γ', "gamma"); put('δ', "delta")
    put('ε', "epsilon"); put('ζ', "zeta"); put('η', "eta"); put('θ', "theta")
    put('ι', "iota"); put('κ', "kappa"); put('λ', "lambda"); put('μ', "mu")
    put('ν', "nu"); put('ξ', "xi"); put('ο', "omicron"); put('π', "pi")
    put('ρ', "rho"); put('σ', "sigma"); put('ς', "final sigma"); put('τ', "tau")
    put('υ', "upsilon"); put('φ', "phi"); put('χ', "chi"); put('ψ', "psi")
    put('ω', "omega")
}

private fun keyName(ch: Char): String = LETTER_NAMES[ch] ?: ch.toString()

private const val SIGMA = 'σ'
private const val FINAL_SIGMA = 'ς'

/** Main key rows (lowercase forms; shift uppercases them). σ/ς is contextual. */
private val KEY_ROWS: List<List<Char>> = listOf(
    listOf('ε', 'ρ', 'τ', 'υ', 'θ', 'ι', 'ο', 'π'),
    listOf('α', 'σ', 'δ', 'φ', 'γ', 'η', 'ξ', 'κ', 'λ'),
    listOf('ζ', 'χ', 'ψ', 'ω', 'β', 'ν', 'μ')
)

/** Variant popup state: the anchor key's root-bounds + its variant glyphs. */
private data class VariantPopup(
    val anchor: Rect,
    val baseName: String,
    val variants: List<Pair<String, String>> // (glyph, contentDescription)
)

/**
 * The key grid. Supporting rows are: [letters x3] + [space / collapse].
 */
@Composable
fun GreekKeyboardPanel(
    modifier: Modifier = Modifier,
    onCollapse: () -> Unit = {}
) {
    val dispatcher = LocalGreekInputDispatcher.current
    val view = LocalView.current
    var shifted by rememberSaveable { mutableStateOf(false) }
    var popup by remember { mutableStateOf<VariantPopup?>(null) }

    fun haptic() {
        // VIRTUAL_KEY and no FLAG_IGNORE_GLOBAL_SETTING: haptics remain
        // ENABLED_IF_USER_SETTING (binding spec).
        view.performHapticFeedback(HapticFeedbackConstants.VIRTUAL_KEY)
    }

    fun commit(text: String) {
        dispatcher.active?.insertText(text, replaceComposing = false)
        if (shifted) shifted = false
    }

    fun commitSigma() {
        // σ/ς: emit σ mid-word and ς automatically when the next char is
        // none/space/punctuation — computed from the live field text. The
        // long-press popup still offers both forms explicitly.
        val v = dispatcher.active?.currentValue
        val s = if (v != null) {
            GreekTextOps.resolveSigma(v.text, v.selection.start, v.selection.end)
        } else {
            SIGMA.toString()
        }
        commit(s)
    }

    Surface(
        modifier = modifier.semantics { paneTitle = "Greek keyboard" },
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        tonalElevation = 2.dp
    ) {
        Box(modifier = Modifier.fillMaxSize().padding(horizontal = 6.dp, vertical = 4.dp)) {
            Column(modifier = Modifier.fillMaxSize()) {
                KEY_ROWS.forEachIndexed { rowIndex, row ->
                    Row(
                        modifier = Modifier.fillMaxWidth().weight(1f),
                        horizontalArrangement = Arrangement.spacedBy(4.dp),
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        if (rowIndex == 2) {
                            PanelKey(
                                label = "⇧",
                                name = "shift",
                                stateDescription = if (shifted) "on" else "off",
                                onHaptic = ::haptic,
                                onClick = { shifted = !shifted },
                                modifier = Modifier.weight(1.4f)
                            )
                        }
                        row.forEach { base ->
                            val display = if (shifted) base.uppercaseChar() else base
                            PanelKey(
                                label = display.toString(),
                                name = keyName(display),
                                greekFont = true,
                                onHaptic = ::haptic,
                                onClick = { if (base == SIGMA && !shifted) commitSigma() else commit(display.toString()) },
                                variants = when {
                                    shifted -> KeyboardVariants.variantsFor(display)
                                        .map { it to "${keyName(display)} variant $it" }
                                    base == SIGMA -> buildList {
                                        add(FINAL_SIGMA.toString() to keyName(FINAL_SIGMA))
                                        add(SIGMA.toString() to keyName(SIGMA))
                                    }
                                    else -> KeyboardVariants.variantsFor(display)
                                        .map { it to "${keyName(display)} variant $it" }
                                },
                                onOpenPopup = { anchor, variants ->
                                    popup = VariantPopup(anchor, keyName(display), variants)
                                },
                                modifier = Modifier.weight(1f)
                            )
                        }
                        if (rowIndex == 2) {
                            BackspaceKey(
                                onDelete = { dispatcher.active?.deleteBackward() },
                                onHaptic = ::haptic,
                                modifier = Modifier.weight(1.4f)
                            )
                        }
                    }
                }
                Row(
                    modifier = Modifier.fillMaxWidth().weight(1f),
                    horizontalArrangement = Arrangement.spacedBy(4.dp),
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    PanelKey(
                        label = "␣",
                        name = "space",
                        onHaptic = ::haptic,
                        onClick = { commit(" ") },
                        modifier = Modifier.weight(5f)
                    )
                    PanelKey(
                        label = "▾",
                        name = "hide Greek keyboard",
                        onHaptic = ::haptic,
                        onClick = onCollapse,
                        modifier = Modifier.weight(1.5f)
                    )
                }
            }

            // Single shared variant Popup (no per-key DropdownMenu): one
            // Popup serves every long-press, anchored above the pressed key.
            // Accessibility: Compose has no Role.Menu/Role.MenuItem constant;
            // the popup uses paneTitle=variants menu and its items are
            // Role.Button with per-glyph content descriptions.
            popup?.let { p ->
                val density = LocalDensity.current
                val popupHeightPx = with(density) { 56.dp.toPx() }.roundToInt()
                Popup(
                    alignment = Alignment.TopStart,
                    offset = IntOffset(
                        x = p.anchor.left.toInt().coerceAtLeast(0),
                        y = (p.anchor.top - popupHeightPx).toInt().coerceAtLeast(0)
                    ),
                    onDismissRequest = { popup = null }
                ) {
                    Surface(
                        shape = RoundedCornerShape(8.dp),
                        color = MaterialTheme.colorScheme.surfaceContainerHighest,
                        tonalElevation = 6.dp,
                        modifier = Modifier.semantics { paneTitle = "${p.baseName} variants" }
                    ) {
                        Row(modifier = Modifier.padding(4.dp)) {
                            p.variants.forEach { (glyph, description) ->
                                TextButton(
                                    onClick = {
                                        haptic()
                                        dispatcher.active?.insertText(
                                            glyph, replaceComposing = false
                                        )
                                        if (shifted) shifted = false
                                        popup = null
                                    },
                                    modifier = Modifier.semantics {
                                        role = Role.Button
                                        contentDescription = description
                                    }
                                ) {
                                    Text(
                                        text = glyph,
                                        style = MaterialTheme.typography.titleLarge,
                                        fontFamily = GreekFontFamily
                                    )
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/**
 * One key. [variants] non-null enables the shared popup via long-press
 * ([onOpenPopup] gets the key's root bounds as anchor); picking a variant
 * calls back through [onVariant].
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun PanelKey(
    label: String,
    name: String,
    onHaptic: () -> Unit,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    stateDescription: String? = null,
    greekFont: Boolean = false,
    variants: List<Pair<String, String>>? = null,
    onOpenPopup: ((Rect, List<Pair<String, String>>) -> Unit)? = null
) {
    var bounds by remember { mutableStateOf(Rect.Zero) }
    val interactionSource = remember { MutableInteractionSource() }
    Surface(
        color = MaterialTheme.colorScheme.surface,
        shape = RoundedCornerShape(6.dp),
        tonalElevation = 1.dp,
        modifier = modifier
    ) {
        Box(
            contentAlignment = Alignment.Center,
            modifier = Modifier
                .fillMaxSize()
                .semantics(mergeDescendants = true) {
                    role = Role.Button
                    contentDescription = name
                    if (stateDescription != null) this.stateDescription = stateDescription
                }
                .onGloballyPositioned { bounds = it.boundsInRoot() }
                .combinedClickable(
                    interactionSource = interactionSource,
                    indication = ripple(),
                    onClick = {
                        onHaptic()
                        onClick()
                    },
                    onLongClick = if (variants.isNullOrEmpty()) null else ({
                        onHaptic()
                        onOpenPopup?.invoke(bounds, variants)
                    })
                )
                .padding(vertical = 2.dp)
        ) {
            Text(
                text = label,
                style = MaterialTheme.typography.titleLarge,
                fontFamily = if (greekFont) GreekFontFamily else null
            )
        }
    }
}

/** Backspace with 500ms press-and-hold delay, 50ms repeat interval
 *  (binding spec); deletes whole grapheme clusters, so ᾄ is ONE delete. */
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun BackspaceKey(
    onDelete: () -> Unit,
    onHaptic: () -> Unit,
    modifier: Modifier = Modifier
) {
    var held by remember { mutableStateOf(false) }
    LaunchedEffect(held) {
        if (!held) return@LaunchedEffect
        delay(500)
        while (true) {
            onDelete()
            onHaptic()
            delay(50)
        }
    }
    val interactionSource = remember { MutableInteractionSource() }
    LaunchedEffect(Unit) {
        interactionSource.interactions.collect { interaction: Interaction ->
            when (interaction) {
                is PressInteraction.Press -> held = true
                is PressInteraction.Release, is PressInteraction.Cancel -> held = false
            }
        }
    }
    Surface(
        color = MaterialTheme.colorScheme.surface,
        shape = RoundedCornerShape(6.dp),
        tonalElevation = 1.dp,
        modifier = modifier
    ) {
        Box(
            contentAlignment = Alignment.Center,
            modifier = Modifier
                .fillMaxSize()
                .semantics(mergeDescendants = true) {
                    role = Role.Button
                    contentDescription = "delete backwards"
                }
                .combinedClickable(
                    interactionSource = interactionSource,
                    indication = ripple(),
                    onClick = {
                        onHaptic()
                        onDelete()
                    }
                )
                .padding(vertical = 2.dp)
        ) {
            Text(text = "⌫", style = MaterialTheme.typography.titleLarge)
        }
    }
}

/** Collapsed strip (re-expand handle): sits above the system-IME inset in
 *  "both" mode and never steals focus. */
@Composable
fun GreekKeyboardCollapsedStrip(onExpand: () -> Unit) {
    Surface(
        color = MaterialTheme.colorScheme.surfaceContainer,
        tonalElevation = 1.dp,
        modifier = Modifier.fillMaxWidth()
    ) {
        Box(modifier = Modifier.fillMaxWidth().height(36.dp)) {
            TextButton(
                onClick = onExpand,
                modifier = Modifier
                    .align(Alignment.Center)
                    .semantics {
                        role = Role.Button
                        contentDescription = "show Greek keyboard"
                    }
            ) {
                Text("▴ Greek")
            }
        }
    }
}

/**
 * Screen-level keyboard mount (Appendix R — "Embedding"): [content] carries
 * the screen; the panel is sibling screen-level content behind
 * `AnimatedVisibility`, padded via `WindowInsets.ime` so "both" mode
 * coexists with the system keyboard. Portrait: fixed 280dp; landscape:
 * capped at 40% of screen height; ≥600dp-wide screens get a side-docked
 * 340dp panel instead.
 *
 * Input mode (device-local, [GreekInputRepository]; NEVER synced per
 * Appendix R): `ime` mounts nothing; `in_app` (default) and `both` mount
 * the panel with a collapse/re-expand strip.
 */
@Composable
fun GreekKeyboardContainer(
    modifier: Modifier = Modifier,
    initiallyExpanded: Boolean = true,
    content: @Composable (Modifier) -> Unit
) {
    val context = LocalContext.current
    val mode by GreekInputRepository.modeFlow(context)
        .collectAsStateWithLifecycle(initialValue = GreekInputMode.DEFAULT)

    if (mode == GreekInputMode.IME) {
        content(modifier.fillMaxSize())
        return
    }

    var expanded by rememberSaveable { mutableStateOf(initiallyExpanded) }
    val configuration = LocalConfiguration.current
    val isWide = configuration.screenWidthDp >= 600
    val isLandscape = configuration.screenWidthDp > configuration.screenHeightDp

    val panelHeight = if (isLandscape) {
        (configuration.screenHeightDp * 0.4f).coerceAtMost(280f).dp
    } else {
        280.dp
    }

    if (isWide) {
        Row(modifier = modifier.fillMaxSize()) {
            Box(modifier = Modifier.weight(1f)) { content(Modifier.fillMaxSize()) }
            Column(
                modifier = Modifier
                    .width(340.dp)
                    .navigationBarsPadding()
                    .windowInsetsPadding(WindowInsets.ime)
            ) {
                if (expanded) {
                    GreekKeyboardPanel(
                        modifier = Modifier.fillMaxSize(),
                        onCollapse = { expanded = false }
                    )
                } else {
                    Box(modifier = Modifier.fillMaxSize()) {
                        TextButton(
                            onClick = { expanded = true },
                            modifier = Modifier
                                .align(Alignment.BottomCenter)
                                .semantics {
                                    role = Role.Button
                                    contentDescription = "show Greek keyboard"
                                }
                        ) { Text("▴") }
                    }
                }
            }
        }
    } else {
        Column(modifier = modifier.fillMaxSize()) {
            Box(modifier = Modifier.weight(1f)) { content(Modifier.fillMaxSize()) }
            AnimatedVisibility(visible = expanded) {
                GreekKeyboardPanel(
                    modifier = Modifier
                        .fillMaxWidth()
                        .height(panelHeight)
                        .windowInsetsPadding(WindowInsets.ime),
                    onCollapse = { expanded = false }
                )
            }
            if (!expanded) {
                Box(modifier = Modifier.windowInsetsPadding(WindowInsets.ime)) {
                    GreekKeyboardCollapsedStrip(onExpand = { expanded = true })
                }
            }
        }
    }
}
