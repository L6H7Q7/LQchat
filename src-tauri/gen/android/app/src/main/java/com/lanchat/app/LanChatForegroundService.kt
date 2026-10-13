package com.lanchat.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.ContentResolver
import android.content.Intent
import android.content.pm.ServiceInfo
import android.media.AudioAttributes
import android.media.MediaPlayer
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.net.wifi.WifiManager
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.PowerManager
import android.os.SystemClock
import androidx.annotation.Keep
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import java.util.UUID
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.TimeoutException
import java.util.concurrent.atomic.AtomicBoolean
import org.json.JSONArray
import org.json.JSONObject

class LanChatForegroundService : Service() {
    companion object {
        const val ACTION_START_SESSION = "com.lanchat.app.action.START_USER_SESSION"
        const val ACTION_STOP_SESSION = "com.lanchat.app.action.STOP_SESSION"
        const val ACTION_RETRY = "com.lanchat.app.action.RETRY_BACKGROUND"
        private const val ACTION_RECOVER = "com.lanchat.app.action.RECOVER_BACKGROUND"
        const val EXTRA_PEER_ID = "lanchat_peer_id"
        internal const val EXTRA_SESSION_TOKEN = "lanchat_session_token"
        private const val EXTRA_RECOVERY_SOURCE = "lanchat_recovery_source"
        private const val SESSION_PREFS = "lanchat_runtime_session"
        private const val SESSION_ACTIVE = "active"
        private const val SESSION_TOKEN = "token"
        private const val ROUTE_STATE_PREFS = "lanchat_notification_route_state"
        private const val ROUTE_STATE_SNAPSHOT = "peers"
        @Volatile private var nativeReadyInProcess = false
        @Volatile private var notificationSessionActive = false
        @Volatile private var activeInstance: LanChatForegroundService? = null
        fun notificationSessionReady(): Boolean = nativeReadyInProcess && notificationSessionActive
        private const val TAG = "LanChatService"
        private const val SERVICE_CHANNEL = "lanchat_background_service"
        internal const val MESSAGE_CHANNEL = "lanchat_messages_v2"
        internal const val DATA_SOUND_CHANNEL = "lanchat_messages_files_sound_v1"
        internal const val DATA_SILENT_CHANNEL = "lanchat_messages_files_silent_v1"
        private const val SERVICE_NOTIFICATION_ID = 4100
        private const val ERROR_NOTIFICATION_ID = 9100
        private const val MESSAGE_NOTIFICATION_BASE = 10000
        private const val DISCONNECT_NOTIFICATION_BASE = 18000
        internal const val DATA_SOUND_COOLDOWN_MS = 800L
        private const val RUNNING_NOTIFICATION_TEXT = "通知转发与接收服务正在运行"
        private const val SHORT_WAKE_TIMEOUT_MS = 15_000L
        private const val TRANSFER_WAKE_TIMEOUT_MS = 120_000L
        private const val STOP_CALL_TIMEOUT_MS = 15_000L
        private const val FORCE_STOP_CALL_TIMEOUT_MS = 6_000L
        internal const val HEALTH_CHECK_INTERVAL_MS = 15_000L
        private const val UNHEALTHY_CHECKS_BEFORE_RESTART = 2
        private const val FIRST_SELF_HEAL_DELAY_MS = 30_000L
        private const val MAX_SELF_HEAL_DELAY_MS = 5 * 60_000L
        private const val STARTUP_RECOVERY_DELAY_MS = 10_000L
        private const val STARTUP_RECOVERY_RETRY_MS = 30_000L
        private const val MAX_STARTUP_RECOVERY_ATTEMPTS = 3

        fun startVisibleUserSession(context: Context) {
            nativeReadyInProcess = true
            BackgroundRuntimeSettings.beginUserSession(context)
            val token = UUID.randomUUID().toString()
            persistSession(context, token)
            start(context, ACTION_START_SESSION, token)
        }

        fun retryVisibleUserSession(context: Context) {
            nativeReadyInProcess = true
            BackgroundRuntimeSettings.beginUserSession(context)
            val token = currentSessionToken(context) ?: UUID.randomUUID().toString().also {
                persistSession(context, it)
            }
            start(context, ACTION_RETRY, token)
        }

        fun startRecoverySession(context: Context, source: String) {
            if (!BackgroundRuntimeSettings.mayRecover(context) && !BackgroundRuntimeSettings.mayStartOnBoot(context)) return
            start(context, ACTION_RECOVER, null, source)
        }

        fun startPackageReplaceRecovery(context: Context) {
            if (!mayRecoverAfterPackageReplace(context)) return
            start(context, ACTION_RECOVER, null, "package-replaced")
        }

        internal fun mayRecoverAfterPackageReplace(context: Context): Boolean =
            BackgroundRuntimeSettings.mayRecover(context)

        private fun start(context: Context, action: String, token: String?, source: String? = null) {
            val intent = Intent(context, LanChatForegroundService::class.java).apply {
                this.action = action
                token?.let { putExtra(EXTRA_SESSION_TOKEN, it) }
                source?.let { putExtra(EXTRA_RECOVERY_SOURCE, it) }
            }
            ContextCompat.startForegroundService(context, intent)
        }

        fun stopVisibleUserSession(context: Context) {
            val token = currentSessionToken(context)
            BackgroundRuntimeSettings.stopCurrentSession(context)
            invalidatePersistedSession(context)
            context.startService(Intent(context, LanChatForegroundService::class.java).apply {
                action = ACTION_STOP_SESSION
                putExtra(EXTRA_SESSION_TOKEN, token)
            })
        }

        fun refreshBatteryAlerts() {
            val service = activeInstance ?: return
            service.mainHandler.post {
                if (!service.exiting.get() && service::batteryAlertController.isInitialized) {
                    service.batteryAlertController.refresh(resetPendingSequence = true)
                }
            }
        }

        fun refreshNotificationRouteStatus() {
            val service = activeInstance ?: return
            service.mainHandler.post { service.renderNotificationRouteStatus() }
        }

        internal fun persistSession(context: Context, token: String): Boolean =
            context.getSharedPreferences(SESSION_PREFS, Context.MODE_PRIVATE).edit()
                .putBoolean(SESSION_ACTIVE, true).putString(SESSION_TOKEN, token).commit()

        internal fun invalidatePersistedSession(context: Context): Boolean =
            context.getSharedPreferences(SESSION_PREFS, Context.MODE_PRIVATE).edit()
                .putBoolean(SESSION_ACTIVE, false).remove(SESSION_TOKEN).commit()

        internal fun currentSessionToken(context: Context): String? {
            val prefs = context.getSharedPreferences(SESSION_PREFS, Context.MODE_PRIVATE)
            if (!prefs.getBoolean(SESSION_ACTIVE, false)) return null
            return prefs.getString(SESSION_TOKEN, null)?.takeIf { it.isNotBlank() }
        }

        internal fun isValidVisibleSessionRequest(context: Context, token: String?, nativeReady: Boolean): Boolean =
            nativeReady && token != null && token == currentSessionToken(context)

        internal fun mayRecoverStickyRestart(context: Context): Boolean =
            BackgroundRuntimeSettings.mayRecover(context)

        internal fun isUnhealthyCoreState(state: String?): Boolean =
            state == "ERROR" || state == "STOPPED"

        internal fun selfHealDelayMs(failedAttempts: Int): Long {
            val shift = failedAttempts.coerceIn(0, 4)
            return (FIRST_SELF_HEAL_DELAY_MS * (1L shl shift)).coerceAtMost(MAX_SELF_HEAL_DELAY_MS)
        }

        internal fun resetProcessNativeReadyForTest() { nativeReadyInProcess = false }
        internal fun serviceStartMode(): Int = START_STICKY
        internal fun isVerifiedStopResponse(response: JSONObject): Boolean {
            val status = response.optJSONObject("status")
            return response.optBoolean("ok") && status?.optString("state") == "STOPPED" &&
                response.optBoolean("tasks_stopped") && response.optBoolean("ports_released") &&
                response.optBoolean("resources_released")
        }
        internal fun notificationIdFor(idSeed: Long): Int = MESSAGE_NOTIFICATION_BASE + ((idSeed.hashCode() and Int.MAX_VALUE) % 8000)
        internal fun disconnectNotificationIdFor(peerId: String): Int =
            DISCONNECT_NOTIFICATION_BASE + ((peerId.hashCode() and Int.MAX_VALUE) % 8000)
        internal fun notificationRouteDisconnectedText(peerName: String): String {
            val name = displayPeerName(peerName)
            val label = if (name.endsWith("设备")) name else "${name}设备"
            return "${label}已从局域网断开"
        }
        internal fun notificationRouteConnectedText(peerName: String): String {
            val name = displayPeerName(peerName)
            return "${name}已接入局域网"
        }

        internal data class NotificationRoutePeerState(
            val id: String,
            val name: String,
            val isOffline: Boolean,
        )

        internal fun monitoredNotificationRoutePeers(
            settings: JSONObject,
            peers: JSONArray,
        ): Map<String, NotificationRoutePeerState> {
            val peerById = (0 until peers.length()).mapNotNull { index ->
                peers.optJSONObject(index)?.let { peer -> peer.optString("id") to peer }
            }.filter { it.first.isNotBlank() }.toMap()
            val monitored = linkedMapOf<String, NotificationRoutePeerState>()
            if (settings.optBoolean("push_enabled")) {
                NotificationSyncSettings.strings(settings.optJSONArray("target_device_ids"))
                    .sorted()
                    .forEach { id ->
                        val peer = peerById[id]
                        monitored[id] = NotificationRoutePeerState(
                            id,
                            displayPeerName(peer?.optString("name").orEmpty()),
                            peer == null || peer.optBoolean("is_offline", true),
                        )
                    }
            }
            if (settings.optBoolean("receive_enabled")) {
                peerById.values
                    .filter { it.optBoolean("pushes_to_local") }
                    .sortedBy { it.optString("id") }
                    .forEach { peer ->
                        val id = peer.optString("id")
                        monitored[id] = NotificationRoutePeerState(
                            id,
                            displayPeerName(peer.optString("name")),
                            peer.optBoolean("is_offline", true),
                        )
                    }
            }
            return monitored
        }

        internal fun newlyDisconnectedNotificationRoutePeers(
            previous: Map<String, NotificationRoutePeerState>?,
            current: Map<String, NotificationRoutePeerState>,
        ): List<NotificationRoutePeerState> {
            if (previous == null) return emptyList()
            return current.values.mapNotNull { peer ->
                val old = previous[peer.id] ?: return@mapNotNull null
                if (!old.isOffline && peer.isOffline) {
                    if (peer.name == "局域网设备" && old.name != "局域网设备") peer.copy(name = old.name)
                    else peer
                } else {
                    null
                }
            }
        }
        internal fun newlyConnectedNotificationRoutePeers(
            previous: Map<String, NotificationRoutePeerState>?,
            current: Map<String, NotificationRoutePeerState>,
        ): List<NotificationRoutePeerState> {
            if (previous == null) return emptyList()
            return current.values.mapNotNull { peer ->
                if (peer.isOffline) return@mapNotNull null
                val old = previous[peer.id] ?: return@mapNotNull null
                if (old.isOffline) {
                    if (peer.name == "局域网设备" && old.name != "局域网设备") {
                        peer.copy(name = old.name)
                    } else {
                        peer
                    }
                } else {
                    null
                }
            }
        }

        internal fun mergeNotificationRoutePeerStates(
            previous: Map<String, NotificationRoutePeerState>?,
            current: Map<String, NotificationRoutePeerState>,
            initializeUnknown: Boolean,
        ): Map<String, NotificationRoutePeerState> {
            // Missing snapshots are unknown, not disconnected. Likewise, an
            // offline -> online snapshot waits for a fresh discovery event so
            // a service/core rebuild cannot manufacture a connection alert.
            val merged = previous.orEmpty().toMutableMap()
            current.values.forEach { peer ->
                val old = merged[peer.id]
                when {
                    initializeUnknown && old == null -> merged[peer.id] = peer
                    initializeUnknown -> merged[peer.id] = old!!.copy(name = peer.name)
                    old == null && peer.isOffline -> merged[peer.id] = peer
                    old == null -> Unit
                    !old.isOffline && peer.isOffline -> merged[peer.id] = peer
                    old.isOffline && !peer.isOffline -> Unit
                    else -> merged[peer.id] = peer
                }
            }
            return merged
        }

        internal fun isNotificationRouteConnectionRelevant(
            settings: JSONObject,
            payload: JSONObject,
        ): Boolean {
            val peerId = payload.optString("id")
            if (peerId.isBlank()) return false
            val isPushTarget = settings.optBoolean("push_enabled") &&
                peerId in NotificationSyncSettings.strings(settings.optJSONArray("target_device_ids"))
            val isReceiveSource = settings.optBoolean("receive_enabled") &&
                payload.optBoolean("pushes_to_local")
            return isPushTarget || isReceiveSource
        }

        internal fun shouldNotifyNotificationRouteConnection(
            previous: Map<String, NotificationRoutePeerState>?,
            settings: JSONObject,
            payload: JSONObject,
        ): Boolean {
            val transition = payload.optString("connection_transition")
            if (transition != "new" && transition != "reconnected" && transition != "unchanged") return false
            if (!isNotificationRouteConnectionRelevant(settings, payload)) return false
            val id = payload.optString("id").takeIf { it.isNotBlank() } ?: return false
            val old = previous?.get(id)
            if (old?.isOffline == false) return false
            return transition != "unchanged" || old?.isOffline == true
        }

        internal fun notificationRoutePeerStatesToJson(
            states: Map<String, NotificationRoutePeerState>,
        ): String = JSONArray(states.values.sortedBy { it.id }.map { peer ->
            JSONObject().put("id", peer.id).put("name", peer.name).put("is_offline", peer.isOffline)
        }).toString()

        internal fun notificationRoutePeerStatesFromJson(
            raw: String?,
        ): Map<String, NotificationRoutePeerState>? {
            if (raw == null) return null
            return runCatching {
                val peers = JSONArray(raw)
                (0 until peers.length()).mapNotNull { index ->
                    val peer = peers.optJSONObject(index) ?: return@mapNotNull null
                    val id = peer.optString("id").takeIf { it.isNotBlank() } ?: return@mapNotNull null
                    id to NotificationRoutePeerState(
                        id,
                        displayPeerName(peer.optString("name")),
                        peer.optBoolean("is_offline", true),
                    )
                }.toMap()
            }.getOrNull()
        }
        internal fun notificationRouteStatusText(settings: JSONObject, peers: JSONArray): String {
            val peerById = (0 until peers.length()).mapNotNull { index ->
                peers.optJSONObject(index)?.let { peer -> peer.optString("id") to peer }
            }.filter { it.first.isNotBlank() }.toMap()
            val targetIds = NotificationSyncSettings.strings(settings.optJSONArray("target_device_ids")).sorted()
            val targets = targetIds.map { id -> peerById[id] }
            val sources = (0 until peers.length()).mapNotNull { index -> peers.optJSONObject(index) }
                .filter { it.optBoolean("pushes_to_local") }
            val pushEnabled = settings.optBoolean("push_enabled")
            val receiveEnabled = settings.optBoolean("receive_enabled")
            val disconnected =
                (pushEnabled && targets.any { it == null || it.optBoolean("is_offline", true) }) ||
                    (receiveEnabled && sources.any { it.optBoolean("is_offline", true) })
            if (disconnected) return "局域网设备已断开"

            val states = mutableListOf<String>()
            if (pushEnabled) {
                states += if (targets.isEmpty()) {
                    "正在等待选择通知转发设备"
                } else {
                    "正在转发通知给 ${targets.joinToString("、") { displayPeerName(it?.optString("name").orEmpty()) }}"
                }
            }
            if (receiveEnabled) {
                states += if (sources.isEmpty()) {
                    "正在接收通知"
                } else {
                    "正在接收来自${sources.joinToString("、") { displayPeerName(it.optString("name")) }}的通知"
                }
            }
            return states.joinToString("；").ifBlank { RUNNING_NOTIFICATION_TEXT }
        }
        private fun displayPeerName(peerName: String): String =
            peerName.trim().takeIf { it.isNotEmpty() } ?: "局域网设备"

        internal fun shouldPlayDataNotificationSound(
            enabled: Boolean,
            lastPlayedAtMs: Long,
            nowMs: Long,
        ): Boolean = enabled && (lastPlayedAtMs <= 0L || nowMs - lastPlayedAtMs >= DATA_SOUND_COOLDOWN_MS)
    }

