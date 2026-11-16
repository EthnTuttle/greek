package com.greekapp.reminder

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ReminderPolicyTest {
    @Test
    fun beforeApi33_runtimePermissionIsNotRequired() {
        assertTrue(
            canPostReminders(
                sdkInt = 32,
                hasPostNotificationsPermission = false,
                notificationsEnabled = true
            )
        )
    }

    @Test
    fun api33WithPermissionAndNotificationsEnabled_canPost() {
        assertTrue(
            canPostReminders(
                sdkInt = 33,
                hasPostNotificationsPermission = true,
                notificationsEnabled = true
            )
        )
    }

    @Test
    fun api33WithoutPermission_cannotPost() {
        assertFalse(
            canPostReminders(
                sdkInt = 33,
                hasPostNotificationsPermission = false,
                notificationsEnabled = true
            )
        )
    }

    @Test
    fun notificationsBlocked_cannotPostOnAnyApiLevel() {
        assertFalse(
            canPostReminders(
                sdkInt = 32,
                hasPostNotificationsPermission = false,
                notificationsEnabled = false
            )
        )
        assertFalse(
            canPostReminders(
                sdkInt = 35,
                hasPostNotificationsPermission = true,
                notificationsEnabled = false
            )
        )
    }
}
