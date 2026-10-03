package com.lanchat.app

import android.app.AlarmManager
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.media.AudioAttributes
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import org.json.JSONArray
import org.json.JSONObject
import java.util.UUID

/** One durable alarm, with its exact batch persisted before registration. No WebView/service dependency. */
object CustomReminderController {
    private const val STORE = "lanchat_custom_reminders_v1"
    private const val ALARM_ID = 48217
    private const val MAX_LATENESS = 5 * 60_000L
    private fun prefs(c: Context) = c.getSharedPreferences(STORE, Context.MODE_PRIVATE)
    private fun alarm(c: Context) = c.getSystemService(AlarmManager::class.java)
    private fun pending(c: Context, token: String = "") = PendingIntent.getBroadcast(c, ALARM_ID,
        Intent(c, CustomReminderReceiver::class.java).putExtra("token", token),
        PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
    private fun stored(c: Context): JSONObject {
        val shared = BackgroundRuntimeSettings.reminderOptions(c)
        return JSONObject(prefs(c).getString("settings", null) ?: "{\"reminders\":[]}")
            .put("globalSettings", JSONObject().put("repeat", shared.repeat).put("repeatGap", shared.repeatGap))
    }
    private fun ints(a: JSONArray?) = (0 until (a?.length() ?: 0)).map { a!!.getInt(it) }.toSet()
    private fun model(o: JSONObject) = CustomReminder(o.getString("id"), o.getString("title"), o.getString("content"),
        o.getString("mode"), o.optString("date"), o.getString("time"), o.optInt("weekday"), o.optInt("monthday", 1),
        ints(o.optJSONArray("weekdays")), ints(o.optJSONArray("monthdays")), o.optBoolean("countdown"),
        o.optInt("dayGap", 1), o.optString("dayTime", "09:00"), o.optBoolean("near"), o.optInt("nearStart", 60), o.optInt("nearGap", 10))
    private fun models(s: JSONObject): List<CustomReminder> = s.getJSONArray("reminders").let { a ->
        (0 until a.length()).map { model(a.getJSONObject(it)) }
    }
    private fun options(s: JSONObject): ReminderOptions = s.getJSONObject("globalSettings").let {
        ReminderOptions(it.getInt("repeat"), it.getInt("repeatGap"))
    }
    private fun validate(s: JSONObject) {
        val all = models(s); val opt = options(s)
        require(all.size <= 200 && all.map { it.id }.distinct().size == all.size) { "最多保存 200 条通知，标识不能重复" }
        require(opt.repeat in 1..20 && (opt.repeat == 1 && opt.repeatGap == 0 || opt.repeatGap in 1..3600)) { "通知次数或间隔无效" }
        all.forEach { r ->
            require(r.id.matches(Regex("[A-Za-z0-9_-]{1,80}")) && r.title.isNotBlank() && r.title.length <= 60 &&
                r.content.isNotBlank() && r.content.length <= 300) { "通知标题、内容或标识无效" }
            require(r.mode in setOf("daily", "weekly", "monthly", "once", "weekdays", "monthdays") &&
                CustomReminderSchedule.validTime(r.time)) { "通知日期模式或时间无效" }
            require((r.mode != "weekly" || r.weekday in 0..6) && (r.mode != "monthly" || r.monthday in 1..31) && r.weekdays.all { it in 0..6 } &&
                r.monthdays.all { it in 1..31 }) { "提醒日期无效" }
            require(r.mode != "once" || CustomReminderSchedule.parseDate(r.date) != null) { "事件日期无效" }
            require(r.mode != "weekdays" || r.weekdays.isNotEmpty()) { "至少选择一个星期" }
            require(r.mode != "monthdays" || r.monthdays.isNotEmpty()) { "至少选择一个日号" }
            require(!r.countdown || (r.dayGap in 1..365 && CustomReminderSchedule.validTime(r.dayTime) &&
                (!r.near || (r.nearStart in 1..10080 && r.nearGap in 1..1440 && r.nearGap <= r.nearStart)))) { "倒计时配置无效" }
        }
    }
    private fun encode(p: ReminderPush) = JSONObject().put("id", p.reminder.id).put("key", p.key)
        .put("title", p.reminder.title).put("content", p.text()).put("at", p.at).put("eventAt", p.eventAt)
        .put("kind", p.kind).put("index", p.index)
    private fun next(c: Context, at: Long = System.currentTimeMillis()): List<ReminderPush> {
        if (!prefs(c).getBoolean("enabled", true)) return emptyList()
        val s = stored(c)
        return CustomReminderSchedule.next(models(s), options(s), at, prefs(c).getStringSet("delivered", emptySet())!!.toSet())
    }
    fun exactAllowed(c: Context) = Build.VERSION.SDK_INT < 31 || alarm(c).canScheduleExactAlarms()
    @Synchronized fun read(c: Context): JSONObject = stored(c).put("next", JSONArray(next(c).map(::encode)))
        .put("exactAllowed", exactAllowed(c)).put("notificationAllowed", NotificationManagerCompat.from(c).areNotificationsEnabled())
        .put("enabled", prefs(c).getBoolean("enabled", true)).put("scheduleError", prefs(c).getString("schedule_error", ""))

    @Synchronized fun command(c: Context, input: String): String = try {
        val request = JSONObject(input)
        when (request.getString("action")) {
            "save" -> {
                val settings = request.getJSONObject("settings")
                // Retain only data fields, never trust a client-supplied next alarm or permission status.
                val clean = JSONObject().put("reminders", settings.getJSONArray("reminders"))
                    .put("globalSettings", stored(c).getJSONObject("globalSettings"))
                validate(clean)
                val old = stored(c)
                if (!prefs(c).edit().putString("settings", clean.toString()).commit()) {
                    prefs(c).edit().putString("settings", old.toString()).commit()
                    error("保存失败，已保留原配置")
                }
                val keep = models(clean).filter { r -> models(old).any { it == r } }.map { it.id }.toSet()
                val delivered = prefs(c).getStringSet("delivered", emptySet())!!.filter { it.substringBefore(':') in keep }.toSet()
                prefs(c).edit().putStringSet("delivered", delivered).commit()
                // Cancel deleted/edited reminder notifications, including pending repeat alerts.
                models(old).filter { r -> r.id !in keep }.forEach {
                    c.getSystemService(NotificationManager::class.java).cancel("custom-reminder:${it.id}", ALARM_ID + 1)
                }
                refresh(c)
            }
            "options" -> {
                val p = prefs(c); val enabled = request.getBoolean("enabled"); val sound = request.getBoolean("sound")
                if (enabled != p.getBoolean("enabled", true) || sound != p.getBoolean("sound", true)) {
                    val oldEnabled = p.getBoolean("enabled", true); val oldSound = p.getBoolean("sound", true)
                    if (!p.edit().putBoolean("enabled", enabled).putBoolean("sound", sound).commit()) {
                        p.edit().putBoolean("enabled", oldEnabled).putBoolean("sound", oldSound).commit()
                        error("保存通知开关失败")
                    }
                    refresh(c)
                }
            }
            "exact_settings" -> if (Build.VERSION.SDK_INT >= 31) c.startActivity(
                Intent(Settings.ACTION_REQUEST_SCHEDULE_EXACT_ALARM, Uri.parse("package:${c.packageName}")).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            "get" -> Unit
            else -> error("未知自定义通知操作")
        }
        read(c).toString()
    } catch (e: Exception) { JSONObject().put("error", e.message ?: "操作失败").toString() }

    @Synchronized fun refresh(c: Context) {
        val manager = alarm(c); manager.cancel(pending(c))
        val store = prefs(c)
        store.edit().remove("pending").remove("token").remove("schedule_error").commit()
        runCatching {
            val batch = next(c)
            if (batch.isEmpty()) return
            val token = UUID.randomUUID().toString()
            check(store.edit().putString("pending", JSONArray(batch.map(::encode)).toString()).putString("token", token).commit())
            val intent = pending(c, token)
            if (exactAllowed(c)) manager.setExactAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, batch.first().at, intent)
            else manager.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, batch.first().at, intent)
        }.onFailure {
            store.edit().putString("schedule_error", it.message ?: "调度失败").commit()
            android.util.Log.e("CustomReminder", "注册提醒失败", it)
        }
    }
    private fun rId(id: String) = 0x52000000 xor id.hashCode()
    private fun post(c: Context, p: JSONObject) {
        val sound = prefs(c).getBoolean("sound", true)
        val channel = if (sound) "lanchat_custom_reminders_sound_v1" else "lanchat_custom_reminders_silent_v1"
        val nm = c.getSystemService(NotificationManager::class.java)
        if (Build.VERSION.SDK_INT >= 26) nm.createNotificationChannel(NotificationChannel(channel,
            if (sound) "自定义通知" else "自定义通知（静音）", NotificationManager.IMPORTANCE_HIGH).apply {
            enableVibration(sound)
            if (sound) setSound(Uri.parse("android.resource://${c.packageName}/${R.raw.lqchat_notification}"),
                AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_NOTIFICATION).build()) else setSound(null, null)
        })
        val open = PendingIntent.getActivity(c, rId(p.getString("id")),
            Intent(c, MainActivity::class.java).putExtra("customReminderId", p.getString("id"))
                .setData(Uri.parse("lqchat://custom-reminder/${p.getString("id")}"))
                .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val builder = NotificationCompat.Builder(c, channel).setSmallIcon(R.mipmap.ic_launcher)
            .setContentTitle(LocalDeviceIdentity.reminderNotificationTitle(LocalDeviceIdentity.read(c), p.getString("title")))
            .setContentText(p.getString("content"))
            .setStyle(NotificationCompat.BigTextStyle().bigText(p.getString("content")))
            .setContentIntent(open).setAutoCancel(true).setOnlyAlertOnce(false)
            .setCategory(NotificationCompat.CATEGORY_REMINDER).setPriority(NotificationCompat.PRIORITY_HIGH)
        if (Build.VERSION.SDK_INT < 26 && sound) builder.setDefaults(NotificationCompat.DEFAULT_ALL)
        nm.notify("custom-reminder:${p.getString("id")}", ALARM_ID + 1, builder.build())
    }
    /** Own notifications bypass the app picker; the listener deliberately excludes this package. */
    private fun push(c: Context, p: JSONObject) {
        if (!LanChatForegroundService.notificationSessionReady()) return
        val settings = NotificationSyncSettings.read(c)
        if (!CustomReminderForwarding.enabled(settings.optBoolean("push_enabled"), settings.optBoolean("lq_reminder_push_enabled"),
                NotificationSyncSettings.strings(settings.optJSONArray("target_device_ids")))) return
        val notification = JSONObject().put("msg_type", "notification")
            .put("event_id", UUID.randomUUID().toString())
            .put("source_device_id", "").put("target_device_id", "")
            .put("package", c.packageName).put("app_name", "LQChat")
            // Transport carries the original title. Receivers prefix the verified source-device name.
            .put("title", p.getString("title")).put("text", p.getString("content"))
            .put("notification_key", "lq-reminder-${p.getString("key")}")
            .put("post_time", System.currentTimeMillis())
        NotificationAppIcon.encode(c, c.packageName)?.let { notification.put("app_icon", it) }
        NotificationSyncNative.send(JSONObject().put("notification", notification).put("settings", settings).toString())
    }
    @Synchronized fun receive(c: Context, token: String?) {
        val store = prefs(c)
        if (token == null || token != store.getString("token", null)) return
        val batch = JSONArray(store.getString("pending", "[]"))
        val now = System.currentTimeMillis()
        if (batch.length() > 0 && now < batch.getJSONObject(0).getLong("at")) { refresh(c); return }
        val delivered = store.getStringSet("delivered", emptySet())!!.toMutableSet()
        for (i in 0 until batch.length()) {
            val p = batch.getJSONObject(i); val key = p.getString("key")
            if (key in delivered) continue
            // Consume before posting: duplicate broadcasts cannot replay a reminder.
            delivered.add(key)
            // Keep a bounded tail, including all same-time notifications.
            val bounded = delivered.sortedByDescending { it.substringBeforeLast(':').substringAfterLast(':').toLongOrNull() ?: 0L }.take(4000).toSet()
            check(store.edit().putStringSet("delivered", bounded).commit())
            if (store.getBoolean("enabled", true) && now - p.getLong("at") <= MAX_LATENESS) {
                if (NotificationManagerCompat.from(c).areNotificationsEnabled()) {
                    runCatching { post(c, p) }.onFailure { android.util.Log.e("CustomReminder", "发送提醒失败", it) }
                }
                runCatching { push(c, p) }.onFailure { android.util.Log.e("CustomReminder", "推送提醒失败", it) }
            }
        }
        refresh(c)
    }
}

class CustomReminderReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent?) {
        if (intent?.action in setOf(Intent.ACTION_BOOT_COMPLETED, Intent.ACTION_USER_UNLOCKED,
                Intent.ACTION_MY_PACKAGE_REPLACED, Intent.ACTION_TIME_CHANGED, Intent.ACTION_TIMEZONE_CHANGED,
                "android.app.action.SCHEDULE_EXACT_ALARM_PERMISSION_STATE_CHANGED")) {
            CustomReminderController.refresh(context)
        } else CustomReminderController.receive(context, intent?.getStringExtra("token"))
    }
}
