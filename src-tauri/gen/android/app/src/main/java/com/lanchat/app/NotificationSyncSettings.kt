package com.lanchat.app

import android.app.NotificationManager
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.os.Build
import android.provider.Settings
import androidx.core.app.NotificationManagerCompat
import org.json.JSONArray
import org.json.JSONObject

/** The single persistent source, readable even when the native library is absent. */
object NotificationSyncSettings {
    fun isListenerEnabled(context: Context): Boolean {
        val enabled = Settings.Secure.getString(context.contentResolver, "enabled_notification_listeners") ?: return false
        return enabled.split(":").any { it == ComponentName(context, LanChatNotificationListener::class.java).flattenToString() }
    }
    private fun prefs(context: Context) = context.getSharedPreferences("notification_sync", Context.MODE_PRIVATE)
    fun read(context: Context): JSONObject {
        val p = prefs(context)
        return JSONObject().put("push_enabled", p.getBoolean("push_enabled", false))
            .put("receive_enabled", p.getBoolean("receive_enabled", false))
            .put("lq_battery_push_enabled", p.getBoolean("lq_battery_push_enabled", false))
            .put("lq_reminder_push_enabled", p.getBoolean("lq_reminder_push_enabled", false))
            .put("allowed_packages", JSONArray(p.getStringSet("allowed_packages", emptySet())!!.sorted()))
            .put("target_device_ids", JSONArray(p.getStringSet("target_device_ids", emptySet())!!.sorted()))
    }
    fun strings(array: JSONArray?): Set<String> = (0 until (array?.length() ?: 0))
        .mapNotNull { array?.optString(it)?.takeIf { value -> value.isNotBlank() } }.toSet()

    private fun selectableApps(context: Context) = context.packageManager.getInstalledApplications(0)
        .asSequence()
        .filter { it.enabled && it.packageName != context.packageName }
        .sortedWith(compareBy({ context.packageManager.getApplicationLabel(it).toString().lowercase() }, { it.packageName }))

    /** One editor commit replaces the complete snapshot. A failed commit restores the prior value. */
    @Synchronized private fun replaceAllowed(context: Context, requested: Set<String>) {
        val eligible = selectableApps(context).map { it.packageName }.toSet()
        require(requested.all { it in eligible }) { "允许列表包含未安装或不可选择的应用" }
        val store = prefs(context)
        val previous = store.getStringSet("allowed_packages", emptySet())!!.toSet()
        if (!store.edit().putStringSet("allowed_packages", requested).commit()) {
            store.edit().putStringSet("allowed_packages", previous).commit()
            error("允许列表保存失败，已保留旧值")
        }
    }

    fun command(context: Context, input: String): String { return try {
        val request = JSONObject(input)
        when (request.optString("action")) {
            "test_icon" -> return JSONObject().put("test_icon", NotificationAppIcon.encode(context, context.packageName)).toString()
            "save" -> {
                val s = request.getJSONObject("settings")
                val store = prefs(context)
                val previous = read(context)
                val saved = store.edit().putBoolean("push_enabled", s.optBoolean("push_enabled"))
                    .putBoolean("receive_enabled", s.optBoolean("receive_enabled"))
                    .putBoolean("lq_battery_push_enabled", s.optBoolean("lq_battery_push_enabled"))
                    .putBoolean("lq_reminder_push_enabled", s.optBoolean("lq_reminder_push_enabled"))
                    .putStringSet("allowed_packages", strings(s.optJSONArray("allowed_packages")) - context.packageName)
                    .putStringSet("target_device_ids", strings(s.optJSONArray("target_device_ids"))).commit()
                if (!saved) {
                    store.edit().putBoolean("push_enabled", previous.optBoolean("push_enabled"))
                        .putBoolean("receive_enabled", previous.optBoolean("receive_enabled"))
                        .putBoolean("lq_battery_push_enabled", previous.optBoolean("lq_battery_push_enabled"))
                        .putBoolean("lq_reminder_push_enabled", previous.optBoolean("lq_reminder_push_enabled"))
                        .putStringSet("allowed_packages", strings(previous.optJSONArray("allowed_packages")))
                        .putStringSet("target_device_ids", strings(previous.optJSONArray("target_device_ids"))).commit()
                    error("设置保存失败，已保留旧值")
                }
                LanChatForegroundService.refreshNotificationRouteStatus()
            }
            "apps" -> {
                val selected = read(context).getJSONArray("allowed_packages")
                val apps = selectableApps(context).map {
                    JSONObject().put("package", it.packageName)
                        .put("name", context.packageManager.getApplicationLabel(it).toString())
                        .put("is_system", (it.flags and android.content.pm.ApplicationInfo.FLAG_SYSTEM) != 0)
                        .put("icon", NotificationAppIcon.encode(context, it.packageName))
                        .put("selected", it.packageName in strings(selected))
                }.toList()
                return JSONObject().put("apps", JSONArray(apps))
                    .put("installed_snapshot", JSONArray(apps.map { it.getString("package") })).toString()
            }
            "replace_allowed" -> {
                replaceAllowed(context, strings(request.optJSONObject("payload")?.optJSONArray("packages")))
            }
            "access" -> context.startActivity(Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            "permission" -> {
                val intent = if (Build.VERSION.SDK_INT >= 26) Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS)
                    .putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName)
                else Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, android.net.Uri.parse("package:${context.packageName}"))
                context.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            }
        }
        val manager = context.getSystemService(NotificationManager::class.java)
        val allowed = NotificationManagerCompat.from(context).areNotificationsEnabled() &&
            (Build.VERSION.SDK_INT < 26 || manager.getNotificationChannel(SyncedNotificationPublisher.CHANNEL)?.importance != NotificationManager.IMPORTANCE_NONE)
        JSONObject().put("settings", read(context)).put("platform", "android")
            .put("access", NotificationManagerCompat.getEnabledListenerPackages(context).contains(context.packageName))
            .put("permission", if (allowed) "granted" else "blocked").toString()
    } catch (error: Exception) { JSONObject().put("error", error.message ?: "设置操作失败").toString() }
    }
}
