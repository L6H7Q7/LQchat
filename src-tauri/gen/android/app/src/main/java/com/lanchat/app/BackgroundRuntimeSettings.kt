package com.lanchat.app

import android.content.Context
import org.json.JSONArray
import org.json.JSONObject

/** Persistent background policy and the current boot/session stop boundary. */
object BackgroundRuntimeSettings {
    private const val PREFS = "background_runtime"
    private const val KEEP_RUNNING = "keep_running"
    private const val START_ON_BOOT = "start_on_boot"
    private const val EXCLUDE_FROM_RECENTS = "exclude_from_recents"
    private const val BATTERY_ALERT_ENABLED = "battery_alert_enabled"
    private const val BATTERY_ALERT_INTERVAL_SECONDS = "battery_alert_interval_seconds"
    private const val BATTERY_ALERT_REPEAT_COUNT = "battery_alert_repeat_count"
    private const val BATTERY_ALERT_LEVELS = "battery_alert_levels"
    private const val USER_STOPPED = "user_stopped"
    private const val BOOT_ID = "boot_id"

    private fun prefs(context: Context) = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    // Migrate once to the shared configuration. Existing custom reminders take precedence.
    @Synchronized private fun initializeReminderSettings(context: Context) {
        val p = prefs(context)
        if (p.getBoolean("shared_reminder_settings", false)) return
        val legacy = runCatching {
            JSONObject(context.getSharedPreferences("lanchat_custom_reminders_v1", Context.MODE_PRIVATE)
                .getString("settings", null) ?: "{}").optJSONObject("globalSettings")
        }.getOrNull()
        val count = (legacy?.optInt("repeat") ?: p.getInt(BATTERY_ALERT_REPEAT_COUNT, 1)).coerceIn(1, 20)
        val gap = if (count == 1) 0 else
            (legacy?.optInt("repeatGap") ?: p.getInt(BATTERY_ALERT_INTERVAL_SECONDS, 5)).coerceIn(1, 3600)
        check(p.edit().putInt(BATTERY_ALERT_REPEAT_COUNT, count)
            .putInt(BATTERY_ALERT_INTERVAL_SECONDS, gap).putBoolean("shared_reminder_settings", true)
            .commit()) { "统一提醒设置初始化失败" }
    }

    @Synchronized fun reminderOptions(context: Context): ReminderOptions {
        initializeReminderSettings(context)
        val p = prefs(context)
        return ReminderOptions(p.getInt(BATTERY_ALERT_REPEAT_COUNT, 1), p.getInt(BATTERY_ALERT_INTERVAL_SECONDS, 0))
    }

    @Synchronized fun read(context: Context): JSONObject {
        initializeReminderSettings(context)
        val p = prefs(context)
        return JSONObject()
            .put("keep_running", p.getBoolean(KEEP_RUNNING, false))
            .put("start_on_boot", p.getBoolean(START_ON_BOOT, false))
            .put("exclude_from_recents", p.getBoolean(EXCLUDE_FROM_RECENTS, false))
            .put("battery_alert_enabled", p.getBoolean(BATTERY_ALERT_ENABLED, false))
            .put("battery_alert_interval_seconds", p.getInt(BATTERY_ALERT_INTERVAL_SECONDS, 0))
            .put("battery_alert_repeat_count", p.getInt(BATTERY_ALERT_REPEAT_COUNT, 1))
            .put("battery_alert_levels", JSONArray(batteryAlertLevels(context).toList()))
            .put("user_stopped", p.getBoolean(USER_STOPPED, false))
            .put("boot_id", p.getLong(BOOT_ID, 0L))
    }

