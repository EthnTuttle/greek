package com.greekapp.reminder

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import com.greekapp.MainActivity

/**
 * Fires the daily "cards are ready" notification when [ReminderScheduler]'s
 * alarm goes off.
 *
 * Notification construction is framework-only: minSdk is 26, so
 * NotificationChannel and `Notification.Builder(context, channelId)` are
 * available without NotificationCompat, and AlarmManager replaces WorkManager
 * — no new dependencies (no androidx.work / appcompat additions).
 */
class ReminderReceiver : BroadcastReceiver() {

    override fun onReceive(context: Context, intent: Intent) {
        val hasPermission = Build.VERSION.SDK_INT < 33 ||
            ContextCompat.checkSelfPermission(
                context,
                android.Manifest.permission.POST_NOTIFICATIONS
            ) == PackageManager.PERMISSION_GRANTED
        if (!canPostReminders(
                sdkInt = Build.VERSION.SDK_INT,
                hasPostNotificationsPermission = hasPermission,
                notificationsEnabled = NotificationManagerCompat.from(context)
                    .areNotificationsEnabled()
            )
        ) {
            return
        }

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
