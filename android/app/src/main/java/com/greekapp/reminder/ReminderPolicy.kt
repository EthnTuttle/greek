package com.greekapp.reminder

/**
 * Host-JVM-testable notification gate. Below API 33 there is no runtime
 * notification permission; user-level notification blocking applies on all APIs.
 */
internal fun canPostReminders(
    sdkInt: Int,
    hasPostNotificationsPermission: Boolean,
    notificationsEnabled: Boolean
): Boolean = notificationsEnabled &&
    (sdkInt < 33 || hasPostNotificationsPermission)
