//! Seven-day, best-effort notification history. It is deliberately isolated from chat storage.
use crate::notification_sync::{Notification, Record};
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{QueryBuilder, Row, Sqlite};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

pub const RETENTION_MILLIS: i64 = 7 * 24 * 60 * 60 * 1000;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Query {
    pub direction: Option<String>,
    pub device_id: Option<String>,
    pub package: Option<String>,
    pub status: Option<String>,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub start_time: Option<i64>,
    /// Exclusive upper bound; callers pass midnight after the selected end date.
    pub end_time: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Page {
    pub records: Vec<Record>,
    pub devices: Vec<HistoryDevice>,
    /// 已持久化的设备名档案：离线设备与尚无记录的设备也能显示可靠名称。
    pub device_names: Vec<HistoryDevice>,
    pub page: u32,
    pub page_size: u32,
    pub has_more: bool,
    pub record_start_time: Option<i64>,
    pub record_end_time: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct HistoryDevice {
    pub peer_id: String,
    pub peer_name: String,
}

pub async fn initialize(pool: &sqlx::Pool<Sqlite>) -> Result<(), sqlx::Error> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS notification_history (
            id TEXT PRIMARY KEY NOT NULL,
            direction TEXT NOT NULL,
            event_id TEXT NOT NULL,
            source_device_id TEXT NOT NULL,
            target_device_id TEXT NOT NULL,
            device_name TEXT NOT NULL,
            package TEXT NOT NULL,
            app_name TEXT NOT NULL,
            app_icon_ref TEXT,
            title TEXT NOT NULL,
            text TEXT NOT NULL,
            notification_key TEXT NOT NULL,
            identity_key TEXT NOT NULL,
            status TEXT NOT NULL,
            failure_reason TEXT,
            observed_at INTEGER NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            content_hash TEXT NOT NULL,
            post_time INTEGER NOT NULL,
            UNIQUE(direction, source_device_id, target_device_id, package, identity_key)
        )",
    )
    .execute(pool)
    .await?;
    migrate_identity_key(pool).await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_notification_history_updated
         ON notification_history(updated_at DESC, id DESC)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_notification_history_filters
         ON notification_history(direction, source_device_id, target_device_id, package, status, updated_at DESC)",
    )
    .execute(pool)
    .await?;
    initialize_device_names(pool).await?;
    backfill_device_names(pool).await?;
    prune(pool).await.map_err(sqlx::Error::Protocol)?;
    Ok(())
}

/// 设备名档案：与七天通知记录分离，记录过期清理后名称仍可复用。
async fn initialize_device_names(pool: &sqlx::Pool<Sqlite>) -> Result<(), sqlx::Error> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS notification_device_names (
            peer_id TEXT PRIMARY KEY NOT NULL,
            name TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_notification_device_names_updated
         ON notification_device_names(updated_at DESC)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// 可靠的设备名：非空、不是设备 ID 本身、长度受限。其余一律视为“没有名字”。
pub fn is_device_name(value: &str, peer_id: &str) -> bool {
    let value = value.trim();
    !value.is_empty() && value != peer_id && value.chars().count() <= 128
}

/// 记忆设备名。只有真实名称才写入，绝不把设备 ID 当名字保存。
pub async fn remember_device_name(
    pool: &sqlx::Pool<Sqlite>,
    peer_id: &str,
    name: &str,
) -> Result<bool, String> {
    if peer_id.is_empty() || peer_id.len() > 128 || !is_device_name(name, peer_id) {
        return Ok(false);
    }
    let name = name.trim();
    let now = chrono::Utc::now().timestamp_millis();
    let changed = sqlx::query(
        "INSERT INTO notification_device_names (peer_id,name,updated_at) VALUES (?,?,?)
         ON CONFLICT(peer_id) DO UPDATE SET name=excluded.name,updated_at=excluded.updated_at
         WHERE notification_device_names.name<>excluded.name",
    )
    .bind(peer_id)
    .bind(name)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|error| error.to_string())?
    .rows_affected();
    Ok(changed > 0)
}

