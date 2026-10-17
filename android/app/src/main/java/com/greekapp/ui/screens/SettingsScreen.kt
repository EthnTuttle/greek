package com.greekapp.ui.screens

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.greekapp.SettingsRepository
import com.greekapp.reminder.ReminderScheduler
import kotlinx.coroutines.launch

@Composable
fun SettingsScreen() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()

    val notificationsEnabled by SettingsRepository.dailyReminderFlow(context)
        .collectAsStateWithLifecycle(initialValue = true)
    // Dark mode: null == follow system; the switch reflects the effective state.
    val darkModePref by SettingsRepository.darkModeFlow(context)
        .collectAsStateWithLifecycle(initialValue = null)
    val darkModeEnabled = darkModePref ?: isSystemInDarkTheme()

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(16.dp)
    ) {
        Text(
            text = "Settings",
            style = MaterialTheme.typography.headlineMedium,
            fontWeight = FontWeight.Bold
        )
        Spacer(modifier = Modifier.height(16.dp))

        Card(
            modifier = Modifier.fillMaxWidth(),
            colors = CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.surfaceVariant
            )
        ) {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(16.dp)
            ) {
                // Daily reminder
                SettingItem(
                    title = "Daily reminder",
                    description = "Get notified when you have cards due",
                    checked = notificationsEnabled,
                    onCheckedChange = {
                        scope.launch { SettingsRepository.setDailyReminder(context, it) }
                        ReminderScheduler.setEnabled(context, it)
                    }
                )

                Divider()

                // Dark mode
                SettingItem(
                    title = "Dark mode",
                    description = "Use dark theme for the app",
                    checked = darkModeEnabled,
                    onCheckedChange = { scope.launch { SettingsRepository.setDarkMode(context, it) } }
                )
            }
        }

        Spacer(modifier = Modifier.height(24.dp))

        // About section
        Card(
            modifier = Modifier.fillMaxWidth(),
            colors = CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.surfaceVariant
            )
        ) {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                Text(
                    text = "About",
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.Bold
                )
                Text(
                    text = "Greek Learning App",
                    style = MaterialTheme.typography.bodyLarge
                )
                Text(
                    text = "Version 1.0.0",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
        }
    }
}

@Composable
private fun SettingItem(
    title: String,
    description: String,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically
    ) {
        Column(
            modifier = Modifier.weight(1f)
        ) {
            Text(
                text = title,
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.Medium
            )
            Text(
                text = description,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
        Switch(
            checked = checked,
            onCheckedChange = onCheckedChange
        )
    }
}
