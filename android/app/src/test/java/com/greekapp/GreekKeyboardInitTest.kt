package com.greekapp.components

import org.junit.Test

/**
 * Regression guard for production audit A1: `GreekKeyboardKt`'s top-level
 * property initializers must not overflow the stack. Before the fix, a local
 * `fun put` shadowed `MutableMap.put` and called itself, so the FIRST touch
 * of the file-level class (any screen mounting the keyboard) threw
 * `StackOverflowError`.
 *
 * This lives in unit tests (not androidTest) because class initialization
 * happens on any JVM; Robolectric/Compose aren't needed to reproduce it.
 */
class GreekKeyboardInitTest {

    @Test
    fun topLevelClass_initializesWithoutStackOverflow() {
        // Forces initialization of the file-level declarations
        // (LETTER_NAMES), which is exactly what crashed at runtime.
        Class.forName("com.greekapp.ui.components.GreekKeyboardKt")
    }
}