pub async fn stored_device_name(pool: &sqlx::Pool<Sqlite>, peer_id: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>("SELECT name FROM notification_device_names WHERE peer_id=?")
        .bind(peer_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .filter(|name| is_device_name(name, peer_id))
}

pub async fn device_names(pool: &sqlx::Pool<Sqlite>) -> Vec<HistoryDevice> {
    sqlx::query("SELECT peer_id,name FROM notification_device_names ORDER BY updated_at DESC,peer_id")
        .fetch_all(pool)
        .await
        .map(|rows| {
            rows.into_iter()
                .filter_map(|row| {
                    let peer_id: String = row.get("peer_id");
                    let peer_name: String = row.get("name");
                    // 只有可靠名称才下发，避免把异常长串或 ID 当作名字。
                    is_device_name(&peer_name, &peer_id).then_some(HistoryDevice {
                        peer_id,
                        peer_name: peer_name.trim().to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 写入记录时解析设备名：先用在线设备的实时名字，再退回已保存的档案名。
/// 两者都没有时返回空字符串，由界面统一显示“未知设备”，不再用 ID 兜底。
pub async fn resolve_device_name(
    pool: &sqlx::Pool<Sqlite>,
    peer_id: &str,
    live_name: Option<&str>,
) -> String {
    if let Some(live) = live_name.filter(|name| is_device_name(name, peer_id)) {
        let _ = remember_device_name(pool, peer_id, live).await;
        return live.trim().to_string();
    }
    stored_device_name(pool, peer_id).await.unwrap_or_default()
}

/// 一次性回填：旧记录里为空或以设备 ID 充当地名的，用真实存在的名称修正。
/// 无法恢复的名称保持为空（界面显示“未知设备”），不编造名称。
async fn backfill_device_names(pool: &sqlx::Pool<Sqlite>) -> Result<(), sqlx::Error> {
    let now = chrono::Utc::now().timestamp_millis();
    // 在线发现写下的设备档案是现成的真实名称来源；表不存在时直接跳过。
    let _ = sqlx::query(
        "INSERT INTO notification_device_names (peer_id,name,updated_at)
         SELECT id,name,? FROM users
         WHERE TRIM(IFNULL(name,''))<>'' AND name<>id AND LENGTH(name)<=128
         ON CONFLICT(peer_id) DO NOTHING",
    )
    .bind(now)
    .execute(pool)
    .await;
    // 用档案名修正空名与“ID 充当地名”的历史记录。
    sqlx::query(
        "UPDATE notification_history
         SET device_name=(SELECT name FROM notification_device_names
             WHERE peer_id=CASE WHEN notification_history.direction='send'
                 THEN notification_history.target_device_id
                 ELSE notification_history.source_device_id END)
         WHERE direction IN ('send','receive')
           AND (TRIM(device_name)='' OR device_name=CASE WHEN direction='send'
                 THEN target_device_id ELSE source_device_id END)
           AND EXISTS (SELECT 1 FROM notification_device_names
             WHERE peer_id=CASE WHEN notification_history.direction='send'
                 THEN notification_history.target_device_id
                 ELSE notification_history.source_device_id END)",
    )
    .execute(pool)
    .await?;
    // 恢复不了的“ID 地名”清空，避免界面再把 UUID 当标题。
    sqlx::query(
        "UPDATE notification_history SET device_name=''
         WHERE direction IN ('send','receive')
           AND device_name=CASE WHEN direction='send' THEN target_device_id ELSE source_device_id END",
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// 删除一台设备在通知域内的全部痕迹：通知记录、设备名档案，并清掉不再被引用的图标。
pub async fn delete_device(pool: &sqlx::Pool<Sqlite>, peer_id: &str) -> Result<(u64, u64), String> {
    let mut transaction = pool.begin().await.map_err(|error| error.to_string())?;
    let records = sqlx::query(
        "DELETE FROM notification_history
         WHERE (direction='send' AND target_device_id=?) OR (direction='receive' AND source_device_id=?)",
    )
    .bind(peer_id)
    .bind(peer_id)
    .execute(&mut *transaction)
    .await
    .map_err(|error| error.to_string())?
    .rows_affected();
    let devices = sqlx::query("DELETE FROM notification_device_names WHERE peer_id=?")
        .bind(peer_id)
        .execute(&mut *transaction)
        .await
        .map_err(|error| error.to_string())?
        .rows_affected();
    transaction.commit().await.map_err(|error| error.to_string())?;
    Ok((records, devices))
}

async fn migrate_identity_key(pool: &sqlx::Pool<Sqlite>) -> Result<(), sqlx::Error> {
    let columns = sqlx::query("PRAGMA table_info(notification_history)")
        .fetch_all(pool)
        .await?;
    if columns
        .iter()
        .any(|row| row.get::<String, _>("name") == "identity_key")
    {
        return Ok(());
    }

    let mut transaction = pool.begin().await?;
    sqlx::query("DROP TABLE IF EXISTS notification_history_v2")
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "CREATE TABLE notification_history_v2 (
            id TEXT PRIMARY KEY NOT NULL,
            direction TEXT NOT NULL,
            event_id TEXT NOT NULL,
            source_device_id TEXT NOT NULL,
            target_device_id TEXT NOT NULL,
            device_name TEXT NOT NULL,
            package TEXT NOT NULL,
            app_name TEXT NOT NULL,
            app_icon_ref TEXT,
            title TEXT NOT NULL,
            text TEXT NOT NULL,
            notification_key TEXT NOT NULL,
            identity_key TEXT NOT NULL,
            status TEXT NOT NULL,
            failure_reason TEXT,
            observed_at INTEGER NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            content_hash TEXT NOT NULL,
            post_time INTEGER NOT NULL,
            UNIQUE(direction, source_device_id, target_device_id, package, identity_key)
        )",
    )
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT INTO notification_history_v2 (
            id,direction,event_id,source_device_id,target_device_id,device_name,
            package,app_name,app_icon_ref,title,text,notification_key,identity_key,status,
            failure_reason,observed_at,created_at,updated_at,content_hash,post_time
         )
         SELECT id,direction,event_id,source_device_id,target_device_id,device_name,
            package,app_name,app_icon_ref,title,text,notification_key,
            CASE WHEN direction='receive' THEN event_id ELSE notification_key END,
            status,failure_reason,observed_at,created_at,updated_at,content_hash,post_time
         FROM notification_history",
    )
    .execute(&mut *transaction)
    .await?;
    sqlx::query("DROP TABLE notification_history")
        .execute(&mut *transaction)
        .await?;
    sqlx::query("ALTER TABLE notification_history_v2 RENAME TO notification_history")
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await
}

async fn icon_root(pool: &sqlx::Pool<Sqlite>) -> Option<PathBuf> {
    let rows = sqlx::query("PRAGMA database_list")
        .fetch_all(pool)
        .await
        .ok()?;
    let database: String = rows
        .into_iter()
        .find(|row| row.try_get::<String, _>("name").ok().as_deref() == Some("main"))?
        .try_get("file")
        .ok()?;
    let parent = PathBuf::from(database).parent()?.to_path_buf();
    Some(parent.join("notification-history-icons-v1"))
}

fn valid_icon_name(value: &str) -> bool {
    value.len() == 68
        && value.ends_with(".png")
        && value.as_bytes()[..64]
            .iter()
            .all(|byte| byte.is_ascii_hexdigit())
}

/// Cache a sanitized icon once and return its content-addressed reference.
fn cache_icon_at(root: &std::path::Path, encoded: Option<&str>) -> Option<String> {
    let bytes = crate::notification_icon::decode(encoded?)?;
    let reference = format!("{:x}.png", Sha256::digest(&bytes));
    std::fs::create_dir_all(root).ok()?;
    let path = root.join(&reference);
    if !path.is_file() {
        std::fs::write(path, bytes).ok()?;
    }
    Some(reference)
}

fn icon_path_at(root: &std::path::Path, reference: Option<&str>) -> Option<PathBuf> {
    let reference = reference?.trim();
    if !valid_icon_name(reference) {
        return None;
    }
    let path = root.join(reference);
    path.is_file().then_some(path)
}

fn load_icon(root: Option<&std::path::Path>, reference: Option<&str>) -> Option<String> {
    let bytes = std::fs::read(icon_path_at(root?, reference)?).ok()?;
    // Re-validate the cache entry before exposing it to the WebView.
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    crate::notification_icon::decode(&encoded)?;
    Some(encoded)
}

pub async fn cached_icon_path(pool: &sqlx::Pool<Sqlite>, encoded: Option<&str>) -> Option<PathBuf> {
    let root = icon_root(pool).await?;
    let reference = cache_icon_at(&root, encoded)?;
    icon_path_at(&root, Some(&reference))
}

fn content_hash(record: &Record, icon_ref: Option<&str>) -> String {
    let mut digest = Sha256::new();
    for value in [
        record.notification.app_name.as_str(),
        record.notification.title.as_str(),
        record.notification.text.as_str(),
        icon_ref.unwrap_or(""),
    ] {
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

pub async fn upsert(pool: &sqlx::Pool<Sqlite>, record: &Record) -> Result<String, String> {
    prune(pool).await?;
    let now = if record.direction == "local" {
        record.notification.post_time
    } else {
        chrono::Utc::now().timestamp_millis()
    };
    let id = uuid::Uuid::new_v4().to_string();
    let root = icon_root(pool).await;
    let icon_ref = root
        .as_deref()
        .and_then(|root| cache_icon_at(root, record.notification.app_icon.as_deref()));
    let hash = content_hash(record, icon_ref.as_deref());
    let direction = if record.view_kind == "notification_local" {
        "local"
    } else if record.view_kind == "notification_push" {
        "send"
    } else {
        "receive"
    };
    // 记录里出现的真实设备名同时写入设备名档案，供离线后继续显示。
    if direction != "local" {
        let _ = remember_device_name(pool, &record.peer_id, &record.peer_name).await;
    }
    // Receive history keeps every captured event; send history keeps tracking the OS notification.
    let identity_key = if direction != "send" {
        &record.notification.event_id
    } else {
        &record.notification.notification_key
    };
    sqlx::query(
        "INSERT INTO notification_history (
            id,direction,event_id,source_device_id,target_device_id,device_name,
            package,app_name,app_icon_ref,title,text,notification_key,identity_key,status,
            failure_reason,observed_at,created_at,updated_at,content_hash,post_time
         ) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
         ON CONFLICT(direction,source_device_id,target_device_id,package,identity_key)
         DO UPDATE SET
            event_id=excluded.event_id,
            device_name=excluded.device_name,
            app_name=excluded.app_name,
            app_icon_ref=excluded.app_icon_ref,
            title=excluded.title,
            text=excluded.text,
            status=excluded.status,
            failure_reason=excluded.failure_reason,
            observed_at=excluded.observed_at,
            updated_at=excluded.updated_at,
            content_hash=excluded.content_hash,
            post_time=excluded.post_time",
    )
    .bind(&id)
    .bind(direction)
    .bind(&record.notification.event_id)
    .bind(&record.notification.source_device_id)
    .bind(&record.notification.target_device_id)
    .bind(&record.peer_name)
    .bind(&record.notification.package)
    .bind(&record.notification.app_name)
    .bind(icon_ref.as_deref())
    .bind(&record.notification.title)
    .bind(&record.notification.text)
    .bind(&record.notification.notification_key)
    .bind(identity_key)
    .bind(&record.status)
    .bind(record.failure_reason.as_deref())
    .bind(now)
    .bind(now)
    .bind(now)
    .bind(hash)
    .bind(record.notification.post_time)
    .execute(pool)
    .await
    .map_err(|error| error.to_string())?;
    sqlx::query_scalar::<_, String>(
        "SELECT id FROM notification_history
         WHERE direction=? AND source_device_id=? AND target_device_id=? AND package=? AND identity_key=?",
    )
    .bind(direction)
    .bind(&record.notification.source_device_id)
    .bind(&record.notification.target_device_id)
    .bind(&record.notification.package)
    .bind(identity_key)
    .fetch_one(pool)
    .await
    .map_err(|error| error.to_string())
}

fn row_to_record(row: sqlx::sqlite::SqliteRow, icon_root: Option<&std::path::Path>) -> Record {
    let direction: String = row.get("direction");
    let source_device_id: String = row.get("source_device_id");
    let target_device_id: String = row.get("target_device_id");
    let icon_ref: Option<String> = row.get("app_icon_ref");
    Record {
        record_id: row.get("id"),
        peer_id: if direction == "send" {
            target_device_id.clone()
        } else {
            source_device_id.clone()
        },
        peer_name: row.get("device_name"),
        view_kind: if direction == "local" {
            "notification_local".into()
        } else if direction == "send" {
            "notification_push".into()
        } else {
            "notification_receive".into()
        },
        direction,
        status: row.get("status"),
        failure_reason: row.get("failure_reason"),
        observed_at: row.get("observed_at"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        content_hash: row.get("content_hash"),
        notification: Notification {
            msg_type: "notification".into(),
            event_id: row.get("event_id"),
            source_device_id,
            target_device_id,
            package: row.get("package"),
            app_name: row.get("app_name"),
            app_icon: load_icon(icon_root, icon_ref.as_deref()),
            title: row.get("title"),
            text: row.get("text"),
            notification_key: row.get("notification_key"),
            post_time: row.get("post_time"),
        },
    }
}

fn history_filters<'a>(builder: &mut QueryBuilder<'a, Sqlite>, query: &'a Query) {
    if let Some(direction) = query
        .direction
        .as_deref()
        .filter(|value| matches!(*value, "send" | "receive" | "local"))
    {
        builder.push(" AND direction=").push_bind(direction);
    }
    if let Some(device) = query.device_id.as_deref().filter(|value| !value.is_empty()) {
        builder
            .push(" AND (source_device_id=")
            .push_bind(device)
            .push(" OR target_device_id=")
            .push_bind(device)
            .push(")");
    }
    if let Some(package) = query.package.as_deref().filter(|value| !value.is_empty()) {
        builder.push(" AND package=").push_bind(package);
    }
    if let Some(status) = query.status.as_deref().filter(|value| !value.is_empty()) {
        builder.push(" AND status=").push_bind(status);
    }
}

pub async fn query(pool: &sqlx::Pool<Sqlite>, query: Query) -> Result<Page, String> {
    prune(pool).await?;
    if matches!((query.start_time, query.end_time), (Some(start), Some(end)) if start >= end) {
        return Err("开始日期不能晚于结束日期".into());
    }
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(100).clamp(1, 100);
    // Date defaults describe all retained records for this device, not just this page/range.
    let mut bounds = QueryBuilder::new(
        "SELECT MIN(post_time) AS first_time,MAX(post_time) AS last_time FROM notification_history WHERE 1=1",
    );
    history_filters(&mut bounds, &query);
    let bounds = bounds.build().fetch_one(pool).await.map_err(|error| error.to_string())?;
    let mut builder: QueryBuilder<Sqlite> = QueryBuilder::new(
        "SELECT id,direction,event_id,source_device_id,target_device_id,device_name,package,app_name,app_icon_ref,title,text,notification_key,status,failure_reason,observed_at,created_at,updated_at,content_hash,post_time FROM notification_history WHERE 1=1",
    );
    history_filters(&mut builder, &query);
    if let Some(start) = query.start_time {
        builder.push(" AND post_time>=").push_bind(start);
    }
    if let Some(end) = query.end_time {
        builder.push(" AND post_time<").push_bind(end);
    }
    builder
        .push(" ORDER BY updated_at DESC,id DESC LIMIT ")
        .push_bind((page_size + 1) as i64)
        .push(" OFFSET ")
        .push_bind(((page - 1) * page_size) as i64);
    let mut rows = builder
        .build()
        .fetch_all(pool)
        .await
        .map_err(|error| error.to_string())?;
    let has_more = rows.len() > page_size as usize;
    rows.truncate(page_size as usize);
    let icon_root = icon_root(pool).await;
    let archived: HashMap<String, String> = device_names(pool)
        .await
        .into_iter()
        .map(|device| (device.peer_id, device.peer_name))
        .collect();
    // Include both directions so old send-only devices remain reachable outside page one.
    // 取“最新且有真实名称”的一行：旧记录里被写入设备 ID 的行不会盖掉已知名称。
    let devices = sqlx::query(
        "SELECT peer_id,device_name FROM (
            SELECT peer_id,device_name,
                ROW_NUMBER() OVER (
                    PARTITION BY peer_id ORDER BY named DESC,updated_at DESC,id DESC
                ) AS position
            FROM (
                SELECT CASE WHEN direction='send' THEN target_device_id ELSE source_device_id END AS peer_id,
                    device_name,updated_at,id,
                    CASE WHEN TRIM(device_name)<>'' AND device_name<>CASE WHEN direction='send'
                            THEN target_device_id ELSE source_device_id END
                        THEN 1 ELSE 0 END AS named
                FROM notification_history WHERE direction IN ('send','receive')
            )
         ) WHERE position=1",
    )
    .fetch_all(pool)
    .await
    .map_err(|error| error.to_string())?
    .into_iter()
    .map(|row| {
        let peer_id: String = row.get("peer_id");
        let stored: String = row.get("device_name");
        let peer_name = if is_device_name(&stored, &peer_id) {
            stored.trim().to_string()
        } else {
            archived.get(&peer_id).cloned().unwrap_or_default()
        };
        HistoryDevice { peer_id, peer_name }
    })
    .collect();
    Ok(Page {
        records: rows
            .into_iter()
            .map(|row| row_to_record(row, icon_root.as_deref()))
            .collect(),
        page,
        page_size,
        has_more,
        record_start_time: bounds.get("first_time"),
        record_end_time: bounds.get("last_time"),
        devices,
        device_names: archived
            .into_iter()
            .map(|(peer_id, peer_name)| HistoryDevice { peer_id, peer_name })
            .collect(),
    })
}

pub async fn get(pool: &sqlx::Pool<Sqlite>, id: &str) -> Result<Option<Record>, String> {
    prune(pool).await?;
    let row = sqlx::query(
        "SELECT id,direction,event_id,source_device_id,target_device_id,device_name,package,app_name,app_icon_ref,title,text,notification_key,status,failure_reason,observed_at,created_at,updated_at,content_hash,post_time FROM notification_history WHERE id=?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|error| error.to_string())?;
    let icon_root = icon_root(pool).await;
    Ok(row.map(|row| row_to_record(row, icon_root.as_deref())))
}

pub async fn prune(pool: &sqlx::Pool<Sqlite>) -> Result<(), String> {
    prune_at(pool, chrono::Utc::now().timestamp_millis()).await
}

async fn prune_at(pool: &sqlx::Pool<Sqlite>, now: i64) -> Result<(), String> {
    let cutoff = now - RETENTION_MILLIS;
    sqlx::query("DELETE FROM notification_history WHERE MAX(updated_at,observed_at) < ?")
        .bind(cutoff)
        .execute(pool)
        .await
        .map_err(|error| error.to_string())?;
    let referenced: HashSet<String> = sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT app_icon_ref FROM notification_history WHERE app_icon_ref IS NOT NULL",
    )
    .fetch_all(pool)
    .await
    .map_err(|error| error.to_string())?
    .into_iter()
    .collect();
    if let Some(root) = icon_root(pool).await {
        if let Ok(entries) = std::fs::read_dir(root) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if entry.file_type().is_ok_and(|kind| kind.is_file())
                    && valid_icon_name(&name)
                    && !referenced.contains(&name)
                {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool() -> sqlx::Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        initialize(&pool).await.unwrap();
        pool
    }
    #[tokio::test]
    async fn peer_transitions_are_named_local_history_without_receive_sources() {
        let pool = pool().await;
        sqlx::query("CREATE TABLE settings (key TEXT PRIMARY KEY,value TEXT NOT NULL)")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO settings VALUES ('user_id','android-a'),('username','安卓 A')")
            .execute(&pool).await.unwrap();
        let manager = crate::peers::PeerManager::new();
        let (sender, events) = tokio::sync::mpsc::unbounded_channel();
        manager.attach_presence_history(sender);
        manager.observe_discovery("android-b".into(), "安卓 B".into(), "127.0.0.1:8888".into(), 128);
        manager.observe_discovery("android-b".into(), "安卓 B".into(), "127.0.0.1:8888".into(), 128);
        manager.force_mark_offline("android-b");
        manager.force_mark_offline("android-b");
        manager.observe_discovery("android-b".into(), "安卓 B".into(), "127.0.0.1:8888".into(), 128);
        let cancel = tokio_util::sync::CancellationToken::new();
        cancel.cancel();
        crate::notification_sync::run_presence_history(pool.clone(), events, cancel).await.unwrap();
        let page = query(&pool, Query {
            direction: Some("local".into()), device_id: Some("android-a".into()), ..Default::default()
        }).await.unwrap();
        assert_eq!(page.records.len(), 3);
        assert!(page.devices.is_empty());
        assert_eq!(page.records.iter().filter(|r| r.notification.text == "安卓 B 已连接").count(), 2);
        assert_eq!(page.records.iter().filter(|r| r.notification.text == "安卓 B 已断开").count(), 1);
        for record in page.records {
            assert_eq!(record.notification.source_device_id, "android-a");
            assert_eq!(record.notification.target_device_id, "android-a");
            assert_eq!(record.direction, "local");
        }
        let remote = query(&pool, Query {
            direction: Some("receive".into()), device_id: Some("android-b".into()), ..Default::default()
        }).await.unwrap();
        assert!(remote.records.is_empty());
    }

    #[tokio::test]
    async fn local_history_is_separate_deduplicated_and_retained_from_trigger_time() {
        let pool = pool().await;
        sqlx::query("CREATE TABLE settings (key TEXT PRIMARY KEY,value TEXT NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO settings VALUES ('user_id','phone'),('username','本机')")
            .execute(&pool)
            .await
            .unwrap();
        let mut message = record("phone", "本机提醒", "success").notification;
        message.event_id = "local-1".into();
        message.post_time = chrono::Utc::now().timestamp_millis();
        let first = crate::notification_sync::record_local(&pool, message.clone())
            .await
            .unwrap();
        assert_eq!(
            crate::notification_sync::record_local(&pool, message.clone())
                .await
                .unwrap(),
            first
        );
        let page = query(
            &pool,
            Query {
                direction: Some("local".into()),
                device_id: Some("phone".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].view_kind, "notification_local");
        assert!(page.devices.is_empty());
        assert_eq!(
            page.records[0].notification.source_device_id,
            page.records[0].notification.target_device_id
        );
        message.event_id = "local-2".into();
        message.post_time -= RETENTION_MILLIS + 1000;
        crate::notification_sync::record_local(&pool, message)
            .await
            .unwrap();
        let page = query(&pool, Query::default()).await.unwrap();
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].record_id, first);
    }

    #[tokio::test]
    async fn date_range_filters_before_pagination_and_bounds_cover_all_device_records() {
        let pool = pool().await;
        let start = chrono::Utc::now().timestamp_millis() - 2 * 24 * 60 * 60 * 1000;
        let end = start + 24 * 60 * 60 * 1000;
        for (index, time) in [start - 1, start, end - 1, end].into_iter().enumerate() {
            let mut item = record("dates", "date fixture", "success");
            item.notification.notification_key = format!("dates-{index}");
            item.notification.post_time = time;
            upsert(&pool, &item).await.unwrap();
        }
        let mut other = record("other", "other device", "success");
        other.notification.post_time = start - 1000;
        upsert(&pool, &other).await.unwrap();
        let first = query(&pool, Query {
            device_id: Some("dates".into()), start_time: Some(start), end_time: Some(end),
            page_size: Some(1), ..Default::default()
        }).await.unwrap();
        assert_eq!(first.record_start_time, Some(start - 1));
        assert_eq!(first.record_end_time, Some(end));
        assert_eq!(first.records.len(), 1);
        assert!(first.has_more);
        let second = query(&pool, Query {
            device_id: Some("dates".into()), start_time: Some(start), end_time: Some(end),
            page_size: Some(1), page: Some(2), ..Default::default()
        }).await.unwrap();
        assert!(!second.has_more);
        let mut times = vec![first.records[0].notification.post_time, second.records[0].notification.post_time];
        times.sort();
        assert_eq!(times, vec![start, end - 1]);
        let empty = query(&pool, Query { device_id:Some("missing".into()), ..Default::default() }).await.unwrap();
        assert_eq!(empty.record_start_time, None);
        assert_eq!(empty.record_end_time, None);
        assert!(query(&pool, Query { start_time:Some(end), end_time:Some(start), ..Default::default() }).await.is_err());
    }

    fn record(target: &str, text: &str, status: &str) -> Record {
        Record {
            record_id: String::new(),
            peer_id: target.into(),
            peer_name: format!("Device {target}"),
            view_kind: "notification_push".into(),
            direction: "send".into(),
            status: status.into(),
            failure_reason: None,
            observed_at: 0,
            created_at: 0,
            updated_at: 0,
            content_hash: String::new(),
            notification: Notification {
                msg_type: "notification".into(),
                event_id: uuid::Uuid::new_v4().to_string(),
                source_device_id: "phone".into(),
                target_device_id: target.into(),
                package: "example.app".into(),
                app_name: "Example".into(),
                app_icon: None,
                title: "Title".into(),
                text: text.into(),
                notification_key: "stable-key".into(),
                post_time: 1,
            },
        }
    }

    fn received(event_id: &str, text: &str, status: &str) -> Record {
        let mut record = record("receiver", text, status);
        record.peer_id = "phone".into();
        record.peer_name = "Device phone".into();
        record.view_kind = "notification_receive".into();
        record.direction = "receive".into();
        record.notification.event_id = event_id.into();
        record
    }

    #[tokio::test]
    async fn unified_devices_include_old_senders_and_merge_both_directions() {
        let pool = pool().await;
        let mut outgoing = record("old-target", "已发送的旧记录", "success");
        outgoing.peer_name = "旧目标".into();
        let old_id = upsert(&pool, &outgoing).await.unwrap();
        sqlx::query("UPDATE notification_history SET updated_at=updated_at-10000 WHERE id=?")
            .bind(old_id).execute(&pool).await.unwrap();
        let mut incoming = received("from-target", "已接收", "success");
        incoming.notification.source_device_id = "old-target".into();
        incoming.notification.target_device_id = "phone".into();
        incoming.peer_id = "old-target".into();
        incoming.peer_name = "目标新名字".into();
        upsert(&pool, &incoming).await.unwrap();
        upsert(&pool, &record("send-only", "仅发送", "success")).await.unwrap();
        let page = query(&pool, Query { page_size:Some(1), ..Default::default() }).await.unwrap();
        assert!(page.has_more);
        assert_eq!(page.devices.len(), 2);
        assert!(page.devices.iter().any(|d| d.peer_id == "send-only"));
        assert!(page.devices.iter().any(|d| d.peer_id == "old-target" && d.peer_name == "目标新名字"));
        let page = query(&pool, Query { device_id:Some("old-target".into()), ..Default::default() }).await.unwrap();
        assert_eq!(page.records.len(), 2);
        assert_eq!(page.records[0].direction, "receive");
        assert_eq!(page.records[1].direction, "send");
        assert!(page.records.iter().all(|r| r.peer_id == "old-target"));
    }

    #[tokio::test]
    async fn sources_include_devices_outside_the_global_page_and_use_the_latest_name() {
        let pool = pool().await;
        let mut old = received("old-event", "50%", "success");
        old.notification.source_device_id = "old-phone".into();
        old.peer_id = "old-phone".into();
        old.peer_name = "旧手机".into();
        let old_id = upsert(&pool, &old).await.unwrap();
        sqlx::query("UPDATE notification_history SET updated_at=updated_at-10000 WHERE id=?")
            .bind(old_id)
            .execute(&pool)
            .await
            .unwrap();
        let mut recent = received("recent-event", "新通知", "success");
        recent.peer_name = "新名字".into();
        upsert(&pool, &recent).await.unwrap();
        let page = query(
            &pool,
            Query {
                page_size: Some(1),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(page.records.len(), 1);
        assert!(page.has_more);
        assert_eq!(page.devices.len(), 2);
        assert!(page
            .devices
            .iter()
            .any(|d| d.peer_id == "old-phone" && d.peer_name == "旧手机"));
        let page = query(
            &pool,
            Query {
                direction: Some("receive".into()),
                device_id: Some("old-phone".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].notification.text, "50%");
    }

    async fn insert_legacy(
        pool: &sqlx::Pool<Sqlite>,
        id: &str,
        direction: &str,
        source: &str,
        target: &str,
        device_name: &str,
        key: &str,
        updated_at: i64,
    ) {
        sqlx::query(
            "INSERT INTO notification_history (
                id,direction,event_id,source_device_id,target_device_id,device_name,
                package,app_name,app_icon_ref,title,text,notification_key,identity_key,status,
                failure_reason,observed_at,created_at,updated_at,content_hash,post_time
             ) VALUES (?,?,?,?,?,?,?,?,NULL,?,?,?,?,?,NULL,?,?,?,?,?)",
        )
        .bind(id)
        .bind(direction)
        .bind(format!("event-{id}"))
        .bind(source)
        .bind(target)
        .bind(device_name)
        .bind("example.app")
        .bind("Example")
        .bind("Title")
        .bind(format!("text-{id}"))
        .bind(key)
        .bind(key)
        .bind("success")
        .bind(updated_at)
        .bind(updated_at)
        .bind(updated_at)
        .bind(format!("hash-{id}"))
        .bind(updated_at)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn newest_record_named_by_device_id_does_not_hide_the_known_name() {
        let pool = pool().await;
        let now = chrono::Utc::now().timestamp_millis();
        // 旧记录带着真实名称，最新一条是旧版本用设备 ID 充当地名的记录。
        insert_legacy(&pool, "old", "send", "phone", "target-x", "客厅电脑", "k1", now - 5000).await;
        insert_legacy(&pool, "new", "send", "phone", "target-x", "target-x", "k2", now).await;
        let page = query(&pool, Query::default()).await.unwrap();
        let device = page
            .devices
            .iter()
            .find(|device| device.peer_id == "target-x")
            .unwrap();
        assert_eq!(device.peer_name, "客厅电脑");
        // 完全没有真实名称的设备不再把 ID 当作显示名。
        insert_legacy(&pool, "only", "receive", "ghost-id", "phone", "ghost-id", "k3", now).await;
        let page = query(&pool, Query::default()).await.unwrap();
        let ghost = page
            .devices
            .iter()
            .find(|device| device.peer_id == "ghost-id")
            .unwrap();
        assert_eq!(ghost.peer_name, "");
    }

    #[tokio::test]
    async fn offline_device_name_is_remembered_and_backfilled_from_the_device_archive() {
        let pool = pool().await;
        assert_eq!(resolve_device_name(&pool, "dev-1", Some("手机 A")).await, "手机 A");
        assert_eq!(stored_device_name(&pool, "dev-1").await.as_deref(), Some("手机 A"));
        assert_eq!(resolve_device_name(&pool, "dev-1", None).await, "手机 A");
        // 只有设备 ID 或空名时永远不写成名称。
        assert_eq!(resolve_device_name(&pool, "dev-2", Some("dev-2")).await, "");
        assert_eq!(resolve_device_name(&pool, "dev-2", Some("   ")).await, "");
        assert!(stored_device_name(&pool, "dev-2").await.is_none());
        assert!(!remember_device_name(&pool, "dev-2", "dev-2").await.unwrap());

        let now = chrono::Utc::now().timestamp_millis();
        insert_legacy(&pool, "legacy", "send", "phone", "dev-3", "dev-3", "k1", now).await;
        let page = query(&pool, Query::default()).await.unwrap();
        assert_eq!(
            page.devices.iter().find(|device| device.peer_id == "dev-3").unwrap().peer_name,
            ""
        );
        // 设备档案（在线发现写入的用户表）里有真实名称时用于回填。
        sqlx::query("CREATE TABLE users (id TEXT PRIMARY KEY,name TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO users VALUES ('dev-3','旧手机')")
            .execute(&pool)
            .await
            .unwrap();
        initialize(&pool).await.unwrap();
        let page = query(&pool, Query::default()).await.unwrap();
        assert_eq!(
            page.devices.iter().find(|device| device.peer_id == "dev-3").unwrap().peer_name,
            "旧手机"
        );
        assert_eq!(
            page.device_names
                .iter()
                .find(|device| device.peer_id == "dev-3")
                .map(|device| device.peer_name.as_str()),
            Some("旧手机")
        );
        assert_eq!(
            page.records
                .iter()
                .find(|record| record.record_id == "legacy")
                .map(|record| record.peer_name.as_str()),
            Some("旧手机")
        );
    }

    #[tokio::test]
    async fn deleting_a_device_removes_only_its_records_and_archive() {
        let pool = pool().await;
        upsert(&pool, &record("keep-me", "保留", "success")).await.unwrap();
        upsert(&pool, &record("drop-me", "删除", "success")).await.unwrap();
        let mut local = record("local-x", "本机", "success");
        local.peer_id = "local-x".into();
        local.peer_name = "本机".into();
        local.view_kind = "notification_local".into();
        local.notification.source_device_id = "local-x".into();
        local.notification.target_device_id = "local-x".into();
        upsert(&pool, &local).await.unwrap();
        remember_device_name(&pool, "keep-me", "保留设备").await.unwrap();
        remember_device_name(&pool, "drop-me", "删除设备").await.unwrap();

        let (records, names) = delete_device(&pool, "drop-me").await.unwrap();
        assert_eq!((records, names), (1, 1));
        let page = query(&pool, Query::default()).await.unwrap();
        assert!(page.records.iter().all(|record| record.peer_id != "drop-me"));
        assert!(page
            .records
            .iter()
            .any(|record| record.peer_id == "keep-me"));
        assert!(page
            .records
            .iter()
            .any(|record| record.direction == "local"));
        assert!(page
            .device_names
            .iter()
            .any(|device| device.peer_id == "keep-me" && device.peer_name == "保留设备"));
        assert!(!page
            .device_names
            .iter()
            .any(|device| device.peer_id == "drop-me"));
        // 重开数据库也不会把清掉的幽灵条目带回来。
        initialize(&pool).await.unwrap();
        let reopened = query(&pool, Query::default()).await.unwrap();
        assert!(reopened.records.iter().all(|record| record.peer_id != "drop-me"));
        assert!(reopened.devices.iter().all(|device| device.peer_id != "drop-me"));
    }

    #[tokio::test]
    async fn deduplicates_updates_and_keeps_targets_independent() {
        let pool = pool().await;
        let first = upsert(&pool, &record("a", "one", "offline")).await.unwrap();
        let duplicate = upsert(&pool, &record("a", "one", "offline")).await.unwrap();
        assert_eq!(first, duplicate);
        let updated = upsert(&pool, &record("a", "two", "success")).await.unwrap();
        assert_eq!(first, updated);
        let second = upsert(&pool, &record("b", "one", "success")).await.unwrap();
        assert_ne!(first, second);
        let page = query(&pool, Query::default()).await.unwrap();
        assert_eq!(page.records.len(), 2);
        assert_eq!(
            page.records
                .iter()
                .find(|item| item.record_id == first)
                .unwrap()
                .notification
                .text,
            "two"
        );
    }

    #[tokio::test]
    async fn receive_history_keeps_each_event_but_updates_its_status_in_place() {
        let pool = pool().await;
        let first = upsert(&pool, &received("event-1", "one", "receiving"))
            .await
            .unwrap();
        let completed = upsert(&pool, &received("event-1", "one", "success"))
            .await
            .unwrap();
        assert_eq!(first, completed);

        let second = upsert(&pool, &received("event-2", "two", "success"))
            .await
            .unwrap();
        assert_ne!(first, second);

        let page = query(
            &pool,
            Query {
                direction: Some("receive".into()),
                ..Query::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(page.records.len(), 2);
        assert_eq!(
            page.records
                .iter()
                .find(|item| item.record_id == first)
                .unwrap()
                .status,
            "success"
        );
    }

    #[tokio::test]
    async fn filters_pages_and_prunes_strictly_older_than_seven_days() {
        let pool = pool().await;
        upsert(&pool, &record("a", "one", "offline")).await.unwrap();
        upsert(&pool, &record("b", "two", "success")).await.unwrap();
        let page = query(
            &pool,
            Query {
                device_id: Some("b".into()),
                status: Some("success".into()),
                page_size: Some(50),
                ..Query::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(page.records.len(), 1);
        let now = chrono::Utc::now().timestamp_millis();
        let cutoff = now - RETENTION_MILLIS;
        sqlx::query(
            "UPDATE notification_history SET observed_at=?,updated_at=? WHERE target_device_id='a'",
        )
        .bind(cutoff - 1)
        .bind(cutoff - 1)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE notification_history SET observed_at=?,updated_at=? WHERE target_device_id='b'",
        )
        .bind(cutoff)
        .bind(cutoff)
        .execute(&pool)
        .await
        .unwrap();
        prune_at(&pool, now).await.unwrap();
        let remaining: Vec<String> =
            sqlx::query_scalar("SELECT target_device_id FROM notification_history")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(remaining, vec!["b"]);
    }

    #[tokio::test]
    async fn legacy_database_is_upgraded_without_touching_chat_and_history_reopens() {
        let directory = std::env::temp_dir().join(format!(
            "lanchat-history-migration-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let database = directory.join("lanchat.db");
        std::fs::File::create(&database).unwrap();
        let url = format!("sqlite:{}", database.to_string_lossy());
        let legacy = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE messages(id INTEGER PRIMARY KEY,content TEXT,msg_type TEXT)")
            .execute(&legacy)
            .await
            .unwrap();
        sqlx::query("INSERT INTO messages(id,content,msg_type) VALUES(1,'synthetic-chat','text')")
            .execute(&legacy)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE notification_history (
                id TEXT PRIMARY KEY NOT NULL,
                direction TEXT NOT NULL,
                event_id TEXT NOT NULL,
                source_device_id TEXT NOT NULL,
                target_device_id TEXT NOT NULL,
                device_name TEXT NOT NULL,
                package TEXT NOT NULL,
                app_name TEXT NOT NULL,
                app_icon_ref TEXT,
                title TEXT NOT NULL,
                text TEXT NOT NULL,
                notification_key TEXT NOT NULL,
                status TEXT NOT NULL,
                failure_reason TEXT,
                observed_at INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                content_hash TEXT NOT NULL,
                post_time INTEGER NOT NULL,
                UNIQUE(direction, source_device_id, target_device_id, package, notification_key)
            )",
        )
        .execute(&legacy)
        .await
        .unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        sqlx::query(
            "INSERT INTO notification_history (
                id,direction,event_id,source_device_id,target_device_id,device_name,
                package,app_name,app_icon_ref,title,text,notification_key,status,failure_reason,
                observed_at,created_at,updated_at,content_hash,post_time
             ) VALUES (
                'legacy-record','receive','legacy-event','phone','receiver','Device phone',
                'example.app','Example',NULL,'Title','legacy','stable-key','success',NULL,
                ?,?,?,'legacy-hash',?
             )",
        )
        .bind(now)
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(&legacy)
        .await
        .unwrap();
        initialize(&legacy).await.unwrap();
        assert!(get(&legacy, "legacy-record").await.unwrap().is_some());
        let additional = upsert(&legacy, &received("new-event", "new", "success"))
            .await
            .unwrap();
        assert_ne!(additional, "legacy-record");
        let received_page = query(
            &legacy,
            Query {
                direction: Some("receive".into()),
                ..Query::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(received_page.records.len(), 2);
        let id = upsert(&legacy, &record("a", "persisted", "success"))
            .await
            .unwrap();
        legacy.close().await;

        let reopened = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .unwrap();
        initialize(&reopened).await.unwrap();
        assert!(get(&reopened, &id).await.unwrap().is_some());
        let chat: String = sqlx::query_scalar("SELECT content FROM messages WHERE id=1")
            .fetch_one(&reopened)
            .await
            .unwrap();
        assert_eq!(chat, "synthetic-chat");
        reopened.close().await;
        let _ = std::fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn bad_icon_degrades_and_pagination_is_bounded() {
        let pool = pool().await;
        let mut first = record("a", "one", "success");
        first.notification.app_icon = Some("https://invalid.example/icon.png".into());
        upsert(&pool, &first).await.unwrap();
        upsert(&pool, &record("b", "two", "success")).await.unwrap();
        let page = query(
            &pool,
            Query {
                page_size: Some(1),
                ..Query::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(page.records.len(), 1);
        assert!(page.has_more);
        let bad = query(
            &pool,
            Query {
                device_id: Some("a".into()),
                ..Query::default()
            },
        )
        .await
        .unwrap();
        assert!(bad.records[0].notification.app_icon.is_none());
    }
}
