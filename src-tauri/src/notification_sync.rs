//! Best-effort notifications with isolated seven-day viewing history. Never enter chat storage or resend paths.
use crate::core_events::{CoreEvent, CoreState};
use crate::core_runtime::CoreRuntime;
use futures_util::{stream::FuturesUnordered, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::sync::{oneshot, Semaphore};

pub const MAX_FRAME: usize = 32 * 1024;
pub const DEADLINE: Duration = Duration::from_secs(5);

#[cfg(windows)]
fn pending_windows_activation() -> &'static Mutex<Option<Value>> {
    static PENDING: OnceLock<Mutex<Option<Value>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(None))
}

#[cfg(windows)]
pub fn store_windows_activation(argument: &str) -> Option<Value> {
    let mut parts = argument.splitn(3, ':');
    if parts.next()? != "history" {
        return None;
    }
    let record_id = parts.next()?;
    let source_device_id = parts.next()?;
    let valid_id = |value: &str| {
        !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
    };
    if !valid_id(record_id) || !valid_id(source_device_id) {
        return None;
    }
    let value = json!({"recordId":record_id,"sourceDeviceId":source_device_id});
    *pending_windows_activation().lock().ok()? = Some(value.clone());
    Some(value)
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub push_enabled: bool,
    pub receive_enabled: bool,
    pub lq_battery_push_enabled: bool,
    pub allowed_packages: Vec<String>,
    pub target_device_ids: Vec<String>,
}

#[cfg(target_os = "android")]
const DISCOVERY_PRESENCE_KEY: &str = "notification_sync_discovery_presence";

#[cfg(target_os = "android")]
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct DiscoveryPresence {
    enabled: bool,
    receive_enabled: bool,
    target_device_ids: Vec<String>,
}

#[cfg(target_os = "android")]
fn valid_device_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
}

#[cfg(target_os = "android")]
async fn persist_discovery_presence(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    settings: &Settings,
) -> Result<(), String> {
    let presence = DiscoveryPresence {
        enabled: settings.push_enabled,
        receive_enabled: settings.receive_enabled,
        target_device_ids: settings
            .target_device_ids
            .iter()
            .filter(|id| valid_device_id(id))
            .take(32)
            .cloned()
            .collect(),
    };
    let value = serde_json::to_string(&presence).map_err(|_| "推送状态保存失败")?;
    sqlx::query("INSERT OR REPLACE INTO settings (key,value) VALUES (?,?)")
        .bind(DISCOVERY_PRESENCE_KEY)
        .bind(value)
        .execute(pool)
        .await
        .map_err(|_| "推送状态保存失败")?;
    Ok(())
}

