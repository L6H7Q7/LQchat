package com.lanchat.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Build
import android.widget.RemoteViews
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import org.json.JSONObject

/** Sole Android content dedup owner; UI records are a read-only native projection. */
object SyncedNotificationPublisher {
    const val CHANNEL = "synced_notifications"
    const val EXTRA_HISTORY_RECORD_ID = "com.lanchat.app.extra.HISTORY_RECORD_ID"
    const val EXTRA_SOURCE_DEVICE_ID = "com.lanchat.app.extra.SOURCE_DEVICE_ID"
    const val EXTRA_NOTIFICATION_PACKAGE = "com.lanchat.app.extra.NOTIFICATION_PACKAGE"
    const val EXTRA_NOTIFICATION_KEY = "com.lanchat.app.extra.NOTIFICATION_KEY"
    private data class Key(val source: String, val pkg: String, val key: String)
    private data class Content(val app: String, val title: String, val text: String, val icon: String)
    internal data class Presentation(val title: String, val sourceLabel: String, val systemTitle: String)
    private val recent = LinkedHashMap<Key, Content>(512, 0.75f, true)
    @Synchronized fun clear() { recent.clear() }

    @Synchronized fun receive(context: Context, payload: JSONObject) {
        if (!LanChatForegroundService.notificationSessionReady()) return
        val request = payload.optString("request_id")
        if (!NotificationSyncNative.pending(request)) return
        var changed = false
        var failureReason = ""
        val success = runCatching {
            check(NotificationSyncSettings.read(context).optBoolean("receive_enabled"))
            val manager = context.getSystemService(NotificationManager::class.java)
            if (Build.VERSION.SDK_INT >= 26) manager.createNotificationChannel(
                NotificationChannel(CHANNEL, "其他设备的通知", NotificationManager.IMPORTANCE_HIGH).apply {
                    description = "来自已选择设备的系统通知"; lockscreenVisibility = Notification.VISIBILITY_PRIVATE
                })
            check(NotificationManagerCompat.from(context).areNotificationsEnabled())
            if (Build.VERSION.SDK_INT >= 26) check(manager.getNotificationChannel(CHANNEL).importance != NotificationManager.IMPORTANCE_NONE)
            val n = payload.getJSONObject("notification")
            val key = Key(n.getString("source_device_id"), n.getString("package"), n.getString("notification_key"))
            val content = Content(n.getString("app_name"), n.getString("title"), n.getString("text"), n.optString("app_icon"))
            if (recent[key] != content) {
                val intent = launchIntent(context, payload, n)
                val requestCode = payload.optString("history_record_id").hashCode() and 0x7fffffff
                val open = PendingIntent.getActivity(context, requestCode, intent, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
                val bitmap = NotificationAppIcon.decode(content.icon)
                val source = payload.optString("source_name", key.source)
                val presentation = presentation(n, content, source)
                val time = SimpleDateFormat("HH:mm", Locale.getDefault()).format(Date(n.optLong("post_time")))
                fun layout(resource: Int) = RemoteViews(context.packageName, resource).apply {
                    if (bitmap != null) setImageViewBitmap(R.id.sync_icon, bitmap)
                    else setImageViewResource(R.id.sync_icon, R.mipmap.ic_launcher)
                    setTextViewText(R.id.sync_app, content.app)
                    setTextViewText(R.id.sync_title, presentation.title)
                    setTextViewText(R.id.sync_text, content.text)
                    setTextViewText(R.id.sync_source, presentation.sourceLabel)
                    setTextViewText(R.id.sync_time, time)
                }
                val notification = NotificationCompat.Builder(context, CHANNEL).setSmallIcon(R.mipmap.ic_launcher)
                    .setContentTitle(presentation.systemTitle)
                    .setContentText(content.text)
                    .setStyle(NotificationCompat.DecoratedCustomViewStyle())
                    .setCustomContentView(layout(R.layout.notification_sync_compact))
                    .setCustomHeadsUpContentView(layout(R.layout.notification_sync_compact))
                    .setCustomBigContentView(layout(R.layout.notification_sync_expanded))
                    .setWhen(n.optLong("post_time")).setShowWhen(false)
                    .setContentIntent(open).setAutoCancel(true).setOnlyAlertOnce(true)
                    .setPriority(NotificationCompat.PRIORITY_HIGH).setVisibility(NotificationCompat.VISIBILITY_PRIVATE).build()
                manager.notify("${key.source}:${key.pkg}:${key.key}", 0, notification)
                recent[key] = content
                while (recent.size > 512) recent.remove(recent.keys.first())
                changed = true
            }
            true
        }.onFailure { failureReason = it.message ?: "receive_rejected" }.getOrDefault(false)
        if (LanChatForegroundService.notificationSessionReady()) NotificationSyncNative.complete(request, success, changed, failureReason)
    }

    private fun presentation(notification: JSONObject, content: Content, source: String): Presentation {
        val isBattery = notification.optString("package") == "com.lanchat.app" &&
            notification.optString("notification_key").startsWith("lq-battery-") &&
            content.title == "电量提醒"
        if (isBattery) {
            val title = titleWithSource(source, "电量")
            return Presentation(title, "", title)
        }
        if (notification.optString("package") == "com.lanchat.app" &&
            notification.optString("notification_key").startsWith("lq-reminder-")) {
            val title = CustomReminderForwarding.remoteTitle(source, content.title)
            return Presentation(title, "", title)
        }
        val title = titleWithSource(source, titleWithApp(content.app, content.title))
        val sourceLabel = source.trim().takeIf { it.isNotEmpty() }?.let { "来自 $it" }.orEmpty()
        return Presentation(title, sourceLabel, title)
    }

    private fun titleWithApp(app: String, title: String): String {
        val appName = app.trim()
        val notificationTitle = title.trim()
        if (appName.isEmpty()) return notificationTitle
        if (notificationTitle.isEmpty() || notificationTitle == appName) return appName
        if (notificationTitle.startsWith("$appName · ")) return notificationTitle
        return "$appName · $notificationTitle"
    }

    private fun titleWithSource(source: String, title: String): String {
        val sourceName = source.trim()
        if (sourceName.isEmpty() || title == sourceName || title.startsWith("$sourceName · ")) return title
        return "$sourceName · $title"
    }

    internal fun presentationForTest(notification: JSONObject, source: String): Presentation =
        presentation(
            notification,
            Content(
                notification.optString("app_name"),
                notification.optString("title"),
                notification.optString("text"),
                notification.optString("app_icon"),
            ),
            source,
        )

    internal fun launchIntent(context: Context, payload: JSONObject, notification: JSONObject): Intent =
        Intent(context, MainActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
            .putExtra(EXTRA_HISTORY_RECORD_ID, payload.optString("history_record_id"))
            .putExtra(EXTRA_SOURCE_DEVICE_ID, notification.optString("source_device_id"))
            .putExtra(EXTRA_NOTIFICATION_PACKAGE, notification.optString("package"))
            .putExtra(EXTRA_NOTIFICATION_KEY, notification.optString("notification_key"))
}
