package com.lanchat.app

import android.content.Context

/** Persists the Rust-owned device name for Android components that can outlive the UI. */
internal object LocalDeviceIdentity {
    private const val PREFS = "lanchat_local_device_identity"
    private const val DEVICE_NAME = "device_name"
    private const val FALLBACK_NAME = "本机"

    fun save(context: Context, name: String): String {
        val normalized = normalize(name)
        context.applicationContext
            .getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .edit()
            .putString(DEVICE_NAME, normalized)
            .commit()
        return normalized
    }

    fun read(context: Context): String {
        val saved = context.applicationContext
            .getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .getString(DEVICE_NAME, null)
        return normalize(saved.orEmpty())
    }

    internal fun batteryNotificationTitle(name: String): String = "${normalize(name)} · 电量"

    internal fun reminderNotificationTitle(name: String, title: String): String =
        CustomReminderForwarding.title(normalize(name), title)

    private fun normalize(name: String): String = name.trim().ifBlank { FALLBACK_NAME }
}
