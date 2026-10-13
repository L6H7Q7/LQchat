package com.lanchat.app

import android.content.ContentValues
import android.content.Context
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteOpenHelper
import org.json.JSONArray
import org.json.JSONObject
import java.util.UUID

/** Durable outbox for local observations, including alarms fired without a WebView/native session. */
object LocalNotificationHistory {
    private const val RETENTION = 7 * 24 * 60 * 60 * 1000L
    private class Store(context: Context) : SQLiteOpenHelper(context, "local_notification_history_v1.db", null, 1) {
        override fun onCreate(db: SQLiteDatabase) {
            db.execSQL("CREATE TABLE pending (id TEXT PRIMARY KEY, at INTEGER NOT NULL, payload TEXT NOT NULL)")
        }
        override fun onUpgrade(db: SQLiteDatabase, old: Int, new: Int) = Unit
    }
    @Synchronized fun record(c: Context, title: String, text: String, key: String) {
        val now = System.currentTimeMillis()
        val id = UUID.randomUUID().toString()
        val message = JSONObject().put("msg_type", "notification").put("event_id", id)
            .put("source_device_id", "").put("target_device_id", "")
            .put("package", c.packageName).put("app_name", "LQChat")
            .put("title", title).put("text", text).put("notification_key", key).put("post_time", now)
        Store(c.applicationContext).use { store ->
            store.writableDatabase.apply {
                delete("pending", "at < ?", arrayOf((now - RETENTION).toString()))
                insertOrThrow("pending", null, ContentValues().apply {
                    put("id", id); put("at", now); put("payload", message.toString())
                })
            }
        }
        // Readiness is optional: storage above succeeds even when the native library is absent.
        runCatching {
            if (LanChatForegroundService.notificationSessionReady()) NotificationSyncNative.localHistoryChanged()
        }
    }
    @Synchronized fun pending(c: Context): JSONArray {
        val result = JSONArray()
        Store(c.applicationContext).use { store ->
            val db = store.writableDatabase
            db.delete("pending", "at < ?", arrayOf((System.currentTimeMillis() - RETENTION).toString()))
            db.rawQuery("SELECT payload FROM pending ORDER BY at,id LIMIT 200", null).use { cursor ->
                while (cursor.moveToNext()) result.put(JSONObject(cursor.getString(0)))
            }
        }
        return result
    }
    @Synchronized fun acknowledge(c: Context, ids: JSONArray) {
        Store(c.applicationContext).use { store ->
            val db = store.writableDatabase
            db.beginTransaction()
            try {
                for (i in 0 until ids.length()) db.delete("pending", "id=?", arrayOf(ids.getString(i)))
                db.setTransactionSuccessful()
            } finally { db.endTransaction() }
        }
    }
}