    init { System.loadLibrary("lanchat") }
    private external fun nativeStartCore(appDataDir: String): String
    private external fun nativeStopCore(): String
    private external fun nativeForceStopCore(): String
    private external fun nativeGetCoreStatus(): String
    private external fun nativeInitializeAndroidContext(): String
    private external fun nativeRegisterServiceEventSink()
    private external fun nativeUnregisterServiceEventSink(): String

    private val worker = Executors.newSingleThreadExecutor()
    private val shutdownWorker = Executors.newSingleThreadExecutor()
    private val nativeStopWorker = Executors.newCachedThreadPool()
    private val mainHandler = Handler(Looper.getMainLooper())
    private val exiting = AtomicBoolean(false)
    private val coreStartRequested = AtomicBoolean(false)
    private var acceptedSessionToken: String? = null
    private var jniReadyForSession = false
    private var platformSessionInitialized = false
    private var consecutiveUnhealthyChecks = 0
    private var selfHealAttempts = 0
    private var nextSelfHealAtElapsedMs = 0L
    private var selfHealRestartInFlight = false
    private var foregroundEstablished = false
    private var startupRecoveryAttempts = 0
    private var notificationRoutePeers = JSONArray()
    private var notificationRouteSnapshotReady = false
    private var previousNotificationRoutePeers: Map<String, NotificationRoutePeerState>? = null
    private var notificationRouteStateInitialized = false
    private val healthCheckRunnable = Runnable { runHealthCheck() }
    private val startupRecoveryRunnable = Runnable { verifyStartupRecovery() }

