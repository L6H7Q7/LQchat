package com.lanchat.app

internal object SharedApkFileName {
    fun fromLabel(label: String): String? {
        val safeLabel = label.replace(Regex("[\\x00-\\x1f\\x7f<>:\"/\\\\|?*]"), "_")
            .trim().trimEnd('.').take(120).trimEnd().trimEnd('.')
        if (safeLabel.isBlank()) return null
        val stem = safeLabel.substringBefore('.').uppercase(java.util.Locale.ROOT)
        val reserved = stem in setOf("CON", "PRN", "AUX", "NUL") ||
            stem.matches(Regex("(?:COM|LPT)[1-9]"))
        return (if (reserved) "_$safeLabel" else safeLabel) + ".apk"
    }
}
