package com.lanchat.app

import android.app.ActivityManager
import android.app.NotificationManager
import android.content.ActivityNotFoundException
import android.content.ClipData
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Looper
import android.os.PowerManager
import android.provider.MediaStore
import android.provider.DocumentsContract
import android.provider.OpenableColumns
import android.provider.Settings
import android.util.Base64
import android.util.Size
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.drawable.BitmapDrawable
import android.webkit.JavascriptInterface
import android.webkit.MimeTypeMap
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.activity.OnBackPressedCallback
import androidx.activity.result.contract.ActivityResultContracts
import androidx.annotation.Keep
import androidx.core.content.FileProvider
import androidx.core.content.ContextCompat
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.io.ByteArrayOutputStream
import java.io.FileNotFoundException

// 通知 Intent 的 Key 常量（来自 tauri-plugin-notification）
private const val NOTIFICATION_INTENT_KEY = "NotificationId"
private const val NOTIFICATION_OBJ_INTENT_KEY = "LocalNotficationObject"
private const val ACTION_INTENT_KEY = "NotificationUserAction"

class MainActivity : TauriActivity() {
    companion object {
        internal fun shouldMoveTaskToBackground(settings: JSONObject): Boolean =
            settings.optBoolean("keep_running") && !settings.optBoolean("user_stopped")
    }

    @Keep
    fun notificationSettings(input: String): String = NotificationSyncSettings.command(this, input)

    @Keep
    fun setLocalDeviceName(name: String): String = LocalDeviceIdentity.save(this, name)

    // ─── JNI：Rust 侧的回调 ───
    private external fun nativeOnSafFileSelected(uri: String, name: String, size: Long)
    private external fun nativeSetUiVisibility(visible: Boolean)

    @Volatile private var pendingSharedFiles: List<SharedFileInfo>? = null
    @Volatile private var shareFrontendReady = false
    @Volatile private var shareNotificationDispatched = false
    private var webView: WebView? = null
    private var lastNotificationFromId: String? = null

    // ─── SAF 文件选择器（持久化权限） ───
    private val safPickerLauncher = registerForActivityResult(
        ActivityResultContracts.OpenDocument()
    ) { uri: Uri? ->
        if (uri != null) {
            handleSafSelectedFile(uri)
        }
    }

