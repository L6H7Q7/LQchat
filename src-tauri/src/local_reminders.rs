//! Windows local reminders. Runs for the app lifetime, independently of the LAN session/WebView.
use crate::notification_sync::{self, Notification};
use crate::reminder_schedule::{self, Options, Push, Reminder};
use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{Pool, Sqlite};
use std::collections::HashSet;

const OPTIONS_KEY: &str = "windows_reminder_options_v1";
const REMINDERS_KEY: &str = "windows_custom_reminders_v1";
const MAX_LATENESS: i64 = 5 * 60 * 1000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeSettings {
    pub battery_alert_enabled: bool,
    pub battery_alert_interval_seconds: i64,
    pub battery_alert_repeat_count: u32,
    pub battery_alert_levels: Vec<i32>,
}
impl Default for RuntimeSettings {
    fn default() -> Self {
        Self {
            battery_alert_enabled: false,
            battery_alert_interval_seconds: 0,
            battery_alert_repeat_count: 1,
            battery_alert_levels: vec![50, 100],
        }
    }
}
impl RuntimeSettings {
    fn options(&self) -> Options {
        Options {
            repeat: self.battery_alert_repeat_count,
            repeat_gap: self.battery_alert_interval_seconds,
        }
    }
    fn validate(&mut self) -> Result<(), String> {
        if self.battery_alert_repeat_count == 1 {
            self.battery_alert_interval_seconds = 0;
        }
        reminder_schedule::validate(&[], &self.options())?;
        self.battery_alert_levels.sort_unstable();
        self.battery_alert_levels.dedup();
        if self.battery_alert_levels.is_empty()
            || self.battery_alert_levels.len() > 10
            || self
                .battery_alert_levels
                .iter()
                .any(|level| !(1..=100).contains(level))
        {
            return Err("请设置 1～10 个有效提醒电量".into());
        }
        Ok(())
    }
}
pub fn battery_level() -> Option<i32> {
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    let mut status = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut status) }.ok()?;
    if status.BatteryFlag == 255 || status.BatteryFlag & 128 != 0 || status.BatteryLifePercent > 100
    {
        None
    } else {
        Some(status.BatteryLifePercent as i32)
    }
}
async fn read<T: serde::de::DeserializeOwned + Default>(
    pool: &Pool<Sqlite>,
    key: &str,
) -> Result<T, String> {
    let text = sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key=?")
        .bind(key)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;
    match text {
        Some(text) => serde_json::from_str(&text).map_err(|e| format!("提醒配置读取失败：{e}")),
        None => Ok(T::default()),
    }
}
async fn write<T: Serialize>(pool: &Pool<Sqlite>, key: &str, value: &T) -> Result<(), String> {
    let text = serde_json::to_string(value).map_err(|e| e.to_string())?;
    sqlx::query("INSERT OR REPLACE INTO settings (key,value) VALUES (?,?)")
        .bind(key)
        .bind(text)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
pub async fn runtime_settings(pool: &Pool<Sqlite>) -> Result<Value, String> {
    let value: RuntimeSettings = read(pool, OPTIONS_KEY).await?;
    let mut json = serde_json::to_value(value).map_err(|e| e.to_string())?;
    let level = battery_level();
    json["battery_available"] = json!(level.is_some());
    json["battery_level"] = json!(level);
    Ok(json)
}
pub async fn save_runtime(pool: &Pool<Sqlite>, value: Value) -> Result<Value, String> {
    let mut settings: RuntimeSettings = serde_json::from_value(value).map_err(|e| e.to_string())?;
    settings.validate()?;
    if settings.battery_alert_enabled && battery_level().is_none() {
        return Err("当前设备无可用电池".into());
    }
    write(pool, OPTIONS_KEY, &settings).await?;
    runtime_settings(pool).await
}
async fn delivered(pool: &Pool<Sqlite>) -> Result<HashSet<String>, String> {
    sqlx::query("CREATE TABLE IF NOT EXISTS local_reminder_deliveries (event_key TEXT PRIMARY KEY,delivered_at INTEGER NOT NULL)")
        .execute(pool).await.map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM local_reminder_deliveries WHERE delivered_at < ?")
        .bind(chrono::Utc::now().timestamp_millis() - crate::notification_history::RETENTION_MILLIS)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    let keys = sqlx::query_scalar::<_, String>("SELECT event_key FROM local_reminder_deliveries")
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(keys.into_iter().collect())
}
pub async fn command(pool: &Pool<Sqlite>, request: Value) -> Result<Value, String> {
    let _ = delivered(pool).await?;
    let settings: RuntimeSettings = read(pool, OPTIONS_KEY).await?;
    let mut reminders: Vec<Reminder> = read(pool, REMINDERS_KEY).await?;
    match request["action"].as_str().unwrap_or("") {
        "get" => {}
        "save" => {
            let new: Vec<Reminder> =
                serde_json::from_value(request["settings"]["reminders"].clone())
                    .map_err(|e| e.to_string())?;
            reminder_schedule::validate(&new, &settings.options())?;
            let changed: Vec<String> = reminders
                .iter()
                .filter(|old| !new.contains(old))
                .map(|r| r.id.clone())
                .collect();
            let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
            sqlx::query("INSERT OR REPLACE INTO settings (key,value) VALUES (?,?)")
                .bind(REMINDERS_KEY)
                .bind(serde_json::to_string(&new).map_err(|e| e.to_string())?)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
            for id in changed {
                // ID validation excludes ':', so this prefix cannot match another reminder.
                sqlx::query("DELETE FROM local_reminder_deliveries WHERE substr(event_key,1,?)=?")
                    .bind(id.len() as i64 + 1)
                    .bind(format!("{id}:"))
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            tx.commit().await.map_err(|e| e.to_string())?;
            reminders = new;
        }
        _ => return Err("未知自定义通知操作".into()),
    }
    reminder_schedule::validate(&reminders, &settings.options())?;
    let keys = delivered(pool).await?;
    let enabled = crate::db::get_notifications_enabled(pool).await;
    let next = if enabled {
        reminder_schedule::next(
            &reminders,
            &settings.options(),
            chrono::Utc::now().timestamp_millis(),
            &keys,
            &Local,
        )
    } else {
        vec![]
    };
    Ok(
        json!({"reminders":reminders,"globalSettings":settings.options(),"next":next.iter().map(Push::json).collect::<Vec<_>>(),
        "exactAllowed":true,"notificationAllowed":crate::notification_windows::permission_enabled(),
        "enabled":enabled,"platform":"windows","scheduleError":""}),
    )
}
fn message(key: &str, title: String, text: String, observed_at: i64) -> Notification {
    Notification {
        msg_type: "notification".into(),
        event_id: format!("{:x}", Sha256::digest(key.as_bytes())),
        source_device_id: String::new(),
        target_device_id: String::new(),
        package: "com.lanchat.app".into(),
        app_name: "LQChat".into(),
        app_icon: None,
        title,
        text,
        notification_key: key.into(),
        post_time: observed_at,
    }
}
async fn emit(
    pool: &Pool<Sqlite>,
    key: &str,
    mut message: Notification,
    battery: bool,
    already: &mut HashSet<String>,
) -> Result<(), String> {
    if already.contains(key) {
        return Ok(());
    }
    let local = crate::db::get_user_id(pool).await?;
    message.source_device_id = local.clone();
    message.target_device_id = local;
    let _record_id = notification_sync::record_local(pool, message.clone()).await?;
    let inserted = sqlx::query(
        "INSERT OR IGNORE INTO local_reminder_deliveries (event_key,delivered_at) VALUES (?,?)",
    )
    .bind(key)
    .bind(message.post_time)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?
    .rows_affected();
    already.insert(key.into());
    if inserted == 0 {
        return Ok(());
    }
    // Unit tests exercise persistence/routing without linking or displaying the app's native GUI.
    #[cfg(not(test))]
    if crate::db::get_notifications_enabled(pool).await {
        let source = crate::db::get_username(pool).await?;
        if let Err(error) = crate::notification_windows::publish_local(
            &message,
            &source,
            &_record_id,
            crate::db::get_notification_sound_enabled(pool).await,
        ) {
            eprintln!("[LocalReminder] system notification: {error}");
        }
    }
    let settings = notification_sync::desktop_settings(pool).await;
    if settings.push_enabled
        && if battery {
            settings.lq_battery_push_enabled
        } else {
            settings.lq_reminder_push_enabled
        }
    {
        notification_sync::dispatch(message, settings);
    }
    Ok(())
}
pub fn threshold(previous: Option<i32>, current: i32, levels: &[i32]) -> Option<i32> {
    match previous {
        None => levels.contains(&current).then_some(current),
        Some(old) if current > old => levels
            .iter()
            .copied()
            .filter(|n| *n > old && *n <= current)
            .max(),
        Some(old) if current < old => levels
            .iter()
            .copied()
            .filter(|n| *n < old && *n >= current)
            .min(),
        _ => None,
    }
}
pub async fn run(pool: Pool<Sqlite>) {
    let mut keys = match delivered(&pool).await {
        Ok(value) => value,
        Err(e) => {
            eprintln!("[LocalReminder] {e}");
            return;
        }
    };
    let mut previous = None;
    let mut signature = String::new();
    let mut battery_signature = String::new();
    let mut pending: Vec<Push> = vec![];
    let mut battery_sequence: Option<(i32, i64, u32, i64)> = None;
    let mut last_clock = chrono::Utc::now().timestamp_millis();
    loop {
        let tick = async {
            let now = chrono::Utc::now().timestamp_millis();
            let mut settings: RuntimeSettings = read(&pool, OPTIONS_KEY).await?;
            settings.validate()?;
            let reminders: Vec<Reminder> = read(&pool, REMINDERS_KEY).await?;
            reminder_schedule::validate(&reminders, &settings.options())?;
            let enabled = crate::db::get_notifications_enabled(&pool).await;
            let new_signature = serde_json::to_string(&(
                settings.clone(),
                &reminders,
                enabled,
                Local::now().offset().local_minus_utc(),
            ))
            .map_err(|e| e.to_string())?;
            if new_signature != signature || now < last_clock {
                signature = new_signature;
                keys = delivered(&pool).await?;
                pending = if enabled {
                    reminder_schedule::next(&reminders, &settings.options(), now, &keys, &Local)
                } else {
                    vec![]
                };
            }
            let new_battery_signature =
                serde_json::to_string(&settings).map_err(|e| e.to_string())?;
            if new_battery_signature != battery_signature || now < last_clock {
                battery_signature = new_battery_signature;
                battery_sequence = None;
            }
            last_clock = now;
            if pending.first().is_some_and(|p| p.at <= now) {
                for push in &pending {
                    if now - push.at <= MAX_LATENESS {
                        let key = push.key();
                        emit(
                            &pool,
                            &key,
                            message(
                                &format!("lq-reminder-{key}"),
                                push.reminder.title.clone(),
                                push.text(),
                                now,
                            ),
                            false,
                            &mut keys,
                        )
                        .await?;
                    }
                }
                pending = reminder_schedule::next(
                    &reminders,
                    &settings.options(),
                    now + 1,
                    &keys,
                    &Local,
                );
            }
            if settings.battery_alert_enabled {
                if let Some(level) = battery_level() {
                    if let Some(threshold) =
                        threshold(previous, level, &settings.battery_alert_levels)
                    {
                        if !battery_sequence.is_some_and(|s| s.0 == threshold) {
                            battery_sequence = Some((threshold, now, 0, now));
                        }
                    }
                    previous = Some(level);
                }
                if let Some((threshold, started, count, due)) = battery_sequence {
                    if now - due > 60_000 {
                        battery_sequence = None;
                    } else if due <= now {
                        let key = format!("lq-battery-{threshold}-{started}-{count}");
                        emit(
                            &pool,
                            &key,
                            message(&key, "电量提醒".into(), format!("{threshold}%"), now),
                            true,
                            &mut keys,
                        )
                        .await?;
                        battery_sequence = if count + 1 >= settings.battery_alert_repeat_count {
                            None
                        } else {
                            Some((
                                threshold,
                                started,
                                count + 1,
                                now + settings.battery_alert_interval_seconds * 1000,
                            ))
                        };
                    }
                }
            } else {
                previous = None;
                battery_sequence = None;
            }
            Ok::<(), String>(())
        }
        .await;
        if let Err(error) = tick {
            eprintln!("[LocalReminder] {error}");
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn runtime_settings_persist_and_failed_validation_keeps_the_old_value() {
        let pool=sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE settings (key TEXT PRIMARY KEY,value TEXT NOT NULL)").execute(&pool).await.unwrap();
        let value=json!({"battery_alert_enabled":false,"battery_alert_repeat_count":3,
            "battery_alert_interval_seconds":5,"battery_alert_levels":[100,50,50]});
        save_runtime(&pool,value).await.unwrap();
        let before=runtime_settings(&pool).await.unwrap();
        assert_eq!(before["battery_alert_levels"],json!([50,100]));
        assert_eq!(before["battery_alert_repeat_count"],3);
        assert!(save_runtime(&pool,json!({"battery_alert_repeat_count":2,"battery_alert_interval_seconds":0})).await.is_err());
        let after=runtime_settings(&pool).await.unwrap();
        assert_eq!(after["battery_alert_repeat_count"],before["battery_alert_repeat_count"]);
        assert_eq!(after["battery_alert_interval_seconds"],before["battery_alert_interval_seconds"]);
        if let Some(level)=battery_level() { assert!((0..=100).contains(&level)); }
        println!("Read-only Windows battery sample: {:?}",battery_level());
    }
    #[test]
    fn battery_crossing_matches_android_in_both_directions() {
        let levels = vec![50, 80, 100];
        assert_eq!(threshold(None, 50, &levels), Some(50));
        assert_eq!(threshold(None, 49, &levels), None);
        assert_eq!(threshold(Some(49), 100, &levels), Some(100));
        assert_eq!(threshold(Some(100), 49, &levels), Some(50));
        assert_eq!(threshold(Some(50), 50, &levels), None);
    }
    #[tokio::test]
    async fn local_trigger_records_once_without_network_or_system_popup() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("CREATE TABLE settings (key TEXT PRIMARY KEY,value TEXT NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO settings VALUES ('user_id','pc'),('username','电脑'),('notifications_enabled','false')").execute(&pool).await.unwrap();
        crate::notification_history::initialize(&pool)
            .await
            .unwrap();
        let mut keys = delivered(&pool).await.unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        let notification = message("lq-reminder-one", "提醒".into(), "内容".into(), now);
        emit(&pool, "one", notification.clone(), false, &mut keys)
            .await
            .unwrap();
        emit(&pool, "one", notification, false, &mut keys)
            .await
            .unwrap();
        let page = crate::notification_history::query(
            &pool,
            crate::notification_history::Query {
                direction: Some("local".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].peer_id, "pc");
        assert_eq!(delivered(&pool).await.unwrap().len(), 1);
        assert!(
            !notification_sync::desktop_settings(&pool)
                .await
                .push_enabled
        );
    }
}
