package com.greekapp.reminder

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import com.greekapp.MainActivity

/**
 * Fires the daily "cards are ready" notification when [ReminderScheduler]'s
 * alarm goes off.
 *
 * Framework-only on purpose: minSdk is 26, so NotificationChannel and
 * `Notification.Builder(context, channelId)` are available without
 * NotificationCompat, and AlarmManager replaces WorkManager — no new
 * dependencies (no androidx.work / appcompat / core-ktx additions).
 */
class ReminderReceiver : BroadcastReceiver() {

    override fun onReceive(context: Context, intent: Intent) {
        ensureChannel(context)

        // Tapping the notification opens the app at its default entry point.
        val openApp = PendingIntent.getActivity(
            context,
            0,
            Intent(context, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE
        )

        val notification = Notification.Builder(context, CHANNEL_ID)
            // Stock framework icon — replace with a dedicated monochrome
            // status-bar icon when one exists; there is no notification icon
            // resource in res/ today.
            .setSmallIcon(android.R.drawable.ic_dialog_info)
            .setContentTitle("Greek review")
            .setContentText("Your cards are ready")
            .setContentIntent(openApp)
            .setAutoCancel(true)
            .build()

        // Note: on Android 13+ (API 33+) the POST_NOTIFICATIONS runtime
        // permission gates whether this is actually shown; the manifest
        // declares it and requesting it is the Settings UI's concern.
        val manager =
            context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        manager.notify(NOTIFICATION_ID, notification)

        // setAndAllowWhileIdle is one-shot, so re-book for tomorrow — or drop
        // the alarm entirely if the user has since toggled reminders off
        // (syncFromPreferences reads the persisted setting).
        ReminderScheduler.syncFromPreferences(context)
    }

    companion object {
        const val CHANNEL_ID = "daily_review"
        const val NOTIFICATION_ID = 1001

        /** Create the notification channel (no-op if it already exists). */
        fun ensureChannel(context: Context) {
            val channel = NotificationChannel(
                CHANNEL_ID,
                "Daily review",
                NotificationManager.IMPORTANCE_DEFAULT
            )
            val manager =
                context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
            manager.createNotificationChannel(channel)
        }
    }
}