    private val safMultiPickerLauncher = registerForActivityResult(
        ActivityResultContracts.OpenMultipleDocuments()
    ) { uris: List<Uri> ->
        val result = JSONArray()
        uris.forEach { uri ->
            try {
                contentResolver.takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION)
            } catch (_: Exception) {
                // 部分文档提供方不支持持久化；Rust 发送端会复制到持久队列作为兜底。
            }
            result.put(queryAttachment(uri))
        }
        dispatchJsonEvent("android-files-selected", result)
    }

    private val downloadDirectoryPickerLauncher = registerForActivityResult(
        ActivityResultContracts.OpenDocumentTree()
    ) { uri: Uri? ->
        if (uri != null) {
            try {
                val takeFlags = Intent.FLAG_GRANT_READ_URI_PERMISSION or
                    Intent.FLAG_GRANT_WRITE_URI_PERMISSION
                contentResolver.takePersistableUriPermission(uri, takeFlags)
                val treeName = runCatching {
                    DocumentsContract.getTreeDocumentId(uri)
                        .substringAfter(':')
                        .ifBlank { "已选择的文件夹" }
                }.getOrDefault("已选择的文件夹")
                dispatchObjectEvent("android-download-directory-selected", JSONObject().apply {
                    put("uri", uri.toString())
                    put("label", treeName)
                })
            } catch (error: Exception) {
                dispatchObjectEvent("android-download-directory-error", JSONObject().apply {
                    put("message", error.message ?: "无法取得文件夹写入权限")
                })
            }
        }
    }

    private val mediaPermissionLauncher = registerForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions()
    ) { permissions ->
        if (permissions.values.any { it }) queryAndDispatchImages()
        else dispatchJsonEvent("android-media-images", JSONArray())
    }

    data class SharedFileInfo(
        val uri: String,
        val fileName: String,
        val fileSize: Long,
        val mimeType: String,
        val fd: Int,
    )

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        applyRecentsPolicy()
        AndroidDownloadStore.initialize(applicationContext)
        captureSharedFiles(intent)
        onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
            private var pending = false

            private fun leaveRootPage() {
                val backgroundSettings = BackgroundRuntimeSettings.read(this@MainActivity)
                if (shouldMoveTaskToBackground(backgroundSettings)) {
                    applyRecentsPolicy(backgroundSettings.optBoolean("exclude_from_recents"))
                    if (moveTaskToBack(true)) return
                }
                isEnabled = false
                try {
                    onBackPressedDispatcher.onBackPressed()
                } finally {
                    isEnabled = true
                }
            }

            override fun handleOnBackPressed() {
                if (pending) return
                val view = webView ?: findWebView(window.decorView)
                if (view == null) {
                    leaveRootPage()
                    return
                }
                pending = true
                // Same-document history is owned by the frontend, not WebView.canGoBack().
                view.evaluateJavascript("""
                    (() => {
                        const pages = ['#settings', '#permissions', '#push-apps', '#push-sources', '#notification-options', '#custom-reminder',
                            '#notifications', '#chat', '#chat-select', '#chat-attachment'];
                        if (history.length > 1 && pages.includes(location.hash)) {
                            history.back();
                            return true;
                        }
                        return false;
                    })()
                """.trimIndent()) { handled ->
                    pending = false
                    if (handled != "true" && !isFinishing && !isDestroyed) leaveRootPage()
                }
            }
        })

        // 开启 WebView 调试（方便 adb logcat 看到 JS console 输出）
        android.webkit.WebView.setWebContentsDebuggingEnabled(true)
        
        // 检测冷启动是否来自通知点击
        checkNotificationLaunch(intent)
    }

    override fun onStart() {
        super.onStart()
        nativeSetUiVisibility(true)
        // nativeSetUiVisibility 成功返回，证明本进程的 Rust 库已由可见 Activity 加载。
        // 只有这里签发的单次令牌可以激活 Service；Service-only 重建没有这个资格。
        LanChatForegroundService.startVisibleUserSession(this)
    }

    override fun onStop() {
        nativeSetUiVisibility(false)
        super.onStop()
    }

    @Keep
    fun retryBackgroundService() {
        LanChatForegroundService.retryVisibleUserSession(this)
    }

    @Keep
    fun getBackgroundRuntimeSettings(): String = BackgroundRuntimeSettings.read(this).toString()

    @Keep
    fun customReminderSettings(input: String): String = CustomReminderController.command(this, input)

    @Keep
    fun setBackgroundRuntimeSettings(input: String): String {
        val saved = BackgroundRuntimeSettings.save(this, JSONObject(input))
        applyRecentsPolicy(saved.optBoolean("exclude_from_recents"))
        LanChatForegroundService.refreshBatteryAlerts()
        CustomReminderController.refresh(this)
        return saved.toString()
    }

    private fun applyRecentsPolicy(enabled: Boolean = BackgroundRuntimeSettings.read(this).optBoolean("exclude_from_recents")) {
        val updateTask = Runnable {
            if (enabled) intent.addFlags(Intent.FLAG_ACTIVITY_EXCLUDE_FROM_RECENTS)
            else intent.removeFlags(Intent.FLAG_ACTIVITY_EXCLUDE_FROM_RECENTS)

            if (isFinishing || isDestroyed) return@Runnable
            runCatching {
                val currentTaskId = taskId
                val activityManager = getSystemService(ActivityManager::class.java)
                val currentTask = activityManager.appTasks.firstOrNull { appTask ->
                    val taskInfo = appTask.taskInfo
                    val appTaskId = if (Build.VERSION.SDK_INT >= 29) {
                        taskInfo.taskId
                    } else {
                        @Suppress("DEPRECATION")
                        taskInfo.id
                    }
                    appTaskId == currentTaskId
                }
                if (currentTask == null) {
                    android.util.Log.w("MainActivity", "未找到当前任务，稍后将在 Activity 重入时重试最近任务策略")
                } else {
                    currentTask.setExcludeFromRecents(enabled)
                }
            }.onFailure { error ->
                android.util.Log.w("MainActivity", "更新最近任务隐藏状态失败", error)
            }
        }

        if (Looper.myLooper() == Looper.getMainLooper()) updateTask.run()
        else runOnUiThread(updateTask)
    }

    @Keep
    fun stopBackgroundReceiveAndExit() {
        LanChatForegroundService.stopVisibleUserSession(this)
        finishAndRemoveTask()
    }

    @Keep
    fun getBatteryOptimizationState(): String {
        val manager = getSystemService(PowerManager::class.java)
        return if (manager.isIgnoringBatteryOptimizations(packageName)) "unrestricted" else "optimized"
    }

    @Keep
    fun openBatteryOptimizationSettings() {
        startActivity(Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS))
    }

    @Keep
    fun getNotificationPermissionState(): String {
        val notificationsEnabled = getSystemService(NotificationManager::class.java).areNotificationsEnabled()
        val runtimePermissionGranted = Build.VERSION.SDK_INT < 33 ||
            checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) ==
            android.content.pm.PackageManager.PERMISSION_GRANTED
        return if (notificationsEnabled && runtimePermissionGranted) "granted" else "denied"
    }

    /**
     * 供 Rust JNI 调用：打开 SAF 文件选择器
     */
    fun launchSafFilePicker() {
        runOnUiThread {
            safPickerLauncher.launch(arrayOf("*/*"))
        }
    }

    @Keep
    fun launchSafMultiFilePicker() {
        runOnUiThread { safMultiPickerLauncher.launch(arrayOf("*/*")) }
    }

    @Keep
    fun launchDownloadDirectoryPicker() {
        runOnUiThread { downloadDirectoryPickerLauncher.launch(null) }
    }

    @Keep
    fun loadMediaImages() {
        runOnUiThread {
            val permission = if (Build.VERSION.SDK_INT >= 33) android.Manifest.permission.READ_MEDIA_IMAGES
                else android.Manifest.permission.READ_EXTERNAL_STORAGE
            if (checkSelfPermission(permission) == android.content.pm.PackageManager.PERMISSION_GRANTED) {
                queryAndDispatchImages()
            } else {
                mediaPermissionLauncher.launch(arrayOf(permission))
            }
        }
    }

    @Keep
    fun loadInstalledApps() {
        Thread {
            val result = JSONArray()
            try {
                packageManager.getInstalledApplications(android.content.pm.PackageManager.GET_META_DATA)
                    .asSequence()
                    .filter { info ->
                        info.packageName != packageName &&
                            (info.flags and android.content.pm.ApplicationInfo.FLAG_SYSTEM) == 0 &&
                            !info.sourceDir.isNullOrBlank() && File(info.sourceDir).canRead()
                    }
                    .sortedBy { packageManager.getApplicationLabel(it).toString().lowercase() }
                    .forEach { info ->
                        val source = File(info.sourceDir)
                        result.put(JSONObject().apply {
                            put("name", "${packageManager.getApplicationLabel(info)}.apk")
                            put("label", packageManager.getApplicationLabel(info).toString())
                            put("packageName", info.packageName)
                            put("path", info.sourceDir)
                            put("size", source.length())
                            put("icon", drawableToDataUrl(packageManager.getApplicationIcon(info)))
                        })
                    }
            } catch (e: Exception) {
                println("[MainActivity] 读取 App 列表失败: ${e.message}")
            }
            dispatchJsonEvent("android-apps-loaded", result)
        }.start()
    }

    private fun queryAttachment(uri: Uri): JSONObject {
        var fileName = "unknown_file"
        var fileSize = 0L
        contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { cursor ->
            if (cursor.moveToFirst()) {
                val nameIndex = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                val sizeIndex = cursor.getColumnIndex(OpenableColumns.SIZE)
                if (nameIndex >= 0) fileName = cursor.getString(nameIndex) ?: fileName
                if (sizeIndex >= 0 && !cursor.isNull(sizeIndex)) fileSize = cursor.getLong(sizeIndex)
            }
        }
        return JSONObject().apply {
            put("uri", uri.toString())
            put("name", fileName)
            put("size", fileSize)
        }
    }

    private fun queryAndDispatchImages() {
        Thread {
            val result = JSONArray()
            val projection = arrayOf(
                MediaStore.Images.Media._ID,
                MediaStore.Images.Media.DISPLAY_NAME,
                MediaStore.Images.Media.SIZE,
                MediaStore.Images.Media.DATE_MODIFIED,
                MediaStore.Images.Media.BUCKET_DISPLAY_NAME
            )
            try {
                contentResolver.query(
                    MediaStore.Images.Media.EXTERNAL_CONTENT_URI,
                    projection,
                    null,
                    null,
                    "${MediaStore.Images.Media.DATE_MODIFIED} DESC"
                )?.use { cursor ->
                    val idIndex = cursor.getColumnIndexOrThrow(MediaStore.Images.Media._ID)
                    val nameIndex = cursor.getColumnIndexOrThrow(MediaStore.Images.Media.DISPLAY_NAME)
                    val sizeIndex = cursor.getColumnIndexOrThrow(MediaStore.Images.Media.SIZE)
                    val bucketIndex = cursor.getColumnIndex(MediaStore.Images.Media.BUCKET_DISPLAY_NAME)
                    var count = 0
                    while (cursor.moveToNext() && count < 120) {
                        val uri = Uri.withAppendedPath(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, cursor.getLong(idIndex).toString())
                        val thumbnail = try {
                            if (Build.VERSION.SDK_INT >= 29) bitmapToDataUrl(contentResolver.loadThumbnail(uri, Size(240, 240), null)) else ""
                        } catch (_: Exception) { "" }
                        result.put(JSONObject().apply {
                            put("uri", uri.toString())
                            put("name", cursor.getString(nameIndex) ?: "image_$count.jpg")
                            put("size", cursor.getLong(sizeIndex))
                            put("album", if (bucketIndex >= 0) cursor.getString(bucketIndex) ?: "其他" else "其他")
                            put("thumbnail", thumbnail)
                        })
                        count++
                    }
                }
            } catch (e: Exception) {
                println("[MainActivity] 读取相册失败: ${e.message}")
            }
            dispatchJsonEvent("android-media-images", result)
        }.start()
    }

    private fun bitmapToDataUrl(bitmap: Bitmap): String {
        val output = ByteArrayOutputStream()
        bitmap.compress(Bitmap.CompressFormat.JPEG, 72, output)
        return "data:image/jpeg;base64," + Base64.encodeToString(output.toByteArray(), Base64.NO_WRAP)
    }

    private fun drawableToDataUrl(drawable: android.graphics.drawable.Drawable): String {
        val bitmap = if (drawable is BitmapDrawable) drawable.bitmap else {
            val width = drawable.intrinsicWidth.coerceAtLeast(1).coerceAtMost(128)
            val height = drawable.intrinsicHeight.coerceAtLeast(1).coerceAtMost(128)
            Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888).also {
                val canvas = Canvas(it)
                drawable.setBounds(0, 0, canvas.width, canvas.height)
                drawable.draw(canvas)
            }
        }
        val output = ByteArrayOutputStream()
        bitmap.compress(Bitmap.CompressFormat.PNG, 90, output)
        return "data:image/png;base64," + Base64.encodeToString(output.toByteArray(), Base64.NO_WRAP)
    }

    private fun dispatchJsonEvent(name: String, payload: JSONArray) {
        if (webView == null) webView = findWebView(window.decorView)
        runOnUiThread {
            webView?.evaluateJavascript(
                "window.dispatchEvent(new CustomEvent(${JSONObject.quote(name)}, {detail: $payload}));",
                null
            )
        }
    }

    private fun handleSafSelectedFile(uri: Uri) {
        try {
            // 持久化读取权限
            val takeFlags = Intent.FLAG_GRANT_READ_URI_PERMISSION
            contentResolver.takePersistableUriPermission(uri, takeFlags)

            // 提取文件名和大小
            var fileName = "unknown_file"
            var fileSize: Long = 0
            contentResolver.query(uri, null, null, null, null)?.use { cursor ->
                if (cursor.moveToFirst()) {
                    val nameIndex = cursor.getColumnIndex(android.provider.OpenableColumns.DISPLAY_NAME)
                    val sizeIndex = cursor.getColumnIndex(android.provider.OpenableColumns.SIZE)
                    if (nameIndex >= 0) fileName = cursor.getString(nameIndex)
                    if (sizeIndex >= 0) fileSize = cursor.getLong(sizeIndex)
                }
            }

            // 回调 Rust 侧，走 Tauri 事件总线广播给前端
            nativeOnSafFileSelected(uri.toString(), fileName, fileSize)

        } catch (e: Exception) {
            println("[MainActivity] SAF 文件选择持久化失败: ${e.message}")
            e.printStackTrace()
        }
    }

    private fun checkNotificationLaunch(intent: Intent?) {
        if (intent == null) return
        val sourceDeviceId = intent.getStringExtra(SyncedNotificationPublisher.EXTRA_SOURCE_DEVICE_ID)
        if (!sourceDeviceId.isNullOrBlank() && intent.hasExtra(SyncedNotificationPublisher.EXTRA_HISTORY_RECORD_ID)) {
            notifySyncedNotificationClicked(intent)
            return
        }
        intent.getStringExtra(LanChatForegroundService.EXTRA_PEER_ID)?.let { peerId ->
            window.decorView.postDelayed({ notifyPeerOpened(peerId) }, 1000)
            return
        }
        val action = intent.getStringExtra(ACTION_INTENT_KEY)
        if (action == "tap") {
            println("[MainActivity] 冷启动来自通知点击")
            // 延迟等 WebView 就绪后再通知 JS
            window.decorView.postDelayed({
                notifyNotificationClicked()
            }, 1000)
        }
    }

    @Keep
    @Synchronized
    fun takeAndroidSharedFiles(): String {
        shareFrontendReady = true
        val files = pendingSharedFiles.orEmpty()
        pendingSharedFiles = null
        val jsonArray = JSONArray()
        files.forEach { file ->
            val jsonObj = JSONObject().apply {
                put("uri", file.uri)
                put("fileName", file.fileName)
                put("fileSize", file.fileSize)
                put("mimeType", file.mimeType)
                put("fd", file.fd)
            }
            jsonArray.put(jsonObj)
        }
        println("[MainActivity] 前端领取 ${files.size} 个分享文件")
        return jsonArray.toString()
    }

    private fun captureSharedFiles(sourceIntent: Intent?): Boolean {
        val action = sourceIntent?.action ?: return false
        if (action != Intent.ACTION_SEND && action != Intent.ACTION_SEND_MULTIPLE) return false
        val mimeType = sourceIntent.type ?: "application/octet-stream"
        @Suppress("DEPRECATION")
        val uris = when (action) {
            Intent.ACTION_SEND -> listOfNotNull(sourceIntent.getParcelableExtra<Uri>(Intent.EXTRA_STREAM))
            else -> sourceIntent.getParcelableArrayListExtra<Uri>(Intent.EXTRA_STREAM).orEmpty()
        }
        val files = uris.mapNotNull { readSharedFile(it, mimeType) }
        if (files.isEmpty()) return false
        synchronized(this) {
            releasePendingSharedFilesLocked()
            pendingSharedFiles = files
            shareNotificationDispatched = false
        }
        println("[MainActivity] 已接收 ${files.size} 个系统分享文件")
        return true
    }

    private fun readSharedFile(uri: Uri, mimeType: String): SharedFileInfo? = runCatching {
        var fileName = uri.lastPathSegment ?: "shared_file"
        var fileSize = 0L
        contentResolver.query(
            uri,
            arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE),
            null,
            null,
            null,
        )?.use { cursor ->
            if (cursor.moveToFirst()) {
                val nameIndex = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                val sizeIndex = cursor.getColumnIndex(OpenableColumns.SIZE)
                if (nameIndex >= 0 && !cursor.isNull(nameIndex)) fileName = cursor.getString(nameIndex)
                if (sizeIndex >= 0 && !cursor.isNull(sizeIndex)) fileSize = cursor.getLong(sizeIndex)
            }
        }
        val descriptor = contentResolver.openFileDescriptor(uri, "r")
            ?: throw FileNotFoundException("无法打开分享文件")
        val fd = descriptor.detachFd()
        SharedFileInfo(uri.toString(), fileName, fileSize, mimeType, fd)
    }.onFailure {
        println("[MainActivity] 读取分享文件失败: ${it.message}")
    }.getOrNull()

    @Synchronized
    private fun notifyShareAvailable() {
        if (!shareFrontendReady || pendingSharedFiles.isNullOrEmpty() || shareNotificationDispatched) return
        if (webView == null) webView = findWebView(window.decorView)
        val currentWebView = webView ?: return
        shareNotificationDispatched = true
        currentWebView.evaluateJavascript(
            "window.dispatchEvent(new CustomEvent('android-share-received'));",
            null,
        )
    }

    @Synchronized
    private fun releasePendingSharedFilesLocked() {
        pendingSharedFiles.orEmpty().forEach { file ->
            if (file.fd >= 0) runCatching { android.os.ParcelFileDescriptor.adoptFd(file.fd).close() }
        }
        pendingSharedFiles = null
        shareNotificationDispatched = false
    }

    override fun onDestroy() {
        nativeSetUiVisibility(false)
        synchronized(this) { releasePendingSharedFilesLocked() }
        super.onDestroy()
    }

    private fun findWebView(view: android.view.View): WebView? {
        println("[MainActivity] 检查 View: ${view.javaClass.name}")
        
        if (view is WebView) {
            println("[MainActivity] 找到 WebView: ${view.javaClass.name}")
            return view
        }
        if (view is android.view.ViewGroup) {
            for (i in 0 until view.childCount) {
                val child = view.getChildAt(i)
                val result = findWebView(child)
                if (result != null) {
                    return result
                }
            }
        }
        return null
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        applyRecentsPolicy()
        if (captureSharedFiles(intent)) {
            notifyShareAvailable()
            return
        }
        
        val sourceDeviceId = intent.getStringExtra(SyncedNotificationPublisher.EXTRA_SOURCE_DEVICE_ID)
        if (!sourceDeviceId.isNullOrBlank() && intent.hasExtra(SyncedNotificationPublisher.EXTRA_HISTORY_RECORD_ID)) {
            notifySyncedNotificationClicked(intent)
            return
        }

        intent.getStringExtra(LanChatForegroundService.EXTRA_PEER_ID)?.let { peerId ->
            notifyPeerOpened(peerId)
            return
        }

        // 检测通知点击：NotificationUserAction == "tap" 表示通知被点击
        val action = intent.getStringExtra(ACTION_INTENT_KEY)
        if (action == "tap") {
            println("[MainActivity] 通知被点击")
            // 通知 JS 从 localStorage 读取 from_id 并导航
            notifyNotificationClicked()
        }
    }
    
    override fun onResume() {
        super.onResume()
        CustomReminderController.refresh(this)
        println("[MainActivity] onResume 被调用")
        notifyShareAvailable()
    }
    

    private fun notifyNotificationClicked() {
        // 通知前端从 localStorage 读取 pendingFromId 并导航
        notifyWebViewWithRetry(0, """
            (function() {
                console.log('[MainActivity] 通知被点击，尝试导航');
                var fromId = localStorage.getItem('pendingNotificationFromId');
                if (fromId) {
                    localStorage.removeItem('pendingNotificationFromId');
                    window.dispatchEvent(new CustomEvent('notification-tapped', {detail: {fromId: fromId}}));
                }
            })();
        """.trimIndent())
    }

    private fun notifySyncedNotificationClicked(intent: Intent) {
        val detail = JSONObject().apply {
            put("recordId", intent.getStringExtra(SyncedNotificationPublisher.EXTRA_HISTORY_RECORD_ID).orEmpty())
            put("sourceDeviceId", intent.getStringExtra(SyncedNotificationPublisher.EXTRA_SOURCE_DEVICE_ID).orEmpty())
            put("package", intent.getStringExtra(SyncedNotificationPublisher.EXTRA_NOTIFICATION_PACKAGE).orEmpty())
            put("notificationKey", intent.getStringExtra(SyncedNotificationPublisher.EXTRA_NOTIFICATION_KEY).orEmpty())
        }
        notifyWebViewWithRetry(0, "window.dispatchEvent(new CustomEvent('synced-notification-tapped', {detail: $detail}));")
    }

    private fun dispatchObjectEvent(name: String, payload: JSONObject) {
        if (webView == null) webView = findWebView(window.decorView)
        runOnUiThread {
            webView?.evaluateJavascript(
                "window.dispatchEvent(new CustomEvent(${JSONObject.quote(name)}, {detail: $payload}));",
                null
            )
        }
    }

    private fun notifyPeerOpened(peerId: String) {
        notifyWebViewWithRetry(0, """
            window.dispatchEvent(new CustomEvent('notification-tapped', {
                detail: {fromId: ${JSONObject.quote(peerId)}}
            }));
        """.trimIndent())
    }
    
    private fun notifyWebView() {
        println("[MainActivity] 准备通知 WebView")
        
        // 使用递归重试机制
        notifyWebViewWithRetry(0)
    }
    
    private fun notifyWebViewWithRetry(attempt: Int, jsCode: String = """
        (function() {
            console.log('[MainActivity] 触发 android-share-received 事件');
            window.dispatchEvent(new CustomEvent('android-share-received'));
        })();
    """.trimIndent()) {
        val maxAttempts = 10
        val delayMs = 500L
        
        if (attempt >= maxAttempts) {
            println("[MainActivity] 达到最大重试次数，放弃通知")
            return
        }
        
        window.decorView.postDelayed({
            // 尝试重新查找 WebView
            if (webView == null) {
                webView = findWebView(window.decorView)
            }
            
            if (webView != null) {
                println("[MainActivity] WebView 已就绪（尝试 ${attempt + 1}），发送事件")
                runOnUiThread {
                    try {
                        webView?.evaluateJavascript(jsCode,
                            { result ->
                                println("[MainActivity] JavaScript 执行结果: $result")
                            }
                        )
                        println("[MainActivity] 已触发 android-share-received 事件")
                    } catch (e: Exception) {
                        println("[MainActivity] 执行 JavaScript 失败: ${e.message}")
                    }
                }
            } else {
                println("[MainActivity] WebView 未就绪（尝试 ${attempt + 1}），继续重试...")
                notifyWebViewWithRetry(attempt + 1, jsCode)
            }
        }, delayMs)
    }

    private data class ExternalFile(val uri: Uri, val mimeType: String)

    private fun storedReceivedFileUri(file: File): Uri? {
        val receivedRoot = File(applicationInfo.dataDir, "received_files").canonicalFile
        val canonicalFile = file.canonicalFile
        if (canonicalFile.parentFile != receivedRoot) return null
        return Uri.Builder()
            .scheme("content")
            .authority("$packageName.fdprovider")
            .appendPath("stored")
            .appendPath(canonicalFile.name)
            .build()
    }

    private fun resolveExternalFile(filePath: String): ExternalFile {
        val uri = if (filePath.startsWith("content://")) {
            Uri.parse(filePath)
        } else {
            val file = File(filePath).canonicalFile
            require(file.isFile) { "文件不存在或不可读取: $filePath" }
            storedReceivedFileUri(file)
                ?: FileProvider.getUriForFile(this, "$packageName.fileprovider", file)
        }

        val extension = (uri.lastPathSegment ?: filePath)
            .substringAfterLast('.', "")
            .lowercase()
        val mimeType = when (extension) {
            "apk" -> "application/vnd.android.package-archive"
            else -> contentResolver.getType(uri)
                ?.takeUnless { it.isBlank() || it == "*/*" }
                ?: MimeTypeMap.getSingleton().getMimeTypeFromExtension(extension)
                ?: "application/octet-stream"
        }
        return ExternalFile(uri, mimeType)
    }

    private fun ensureExternalFileReadable(target: ExternalFile) {
        val descriptor = contentResolver.openAssetFileDescriptor(target.uri, "r")
            ?: throw FileNotFoundException("文件已删除或无法读取")
        descriptor.use { /* 只做存在性和权限预检，实际读取由目标应用完成。 */ }
    }

    private fun errorResult(action: String, error: Exception): String {
        val message = when (error) {
            is ActivityNotFoundException -> "没有找到可处理此文件的应用"
            is FileNotFoundException -> "文件已删除或无法读取"
            is SecurityException -> "文件访问权限已失效，请重新接收或选择该文件"
            else -> error.message ?: error.javaClass.simpleName
        }
        println("[MainActivity] ${action}失败: $message")
        error.printStackTrace()
        return "ERROR:$message"
    }

    // 由 Rust JNI 调用。返回值会一路传回前端，避免原生失败时界面毫无反馈。
    @Keep
    fun openFile(filePath: String): String {
        return try {
            val target = resolveExternalFile(filePath)
            ensureExternalFileReadable(target)
            val viewIntent = Intent(Intent.ACTION_VIEW).apply {
                setDataAndType(target.uri, target.mimeType)
                clipData = ClipData.newUri(contentResolver, "open_file", target.uri)
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }
            startActivity(Intent.createChooser(viewIntent, "选择应用打开").apply {
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            })
            println("[MainActivity] 打开文件选择器已启动: ${target.uri}, ${target.mimeType}")
            "OK"
        } catch (error: Exception) {
            errorResult("打开文件", error)
        }
    }

    @Keep
    fun shareFile(filePath: String): String {
        return try {
            val target = resolveExternalFile(filePath)
            ensureExternalFileReadable(target)
            val sendIntent = Intent(Intent.ACTION_SEND).apply {
                type = target.mimeType
                putExtra(Intent.EXTRA_STREAM, target.uri)
                clipData = ClipData.newUri(contentResolver, "share_file", target.uri)
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }
            startActivity(Intent.createChooser(sendIntent, "分享文件").apply {
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            })
            println("[MainActivity] 分享选择器已启动: ${target.uri}, ${target.mimeType}")
            "OK"
        } catch (error: Exception) {
            errorResult("分享文件", error)
        }
    }

    @Keep
    fun getFileState(filePath: String): String {
        return try {
            val target = resolveExternalFile(filePath)
            ensureExternalFileReadable(target)
            "AVAILABLE"
        } catch (_: FileNotFoundException) {
            "DELETED"
        } catch (error: IllegalArgumentException) {
            val localFileExists = !filePath.startsWith("content://") && File(filePath).isFile
            if (localFileExists) {
                "PROVIDER_ERROR:${error.message ?: "文件共享配置错误"}"
            } else {
                "DELETED"
            }
        } catch (_: SecurityException) {
            "INACCESSIBLE"
        } catch (error: Exception) {
            "ERROR:${error.message ?: error.javaClass.simpleName}"
        }
    }
}