    @Synchronized fun save(context: Context, input: JSONObject): JSONObject {
        initializeReminderSettings(context)
        val p = prefs(context)
        val keepRunning = input.optBoolean("keep_running", p.getBoolean(KEEP_RUNNING, false))
        val startOnBoot = input.optBoolean("start_on_boot", p.getBoolean(START_ON_BOOT, false))
        val excludeFromRecents = input.optBoolean(
            "exclude_from_recents",
            p.getBoolean(EXCLUDE_FROM_RECENTS, false),
        )
        val batteryAlertEnabled = input.optBoolean(
            "battery_alert_enabled",
            p.getBoolean(BATTERY_ALERT_ENABLED, false),
        )
        val requestedInterval = input.optInt(
            "battery_alert_interval_seconds",
            p.getInt(BATTERY_ALERT_INTERVAL_SECONDS, 0),
        )
        val repeatCount = input.optInt(
            "battery_alert_repeat_count",
            p.getInt(BATTERY_ALERT_REPEAT_COUNT, 1),
        )
        require(repeatCount in 1..20) { "提醒次数必须是 1～20 次" }
        val intervalSeconds = if (repeatCount == 1) 0 else requestedInterval
        require(intervalSeconds == 0 && repeatCount == 1 || intervalSeconds in 1..3600) {
            "多次提醒时，请设置 1～3600 秒的间隔"
        }
        val existingLevels = batteryAlertLevels(context)
        val levelsArray = input.optJSONArray("battery_alert_levels")
        val levels = if (levelsArray == null) existingLevels else buildList {
            for (index in 0 until levelsArray.length()) {
                val level = levelsArray.optInt(index, -1)
                require(level in 1..100) { "提醒电量必须是 1～100 的整数" }
                add(level)
            }
        }.distinct().sorted()
        require(levels.isNotEmpty() && levels.size <= 10) { "请设置 1～10 个提醒电量" }
        check(p.edit()
            .putBoolean(KEEP_RUNNING, keepRunning)
            .putBoolean(START_ON_BOOT, startOnBoot)
            .putBoolean(EXCLUDE_FROM_RECENTS, excludeFromRecents)
            .putBoolean(BATTERY_ALERT_ENABLED, batteryAlertEnabled)
            .putInt(BATTERY_ALERT_INTERVAL_SECONDS, intervalSeconds)
            .putInt(BATTERY_ALERT_REPEAT_COUNT, repeatCount)
            .putString(BATTERY_ALERT_LEVELS, levels.joinToString(","))
            .commit()) { "后台设置保存失败" }
        return read(context)
    }

    @Synchronized fun beginUserSession(context: Context) {
        prefs(context).edit().putBoolean(USER_STOPPED, false).commit()
    }

    @Synchronized fun stopCurrentSession(context: Context) {
        prefs(context).edit().putBoolean(USER_STOPPED, true).commit()
    }

    @Synchronized fun beginBootSession(context: Context, bootId: Long) {
        prefs(context).edit().putLong(BOOT_ID, bootId).putBoolean(USER_STOPPED, false).commit()
    }

    fun mayRecover(context: Context): Boolean {
        val p = prefs(context)
        return p.getBoolean(KEEP_RUNNING, false) && !p.getBoolean(USER_STOPPED, false)
    }

    fun mayStartOnBoot(context: Context): Boolean =
        prefs(context).getBoolean(START_ON_BOOT, false)

    fun batteryAlertEnabled(context: Context): Boolean =
        prefs(context).getBoolean(BATTERY_ALERT_ENABLED, false)

    fun batteryAlertIntervalMs(context: Context): Long =
        reminderOptions(context).repeatGap * 1_000L

    fun batteryAlertRepeatCount(context: Context): Int =
        reminderOptions(context).repeat

    fun batteryAlertLevels(context: Context): Set<Int> =
        prefs(context).getString(BATTERY_ALERT_LEVELS, null)
            ?.split(',')
            ?.mapNotNull(String::toIntOrNull)
            ?.filter { it in 1..100 }
            ?.distinct()
            ?.sorted()
            ?.takeIf { it.isNotEmpty() }
            ?.toSet()
            ?: linkedSetOf(50, 100)
}
