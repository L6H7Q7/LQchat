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
    pub lq_reminder_push_enabled: bool,
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

#[cfg(any(target_os = "android", test))]
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
        let settings = desktop_settings(pool).await;
        return (
            settings.push_enabled,
            settings.receive_enabled,
            settings.target_device_ids,
        );
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
        (
            presence.enabled,
            presence.receive_enabled,
            presence.target_device_ids,
        )
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
pub(crate) async fn record(pool: &sqlx::Pool<sqlx::Sqlite>, mut entry: Record) -> Option<String> {
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

pub async fn record_local(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    notification: Notification,
) -> Result<String, String> {
    persist_local(pool, notification, true).await
}

pub(crate) async fn run_presence_history(
    pool: sqlx::Pool<sqlx::Sqlite>,
    mut events: tokio::sync::mpsc::UnboundedReceiver<crate::peers::PresenceHistoryEvent>,
    cancellation: tokio_util::sync::CancellationToken,
) -> Result<(), String> {
    loop {
        let event = tokio::select! {
            _ = cancellation.cancelled() => {
                // Finish queued transitions before the session closes its database.
                events.close();
                while let Some(event) = events.recv().await {
                    save_presence_history(&pool, event).await;
                }
                return Ok(());
            },
            event = events.recv() => event,
        };
        let Some(event) = event else { return Ok(()) };
        save_presence_history(&pool, event).await;
    }
}

async fn save_presence_history(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    event: crate::peers::PresenceHistoryEvent,
) {
    // 发现/断开事件携带实时设备名，这是设备名档案最可靠的来源。
    let name = if event.peer_name.trim().is_empty() {
        crate::notification_history::stored_device_name(pool, &event.peer_id)
            .await
            .unwrap_or_else(|| event.peer_id.clone())
    } else {
        let _ = crate::notification_history::remember_device_name(
            pool,
            &event.peer_id,
            &event.peer_name,
        )
        .await;
        event.peer_name.clone()
    };
    let state = if event.connected { "连接" } else { "断开" };
    let notification = Notification {
        msg_type: "notification".into(),
        event_id: event.event_id,
        source_device_id: String::new(),
        target_device_id: String::new(),
        package: "com.lanchat.app.connection".into(),
        app_name: "LQChat".into(),
        app_icon: None,
        title: format!("设备{state}"),
        text: format!("{} 已{state}", name.chars().take(256).collect::<String>()),
        notification_key: format!("presence:{}", event.peer_id),
        post_time: event.post_time,
    };
    // History only: no system toast and no outgoing notification dispatch.
    if let Err(error) = record_local(pool, notification).await {
        eprintln!("[PresenceHistory] write failed: {error}");
    }
}
async fn persist_local(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    mut notification: Notification,
    notify: bool,
) -> Result<String, String> {
    let local_id = crate::db::get_user_id(pool).await?;
    let name = crate::db::get_username(pool).await?;
    if notification.event_id.is_empty()
        || notification.event_id.len() > 128
        || notification.title.chars().count() > 256
        || notification.text.chars().count() > 4096
    {
        return Err("invalid_local_notification".into());
    }
    notification.source_device_id = local_id.clone();
    notification.target_device_id = local_id.clone();
    let entry = Record {
        record_id: String::new(),
        peer_id: local_id,
        peer_name: name,
        view_kind: "notification_local".into(),
        direction: "local".into(),
        status: "success".into(),
        failure_reason: None,
        observed_at: 0,
        created_at: 0,
        updated_at: 0,
        content_hash: String::new(),
        notification,
    };
    if notify {
        record(pool, entry)
            .await
            .ok_or_else(|| "local_history_write_failed".into())
    } else {
        crate::notification_history::upsert(pool, &entry).await
    }
}

#[cfg(not(target_os = "android"))]
pub async fn desktop_settings(pool: &sqlx::Pool<sqlx::Sqlite>) -> Settings {
    let stored = sqlx::query_scalar::<_, String>(
        "SELECT value FROM settings WHERE key='notification_sync_desktop_v1'",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    let mut settings = stored
        .and_then(|s| serde_json::from_str::<Settings>(&s).ok())
        .unwrap_or_default();
    settings.receive_enabled = receive_enabled(pool).await;
    settings
}

#[cfg(target_os = "android")]
async fn import_local_history(pool: &sqlx::Pool<sqlx::Sqlite>) -> Result<(), String> {
    let mut changed = false;
    loop {
        let pending = crate::notification_android::settings(json!({"action":"local_history"}))?;
        let mut acknowledged = Vec::new();
        for value in pending["notifications"].as_array().into_iter().flatten() {
            if let Ok(message) = serde_json::from_value::<Notification>(value.clone()) {
                if message.post_time
                    >= chrono::Utc::now().timestamp_millis()
                        - crate::notification_history::RETENTION_MILLIS
                {
                    persist_local(pool, message.clone(), false).await?;
                }
                acknowledged.push(message.event_id);
            }
        }
        if !acknowledged.is_empty() {
            changed = true;
            crate::notification_android::settings(
                json!({"action":"ack_local_history","payload":{"ids":acknowledged}}),
            )?;
        }
        if acknowledged.len() < 200 {
            break;
        }
    }
    if changed {
        CoreRuntime::global()
            .event_bus()
            .publish(CoreEvent::NotificationRecordsChanged);
    }
    Ok(())
}

/// Local route observations belong to viewing history, never to the transport/resend path.
#[derive(Clone, Debug, Deserialize)]
#[cfg(any(target_os = "android", test))]
pub struct RouteHistoryEvent {
    pub peer_id: String,
    pub peer_name: String,
    pub event_id: String,
    pub connected: bool,
    pub post_time: i64,
}

#[cfg(any(target_os = "android", test))]
fn route_history_record(event: RouteHistoryEvent, local_id: &str) -> Result<Record, String> {
    let notification = Notification {
        msg_type: "notification".into(),
        event_id: event.event_id.clone(),
        source_device_id: event.peer_id.clone(),
        target_device_id: local_id.into(),
        package: "com.lanchat.app".into(),
        app_name: "LQChat".into(),
        app_icon: None,
        title: if event.connected {
            "设备连接"
        } else {
            "设备断开"
        }
        .into(),
        text: if event.connected {
            "已接入局域网"
        } else {
            "已从局域网断开"
        }
        .into(),
        notification_key: format!("lq-route-{}", event.event_id),
        post_time: event.post_time,
    };
    if !valid_device_id(&event.peer_id) || event.peer_name.chars().count() > 128 {
        return Err("invalid_route_event".into());
    }
    notification.validate(local_id)?;
    Ok(Record {
        record_id: String::new(),
        peer_id: event.peer_id,
        peer_name: event.peer_name,
        view_kind: "notification_receive".into(),
        direction: "receive".into(),
        status: "success".into(),
        failure_reason: None,
        observed_at: 0,
        created_at: 0,
        updated_at: 0,
        content_hash: String::new(),
        notification,
    })
}

#[cfg(target_os = "android")]
pub fn dispatch_route_history(event: RouteHistoryEvent) {
    let Some((handle, cancel, _)) = CoreRuntime::global().notification_session() else {
        return;
    };
    let Some((pool, peers)) = CoreRuntime::global().shared_resources() else {
        return;
    };
    handle.spawn(async move {
        let save = async {
            let local_id = crate::db::get_user_id(&pool).await?;
            let _ = crate::notification_history::remember_device_name(
                &pool,
                &event.peer_id,
                &event.peer_name,
            )
            .await;
            // A send-only target must not become a receive-history source just
            // because Android displayed its connection toast. All peers are
            // independently recorded in local history by the presence worker.
            if !peers.get_all_peers().iter().any(|peer|
                peer.id == event.peer_id && receives_from_peer(&local_id, peer)
            ) {
                return Ok(String::new());
            }
            let entry = route_history_record(event, &local_id)?;
            record(&pool, entry)
                .await
                .ok_or_else(|| "route_history_write_failed".to_string())
        };
        tokio::select! {
            _ = cancel.cancelled() => {},
            result = save => if let Err(error) = result {
                eprintln!("[NotificationHistory] route event failed: {error}");
            }
        }
    });
}

fn receives_from_peer(local_id: &str, peer: &crate::peers::Peer) -> bool {
    peer.notification_push_enabled
        && peer.notification_push_target_device_ids.iter().any(|id| id == local_id)
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
    // 设备名档案优先于设备 ID；没有可用名称时留空，由界面显示“未知设备”。
    let live_name = peers
        .get_all_peers()
        .into_iter()
        .find(|p| p.id == message.source_device_id)
        .map(|p| p.name);
    let name = crate::notification_history::resolve_device_name(
        pool,
        &message.source_device_id,
        live_name.as_deref(),
    )
    .await;
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
        if peer
            .as_ref()
            .is_some_and(|peer| !peer.notification_receive_enabled)
        {
            continue;
        }
        // 目标不在在线列表时沿用档案里的名字，绝不用目标 ID 充当地名。
        let peer_name = crate::notification_history::resolve_device_name(
            &pool,
            &target,
            peer.as_ref().map(|p| p.name.as_str()),
        )
        .await;
        let mut entry = Record {
            record_id: String::new(),
            peer_id: target.clone(),
            peer_name,
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
            let pushes_to_local = receives_from_peer(local_id, &peer);
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

#[cfg(any(target_os = "android", windows))]
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
            if settings.target_device_ids.len() > 32
                || settings.target_device_ids.iter().any(|id| {
                    id.is_empty()
                        || id.len() > 128
                        || !id
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
                })
            {
                return Err("推送目标设备无效".into());
            }
            let mut tx = pool.begin().await.map_err(|_| "设置保存失败")?;
            sqlx::query("INSERT OR REPLACE INTO settings (key,value) VALUES ('notification_sync_desktop_v1',?)")
                .bind(serde_json::to_string(&settings).map_err(|_| "设置保存失败")?).execute(&mut *tx).await.map_err(|_| "设置保存失败")?;
            sqlx::query("INSERT OR REPLACE INTO settings (key,value) VALUES ('notification_sync_receive',?)")
                .bind(if settings.receive_enabled {"true"} else {"false"}).execute(&mut *tx).await.map_err(|_| "设置保存失败")?;
            tx.commit().await.map_err(|_| "设置保存失败")?;
        }
        let settings = desktop_settings(pool).await;
        Ok(
            json!({"settings":settings,"platform":if cfg!(windows) {"windows"} else {"unsupported"},"permission":"unknown"}),
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
    #[cfg(target_os = "android")]
    import_local_history(&state.pool).await?;
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

#[cfg(not(target_os = "android"))]
async fn load_notification_settings(pool: &sqlx::Pool<sqlx::Sqlite>) -> Result<Settings, String> {
    Ok(desktop_settings(pool).await)
}

#[cfg(target_os = "android")]
async fn load_notification_settings(_pool: &sqlx::Pool<sqlx::Sqlite>) -> Result<Settings, String> {
    let value = crate::notification_android::settings(json!({"action":"get"}))?;
    serde_json::from_value(value["settings"].clone()).map_err(|_| "设置读取失败".to_string())
}

#[cfg(not(target_os = "android"))]
async fn store_notification_settings(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    settings: &Settings,
) -> Result<(), String> {
    if settings.target_device_ids.len() > 32
        || settings.target_device_ids.iter().any(|id| {
            id.is_empty()
                || id.len() > 128
                || !id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
        })
    {
        return Err("推送目标设备无效".into());
    }
    let mut tx = pool.begin().await.map_err(|_| "设置保存失败")?;
    sqlx::query(
        "INSERT OR REPLACE INTO settings (key,value) VALUES ('notification_sync_desktop_v1',?)",
    )
    .bind(serde_json::to_string(settings).map_err(|_| "设置保存失败")?)
    .execute(&mut *tx)
    .await
    .map_err(|_| "设置保存失败")?;
    sqlx::query("INSERT OR REPLACE INTO settings (key,value) VALUES ('notification_sync_receive',?)")
        .bind(if settings.receive_enabled { "true" } else { "false" })
        .execute(&mut *tx)
        .await
        .map_err(|_| "设置保存失败")?;
    tx.commit().await.map_err(|_| "设置保存失败")?;
    Ok(())
}

#[cfg(target_os = "android")]
async fn store_notification_settings(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    settings: &Settings,
) -> Result<(), String> {
    crate::notification_android::settings(json!({"action":"save","settings":settings}))?;
    persist_discovery_presence(pool, settings).await
}

/// 从推送目标里移除已清理的设备，返回（新设置，被移除的 ID 列表）。
pub fn without_target_devices(settings: &Settings, remove: &[String]) -> (Settings, Vec<String>) {
    let mut next = settings.clone();
    let mut removed: Vec<String> = Vec::new();
    next.target_device_ids.retain(|id| {
        if remove.iter().any(|value| value == id) {
            removed.push(id.clone());
            false
        } else {
            true
        }
    });
    (next, removed)
}

/// 待清理的候选 ID：校验格式、去重，并永远排除本机。
pub fn cleanup_candidate_ids(device_ids: Vec<String>, local_id: &str) -> Result<Vec<String>, String> {
    let mut requested: Vec<String> = Vec::new();
    for id in device_ids {
        let id = id.trim().to_string();
        if id.is_empty()
            || id.len() > 128
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
        {
            return Err("通知设备 ID 无效".into());
        }
        // 本机记录永远保留。
        if id == local_id || requested.contains(&id) {
            continue;
        }
        requested.push(id);
    }
    Ok(requested)
}

/// 单台设备的离线判定：在线一定保留；已标记离线但地址仍能确认同一身份时也保留；
/// 没有地址可探测（本次会话没发现过）按离线处理，与列表显示的“离线”一致。
pub fn device_is_offline(peer: Option<&crate::peers::Peer>, reachable: bool) -> bool {
    match peer {
        Some(peer) if !peer.is_offline => false,
        Some(peer) if !peer.addr.is_empty() => !reachable,
        Some(_) => true,
        None => true,
    }
}

/// 逐台判定离线：在线与“探测仍可达”的设备一律保留；其余（含本次会话未发现过的）
/// 按离线处理，与界面显示的“离线”一致。
pub async fn classify_offline_devices(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    peers: &[crate::peers::Peer],
    client: &reqwest::Client,
    requested: Vec<String>,
) -> (Vec<String>, Vec<Value>) {
    let mut offline: Vec<String> = Vec::new();
    let mut kept: Vec<Value> = Vec::new();
    for id in requested {
        let peer = peers.iter().find(|peer| peer.id == id);
        let reachable = match peer {
            Some(peer) if peer.is_offline && !peer.addr.is_empty() => {
                crate::peers::same_peer_reachable(client, &id, &peer.addr).await
            }
            _ => false,
        };
        if device_is_offline(peer, reachable) {
            offline.push(id);
            continue;
        }
        let name = match crate::notification_history::stored_device_name(pool, &id).await {
            Some(name) => name,
            None => peer.map(|peer| peer.name.clone()).unwrap_or_default(),
        };
        let reason = if peer.is_some_and(|peer| !peer.is_offline) {
            "在线"
        } else {
            "探测仍可连接"
        };
        kept.push(json!({"peer_id":id,"name":name,"reason":reason}));
    }
    (offline, kept)
}

/// 通知域清理：同一份实现供“通知设备卡清理”和“统一清理”使用，
/// 保证删除记录与移除推送目标的先后和失败语义完全一致。
pub async fn clear_notification_devices_for(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    peers: &[crate::peers::Peer],
    client: &reqwest::Client,
    requested: Vec<String>,
) -> Result<Value, String> {
    let (offline, kept) = classify_offline_devices(pool, peers, client, requested).await;
    let mut cleared: Vec<String> = Vec::new();
    let mut failures: Vec<Value> = Vec::new();
    let mut records_deleted: u64 = 0;
    let mut names_deleted: u64 = 0;
    for id in &offline {
        match crate::notification_history::delete_device(pool, id).await {
            Ok((records, names)) => {
                records_deleted += records;
                names_deleted += names;
                cleared.push(id.clone());
            }
            Err(reason) => failures.push(json!({"peer_id":id,"reason":reason})),
        }
    }
    // 记录删除成功之后再移除推送目标：删除失败时引用保留，重试时该设备仍可见、可收敛。
    let mut targets_removed: Vec<String> = Vec::new();
    let mut target_error: Option<String> = None;
    if !cleared.is_empty() {
        match load_notification_settings(pool).await {
            Ok(settings) => {
                let (next, removed) = without_target_devices(&settings, &cleared);
                if !removed.is_empty() {
                    match store_notification_settings(pool, &next).await {
                        Ok(()) => targets_removed = removed,
                        Err(error) => target_error = Some(error),
                    }
                }
            }
            Err(error) => target_error = Some(error),
        }
        let _ = crate::notification_history::prune(pool).await;
        CoreRuntime::global()
            .event_bus()
            .publish(CoreEvent::NotificationRecordsChanged);
    }
    Ok(json!({
        "cleared":cleared, "kept":kept, "failures":failures,
        "records_deleted":records_deleted, "names_deleted":names_deleted,
        "targets_removed":targets_removed, "target_error":target_error,
    }))
}

/// 核心是否真的在运行：只有它在跑时，设备列表的读取结果（哪怕是空列表）才可信。
/// 不能用“peers 为空”推断状态不可读——重开后没有任何其他设备是合法状态。
fn notification_core_available() -> bool {
    CoreRuntime::global().shared_resources().is_some()
}

/// 清理通知设备里已离线的设备：通知记录、设备名档案与推送目标引用一起删除。
/// 本机与在线设备永不清理；核心不可用时拒绝执行，不猜测在线状态。
#[cfg(feature = "desktop")]
#[tauri::command]
pub async fn notification_clear_offline_devices(
    device_ids: Vec<String>,
    state: tauri::State<'_, crate::db::DbState>,
    peer_state: tauri::State<'_, crate::commands::PeerState>,
) -> Result<Value, String> {
    let pool = &state.pool;
    let local_id = crate::db::get_user_id(pool).await?;
    let requested = cleanup_candidate_ids(device_ids, &local_id)?;
    if requested.is_empty() {
        return Ok(json!({
            "cleared":[], "kept":[], "failures":[],
            "records_deleted":0, "names_deleted":0, "targets_removed":[],
            "target_error":null,
        }));
    }
    if !notification_core_available() {
        return Err("核心未运行，暂时无法确认设备在线状态，请稍后重试".into());
    }
    let peers = peer_state.active_manager().get_all_peers();
    let client = crate::network::lan_http_client(Some(crate::peers::PRESENCE_PROBE_TIMEOUT))?;
    clear_notification_devices_for(pool, &peers, &client, requested).await
}

/// Windows 统一清理：复用“局域网聊天设备”标题右侧已有的“清除”按钮，
/// 一次确认同时处理聊天域与通知域；两域各自判定离线、各自按 ID 执行必要操作。
#[cfg(feature = "desktop")]
#[tauri::command]
pub async fn clear_offline_devices(
    chat_device_ids: Option<Vec<String>>,
    notification_device_ids: Option<Vec<String>>,
    state: tauri::State<'_, crate::db::DbState>,
    peer_state: tauri::State<'_, crate::commands::PeerState>,
) -> Result<Value, String> {
    let pool = &state.pool;
    let local_id = crate::db::get_user_id(pool).await?;
    let manager = peer_state.active_manager();
    if !notification_core_available() {
        return Err("核心未运行，暂时无法确认设备在线状态，请稍后重试".into());
    }
    let chat_ids = cleanup_candidate_ids(chat_device_ids.unwrap_or_default(), &local_id)?;
    let note_ids = cleanup_candidate_ids(notification_device_ids.unwrap_or_default(), &local_id)?;
    if chat_ids.is_empty() && note_ids.is_empty() {
        return Ok(json!({
            "chat":{"cleared":[],"kept":[],"failures":[],"aborted":false},
            "notifications":{"cleared":[],"kept":[],"failures":[],
                "records_deleted":0,"names_deleted":0,"targets_removed":[],"target_error":null},
        }));
    }
    let peers = manager.get_all_peers();
    let client = crate::network::lan_http_client(Some(crate::peers::PRESENCE_PROBE_TIMEOUT))?;
    // 先通知域：此刻设备档案仍在，名称与在线状态都能读到（聊天域先删会丢失原目标）。
    let notifications = clear_notification_devices_for(pool, &peers, &client, note_ids).await?;
    // 再聊天域：用同一批已判定离线的设备；未发现的设备不在此域处理。
    let (chat_offline, chat_kept) =
        classify_offline_devices(pool, &peers, &client, chat_ids).await;
    let mut chat_report = match manager
        .clear_offline_with_history_for(pool, &local_id, chat_offline)
        .await
    {
        Ok(report) => report,
        // 存储层异常时保留聊天记录与本机文件，并如实回报，不影响已完成的其它域。
        Err(error) => json!({"cleared":[], "failures":[], "aborted":true, "error":error}),
    };
    chat_report["kept"] = json!(chat_kept);
    Ok(json!({"chat":chat_report, "notifications":notifications}))
}
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
    #[cfg(windows)]
    {
        let (pool, _) = CoreRuntime::global()
            .shared_resources()
            .ok_or("会话未运行")?;
        let settings = desktop_settings(&pool).await;
        if settings.target_device_ids.is_empty() {
            return Err("请先选择目标设备".into());
        }
        return send(
            Notification {
                msg_type: "notification".into(),
                event_id: uuid::Uuid::new_v4().to_string(),
                source_device_id: String::new(),
                target_device_id: String::new(),
                package: "lanchat.test".into(),
                app_name: "LQChat 测试".into(),
                app_icon: None,
                title: "通知推送测试".into(),
                text: format!("测试时间：{}", chrono::Local::now().format("%H:%M:%S")),
                notification_key: "manual-test".into(),
                post_time: chrono::Utc::now().timestamp_millis(),
            },
            settings,
        )
        .await;
    }
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
    #[cfg(not(any(target_os = "android", windows)))]
    Err("当前平台不支持通知推送".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn route_history_keeps_transitions_and_battery_under_the_same_device() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::notification_history::initialize(&pool)
            .await
            .unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        for (event_id, connected) in [
            ("connect-1", true),
            ("disconnect-1", false),
            ("connect-2", true),
        ] {
            let entry = route_history_record(
                RouteHistoryEvent {
                    peer_id: "phone".into(),
                    peer_name: "IQOO".into(),
                    event_id: event_id.into(),
                    connected,
                    post_time: now,
                },
                "pc",
            )
            .unwrap();
            assert!(!entry.notification.title.contains("IQOO"));
            assert!(!entry.notification.text.contains("IQOO"));
            // A retry updates the same observation, while each transition stays separate.
            let id = record(&pool, entry.clone()).await.unwrap();
            assert_eq!(record(&pool, entry).await.unwrap(), id);
        }
        let mut battery = route_history_record(
            RouteHistoryEvent {
                peer_id: "phone".into(),
                peer_name: "IQOO".into(),
                event_id: "battery-1".into(),
                connected: true,
                post_time: now,
            },
            "pc",
        )
        .unwrap();
        battery.notification.title = "电量提醒".into();
        battery.notification.text = "50%".into();
        battery.notification.notification_key = "lq-battery-50-1-0".into();
        record(&pool, battery).await.unwrap();
        let page = crate::notification_history::query(
            &pool,
            crate::notification_history::Query {
                device_id: Some("phone".into()),
                direction: Some("receive".into()),
                page_size: Some(1),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(page.records.len(), 1);
        assert!(page.has_more);
        assert_eq!(page.devices.len(), 1);
        assert_eq!(page.devices[0].peer_name, "IQOO");
        let page = crate::notification_history::query(&pool, Default::default())
            .await
            .unwrap();
        assert_eq!(page.records.len(), 4);
        assert!(page.records.iter().all(|r| r.peer_id == "phone"));
        assert!(page
            .records
            .iter()
            .any(|r| r.notification.title == "设备断开"));
        assert!(page.records.iter().any(|r| r.notification.text == "50%"));
        assert!(route_history_record(
            RouteHistoryEvent {
                peer_id: "pc".into(),
                peer_name: "本机".into(),
                event_id: "self".into(),
                connected: true,
                post_time: now,
            },
            "pc"
        )
        .is_err());
        pool.close().await;
    }
    #[cfg(not(target_os = "android"))]
    #[tokio::test]
    async fn discovery_presence_advertises_windows_receive_setting() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(discovery_presence(&pool).await, (false, false, Vec::new()));
        sqlx::query(
            "INSERT INTO settings (key, value) VALUES ('notification_sync_receive', 'true')",
        )
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(discovery_presence(&pool).await, (false, true, Vec::new()));
        let settings = Settings { push_enabled:true,receive_enabled:true,
            target_device_ids:vec!["phone".into()],lq_battery_push_enabled:true,lq_reminder_push_enabled:true,..Default::default() };
        sqlx::query("INSERT INTO settings (key,value) VALUES ('notification_sync_desktop_v1',?)")
            .bind(serde_json::to_string(&settings).unwrap()).execute(&pool).await.unwrap();
        assert_eq!(discovery_presence(&pool).await, (true,true,vec!["phone".into()]));
        let restored=desktop_settings(&pool).await;
        assert!(restored.lq_battery_push_enabled && restored.lq_reminder_push_enabled);
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
    fn offline_cleanup_never_targets_online_or_local_devices() {
        let mut peer = crate::peers::Peer {
            id: "phone".into(),
            name: "手机".into(),
            addr: "192.168.5.10:8888".into(),
            last_seen: 1,
            is_offline: false,
            available_memory_mb: 0,
            notification_push_enabled: false,
            notification_receive_enabled: true,
            notification_push_target_device_ids: Vec::new(),
        };
        // 在线设备即使探测不可达也不清理。
        assert!(!device_is_offline(Some(&peer), false));
        peer.is_offline = true;
        // 已标记离线但地址仍确认是同一台设备时保留。
        assert!(!device_is_offline(Some(&peer), true));
        // 探测失败才算离线。
        assert!(device_is_offline(Some(&peer), false));
        // 没有地址可探测（本次会话没发现过）按离线处理。
        peer.addr = String::new();
        assert!(device_is_offline(Some(&peer), true));
        // 设备档案里完全没有该设备：与列表一致地按离线处理。
        assert!(device_is_offline(None, false));

        let candidates = cleanup_candidate_ids(
            vec![
                " pc ".into(),
                "phone".into(),
                "pc".into(),
                "".into(),
            ]
            .into_iter()
            .filter(|id| id != "")
            .collect(),
            "pc",
        )
        .unwrap();
        assert_eq!(candidates, vec!["phone".to_string()]);
        assert!(cleanup_candidate_ids(vec!["bad id".into()], "pc").is_err());
        assert!(cleanup_candidate_ids(vec!["pc".into()], "pc").unwrap().is_empty());
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
    fn send_only_target_is_not_a_receive_history_source() {
        let manager = crate::peers::PeerManager::new();
        manager.observe_discovery("android-b".into(), "安卓 B".into(), "127.0.0.1:8888".into(), 128);
        manager.update_notification_presence("android-b", false, true, Vec::new());
        assert!(!receives_from_peer("android-a", &manager.get_all_peers()[0]));
        manager.update_notification_presence("android-b", true, true, vec!["other-device".into()]);
        assert!(!receives_from_peer("android-a", &manager.get_all_peers()[0]));
        manager.update_notification_presence("android-b", true, true, vec!["android-a".into()]);
        assert!(receives_from_peer("android-a", &manager.get_all_peers()[0]));
        manager.force_mark_offline("android-b");
        assert!(receives_from_peer("android-a", &manager.get_all_peers()[0]));
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
