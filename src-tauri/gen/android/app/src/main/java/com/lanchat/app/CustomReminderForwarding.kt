package com.lanchat.app

/** Shared reminder forwarding policy, separate from platform permission and session readiness. */
internal object CustomReminderForwarding {
    fun enabled(pushEnabled: Boolean, reminderEnabled: Boolean, targets: Set<String>): Boolean =
        pushEnabled && reminderEnabled && targets.any { it.isNotBlank() }

    fun title(deviceName: String, title: String): String {
        val source = deviceName.trim().ifBlank { "本机" }
        return remoteTitle(source, title)
    }

    fun remoteTitle(deviceName: String, title: String): String {
        val source = deviceName.trim()
        val clean = title.trim()
        return if (source.isEmpty() || clean == source || clean.startsWith("$source · ")) clean else "$source · $clean"
    }
}