pub async fn discovery_presence(pool: &sqlx::Pool<sqlx::Sqlite>) -> (bool, bool, Vec<String>) {
    #[cfg(not(target_os = "android"))]
    {
        return (false, receive_enabled(pool).await, Vec::new());
    }

    #[cfg(target_os = "android")]
    {
        let value = sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key=?")
            .bind(DISCOVERY_PRESENCE_KEY)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
        let presence = value
            .and_then(|value| serde_json::from_str::<DiscoveryPresence>(&value).ok())
            .unwrap_or_default();
        (presence.enabled, presence.receive_enabled, presence.target_device_ids)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Notification {
    pub msg_type: String,
    pub event_id: String,
    pub source_device_id: String,
    pub target_device_id: String,
    pub package: String,
    pub app_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_icon: Option<String>,
    pub title: String,
    pub text: String,
    pub notification_key: String,
    pub post_time: i64,
}

impl Notification {
    pub fn key(&self) -> (String, String, String) {
        (
            self.source_device_id.clone(),
            self.package.clone(),
            self.notification_key.clone(),
        )
    }
    pub fn same_content(&self, other: &Self) -> bool {
        self.app_name == other.app_name
            && self.title == other.title
            && self.text == other.text
            && self.app_icon == other.app_icon
    }
    pub fn validate(&self, local: &str) -> Result<(), String> {
        let device_id = |s: &str| {
            !s.is_empty()
                && s.len() <= 128
                && s.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
        };
        if self.msg_type != "notification"
            || self.target_device_id != local
            || self.source_device_id == local
            || !device_id(&self.source_device_id)
            || !device_id(&self.target_device_id)
            || self.event_id.is_empty()
            || self.event_id.len() > 128
            || self.package.is_empty()
            || self.package.len() > 256
            || self.package.contains(':')
            || self.notification_key.is_empty()
            || self.notification_key.len() > 4096
            || self.title.chars().count() > 256
            || self.text.chars().count() > 4096
            || self.app_name.chars().count() > 128
            || (self.title.is_empty() && self.text.is_empty())
            || serde_json::to_vec(self).map_err(|_| "invalid")?.len() > MAX_FRAME
        {
            return Err("invalid_notification".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Record {
    #[serde(default)]
    pub record_id: String,
    pub peer_id: String,
    pub peer_name: String,
    pub view_kind: String,
    #[serde(default)]
    pub direction: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    #[serde(default)]
    pub observed_at: i64,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub content_hash: String,
    pub notification: Notification,
}

struct Pending {
    tx: oneshot::Sender<Completion>,
    generation: u64,
}
struct Completion {
    success: bool,
    failure_reason: Option<String>,
}
#[derive(Default)]
struct Memory {
    generation: u64,
    pending: HashMap<String, Pending>,
}
fn memory() -> &'static Mutex<Memory> {
    static STATE: OnceLock<Mutex<Memory>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(Memory::default()))
}
fn slots() -> Arc<Semaphore> {
    static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    SLOTS.get_or_init(|| Arc::new(Semaphore::new(16))).clone()
}
fn reset_generation(generation: u64) {
    let mut state = memory().lock().unwrap();
    if state.generation != generation {
        *state = Memory {
            generation,
            ..Memory::default()
        };
    }
}
async fn record(pool: &sqlx::Pool<sqlx::Sqlite>, mut entry: Record) -> Option<String> {
    match crate::notification_history::upsert(pool, &entry).await {
        Ok(id) => {
            entry.record_id = id.clone();
            CoreRuntime::global()
                .event_bus()
                .publish(CoreEvent::NotificationRecordsChanged);
            Some(id)
        }
        Err(error) => {
            eprintln!("[NotificationHistory] write failed: {error}");
            None
        }
    }
}

pub fn on_state_event(event: &CoreEvent) {
    if let CoreEvent::CoreStateChanged(status) | CoreEvent::CoreError(status) = event {
        if status.state != CoreState::Running {
            *memory().lock().unwrap() = Memory::default();
            #[cfg(all(windows, feature = "desktop"))]
            crate::notification_windows::clear();
        } else {
            reset_generation(status.generation);
        }
    }
}

pub fn pending_active(request_id: &str) -> bool {
    let status = CoreRuntime::global().status();
    status.state == CoreState::Running
        && memory()
            .lock()
            .unwrap()
            .pending
            .get(request_id)
            .is_some_and(|p| p.generation == status.generation && !p.tx.is_closed())
}

pub fn complete(request_id: &str, success: bool, _changed: bool, failure_reason: Option<String>) {
    let pending = memory().lock().unwrap().pending.remove(request_id);
    if let Some(p) = pending {
        let status = CoreRuntime::global().status();
        let success =
            success && status.state == CoreState::Running && status.generation == p.generation;
        let _ = p.tx.send(Completion {
            success,
            failure_reason: if success {
                None
            } else {
                failure_reason.or_else(|| Some("receive_rejected".into()))
            },
        });
    }
}

struct PendingGuard(String);
impl Drop for PendingGuard {
    fn drop(&mut self) {
        memory().lock().unwrap().pending.remove(&self.0);
    }
}

/// Called only for notification frames, before generic chat/stream handling.
pub async fn receive(
    value: Value,
    wire_size: usize,
    pool: &sqlx::Pool<sqlx::Sqlite>,
    peers: &crate::peers::PeerManager,
) -> Value {
    let event_id = value
        .get("event_id")
        .and_then(Value::as_str)
        .filter(|s| s.len() <= 128)
        .unwrap_or("")
        .to_owned();
    let target = crate::db::get_user_id(pool).await.unwrap_or_default();
    let mut reply = json!({"msg_type":"notification_result", "event_id":event_id,"target_device_id":target,"status":"failure"});
    if wire_size > MAX_FRAME {
        return reply;
    }
    let Ok(mut message) = serde_json::from_value::<Notification>(value) else {
        return reply;
    };
    if message.validate(&target).is_err() {
        return reply;
    }
    message.app_icon = crate::notification_icon::sanitize(message.app_icon.take());
    let Some((_, cancellation, generation)) = CoreRuntime::global().notification_session() else {
        return reply;
    };
    reset_generation(generation);
    let name = peers
        .get_all_peers()
        .into_iter()
        .find(|p| p.id == message.source_device_id)
        .map(|p| p.name)
        .unwrap_or_else(|| message.source_device_id.clone());
    let mut receiving = Record {
        record_id: String::new(),
        peer_id: message.source_device_id.clone(),
        peer_name: name.clone(),
        view_kind: "notification_receive".into(),
        direction: "receive".into(),
        status: "receiving".into(),
        failure_reason: None,
        observed_at: 0,
        created_at: 0,
        updated_at: 0,
        content_hash: String::new(),
        notification: message.clone(),
    };
    let request_id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = oneshot::channel();
    let accepted = {
        let mut state = memory().lock().unwrap();
        if state.pending.len() >= 16 || state.generation != generation {
            false
        } else {
            state
                .pending
                .insert(request_id.clone(), Pending { tx, generation });
            true
        }
    };
    if !accepted {
        receiving.status = "rejected".into();
        receiving.failure_reason = Some("busy".into());
        record(pool, receiving).await;
        return reply;
    }
    let _guard = PendingGuard(request_id.clone());
    let history_record_id = record(pool, receiving.clone()).await.unwrap_or_default();
    CoreRuntime::global()
        .event_bus()
        .publish(CoreEvent::NotificationReceived(json!({
            "request_id":request_id,"history_record_id":history_record_id,
            "notification":message,"source_name":name
        })));
    let completion = tokio::select! {
        _ = cancellation.cancelled() => Completion { success:false, failure_reason:Some("cancelled".into()) },
        result = tokio::time::timeout(DEADLINE, rx) => match result {
            Ok(Ok(completion)) => completion,
            Ok(Err(_)) => Completion { success:false, failure_reason:Some("receive_rejected".into()) },
            Err(_) => Completion { success:false, failure_reason:Some("timeout".into()) },
        },
    };
    receiving.status = if completion.success {
        "success"
    } else if completion.failure_reason.as_deref() == Some("timeout") {
        "timeout"
    } else if completion.failure_reason.as_deref() == Some("cancelled") {
        "cancelled"
    } else {
        "rejected"
    }
    .into();
    receiving.failure_reason = completion.failure_reason;
    record(pool, receiving).await;
    if completion.success {
        reply["status"] = json!("success");
    }
    reply
}

pub async fn send(mut message: Notification, settings: Settings) -> Result<(), String> {
    let Some((_, cancel, generation)) = CoreRuntime::global().notification_session() else {
        return Err("会话未运行".into());
    };
    if !settings.push_enabled {
        return Err("信息推送未开启".into());
    }
    reset_generation(generation);
    message.app_icon = crate::notification_icon::sanitize(message.app_icon.take());
    let (pool, peers) = CoreRuntime::global()
        .shared_resources()
        .ok_or("会话未运行")?;
    let local = tokio::select! { _ = cancel.cancelled() => return Err("会话已停止".into()), value = crate::db::get_user_id(&pool) => value? };
    let mut seen = HashSet::new();
    let mut requests = FuturesUnordered::new();
    for target in settings.target_device_ids {
        if target == local || !seen.insert(target.clone()) {
            continue;
        }
        let mut message = message.clone();
        message.source_device_id = local.clone();
        message.target_device_id = target.clone();
        // Preserve the existing 32 KiB transport limit. Text takes priority over the icon.
        if serde_json::to_vec(&message).map_or(true, |bytes| bytes.len() > MAX_FRAME) {
            message.app_icon = None;
        }
        if message.validate(&target).is_err() {
            continue;
        }
        let peer = peers.get_all_peers().into_iter().find(|p| p.id == target);
        if peer.as_ref().is_some_and(|peer| !peer.notification_receive_enabled) {
            continue;
        }
        let mut entry = Record {
            record_id: String::new(),
            peer_id: target.clone(),
            peer_name: peer.as_ref().map(|p| p.name.clone()).unwrap_or(target),
            view_kind: "notification_push".into(),
            direction: "send".into(),
            status: "offline".into(),
            failure_reason: Some("offline".into()),
            observed_at: 0,
            created_at: 0,
            updated_at: 0,
            content_hash: String::new(),
            notification: message.clone(),
        };
        let Some(peer) = peer.filter(|p| !p.is_offline && !p.addr.is_empty()) else {
            record(&pool, entry).await;
            continue;
        };
        let Ok(permit) = slots().try_acquire_owned() else {
            entry.status = "busy".into();
            entry.failure_reason = Some("busy".into());
            record(&pool, entry).await;
            continue;
        };
        entry.status = "sending".into();
        entry.failure_reason = None;
        record(&pool, entry.clone()).await;
        let cancel = cancel.clone();
        let pool = pool.clone();
        requests.push(async move {
            let _permit = permit;
            let result = tokio::select! {
                _ = cancel.cancelled() => return,
                result = tokio::time::timeout(DEADLINE, crate::network::messaging::send_notification_once(&peer.addr, &message)) => result,
            };
            let (status, reason) = match result {
                Ok(Ok(())) => ("success", None),
                Ok(Err(error)) => ("failure", Some(error)),
                Err(_) => ("timeout", Some("timeout".into())),
            };
            entry.status = status.into();
            entry.failure_reason = reason;
            record(&pool, entry).await;
        });
    }
    while requests.next().await.is_some() {}
    Ok(())
}

pub fn route_status_snapshot(local_id: &str, mut peers: Vec<crate::peers::Peer>) -> Value {
    peers.sort_by(|left, right| left.id.cmp(&right.id));
    json!({
        "peers": peers.into_iter().map(|peer| {
            let pushes_to_local = peer.notification_push_enabled &&
                peer.notification_push_target_device_ids.iter().any(|id| id == local_id);
            json!({
                "id": peer.id,
                "name": peer.name,
                "is_offline": peer.is_offline,
                "pushes_to_local": pushes_to_local,
            })
        }).collect::<Vec<_>>(),
    })
}

#[cfg(target_os = "android")]
pub async fn run_route_status_monitor(
    pool: sqlx::Pool<sqlx::Sqlite>,
    peers: Arc<crate::peers::PeerManager>,
    event_bus: crate::core_events::CoreEventBus,
    cancellation: tokio_util::sync::CancellationToken,
) -> Result<(), String> {
    let local_id = crate::db::get_user_id(&pool).await?;
    let mut previous = Value::Null;
    loop {
        let snapshot = route_status_snapshot(&local_id, peers.get_all_peers());
        if snapshot != previous {
            previous = snapshot.clone();
            event_bus.publish(CoreEvent::NotificationRouteStatus(snapshot));
        }
        tokio::select! {
            _ = cancellation.cancelled() => return Ok(()),
            _ = tokio::time::sleep(Duration::from_secs(2)) => {}
        }
    }
}

#[cfg(target_os = "android")]
pub fn dispatch(message: Notification, settings: Settings) {
    // Admission is immediate, so a burst cannot create an unbounded native task backlog.
    static INTAKE: OnceLock<Arc<Semaphore>> = OnceLock::new();
    let Ok(permit) = INTAKE
        .get_or_init(|| Arc::new(Semaphore::new(16)))
        .clone()
        .try_acquire_owned()
    else {
        return;
    };
    let Some((handle, cancel, _)) = CoreRuntime::global().notification_session() else {
        return;
    };
    handle.spawn(async move {
        let _permit = permit;
        tokio::select! { _ = cancel.cancelled() => {}, _ = send(message, settings) => {} }
    });
}

#[cfg(feature = "desktop")]
#[tauri::command]
pub async fn notification_settings(
    settings: Option<Settings>,
    #[allow(unused_variables)] state: tauri::State<'_, crate::db::DbState>,
) -> Result<Value, String> {
    #[cfg(target_os = "android")]
    {
        let value = crate::notification_android::settings(
            json!({"action":if settings.is_some() {"save"} else {"get"},"settings":settings}),
        )?;
        if let Ok(settings) = serde_json::from_value::<Settings>(value["settings"].clone()) {
            if let Err(error) = persist_discovery_presence(&state.pool, &settings).await {
                eprintln!("[NotificationSync] {error}");
            }
        }
        return Ok(value);
    }
    #[cfg(not(target_os = "android"))]
    {
        // UI configuration does not depend on the later Core start completing.
        let pool = &state.pool;
        if let Some(settings) = settings {
            sqlx::query("INSERT OR REPLACE INTO settings (key,value) VALUES ('notification_sync_receive',?)")
                .bind(if settings.receive_enabled {"true"} else {"false"}).execute(pool).await.map_err(|_| "设置保存失败")?;
        }
        let enabled = receive_enabled(pool).await;
        Ok(
            json!({"settings":Settings { receive_enabled:enabled,..Settings::default() },"platform":if cfg!(windows) {"windows"} else {"unsupported"},"permission":"unknown"}),
        )
    }
}
#[cfg(not(target_os = "android"))]
pub async fn receive_enabled(pool: &sqlx::Pool<sqlx::Sqlite>) -> bool {
    sqlx::query_scalar::<_, String>(
        "SELECT value FROM settings WHERE key='notification_sync_receive'",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .as_deref()
        == Some("true")
}

#[cfg(feature = "desktop")]
#[tauri::command]
pub async fn notification_records(
    query: Option<crate::notification_history::Query>,
    state: tauri::State<'_, crate::db::DbState>,
) -> Result<crate::notification_history::Page, String> {
    crate::notification_history::query(&state.pool, query.unwrap_or_default()).await
}

#[cfg(feature = "desktop")]
#[tauri::command]
pub async fn notification_record(
    record_id: String,
    state: tauri::State<'_, crate::db::DbState>,
) -> Result<Option<Record>, String> {
    crate::notification_history::get(&state.pool, &record_id).await
}

#[cfg(feature = "desktop")]
#[tauri::command]
pub fn notification_pending_activation() -> Option<Value> {
    #[cfg(windows)]
    return pending_windows_activation().lock().ok()?.take();
    #[cfg(not(windows))]
    None
}

#[cfg(feature = "desktop")]
#[tauri::command]
pub fn notification_action(action: String, payload: Option<Value>) -> Result<Value, String> {
    #[cfg(target_os = "android")]
    {
        return crate::notification_android::settings(json!({"action":action,"payload":payload}));
    }
    #[cfg(windows)]
    if action == "permission" {
        std::process::Command::new("explorer.exe")
            .arg("ms-settings:notifications")
            .spawn()
            .map_err(|_| "无法打开系统设置")?;
        return Ok(json!({}));
    }
    #[cfg(not(target_os = "android"))]
    let _ = payload;
    #[cfg(not(target_os = "android"))]
    Err("此平台不支持该操作".into())
}

#[cfg(feature = "desktop")]
#[tauri::command]
pub async fn notification_test() -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        let value = crate::notification_android::settings(json!({"action":"get"}))?;
        let settings: Settings =
            serde_json::from_value(value["settings"].clone()).map_err(|_| "设置读取失败")?;
        if settings.target_device_ids.is_empty() {
            return Err("请先选择目标设备".into());
        }
        let icon = crate::notification_android::settings(json!({"action":"test_icon"}))?;
        let message = Notification {
            msg_type: "notification".into(),
            event_id: uuid::Uuid::new_v4().to_string(),
            source_device_id: String::new(),
            target_device_id: String::new(),
            package: "lanchat.test".into(),
            app_name: "LQChat 测试".into(),
            app_icon: icon["test_icon"].as_str().map(str::to_owned),
            title: "通知推送测试".into(),
            text: format!("测试时间：{}", chrono::Local::now().format("%H:%M:%S%.3f")),
            notification_key: "manual-test".into(),
            post_time: chrono::Utc::now().timestamp_millis(),
        };
        return send(message, settings).await;
    }
    #[cfg(not(target_os = "android"))]
    Err("Windows 首版仅接收通知".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(target_os = "android"))]
    #[tokio::test]
    async fn discovery_presence_advertises_windows_receive_setting() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(discovery_presence(&pool).await, (false, false, Vec::new()));
        sqlx::query("INSERT INTO settings (key, value) VALUES ('notification_sync_receive', 'true')")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(discovery_presence(&pool).await, (false, true, Vec::new()));
    }
    pub fn sample() -> Notification {
        Notification {
            msg_type: "notification".into(),
            event_id: "event-1".into(),
            source_device_id: "phone".into(),
            target_device_id: "pc".into(),
            package: "app.chat".into(),
            app_name: "Chat".into(),
            app_icon: None,
            title: "Title".into(),
            text: "Text".into(),
            notification_key: "key:1".into(),
            post_time: 1,
        }
    }
    #[test]
    fn routing_and_unicode_limits_reject_invalid_frames() {
        let mut n = sample();
        assert!(n.validate("pc").is_ok());
        assert!(n.validate("other").is_err());
        n.source_device_id = "pc".into();
        assert!(n.validate("pc").is_err());
        n = sample();
        n.text = "😀".repeat(4096);
        assert!(n.validate("pc").is_ok());
        n.text.push('😀');
        assert!(n.validate("pc").is_err());
        n = sample();
        n.source_device_id = "spoof:app".into();
        assert!(n.validate("pc").is_err());
    }
    #[test]
    fn content_identity_ignores_event_time_but_separates_sources() {
        let a = sample();
        let mut b = a.clone();
        b.event_id = "event-2".into();
        b.post_time = 2;
        assert_eq!(a.key(), b.key());
        assert!(a.same_content(&b));
        b.text.push('!');
        assert!(!a.same_content(&b));
        b.source_device_id = "another-phone".into();
        assert_ne!(a.key(), b.key());
        b = a.clone();
        b.app_icon = Some("changed-icon".into());
        assert!(!a.same_content(&b));
    }
    #[test]
    fn legacy_text_notification_without_icon_still_deserializes() {
        let mut value = serde_json::to_value(sample()).unwrap();
        value.as_object_mut().unwrap().remove("app_icon");
        let n: Notification = serde_json::from_value(value).unwrap();
        assert!(n.app_icon.is_none());
        assert!(n.validate("pc").is_ok());
    }

    #[test]
    fn route_status_snapshot_marks_targets_and_inbound_sources() {
        let peers = vec![crate::peers::Peer {
            id: "phone".into(),
            name: "IQOO".into(),
            addr: "192.168.1.2:8888".into(),
            last_seen: 1,
            is_offline: true,
            available_memory_mb: 100,
            notification_push_enabled: true,
            notification_receive_enabled: true,
            notification_push_target_device_ids: vec!["local".into()],
        }];
        let snapshot = route_status_snapshot("local", peers);
        assert_eq!(snapshot["peers"][0]["name"], "IQOO");
        assert_eq!(snapshot["peers"][0]["is_offline"], true);
        assert_eq!(snapshot["peers"][0]["pushes_to_local"], true);
    }
    #[cfg(windows)]
    #[test]
    fn windows_history_activation_accepts_only_the_internal_shape() {
        assert!(store_windows_activation("https://example.invalid/route").is_none());
        assert!(store_windows_activation("history:record:bad/source").is_none());
        let value = store_windows_activation("history:record-1:phone-a").unwrap();
        assert_eq!(value["recordId"], "record-1");
        assert_eq!(notification_pending_activation().unwrap(), value);
        assert!(notification_pending_activation().is_none());
    }
}
