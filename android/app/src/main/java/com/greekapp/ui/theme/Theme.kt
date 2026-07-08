package com.greekapp.ui.theme

import android.app.Activity
import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.SideEffect
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.core.view.WindowCompat

private val DarkColorScheme = darkColorScheme(
    primary = AmberDark,
    onPrimary = DarkBackground,
    primaryContainer = DarkSurfaceVariant,
    onPrimaryContainer = AmberAccent,
    secondary = OliveLight,
    onSecondary = DarkBackground,
    secondaryContainer = DarkSurfaceVariant,
    onSecondaryContainer = OliveLight,
    tertiary = AmberAccent,
    onTertiary = DarkBackground,
    background = DarkBackground,
    onBackground = ParchmentLight,
    surface = DarkSurface,
    onSurface = ParchmentLight,
    surfaceVariant = DarkSurfaceVariant,
    onSurfaceVariant = ParchmentDark,
    outline = AmberDark,
)

private val LightColorScheme = lightColorScheme(
    primary = DeepBrown,
    onPrimary = ParchmentLight,
    primaryContainer = ParchmentDark,
    onPrimaryContainer = DeepBrown,
    secondary = OliveDark,
    onSecondary = ParchmentLight,
    secondaryContainer = ParchmentDark,
    onSecondaryContainer = OliveDark,
    tertiary = AmberAccent,
    onTertiary = DeepBrown,
    background = ParchmentLight,
    onBackground = DeepBrown,
    surface = ParchmentLight,
    onSurface = DeepBrown,
    surfaceVariant = ParchmentDark,
    onSurfaceVariant = DeepBrown,
    outline = AmberAccent,
)

@Composable
fun GreekTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    dynamicColor: Boolean = false, // Dynamic color disabled to maintain parchment aesthetic
    content: @Composable () -> Unit
) {
    val colorScheme = when {
        dynamicColor && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S -> {
            val context = LocalContext.current
            if (darkTheme) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
        }
        darkTheme -> DarkColorScheme
        else -> LightColorScheme
    }

    val view = LocalView.current
    if (!view.isInEditMode) {
        SideEffect {
            val window = (view.context as Activity).window
            window.statusBarColor = colorScheme.background.toArgb()
            WindowCompat.getInsetsController(window, view).isAppearanceLightStatusBars = !darkTheme
        }
    }

    MaterialTheme(
        colorScheme = colorScheme,
        typography = Typography,
        content = content
    )
}
