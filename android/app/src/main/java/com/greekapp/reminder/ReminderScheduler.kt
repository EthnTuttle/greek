package com.greekapp.reminder

import android.app.AlarmManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import com.greekapp.SettingsKeys
import com.greekapp.settingsDataStore
import java.util.Calendar
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

/**
 * Schedules/cancels the daily 19:00 (local) review reminder with plain
 * [AlarmManager].
 *
 * WorkManager is deliberately NOT used: the zero-dependency constraint for
 * this feature forbids androidx.work, and an exact alarm would need the
 * SCHEDULE_EXACT_ALARM permission. The trade-off is an INEXACT daily cadence —
 * [AlarmManager.setAndAllowWhileIdle] lets the OS batch the wakeup (at most
 * about once per hour per app while idle) — which is fine for a "your cards
 * are ready" nudge where the exact minute doesn't matter. The alarm is
 * re-booked for the following day each time [ReminderReceiver] fires.
 */
object ReminderScheduler {

    private const val REQUEST_CODE = 0
    private const val REMINDER_HOUR = 19

    /**
     * Enable → book the next 19:00 local-time alarm. Disable → cancel any
     * pending alarm. Safe to call repeatedly: the matching PendingIntent is
     * replaced via FLAG_UPDATE_CURRENT, so re-enabling never stacks alarms.
     */
    fun setEnabled(context: Context, enabled: Boolean) {
        val alarmManager = context.getSystemService(Context.ALARM_SERVICE) as AlarmManager
        val pendingIntent = reminderIntent(context)
        if (enabled) {
            alarmManager.setAndAllowWhileIdle(
                AlarmManager.RTC_WAKEUP,
                nextReminderTimeMillis(),
                pendingIntent
            )
        } else {
            alarmManager.cancel(pendingIntent)
            pendingIntent.cancel()
        }
    }

    /**
     * Re-arm (or cancel) the alarm from the persisted setting. Reads the SAME
     * DataStore as the Settings screen — preferences file "settings"
     * ([Context.settingsDataStore] in GreekApplication.kt), key
     * [SettingsKeys.DAILY_REMINDER] ("daily_reminder", Boolean, default true,
     * matching SettingsRepository.dailyReminderFlow).
     *
     * Non-suspending so Application.onCreate and BroadcastReceiver.onReceive
     * can call it with a single line; the DataStore read hops to
     * Dispatchers.IO.
     */
    fun syncFromPreferences(context: Context) {
        val appContext = context.applicationContext
        CoroutineScope(Dispatchers.IO).launch {
            val enabled =
                appContext.settingsDataStore.data.first()[SettingsKeys.DAILY_REMINDER] ?: true
            setEnabled(appContext, enabled)
        }
    }

    /** Next 19:00 local time — today if still ahead, otherwise tomorrow. */
    private fun nextReminderTimeMillis(): Long {
        val next = Calendar.getInstance().apply {
            set(Calendar.HOUR_OF_DAY, REMINDER_HOUR)
            set(Calendar.MINUTE, 0)
            set(Calendar.SECOND, 0)
            set(Calendar.MILLISECOND, 0)
        }
        if (next.timeInMillis <= System.currentTimeMillis()) {
            next.add(Calendar.DAY_OF_YEAR, 1)
        }
        return next.timeInMillis
    }

    private fun reminderIntent(context: Context): PendingIntent =
        PendingIntent.getBroadcast(
            context,
            REQUEST_CODE,
            Intent(context, ReminderReceiver::class.java),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )
}