    private lateinit var notificationManager: NotificationManager
    private lateinit var connectivityManager: ConnectivityManager
    private lateinit var wifiManager: WifiManager
    private lateinit var powerManager: PowerManager
    private lateinit var batteryAlertController: BatteryAlertController
    private var multicastLock: WifiManager.MulticastLock? = null
    private var transferWakeLock: PowerManager.WakeLock? = null
    private var networkCallback: ConnectivityManager.NetworkCallback? = null
    private var activeNotificationSound: MediaPlayer? = null
    private var lastNotificationSoundAtMs = 0L

    override fun onCreate() {
        super.onCreate()
        ServiceRecoveryDiagnostics.record(this, "service_on_create")
        AndroidDownloadStore.initialize(applicationContext)
        notificationManager = getSystemService(NotificationManager::class.java)
        connectivityManager = getSystemService(ConnectivityManager::class.java)
        wifiManager = applicationContext.getSystemService(WifiManager::class.java)
        powerManager = getSystemService(PowerManager::class.java)
        createNotificationChannels()
        activeInstance = this
        previousNotificationRoutePeers = loadNotificationRoutePeerStates()
        batteryAlertController = BatteryAlertController(
            applicationContext,
            notificationManager,
            mainHandler,
        ) { servicePendingIntent() }
        batteryAlertController.refresh()
        mainHandler.postDelayed(startupRecoveryRunnable, STARTUP_RECOVERY_DELAY_MS)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        ServiceRecoveryDiagnostics.record(
            this,
            "service_start_request",
            intent?.action ?: "sticky-null-intent",
        )
        if (intent == null) {
            acceptRecoverySession(startId, allowStartOnBoot = false)
            return serviceStartMode()
        }

        when (intent.action) {
            ACTION_START_SESSION -> acceptVisibleSessionOrStop(intent, startId, retry = false)
            ACTION_RETRY -> acceptVisibleSessionOrStop(intent, startId, retry = true)
            ACTION_RECOVER -> acceptRecoverySession(startId, allowStartOnBoot = true)
            ACTION_STOP_SESSION -> {
                val token = intent.getStringExtra(EXTRA_SESSION_TOKEN)
                if ((token != null && token == acceptedSessionToken && jniReadyForSession) ||
                    (token == null && !BackgroundRuntimeSettings.mayRecover(this))) {
                    beginExit()
                } else {
                    rejectUnexpectedStart(startId, "无有效会话的停止请求")
                }
            }
            else -> rejectUnexpectedStart(startId, "未知的 Service 启动请求")
        }
        return serviceStartMode()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onTaskRemoved(rootIntent: Intent?) {
        if (!BackgroundRuntimeSettings.mayRecover(this)) beginExit()
        super.onTaskRemoved(rootIntent)
    }

    override fun onDestroy() {
        ServiceRecoveryDiagnostics.record(this, "service_on_destroy")
        val mayRecover = BackgroundRuntimeSettings.mayRecover(this)
        if (::batteryAlertController.isInitialized) {
            batteryAlertController.stop(clearState = !mayRecover)
        }
        if (activeInstance === this) activeInstance = null
        mainHandler.removeCallbacks(startupRecoveryRunnable)
        foregroundEstablished = false
        stopHealthMonitor()
        releaseNotificationSound()
        if (!mayRecover) beginExit()
        super.onDestroy()
    }

    private fun acceptRecoverySession(startId: Int, allowStartOnBoot: Boolean) {
        val mayRecover = mayRecoverStickyRestart(this)
        val allowed = mayRecover || (allowStartOnBoot && BackgroundRuntimeSettings.mayStartOnBoot(this))
        if (!allowed) {
            if (allowStartOnBoot) rejectUnexpectedStart(startId, "后台恢复未开启")
            else rejectUnauthorizedStickyRestart(startId, "系统重建未获得后台常驻授权")
            return
        }
        runCatching { nativeReadyInProcess = true; System.loadLibrary("lanchat") }
            .onFailure { rejectUnexpectedStart(startId, "Rust 核心加载失败: ${it.message}") }
        if (exiting.get()) return
        acceptedSessionToken = currentSessionToken(this) ?: UUID.randomUUID().toString().also { persistSession(this, it) }
        jniReadyForSession = true
        initializePlatformSession()
        startCoreAsync()
    }

    private fun acceptVisibleSessionOrStop(intent: Intent, startId: Int, retry: Boolean) {
        val token = intent.getStringExtra(EXTRA_SESSION_TOKEN)
        if (!isValidVisibleSessionRequest(this, token, nativeReadyInProcess)) {
            rejectUnexpectedStart(startId, "会话令牌无效或 Rust 尚未由可见 Activity 加载")
            return
        }

        acceptedSessionToken = token
        jniReadyForSession = true
        initializePlatformSession()
        startCoreAsync(force = retry)
    }

    private fun initializePlatformSession() {
        if (platformSessionInitialized) return
        platformSessionInitialized = true
        startInForeground(buildServiceNotification("正在启动后台接收服务"))
        registerWifiObserver()
        val contextReady = runCatching {
            val response = JSONObject(nativeInitializeAndroidContext())
            if (!response.optBoolean("ok")) {
                android.util.Log.e(
                    TAG,
                    "前台服务 Android Context 初始化失败: ${response.optString("error")}",
                )
            }
            response.optBoolean("ok")
        }.onFailure {
            android.util.Log.e(TAG, "前台服务 Android Context 初始化异常", it)
        }.getOrDefault(false)
        if (!contextReady) {
            android.util.Log.w(TAG, "文件仍可接收至 staging 目录，但暂时无法导出到所选系统目录")
            ServiceRecoveryDiagnostics.record(this, "android_context_failed")
        } else {
            ServiceRecoveryDiagnostics.record(this, "android_context_ready")
        }
        nativeRegisterServiceEventSink()
        ServiceRecoveryDiagnostics.record(this, "event_sink_registered")
        startHealthMonitor()
    }

    private fun rejectUnexpectedStart(startId: Int, reason: String) {
        notificationSessionActive = false
        SyncedNotificationPublisher.clear()
        previousNotificationRoutePeers = null
        invalidatePersistedSession(this)
        acceptedSessionToken = null
        jniReadyForSession = false
        exiting.set(true)
        android.util.Log.w(TAG, "拒绝 Service-only 启动：$reason")
        runCatching { stopForeground(STOP_FOREGROUND_REMOVE) }
        if (::notificationManager.isInitialized) {
            notificationManager.cancel(SERVICE_NOTIFICATION_ID)
        }
        stopSelfResult(startId)
    }

    private fun rejectUnauthorizedStickyRestart(startId: Int, reason: String) {
        android.util.Log.w(TAG, "拒绝未授权的 Sticky 重建：$reason")
        if (!stopSelfResult(startId)) {
            android.util.Log.i(TAG, "Sticky 重建停止请求已过期，保留更新的 Service 启动请求")
        }
    }

    private fun startCoreAsync(force: Boolean = false, selfHealing: Boolean = false): Boolean {
        if (exiting.get() || !jniReadyForSession || acceptedSessionToken == null) return false
        if (!force && !coreStartRequested.compareAndSet(false, true)) return false
        if (force) coreStartRequested.set(true)
        if (selfHealing) selfHealRestartInFlight = true
        notificationRoutePeers = JSONArray()
        notificationRouteSnapshotReady = false
        updateServiceNotification("正在启动后台接收服务")
        val submitted = runCatching {
            worker.execute {
                val response = runCatching { JSONObject(nativeStartCore(applicationInfo.dataDir)) }
                    .getOrElse { JSONObject().put("ok", false).put("error", it.message ?: "JNI 启动失败") }
                mainHandler.post {
                    if (selfHealing) selfHealRestartInFlight = false
                    applyCoreResult(response)
                }
            }
        }.onFailure {
            if (selfHealing) selfHealRestartInFlight = false
            android.util.Log.e(TAG, "提交核心启动任务失败", it)
        }.isSuccess
        ServiceRecoveryDiagnostics.record(
            this,
            if (submitted) "core_start_submitted" else "core_start_submit_failed",
            "self_healing=$selfHealing",
        )
        return submitted
    }

    private fun verifyStartupRecovery() {
        if (exiting.get() || !BackgroundRuntimeSettings.mayRecover(this)) return
        if (foregroundEstablished && platformSessionInitialized) return
        if (startupRecoveryAttempts >= MAX_STARTUP_RECOVERY_ATTEMPTS) {
            ServiceRecoveryDiagnostics.record(
                this,
                "startup_recovery_exhausted",
                "foreground=$foregroundEstablished platform=$platformSessionInitialized",
            )
            return
        }
        startupRecoveryAttempts++
        ServiceRecoveryDiagnostics.record(
            this,
            "startup_recovery_retry",
            "attempt=$startupRecoveryAttempts foreground=$foregroundEstablished platform=$platformSessionInitialized",
        )
        if (!platformSessionInitialized) {
            runCatching {
                startRecoverySession(applicationContext, "service-startup-watchdog")
            }.onFailure {
                android.util.Log.w(TAG, "重新请求后台服务恢复失败", it)
            }
        } else {
            startInForeground(buildServiceNotification("正在恢复后台接收服务"))
            startCoreAsync(force = true)
        }
        mainHandler.postDelayed(startupRecoveryRunnable, STARTUP_RECOVERY_RETRY_MS)
    }

    private fun startHealthMonitor() {
        mainHandler.removeCallbacks(healthCheckRunnable)
        mainHandler.postDelayed(healthCheckRunnable, HEALTH_CHECK_INTERVAL_MS)
    }

    private fun stopHealthMonitor() {
        mainHandler.removeCallbacks(healthCheckRunnable)
        resetSelfHealingState()
    }

    private fun resetSelfHealingState() {
        consecutiveUnhealthyChecks = 0
        selfHealAttempts = 0
        nextSelfHealAtElapsedMs = 0L
        selfHealRestartInFlight = false
    }

    private fun healthMonitorAllowed(): Boolean =
        !exiting.get() && platformSessionInitialized && jniReadyForSession &&
            acceptedSessionToken != null && BackgroundRuntimeSettings.mayRecover(this)

    private fun runHealthCheck() {
        if (!healthMonitorAllowed()) {
            stopHealthMonitor()
            return
        }
        runCatching {
            worker.execute {
                val state = runCatching {
                    JSONObject(nativeGetCoreStatus()).optJSONObject("status")?.optString("state")
                }.onFailure {
                    android.util.Log.w(TAG, "读取核心健康状态失败", it)
                }.getOrNull()
                mainHandler.post { handleHealthCheckResult(state) }
            }
        }.onFailure {
            android.util.Log.w(TAG, "提交核心健康检查失败", it)
        }
    }

    private fun handleHealthCheckResult(state: String?) {
        if (!healthMonitorAllowed()) {
            stopHealthMonitor()
            return
        }
        when {
            state == "RUNNING" -> resetSelfHealingState()
            isUnhealthyCoreState(state) || state == null -> {
                consecutiveUnhealthyChecks++
                val now = SystemClock.elapsedRealtime()
                if (consecutiveUnhealthyChecks >= UNHEALTHY_CHECKS_BEFORE_RESTART &&
                    !selfHealRestartInFlight && now >= nextSelfHealAtElapsedMs
                ) {
                    val delay = selfHealDelayMs(selfHealAttempts)
                    nextSelfHealAtElapsedMs = now + delay
                    selfHealAttempts++
                    android.util.Log.w(
                        TAG,
                        "核心连续异常，执行第 $selfHealAttempts 次自愈重启；下次最早等待 ${delay / 1000} 秒",
                    )
                    startCoreAsync(force = true, selfHealing = true)
                }
            }
            else -> consecutiveUnhealthyChecks = 0
        }
        if (healthMonitorAllowed()) {
            mainHandler.postDelayed(healthCheckRunnable, HEALTH_CHECK_INTERVAL_MS)
        }
    }

    private fun applyCoreResult(response: JSONObject) {
        val status = response.optJSONObject("status")
        notificationSessionActive = status?.optString("state") == "RUNNING" && jniReadyForSession && !exiting.get()
        if (!notificationSessionActive) SyncedNotificationPublisher.clear()
        when (status?.optString("state")) {
            "RUNNING" -> {
                resetSelfHealingState()
                ServiceRecoveryDiagnostics.record(this, "core_running")
                notificationManager.cancel(ERROR_NOTIFICATION_ID)
                renderNotificationRouteStatus()
                updateMulticastLock()
            }
            "STARTING" -> updateServiceNotification("正在启动后台接收服务")
            "ERROR" -> {
                ServiceRecoveryDiagnostics.record(
                    this,
                    "core_error",
                    status.optString("last_error_message", response.optString("error")),
                )
                releaseMulticastLock()
                updateServiceNotification("后台接收发生异常，点击查看")
                postErrorNotification(status.optString("last_error_message", response.optString("error")))
            }
            "STOPPED" -> {
                ServiceRecoveryDiagnostics.record(this, "core_stopped")
                releaseMulticastLock()
            }
        }
    }

    @Keep
    fun onNativeCoreEvent(eventJson: String) {
        mainHandler.post {
            if (exiting.get()) return@post
            runCatching {
                val envelope = JSONObject(eventJson)
                val event = envelope.getJSONObject("event")
                val type = event.getString("type")
                val payload = event.optJSONObject("payload") ?: JSONObject()
                when (type) {
                    "notification_route_status" -> {
                        notificationRoutePeers = payload.optJSONArray("peers") ?: JSONArray()
                        notificationRouteSnapshotReady = true
                        renderNotificationRouteStatus()
                    }
                    "peer_discovered" -> handleNotificationRouteConnection(payload)
                    "notification_received" -> SyncedNotificationPublisher.receive(this, payload)
                    "core_state_changed", "core_error" -> applyCoreResult(
                        JSONObject().put("ok", type != "core_error").put("status", payload)
                    )
                    "file_transfer_started", "file_transfer_progress" -> {
                        touchTransferWakeLock()
                    }
                    "file_transfer_completed" -> {
                        releaseTransferWakeLock()
                        withShortWakeLock {
                            if (payload.optString("from_id").isNotBlank() &&
                                envelope.optBoolean("notifications_enabled", true)
                            ) {
                                val soundEnabled = envelope.optBoolean("notification_sound_enabled", true)
                                if (envelope.optBoolean("ui_visible")) {
                                    playDataNotificationSoundIfAllowed(soundEnabled)
                                } else {
                                    postDataNotification(type, payload, soundEnabled)
                                }
                            }
                        }
                    }
                    "message_received", "file_offer_received" -> {
                        withShortWakeLock {
                            if (payload.optString("from_id").isNotBlank() &&
                                envelope.optBoolean("notifications_enabled", true)
                            ) {
                                val soundEnabled = envelope.optBoolean("notification_sound_enabled", true)
                                if (envelope.optBoolean("ui_visible")) {
                                    playDataNotificationSoundIfAllowed(soundEnabled)
                                } else {
                                    postDataNotification(type, payload, soundEnabled)
                                }
                            }
                        }
                    }
                }
            }.onFailure { error ->
                android.util.Log.e(TAG, "AndroidEventBridge 事件处理失败", error)
            }
        }
    }

    private fun beginExit() {
        ServiceRecoveryDiagnostics.record(this, "service_exit_requested")
        if (::batteryAlertController.isInitialized) {
            batteryAlertController.stop(clearState = true)
        }
        notificationSessionActive = false
        SyncedNotificationPublisher.clear()
        previousNotificationRoutePeers = null
        invalidatePersistedSession(this)
        if (!exiting.compareAndSet(false, true)) return
        mainHandler.removeCallbacks(startupRecoveryRunnable)
        foregroundEstablished = false
        stopHealthMonitor()
        coreStartRequested.set(false)
        if (!jniReadyForSession) {
            finishPlatformExit()
            return
        }
        shutdownWorker.execute {
            val firstStop = invokeNativeStopWithTimeout(force = false, STOP_CALL_TIMEOUT_MS)
            val finalStop = if (isVerifiedStopResponse(firstStop)) {
                firstStop
            } else {
                android.util.Log.w(TAG, "正常停止未满足后置条件，执行强制取消兜底: $firstStop")
                invokeNativeStopWithTimeout(force = true, FORCE_STOP_CALL_TIMEOUT_MS)
            }
            val sinkReleased = runCatching {
                val response = JSONObject(nativeUnregisterServiceEventSink())
                response.optBoolean("ok") && response.optInt("subscriber_count", -1) == 0
            }
                .onFailure { android.util.Log.e(TAG, "注销 AndroidEventBridge 失败", it) }
                .getOrDefault(false)
            val verified = isVerifiedStopResponse(finalStop) && sinkReleased
            if (verified) {
                android.util.Log.i(TAG, "核心停止后置条件已验证: $finalStop")
            } else {
                android.util.Log.e(TAG, "核心停止未能验证为完整退出: $finalStop")
            }
            worker.shutdownNow()
            nativeStopWorker.shutdownNow()
            mainHandler.post {
                unregisterWifiObserver()
                releaseTransferWakeLock()
                releaseMulticastLock()
                stopForeground(STOP_FOREGROUND_REMOVE)
                notificationManager.cancel(SERVICE_NOTIFICATION_ID)
                stopSelf()
            }
            shutdownWorker.shutdown()
        }
    }

    private fun finishPlatformExit() {
        unregisterWifiObserver()
        releaseTransferWakeLock()
        releaseMulticastLock()
        runCatching { stopForeground(STOP_FOREGROUND_REMOVE) }
        if (::notificationManager.isInitialized) {
            notificationManager.cancel(SERVICE_NOTIFICATION_ID)
        }
        stopSelf()
    }

    private fun invokeNativeStopWithTimeout(force: Boolean, timeoutMs: Long): JSONObject {
        val future = nativeStopWorker.submit<String> {
            if (force) nativeForceStopCore() else nativeStopCore()
        }
        return try {
            JSONObject(future.get(timeoutMs, TimeUnit.MILLISECONDS))
        } catch (error: TimeoutException) {
            future.cancel(true)
            JSONObject()
                .put("ok", false)
                .put("error_code", if (force) "FORCE_STOP_JNI_TIMEOUT" else "STOP_JNI_TIMEOUT")
                .put("error_message", "等待 Rust Core 停止超时")
        } catch (error: Exception) {
            JSONObject()
                .put("ok", false)
                .put("error_code", "STOP_JNI_FAILED")
                .put("error_message", error.message ?: "JNI 停止失败")
        }
    }

    private fun createNotificationChannels() {
        if (Build.VERSION.SDK_INT < 26) return
        notificationManager.createNotificationChannel(
            NotificationChannel(
                SERVICE_CHANNEL,
                "LQChat 后台接收",
                NotificationManager.IMPORTANCE_LOW,
            ).apply {
                description = "显示当前后台接收服务状态"
                setSound(null, null)
                setShowBadge(false)
            },
        )
        val soundAttributes = AudioAttributes.Builder()
            .setUsage(AudioAttributes.USAGE_NOTIFICATION)
            .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
            .build()
        notificationManager.createNotificationChannel(
            NotificationChannel(
                DATA_SOUND_CHANNEL,
                "LQChat 消息和文件音效",
                NotificationManager.IMPORTANCE_HIGH,
            ).apply {
                description = "新消息、图片、文件邀请和文件接收完成"
                enableVibration(true)
                setShowBadge(true)
                setSound(dataNotificationSoundUri(), soundAttributes)
                lockscreenVisibility = Notification.VISIBILITY_PRIVATE
            },
        )
        notificationManager.createNotificationChannel(
            NotificationChannel(
                DATA_SILENT_CHANNEL,
                "LQChat 消息和文件（静音）",
                NotificationManager.IMPORTANCE_HIGH,
            ).apply {
                description = "通知音效关闭时使用"
                enableVibration(true)
                setShowBadge(true)
                setSound(null, null)
                lockscreenVisibility = Notification.VISIBILITY_PRIVATE
            },
        )
        notificationManager.createNotificationChannel(
            NotificationChannel(
                MESSAGE_CHANNEL,
                "LQChat 连接与服务状态",
                NotificationManager.IMPORTANCE_HIGH,
            ).apply {
                description = "局域网设备连接、断开和服务错误"
                enableVibration(true)
                setShowBadge(true)
                lockscreenVisibility = Notification.VISIBILITY_PRIVATE
            },
        )
    }

    private fun startInForeground(notification: Notification): Boolean {
        val started = runCatching {
            if (Build.VERSION.SDK_INT >= 34) {
                startForeground(
                    SERVICE_NOTIFICATION_ID,
                    notification,
                    ServiceInfo.FOREGROUND_SERVICE_TYPE_REMOTE_MESSAGING or
                        ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE,
                )
            } else if (Build.VERSION.SDK_INT >= 29) {
                startForeground(
                    SERVICE_NOTIFICATION_ID,
                    notification,
                    ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE,
                )
            } else {
                startForeground(SERVICE_NOTIFICATION_ID, notification)
            }
        }.onFailure {
            ServiceRecoveryDiagnostics.record(this, "foreground_start_failed", it.toString())
            android.util.Log.e(TAG, "前台服务状态建立失败", it)
        }.isSuccess
        foregroundEstablished = started
        if (started) {
            ServiceRecoveryDiagnostics.record(this, "foreground_started")
        }
        return started
    }

    private fun servicePendingIntent(peerId: String? = null): PendingIntent {
        val intent = Intent(this, MainActivity::class.java).apply {
            flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
            if (!peerId.isNullOrBlank()) putExtra(EXTRA_PEER_ID, peerId)
        }
        return PendingIntent.getActivity(
            this,
            peerId?.hashCode() ?: 0,
            intent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
    }

    private fun buildServiceNotification(text: String): Notification =
        NotificationCompat.Builder(this, SERVICE_CHANNEL)
            .setSmallIcon(R.mipmap.ic_launcher)
            .setContentTitle("LQChat")
            .setContentText(text)
            .setContentIntent(servicePendingIntent())
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .build()

    private fun updateServiceNotification(text: String) {
        notificationManager.notify(SERVICE_NOTIFICATION_ID, buildServiceNotification(text))
    }

    private fun renderNotificationRouteStatus() {
        if (!notificationSessionActive || !::notificationManager.isInitialized) return
        val settings = NotificationSyncSettings.read(this)
        val text = if (notificationRouteSnapshotReady) {
            val currentPeers = monitoredNotificationRoutePeers(settings, notificationRoutePeers)
            if (notificationRouteStateInitialized) {
                newlyDisconnectedNotificationRoutePeers(previousNotificationRoutePeers, currentPeers)
                    .forEach(::postNotificationRouteDisconnected)
            }
            previousNotificationRoutePeers = mergeNotificationRoutePeerStates(
                previousNotificationRoutePeers,
                currentPeers,
                initializeUnknown = !notificationRouteStateInitialized,
            )
            notificationRouteStateInitialized = true
            persistNotificationRoutePeerStates()
            notificationRouteStatusText(settings, notificationRoutePeers)
        } else {
            RUNNING_NOTIFICATION_TEXT
        }
        updateServiceNotification(text)
    }

    private fun handleNotificationRouteConnection(payload: JSONObject) {
        val settings = NotificationSyncSettings.read(this)
        if (!shouldNotifyNotificationRouteConnection(previousNotificationRoutePeers, settings, payload)) return
        val transition = payload.optString("connection_transition")
        val id = payload.optString("id").takeIf { it.isNotBlank() } ?: return
        val peer = NotificationRoutePeerState(id, displayPeerName(payload.optString("name")), false)
        postNotificationRouteConnected(peer)
        previousNotificationRoutePeers = previousNotificationRoutePeers.orEmpty() + (id to peer)
        persistNotificationRoutePeerStates()
        android.util.Log.i(TAG, "通知转发设备接入：${peer.name} (${peer.id})，原因=$transition")
    }

    private fun loadNotificationRoutePeerStates(): Map<String, NotificationRoutePeerState>? =
        notificationRoutePeerStatesFromJson(
            getSharedPreferences(ROUTE_STATE_PREFS, Context.MODE_PRIVATE)
                .getString(ROUTE_STATE_SNAPSHOT, null),
        )

    private fun persistNotificationRoutePeerStates() {
        val states = previousNotificationRoutePeers ?: return
        getSharedPreferences(ROUTE_STATE_PREFS, Context.MODE_PRIVATE).edit()
            .putString(ROUTE_STATE_SNAPSHOT, notificationRoutePeerStatesToJson(states))
            .apply()
    }

    private fun postNotificationRouteDisconnected(peer: NotificationRoutePeerState) {
        recordNotificationRouteHistory(peer, connected = false)
        val body = notificationRouteDisconnectedText(peer.name)
        val notification = NotificationCompat.Builder(this, MESSAGE_CHANNEL)
            .setSmallIcon(R.mipmap.ic_launcher)
            .setContentTitle("LQChat 设备断开")
            .setContentText(body)
            .setStyle(NotificationCompat.BigTextStyle().bigText(body))
            .setContentIntent(servicePendingIntent(peer.id))
            .setAutoCancel(true)
            .setCategory(NotificationCompat.CATEGORY_STATUS)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setDefaults(NotificationCompat.DEFAULT_ALL)
            .setVisibility(NotificationCompat.VISIBILITY_PRIVATE)
            .build()
        runCatching {
            notificationManager.notify(disconnectNotificationIdFor(peer.id), notification)
        }.onFailure { android.util.Log.w(TAG, "设备断开提醒发送失败", it) }
    }

    private fun postNotificationRouteConnected(peer: NotificationRoutePeerState) {
        recordNotificationRouteHistory(peer, connected = true)
        val body = notificationRouteConnectedText(peer.name)
        val notification = NotificationCompat.Builder(this, MESSAGE_CHANNEL)
            .setSmallIcon(R.mipmap.ic_launcher)
            .setContentTitle("LQChat 连接成功")
            .setContentText(body)
            .setStyle(NotificationCompat.BigTextStyle().bigText(body))
            .setContentIntent(servicePendingIntent(peer.id))
            .setAutoCancel(true)
            .setCategory(NotificationCompat.CATEGORY_STATUS)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setDefaults(NotificationCompat.DEFAULT_ALL)
            .setVisibility(NotificationCompat.VISIBILITY_PRIVATE)
            .build()
        runCatching {
            notificationManager.notify(disconnectNotificationIdFor(peer.id), notification)
        }.onFailure { android.util.Log.w(TAG, "设备连接提醒发送失败", it) }
    }

    private fun recordNotificationRouteHistory(peer: NotificationRoutePeerState, connected: Boolean) {
        // Save the observation even if Android blocks the system notification.
        runCatching {
            NotificationSyncNative.recordRoute(
                JSONObject().put("peer_id", peer.id).put("peer_name", peer.name)
                    .put("event_id", UUID.randomUUID().toString()).put("connected", connected)
                    .put("post_time", System.currentTimeMillis()).toString(),
            )
        }.onFailure { android.util.Log.w(TAG, "设备连接状态记录失败", it) }
    }

    private fun dataNotificationSoundUri(): Uri = Uri.parse(
        "${ContentResolver.SCHEME_ANDROID_RESOURCE}://$packageName/${R.raw.lqchat_notification}",
    )

    private fun claimDataNotificationSound(enabled: Boolean): Boolean {
        val now = SystemClock.elapsedRealtime()
        if (!shouldPlayDataNotificationSound(enabled, lastNotificationSoundAtMs, now)) return false
        lastNotificationSoundAtMs = now
        return true
    }

    private fun playDataNotificationSoundIfAllowed(enabled: Boolean) {
        if (!claimDataNotificationSound(enabled)) return
        runCatching {
            releaseNotificationSound()
            val descriptor = resources.openRawResourceFd(R.raw.lqchat_notification)
            val player = MediaPlayer()
            player.setAudioAttributes(
                AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_NOTIFICATION)
                    .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                    .build(),
            )
            player.setDataSource(descriptor.fileDescriptor, descriptor.startOffset, descriptor.length)
            descriptor.close()
            player.setOnCompletionListener {
                if (activeNotificationSound === it) activeNotificationSound = null
                it.release()
            }
            player.setOnErrorListener { failed, _, _ ->
                if (activeNotificationSound === failed) activeNotificationSound = null
                failed.release()
                true
            }
            player.prepare()
            activeNotificationSound = player
            player.start()
        }.onFailure {
            lastNotificationSoundAtMs = 0L
            android.util.Log.w(TAG, "通知音效播放失败", it)
        }
    }

    private fun releaseNotificationSound() {
        activeNotificationSound?.let { player ->
            runCatching { if (player.isPlaying) player.stop() }
            runCatching { player.release() }
        }
        activeNotificationSound = null
    }

    private fun postDataNotification(type: String, payload: JSONObject, soundEnabled: Boolean) {
        val peerId = payload.optString("from_id")
        val fromName = payload.optString("from_name", "局域网设备")
        val body = when (type) {
            "file_offer_received" -> "请求发送文件：${payload.optString("file_name", payload.optString("content"))}"
            "file_transfer_completed" -> "文件接收完成：${payload.optString("file_name", payload.optString("content"))}"
            else -> payload.optString("content", "收到一条新消息")
        }
        val idSeed = payload.optLong("id", System.currentTimeMillis())
        val notificationId = notificationIdFor(idSeed)
        val playSound = claimDataNotificationSound(soundEnabled)
        val channel = if (playSound) DATA_SOUND_CHANNEL else DATA_SILENT_CHANNEL
        val builder = NotificationCompat.Builder(this, channel)
            .setSmallIcon(R.mipmap.ic_launcher)
            .setContentTitle(fromName)
            .setContentText(body)
            .setStyle(NotificationCompat.BigTextStyle().bigText(body))
            .setContentIntent(servicePendingIntent(peerId))
            .setAutoCancel(true)
            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setDefaults(NotificationCompat.DEFAULT_LIGHTS or NotificationCompat.DEFAULT_VIBRATE)
            .setVisibility(NotificationCompat.VISIBILITY_PRIVATE)
        if (Build.VERSION.SDK_INT < 26 && playSound) builder.setSound(dataNotificationSoundUri())
        val notification = builder.build()
        runCatching { notificationManager.notify(notificationId, notification) }
            .onFailure { android.util.Log.w(TAG, "消息通知发送失败", it) }
    }

    private fun postErrorNotification(message: String) {
        val notification = NotificationCompat.Builder(this, MESSAGE_CHANNEL)
            .setSmallIcon(R.mipmap.ic_launcher)
            .setContentTitle("LQChat 后台接收发生异常")
            .setContentText(message.ifBlank { "点击打开并重试" })
            .setContentIntent(servicePendingIntent())
            .setAutoCancel(true)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setDefaults(NotificationCompat.DEFAULT_ALL)
            .build()
        runCatching { notificationManager.notify(ERROR_NOTIFICATION_ID, notification) }
    }

    private fun registerWifiObserver() {
        val callback = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) = updateMulticastLock()
            override fun onLost(network: Network) = updateMulticastLock()
            override fun onCapabilitiesChanged(network: Network, capabilities: NetworkCapabilities) =
                updateMulticastLock()
        }
        networkCallback = callback
        runCatching {
            connectivityManager.registerNetworkCallback(
                NetworkRequest.Builder().addTransportType(NetworkCapabilities.TRANSPORT_WIFI).build(),
                callback,
            )
        }
    }

    private fun unregisterWifiObserver() {
        networkCallback?.let { callback -> runCatching { connectivityManager.unregisterNetworkCallback(callback) } }
        networkCallback = null
    }

    private fun isWifiAvailable(): Boolean {
        val active = connectivityManager.activeNetwork ?: return false
        return wifiManager.isWifiEnabled &&
            connectivityManager.getNetworkCapabilities(active)
                ?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true
    }

    private fun updateMulticastLock() {
        mainHandler.post {
            if (!jniReadyForSession || acceptedSessionToken == null) {
                releaseMulticastLock()
                return@post
            }
            val running = runCatching {
                JSONObject(nativeGetCoreStatus()).optJSONObject("status")?.optString("state") == "RUNNING"
            }.getOrDefault(false)
            if (!exiting.get() && running && isWifiAvailable()) acquireMulticastLock()
            else releaseMulticastLock()
        }
    }

    private fun acquireMulticastLock() {
        if (multicastLock?.isHeld == true) return
        multicastLock = (multicastLock ?: wifiManager.createMulticastLock("LANChat:Discovery").apply {
            setReferenceCounted(false)
        }).also { lock ->
            runCatching { lock.acquire() }
                .onSuccess { android.util.Log.i(TAG, "MulticastLock acquired") }
                .onFailure { android.util.Log.e(TAG, "MulticastLock acquire failed", it) }
        }
    }

    private fun releaseMulticastLock() {
        multicastLock?.let { lock ->
            if (lock.isHeld) runCatching { lock.release() }
            android.util.Log.i(TAG, "MulticastLock released")
        }
        multicastLock = null
    }

    private inline fun withShortWakeLock(block: () -> Unit) {
        val lock = powerManager.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "LANChat:EventProcessing")
            .apply { setReferenceCounted(false) }
        try {
            lock.acquire(SHORT_WAKE_TIMEOUT_MS)
            block()
        } finally {
            if (lock.isHeld) runCatching { lock.release() }
        }
    }

    private fun touchTransferWakeLock() {
        releaseTransferWakeLock()
        transferWakeLock = powerManager.newWakeLock(
            PowerManager.PARTIAL_WAKE_LOCK,
            "LANChat:ActiveTransfer",
        ).apply {
            setReferenceCounted(false)
            acquire(TRANSFER_WAKE_TIMEOUT_MS)
        }
    }

    private fun releaseTransferWakeLock() {
        transferWakeLock?.let { if (it.isHeld) runCatching { it.release() } }
        transferWakeLock = null
    }

}
