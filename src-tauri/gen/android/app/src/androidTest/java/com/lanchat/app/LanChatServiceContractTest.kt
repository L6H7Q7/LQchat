package com.lanchat.app

import android.content.ComponentName
import android.content.Context
import android.content.pm.ServiceInfo
import android.content.pm.ActivityInfo
import android.content.Intent
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.json.JSONObject

@RunWith(AndroidJUnit4::class)
class LanChatServiceContractTest {

    @Test
    fun safDownloadBridgeKeepsJniEntryPoint() {
        assertNotNull(
            AndroidDownloadStore::class.java.getDeclaredMethod(
                "exportReceivedFile",
                String::class.java,
                String::class.java,
                String::class.java,
            )
        )
        assertNotNull(MainActivity::class.java.getDeclaredMethod("launchDownloadDirectoryPicker"))
        assertNotNull(MainActivity::class.java.getDeclaredMethod("takeAndroidSharedFiles"))
    }

    @Test
    fun systemShareTargetsMainActivityDirectly() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val intent = Intent(Intent.ACTION_SEND).apply {
            type = "image/jpeg"
            setPackage(context.packageName)
        }
        val resolved = context.packageManager.resolveActivity(
            intent,
            android.content.pm.PackageManager.MATCH_DEFAULT_ONLY,
        )
        assertNotNull(resolved)
        assertEquals(MainActivity::class.java.name, resolved!!.activityInfo.name)
    }
    @Test
    fun serviceManifestKeepsSingleProcessRecoveryContract() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        @Suppress("DEPRECATION")
        val info = context.packageManager.getServiceInfo(
            ComponentName(context, LanChatForegroundService::class.java),
            0,
        )

        assertFalse(info.exported)
        assertTrue(info.flags and ServiceInfo.FLAG_STOP_WITH_TASK == 0)
        assertTrue(
            info.foregroundServiceType and ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE != 0,
        )
        if (android.os.Build.VERSION.SDK_INT >= 34) {
            assertTrue(
                info.foregroundServiceType and ServiceInfo.FOREGROUND_SERVICE_TYPE_REMOTE_MESSAGING != 0,
            )
        }

        @Suppress("DEPRECATION")
        val packageInfo = context.packageManager.getPackageInfo(
            context.packageName,
            android.content.pm.PackageManager.GET_SERVICES,
        )
        assertEquals(
            1,
            packageInfo.services.orEmpty().count {
                it.name == LanChatForegroundService::class.java.name
            },
        )
    }

    @Test
    fun serviceReturnsStickyAndDeclaresDefensiveExitCallbacks() {
        assertEquals(android.app.Service.START_STICKY, LanChatForegroundService.serviceStartMode())
        assertNotNull(
            LanChatForegroundService::class.java.getDeclaredMethod(
                "onTaskRemoved",
                Intent::class.java,
            ),
        )
        assertNotNull(LanChatForegroundService::class.java.getDeclaredMethod("onDestroy"))
    }

    @Test
    fun serviceOnlyStartWithoutVisibleSessionIsRejectedBeforeNativeAccess() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        LanChatForegroundService.invalidatePersistedSession(context)
        LanChatForegroundService.resetProcessNativeReadyForTest()

        assertFalse(
            LanChatForegroundService.isValidVisibleSessionRequest(
                context,
                token = null,
                nativeReady = false,
            ),
        )

        val staleToken = "stale-session-token"
        LanChatForegroundService.persistSession(context, staleToken)
        assertFalse(
            LanChatForegroundService.isValidVisibleSessionRequest(
                context,
                token = staleToken,
                nativeReady = false,
            ),
        )
        assertFalse(
            LanChatForegroundService.isValidVisibleSessionRequest(
                context,
                token = null,
                nativeReady = true,
            ),
        )
    }

    @Test
    fun visibleSessionTokenIsSingleProcessAndExitInvalidationIsIdempotent() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val token = "visible-session-token"
        assertTrue(LanChatForegroundService.persistSession(context, token))
        assertTrue(
            LanChatForegroundService.isValidVisibleSessionRequest(
                context,
                token = token,
                nativeReady = true,
            ),
        )
        assertTrue(LanChatForegroundService.invalidatePersistedSession(context))
        assertTrue(LanChatForegroundService.invalidatePersistedSession(context))
        assertEquals(null, LanChatForegroundService.currentSessionToken(context))
        assertFalse(
            LanChatForegroundService.isValidVisibleSessionRequest(
                context,
                token = token,
                nativeReady = true,
            ),
        )
    }

    @Test
    fun jniStopMethodsReturnStructuredJsonStrings() {
        val normal = LanChatForegroundService::class.java.getDeclaredMethod("nativeStopCore")
        val forced = LanChatForegroundService::class.java.getDeclaredMethod("nativeForceStopCore")
        val unregister = LanChatForegroundService::class.java.getDeclaredMethod(
            "nativeUnregisterServiceEventSink",
        )
        assertEquals(String::class.java, normal.returnType)
        assertEquals(String::class.java, forced.returnType)
        assertEquals(String::class.java, unregister.returnType)

        val verified = JSONObject()
            .put("ok", true)
            .put("status", JSONObject().put("state", "STOPPED"))
            .put("tasks_stopped", true)
            .put("ports_released", true)
            .put("resources_released", true)
        assertTrue(LanChatForegroundService.isVerifiedStopResponse(verified))
        verified.put("ports_released", false)
        assertFalse(LanChatForegroundService.isVerifiedStopResponse(verified))
    }

    @Test
    fun mainActivityUsesSoftwareRenderingToAvoidVendorGpuTeardownCrash() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        @Suppress("DEPRECATION")
        val info = context.packageManager.getActivityInfo(
            ComponentName(context, MainActivity::class.java),
            0,
        )
        assertTrue(info.flags and ActivityInfo.FLAG_HARDWARE_ACCELERATED == 0)
    }

    @Test
    fun serviceOwnsAndroidContextInitializationWithoutActivity() {
        val initialize = LanChatForegroundService::class.java.getDeclaredMethod(
            "nativeInitializeAndroidContext",
        )
        assertEquals(String::class.java, initialize.returnType)
    }

    @Test
    fun notificationIdsAreDeterministicAndSeparateFromServiceNotification() {
        val first = LanChatForegroundService.notificationIdFor(42L)
        val duplicate = LanChatForegroundService.notificationIdFor(42L)
        val different = LanChatForegroundService.notificationIdFor(43L)
        assertEquals(first, duplicate)
        assertNotEquals(first, different)
        assertTrue(first in 10_000 until 18_000)
        assertNotEquals(4_100, first)
        assertEquals("lanchat_messages_v2", LanChatForegroundService.MESSAGE_CHANNEL)
        assertEquals("lanchat_messages_files_sound_v1", LanChatForegroundService.DATA_SOUND_CHANNEL)
        assertEquals("lanchat_messages_files_silent_v1", LanChatForegroundService.DATA_SILENT_CHANNEL)
        assertFalse(LanChatForegroundService.shouldPlayDataNotificationSound(false, 0L, 1_000L))
        assertTrue(LanChatForegroundService.shouldPlayDataNotificationSound(true, 0L, 1_000L))
        assertFalse(LanChatForegroundService.shouldPlayDataNotificationSound(true, 1_000L, 1_799L))
        assertTrue(LanChatForegroundService.shouldPlayDataNotificationSound(true, 1_000L, 1_800L))
    }

    @Test
    fun foregroundNotificationTracksSettingsAndPeerConnectivity() {
        val onlinePeers = org.json.JSONArray()
            .put(JSONObject().put("id", "pc").put("name", "我的电脑")
                .put("is_offline", false).put("pushes_to_local", false))
            .put(JSONObject().put("id", "phone").put("name", "IQOO")
                .put("is_offline", false).put("pushes_to_local", true))
        val bothDirections = JSONObject().put("push_enabled", true).put("receive_enabled", true)
            .put("target_device_ids", org.json.JSONArray().put("pc"))
        assertEquals(
            "正在转发通知给 我的电脑；正在接收来自IQOO的通知",
            LanChatForegroundService.notificationRouteStatusText(bothDirections, onlinePeers),
        )

        onlinePeers.getJSONObject(0).put("is_offline", true)
        assertEquals(
            "局域网设备已断开",
            LanChatForegroundService.notificationRouteStatusText(bothDirections, onlinePeers),
        )

        val receiveOnly = JSONObject().put("receive_enabled", true)
            .put("target_device_ids", org.json.JSONArray())
        assertEquals(
            "正在接收来自IQOO的通知",
            LanChatForegroundService.notificationRouteStatusText(receiveOnly, onlinePeers),
        )
        onlinePeers.getJSONObject(1).put("is_offline", true)
        assertEquals(
            "局域网设备已断开",
            LanChatForegroundService.notificationRouteStatusText(receiveOnly, onlinePeers),
        )
    }

    @Test
    fun notificationRouteAlertsOnlyOnConnectionStateTransitions() {
        val settings = JSONObject().put("push_enabled", true).put("receive_enabled", true)
            .put("target_device_ids", org.json.JSONArray().put("pc"))
        val peers = org.json.JSONArray()
            .put(JSONObject().put("id", "pc").put("name", "我的电脑")
                .put("is_offline", false).put("pushes_to_local", false))
            .put(JSONObject().put("id", "phone").put("name", "IQOO")
                .put("is_offline", false).put("pushes_to_local", true))

        val initial = LanChatForegroundService.monitoredNotificationRoutePeers(settings, peers)
        assertTrue(LanChatForegroundService.newlyDisconnectedNotificationRoutePeers(null, initial).isEmpty())
        assertTrue(LanChatForegroundService.newlyConnectedNotificationRoutePeers(null, initial).isEmpty())
        assertTrue(
            LanChatForegroundService.newlyConnectedNotificationRoutePeers(initial, initial).isEmpty(),
        )

        val initialized = LanChatForegroundService.mergeNotificationRoutePeerStates(
            null,
            initial,
            initializeUnknown = true,
        )
        val temporarilyMissing = LanChatForegroundService.mergeNotificationRoutePeerStates(
            initialized,
            initial - "phone",
            initializeUnknown = false,
        )
        assertEquals(initialized["phone"], temporarilyMissing["phone"])
        assertTrue(
            LanChatForegroundService.newlyConnectedNotificationRoutePeers(temporarilyMissing, initial).isEmpty(),
        )
        val restored = LanChatForegroundService.notificationRoutePeerStatesFromJson(
            LanChatForegroundService.notificationRoutePeerStatesToJson(initial),
        )
        assertEquals(initial, restored)
        val rememberedOffline = initialized + (
            "pc" to initialized.getValue("pc").copy(isOffline = true)
        )
        assertTrue(
            LanChatForegroundService.mergeNotificationRoutePeerStates(
                rememberedOffline,
                initial,
                initializeUnknown = true,
            ).getValue("pc").isOffline,
        )
        assertTrue(
            LanChatForegroundService.mergeNotificationRoutePeerStates(
                rememberedOffline,
                initial,
                initializeUnknown = false,
            ).getValue("pc").isOffline,
        )
        assertEquals(null, LanChatForegroundService.notificationRoutePeerStatesFromJson(null))
        assertEquals(null, LanChatForegroundService.notificationRoutePeerStatesFromJson("bad-json"))

        peers.getJSONObject(0).put("is_offline", true)
        val pcOffline = LanChatForegroundService.monitoredNotificationRoutePeers(settings, peers)
        assertEquals(
            listOf("我的电脑"),
            LanChatForegroundService.newlyDisconnectedNotificationRoutePeers(initial, pcOffline).map { it.name },
        )
        assertTrue(
            LanChatForegroundService.newlyDisconnectedNotificationRoutePeers(pcOffline, pcOffline).isEmpty(),
        )

        peers.getJSONObject(0).put("is_offline", false)
        val pcOnlineAgain = LanChatForegroundService.monitoredNotificationRoutePeers(settings, peers)
        assertEquals(
            listOf("我的电脑"),
            LanChatForegroundService.newlyConnectedNotificationRoutePeers(pcOffline, pcOnlineAgain)
                .map { it.name },
        )
        peers.getJSONObject(0).put("is_offline", true)
        peers.getJSONObject(1).put("is_offline", true)
        val bothOffline = LanChatForegroundService.monitoredNotificationRoutePeers(settings, peers)
        assertEquals(
            setOf("我的电脑", "IQOO"),
            LanChatForegroundService.newlyDisconnectedNotificationRoutePeers(pcOnlineAgain, bothOffline)
                .map { it.name }.toSet(),
        )
        assertNotEquals(
            LanChatForegroundService.disconnectNotificationIdFor("pc"),
            LanChatForegroundService.disconnectNotificationIdFor("phone"),
        )
        assertEquals(
            "我的电脑设备已从局域网断开",
            LanChatForegroundService.notificationRouteDisconnectedText("我的电脑"),
        )
        assertEquals(
            "办公室设备已从局域网断开",
            LanChatForegroundService.notificationRouteDisconnectedText("办公室设备"),
        )
        assertEquals(
            "我的电脑已接入局域网",
            LanChatForegroundService.notificationRouteConnectedText("我的电脑"),
        )
        assertEquals(
            "办公室设备已接入局域网",
            LanChatForegroundService.notificationRouteConnectedText("办公室设备"),
        )
        assertTrue(
            LanChatForegroundService.isNotificationRouteConnectionRelevant(
                settings,
                JSONObject().put("id", "pc").put("pushes_to_local", false),
            ),
        )
        assertTrue(
            LanChatForegroundService.isNotificationRouteConnectionRelevant(
                settings,
                JSONObject().put("id", "phone").put("pushes_to_local", true),
            ),
        )
        assertFalse(
            LanChatForegroundService.isNotificationRouteConnectionRelevant(
                settings,
                JSONObject().put("id", "other").put("pushes_to_local", false),
            ),
        )
        val newPc = JSONObject().put("id", "pc").put("connection_transition", "new")
        val heartbeatPc = JSONObject().put("id", "pc").put("connection_transition", "unchanged")
        assertTrue(LanChatForegroundService.shouldNotifyNotificationRouteConnection(null, settings, newPc))
        assertFalse(LanChatForegroundService.shouldNotifyNotificationRouteConnection(null, settings, heartbeatPc))
        assertFalse(
            LanChatForegroundService.shouldNotifyNotificationRouteConnection(initial, settings, newPc),
        )
        assertTrue(
            LanChatForegroundService.shouldNotifyNotificationRouteConnection(
                rememberedOffline,
                settings,
                heartbeatPc,
            ),
        )
    }

    @Test
    fun foregroundServicePermissionsArePackaged() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val packageInfo = context.packageManager.getPackageInfo(
            context.packageName,
            android.content.pm.PackageManager.GET_PERMISSIONS,
        )
        val permissions = packageInfo.requestedPermissions?.toSet().orEmpty()
        assertTrue("android.permission.FOREGROUND_SERVICE" in permissions)
        assertTrue("android.permission.FOREGROUND_SERVICE_REMOTE_MESSAGING" in permissions)
        assertTrue("android.permission.FOREGROUND_SERVICE_CONNECTED_DEVICE" in permissions)
        assertTrue("android.permission.CHANGE_WIFI_MULTICAST_STATE" in permissions)
        assertTrue("android.permission.POST_NOTIFICATIONS" in permissions)
        assertTrue("android.permission.RECEIVE_BOOT_COMPLETED" in permissions)
    }

    @Test
    fun backgroundPolicySeparatesPreferencesFromCurrentStopBoundary() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val saved = BackgroundRuntimeSettings.save(context, JSONObject()
            .put("keep_running", true).put("start_on_boot", true).put("exclude_from_recents", true)
            .put("battery_alert_enabled", true)
            .put("battery_alert_interval_seconds", 12)
            .put("battery_alert_repeat_count", 4)
            .put("battery_alert_levels", org.json.JSONArray(listOf(25, 80))))
        assertTrue(saved.getBoolean("keep_running"))
        assertTrue(saved.getBoolean("battery_alert_enabled"))
        assertTrue(BackgroundRuntimeSettings.batteryAlertEnabled(context))
        assertEquals(12_000L, BackgroundRuntimeSettings.batteryAlertIntervalMs(context))
        assertEquals(4, BackgroundRuntimeSettings.batteryAlertRepeatCount(context))
        assertEquals(setOf(25, 80), BackgroundRuntimeSettings.batteryAlertLevels(context))
        BackgroundRuntimeSettings.beginUserSession(context)
        assertTrue(BackgroundRuntimeSettings.mayRecover(context))
        BackgroundRuntimeSettings.stopCurrentSession(context)
        assertTrue(BackgroundRuntimeSettings.mayStartOnBoot(context))
        assertFalse(BackgroundRuntimeSettings.mayRecover(context))
        BackgroundRuntimeSettings.beginBootSession(context, 123L)
        assertTrue(BackgroundRuntimeSettings.mayRecover(context))
    }

    @Test
    fun rootBackKeepsTheActivityProcessAliveOnlyForAnActiveBackgroundSession() {
        assertTrue(MainActivity.shouldMoveTaskToBackground(JSONObject()
            .put("keep_running", true).put("user_stopped", false)))
        assertFalse(MainActivity.shouldMoveTaskToBackground(JSONObject()
            .put("keep_running", false).put("user_stopped", false)))
        assertFalse(MainActivity.shouldMoveTaskToBackground(JSONObject()
            .put("keep_running", true).put("user_stopped", true)))
    }

    @Test
    fun batteryAlertsUseConfiguredLevelsAndChooseTheCrossingNearestCurrentLevel() {
        assertEquals(5_000L, BatteryAlertController.DEFAULT_REMINDER_INTERVAL_MS)
        assertEquals(3, BatteryAlertController.DEFAULT_REMINDER_COUNT)
        val levels = setOf(25, 50, 80, 100)
        assertEquals(50, BatteryAlertController.thresholdFor(-1, 50, levels))
        assertEquals(100, BatteryAlertController.thresholdFor(-1, 100, levels))
        assertEquals(80, BatteryAlertController.thresholdFor(49, 90, levels))
        assertEquals(50, BatteryAlertController.thresholdFor(90, 40, levels))
        assertEquals(100, BatteryAlertController.thresholdFor(99, 100, levels))
        assertEquals(null, BatteryAlertController.thresholdFor(-1, 30, levels))
        assertEquals(null, BatteryAlertController.thresholdFor(50, 50, levels))
        val targets = org.json.JSONArray(listOf("phone-b"))
        assertTrue(BatteryAlertController.shouldPush(org.json.JSONObject()
            .put("push_enabled", true)
            .put("lq_battery_push_enabled", true)
            .put("target_device_ids", targets)))
        assertFalse(BatteryAlertController.shouldPush(org.json.JSONObject()
            .put("push_enabled", false)
            .put("lq_battery_push_enabled", true)
            .put("target_device_ids", targets)))
        assertFalse(BatteryAlertController.shouldPush(org.json.JSONObject()
            .put("push_enabled", true)
            .put("lq_battery_push_enabled", false)
            .put("target_device_ids", targets)))
    }

    @Test
    fun batteryAlertSettingsRejectUnsafeRangesBeforePersisting() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        assertThrows(IllegalArgumentException::class.java) {
            BackgroundRuntimeSettings.save(context, JSONObject().put("battery_alert_repeat_count", 2).put("battery_alert_interval_seconds", 0))
        }
        assertThrows(IllegalArgumentException::class.java) {
            BackgroundRuntimeSettings.save(context, JSONObject().put("battery_alert_repeat_count", 21))
        }
        assertThrows(IllegalArgumentException::class.java) {
            BackgroundRuntimeSettings.save(
                context,
                JSONObject().put("battery_alert_levels", org.json.JSONArray(listOf(0, 101))),
            )
        }
    }

    @Test
    fun stickyRestartRequiresBackgroundPersistenceAndRespectsUserStop() {
        val context = ApplicationProvider.getApplicationContext<Context>()

        BackgroundRuntimeSettings.save(context, JSONObject()
            .put("keep_running", false).put("start_on_boot", true))
        BackgroundRuntimeSettings.beginUserSession(context)
        assertFalse(LanChatForegroundService.mayRecoverStickyRestart(context))

        BackgroundRuntimeSettings.save(context, JSONObject().put("keep_running", true))
        assertTrue(LanChatForegroundService.mayRecoverStickyRestart(context))

        BackgroundRuntimeSettings.stopCurrentSession(context)
        assertFalse(LanChatForegroundService.mayRecoverStickyRestart(context))
    }

    @Test
    fun packageReplaceRecoveryRequiresBackgroundPersistenceAndRespectsUserStop() {
        val context = ApplicationProvider.getApplicationContext<Context>()

        BackgroundRuntimeSettings.save(context, JSONObject()
            .put("keep_running", false).put("start_on_boot", true))
        BackgroundRuntimeSettings.beginUserSession(context)
        assertFalse(LanChatForegroundService.mayRecoverAfterPackageReplace(context))

        BackgroundRuntimeSettings.save(context, JSONObject().put("keep_running", true))
        assertTrue(LanChatForegroundService.mayRecoverAfterPackageReplace(context))

        BackgroundRuntimeSettings.stopCurrentSession(context)
        assertFalse(LanChatForegroundService.mayRecoverAfterPackageReplace(context))
    }

    @Test
    fun selfHealingOnlyTargetsStoppedOrFailedCoreAndUsesBoundedBackoff() {
        assertTrue(LanChatForegroundService.isUnhealthyCoreState("ERROR"))
        assertTrue(LanChatForegroundService.isUnhealthyCoreState("STOPPED"))
        assertFalse(LanChatForegroundService.isUnhealthyCoreState("STARTING"))
        assertFalse(LanChatForegroundService.isUnhealthyCoreState("RUNNING"))
        assertFalse(LanChatForegroundService.isUnhealthyCoreState(null))

        assertEquals(30_000L, LanChatForegroundService.selfHealDelayMs(0))
        assertEquals(60_000L, LanChatForegroundService.selfHealDelayMs(1))
        assertEquals(120_000L, LanChatForegroundService.selfHealDelayMs(2))
        assertEquals(240_000L, LanChatForegroundService.selfHealDelayMs(3))
        assertEquals(300_000L, LanChatForegroundService.selfHealDelayMs(4))
        assertEquals(300_000L, LanChatForegroundService.selfHealDelayMs(100))
    }

    @Test
    fun notificationListenerOnlyRequestsRecoveryWhenBackgroundSessionIsMissing() {
        assertTrue(LanChatNotificationListener.shouldRequestBackgroundRecovery(true, false))
        assertFalse(LanChatNotificationListener.shouldRequestBackgroundRecovery(true, true))
        assertFalse(LanChatNotificationListener.shouldRequestBackgroundRecovery(false, false))
        assertEquals(60_000L, LanChatNotificationListener.RECOVERY_CHECK_INTERVAL_MS)
    }

    @Test
    fun serviceRecoveryDiagnosticsPersistLastStageWithoutLaunchingActivity() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        ServiceRecoveryDiagnostics.record(context, "contract_test", "background-only")
        val diagnostics = ServiceRecoveryDiagnostics.read(context)
        assertEquals("contract_test", diagnostics.getString("stage"))
        assertEquals("background-only", diagnostics.getString("detail"))
        assertTrue(diagnostics.getLong("updated_at") > 0L)
        assertTrue(diagnostics.getInt("pid") > 0)
    }
}
