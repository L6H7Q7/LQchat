package com.lanchat.app

import android.app.Notification
import android.content.ComponentName
import android.os.Handler
import android.os.Looper
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import androidx.annotation.Keep
import java.util.UUID
import org.json.JSONObject

@Keep
object NotificationSyncNative {
    external fun send(data: String)
    external fun recordRoute(data: String)
    external fun localHistoryChanged()
    external fun pending(requestId: String): Boolean
    external fun complete(requestId: String, success: Boolean, changed: Boolean, reason: String)
}

class LanChatNotificationListener : NotificationListenerService() {
    companion object {
        internal const val RECOVERY_CHECK_INTERVAL_MS = 60_000L

        internal fun shouldRequestBackgroundRecovery(
            mayRecover: Boolean,
            notificationSessionReady: Boolean,
        ): Boolean = mayRecover && !notificationSessionReady

        internal fun shouldIgnoreProgress(notification: Notification): Boolean {
            val extras = notification.extras
            // Test values, not key presence: clearing progress can leave zero/false extras behind.
            return extras.getInt(Notification.EXTRA_PROGRESS_MAX, 0) > 0 ||
                extras.getBoolean(Notification.EXTRA_PROGRESS_INDETERMINATE, false)
        }
    }

    private val rebindHandler = Handler(Looper.getMainLooper())
    private var rebindAttempt = 0
    private val recoveryCheck = object : Runnable {
        override fun run() {
            val mayRecover = BackgroundRuntimeSettings.mayRecover(this@LanChatNotificationListener)
            if (shouldRequestBackgroundRecovery(
                    mayRecover,
                    LanChatForegroundService.notificationSessionReady(),
                )
            ) {
                runCatching {
                    LanChatForegroundService.startRecoverySession(
                        this@LanChatNotificationListener,
                        "notification-listener-watchdog",
                    )
                }.onFailure {
                    android.util.Log.w("NotificationSync", "通知监听触发后台恢复失败", it)
                }
            }
            if (mayRecover) {
                rebindHandler.postDelayed(this, RECOVERY_CHECK_INTERVAL_MS)
            }
        }
    }

    override fun onListenerConnected() {
        super.onListenerConnected()
        rebindAttempt = 0
        rebindHandler.removeCallbacks(recoveryCheck)
        recoveryCheck.run()
    }

    override fun onListenerDisconnected() {
        super.onListenerDisconnected()
        rebindHandler.removeCallbacks(recoveryCheck)
        if (!NotificationSyncSettings.isListenerEnabled(this) || !BackgroundRuntimeSettings.mayRecover(this)) return
        val delay = (1L shl minOf(rebindAttempt++, 5)) * 1000L
        rebindHandler.postDelayed({
            if (NotificationSyncSettings.isListenerEnabled(this) && BackgroundRuntimeSettings.mayRecover(this)) {
                runCatching { requestRebind(ComponentName(this, LanChatNotificationListener::class.java)) }
            }
        }, delay)
    }

    override fun onDestroy() {
        rebindHandler.removeCallbacksAndMessages(null)
        super.onDestroy()
    }
    override fun onNotificationPosted(sbn: StatusBarNotification?) {
        // This gate is JVM-only; an OS-created listener must never initialize native code.
        if (sbn == null || !LanChatForegroundService.notificationSessionReady()) return
        if (sbn.packageName == packageName) return
        val settings = NotificationSyncSettings.read(this)
        if (!settings.optBoolean("push_enabled") || settings.getJSONArray("target_device_ids").length() == 0) return
        if (sbn.packageName !in NotificationSyncSettings.strings(settings.optJSONArray("allowed_packages"))) return
        runCatching {
            // Drop every standard progress notification, including 0% and 100%, before payload/icon/JNI work.
            if (shouldIgnoreProgress(sbn.notification)) return
            val extras = sbn.notification.extras
            val title = limit(extras.getCharSequence(Notification.EXTRA_TITLE)?.toString().orEmpty(), 256)
            val big = extras.getCharSequence(Notification.EXTRA_BIG_TEXT)?.toString().orEmpty()
            val text = limit(big.ifBlank { extras.getCharSequence(Notification.EXTRA_TEXT)?.toString().orEmpty() }, 4096)
            if (title.isBlank() && text.isBlank()) return
            val name = runCatching { packageManager.getApplicationLabel(packageManager.getApplicationInfo(sbn.packageName, 0)).toString() }.getOrDefault(sbn.packageName)
            val notification = JSONObject().put("msg_type", "notification").put("event_id", UUID.randomUUID().toString())
                .put("source_device_id", "").put("target_device_id", "").put("package", sbn.packageName)
                .put("app_name", limit(name, 128)).put("title", title).put("text", text)
                .put("notification_key", sbn.key).put("post_time", sbn.postTime)
            NotificationAppIcon.encode(this, sbn.packageName)?.let { notification.put("app_icon", it) }
            if (LanChatForegroundService.notificationSessionReady()) {
                NotificationSyncNative.send(JSONObject().put("notification", notification).put("settings", settings).toString())
            }
        }.onFailure { android.util.Log.w("NotificationSync", "通知采集失败") }
    }
    private fun limit(value: String, maximum: Int): String =
        value.substring(0, value.offsetByCodePoints(0, minOf(value.codePointCount(0, value.length), maximum)))
}
