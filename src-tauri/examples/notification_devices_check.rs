//! 隔离执行的通知设备名/离线清理检查：只用内存数据库，不接触真实数据。
use lanchat::notification_history::{self as history, HistoryDevice, Query};
use lanchat::notification_sync::{self as sync, Notification, Record, Settings};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Pool, Row, Sqlite};
use std::sync::Mutex;

#[derive(Default)]
struct Report {
    results: Mutex<Vec<(String, bool, String)>>,
}

impl Report {
    fn check(&self, name: &str, ok: bool, detail: impl Into<String>) {
        let detail = detail.into();
        println!("{} {}", if ok { "PASS" } else { "FAIL" }, format!("{name} :: {detail}"));
        self.results.lock().unwrap().push((name.to_string(), ok, detail));
    }
}

fn record(target: &str, name: &str, text: &str, key: &str) -> Record {
    Record {
        record_id: String::new(),
        peer_id: target.into(),
        peer_name: name.into(),
        view_kind: "notification_push".into(),
        direction: "send".into(),
        status: "success".into(),
        failure_reason: None,
        observed_at: 0,
        created_at: 0,
        updated_at: 0,
        content_hash: String::new(),
        notification: Notification {
            msg_type: "notification".into(),
            event_id: format!("event-{target}-{key}"),
            source_device_id: "phone".into(),
            target_device_id: target.into(),
            package: "example.app".into(),
            app_name: "Example".into(),
            app_icon: None,
            title: "Title".into(),
            text: text.into(),
            notification_key: key.into(),
            post_time: 1,
        },
    }
}

async fn insert_legacy(
    pool: &Pool<Sqlite>,
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
    .expect("legacy insert");
}

fn device_name_of<'a>(devices: &'a [HistoryDevice], peer_id: &str) -> Option<&'a str> {
    devices
        .iter()
        .find(|device| device.peer_id == peer_id)
        .map(|device| device.peer_name.as_str())
}

async fn run(report: &Report) {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory database");
    history::initialize(&pool).await.expect("schema");
    let now = chrono::Utc::now().timestamp_millis();
    sqlx::query("CREATE TABLE settings (key TEXT PRIMARY KEY,value TEXT NOT NULL)")
        .execute(&pool)
        .await
        .expect("settings table");

    // 1. 旧记录把设备 ID 写成地名 → 不再作为显示名。
    insert_legacy(&pool, "uuid-old", "send", "phone", "4f1999f0-a71d", "4f1999f0-a71d", "k1", now - 60000).await;
    let page = history::query(&pool, Query::default()).await.expect("query");
    report.check(
        "旧 UUID 记录不再用 ID 当标题（返回空名，界面对应“未知设备”）",
        device_name_of(&page.devices, "4f1999f0-a71d") == Some(""),
        format!("{:?}", device_name_of(&page.devices, "4f1999f0-a71d")),
    );

    // 2. 有真实名称时，最新一条“ID 地名”的旧记录不得盖掉它。
    insert_legacy(&pool, "known-old", "send", "phone", "device-known", "客厅电脑", "k2", now - 50000).await;
    insert_legacy(&pool, "known-new", "send", "phone", "device-known", "device-known", "k3", now - 40000).await;
    let page = history::query(&pool, Query::default()).await.expect("query");
    report.check(
        "最新一条是旧版本写入的 ID 时仍显示真实名称",
        device_name_of(&page.devices, "device-known") == Some("客厅电脑"),
        format!("{:?}", device_name_of(&page.devices, "device-known")),
    );

    // 3. 在线发现/推送写入的名字进入设备名档案，离线后仍可解析。
    let written = history::upsert(&pool, &record("device-known", "客厅电脑", "推送", "k4")).await;
    report.check("推送记录写入成功", written.is_ok(), format!("{written:?}"));
    let archived = history::stored_device_name(&pool, "device-known").await;
    report.check(
        "设备名写入档案",
        archived.as_deref() == Some("客厅电脑"),
        format!("{archived:?}"),
    );
    let offline_resolved = history::resolve_device_name(&pool, "device-known", None).await;
    report.check(
        "设备离线（无在线名）时仍解析出名称",
        offline_resolved == "客厅电脑",
        offline_resolved.clone(),
    );
    let live = history::resolve_device_name(&pool, "device-known", Some("新名字")).await;
    report.check(
        "在线名优先并更新档案",
        live == "新名字" && history::stored_device_name(&pool, "device-known").await.as_deref() == Some("新名字"),
        live,
    );

    // 4. 设备档案（users 表）里的真实名称可回填旧记录。
    sqlx::query("CREATE TABLE users (id TEXT PRIMARY KEY,name TEXT)")
        .execute(&pool)
        .await
        .expect("users table");
    sqlx::query("INSERT INTO users VALUES ('device-legacy','旧手机')")
        .execute(&pool)
        .await
        .expect("users row");
    insert_legacy(&pool, "legacy-row", "send", "phone", "device-legacy", "device-legacy", "k5", now - 30000).await;
    history::initialize(&pool).await.expect("reinitialize");
    let page = history::query(&pool, Query::default()).await.expect("query");
    report.check(
        "旧记录用真实档案名回填",
        device_name_of(&page.devices, "device-legacy") == Some("旧手机"),
        format!("{:?}", device_name_of(&page.devices, "device-legacy")),
    );
    let mut recovered: Option<String> = None;
    for row in page.records.iter().filter(|row| row.record_id == "legacy-row") {
        recovered = Some(row.peer_name.clone());
    }
    report.check(
        "回填写入通知记录本身",
        recovered.as_deref() == Some("旧手机"),
        format!("{recovered:?}"),
    );
    report.check(
        "无法恢复名称的设备保持空名",
        device_name_of(&page.devices, "4f1999f0-a71d") == Some(""),
        format!("{:?}", device_name_of(&page.devices, "4f1999f0-a71d")),
    );
    report.check(
        "设备名档案同时下发（供离线设备显示）",
        page.device_names.iter().any(|device| device.peer_id == "device-known"),
        format!("{:?}", page.device_names.iter().map(|d| &d.peer_id).collect::<Vec<_>>()),
    );

    // 5. 清理单台离线设备：删除记录与档案，保留在线设备与本机记录。
    let mut local = record("phone", "本机", "本机记录", "k6");
    local.peer_id = "phone".into();
    local.view_kind = "notification_local".into();
    local.direction = "local".into();
    local.notification.source_device_id = "phone".into();
    local.notification.target_device_id = "phone".into();
    local.notification.post_time = now;
    history::upsert(&pool, &local).await.expect("local record");
    history::upsert(&pool, &record("device-known", "客厅电脑", "在线设备", "k7"))
        .await
        .expect("online device record");
    let (records, names) = history::delete_device(&pool, "device-legacy").await.expect("delete");
    report.check(
        "清理删除该设备记录与档案",
        records >= 1 && names == 1,
        format!("records={records} names={names}"),
    );
    let page = history::query(&pool, Query::default()).await.expect("query");
    report.check(
        "被清理设备不再出现",
        !page.records.iter().any(|row| row.peer_id == "device-legacy")
            && !page.devices.iter().any(|device| device.peer_id == "device-legacy")
            && !page.device_names.iter().any(|device| device.peer_id == "device-legacy"),
        format!("{} records", page.records.len()),
    );
    report.check(
        "在线设备记录保留",
        page.records.iter().any(|row| row.peer_id == "device-known"),
        "",
    );
    report.check(
        "本机记录保留",
        page.records.iter().any(|row| row.direction == "local"),
        "",
    );

    // 6. 重开（重新初始化）后不恢复已清理的幽灵条目。
    history::initialize(&pool).await.expect("reinitialize");
    let page = history::query(&pool, Query::default()).await.expect("query");
    report.check(
        "重载后不恢复已清理条目",
        !page.devices.iter().any(|device| device.peer_id == "device-legacy")
            && !page.records.iter().any(|row| row.peer_id == "device-legacy"),
        format!("{} devices", page.devices.len()),
    );

    // 7. 推送目标引用同步移除。
    let settings = Settings {
        push_enabled: true,
        target_device_ids: vec!["device-legacy".into(), "device-known".into(), "online-peer".into()],
        ..Default::default()
    };
    let (next, removed) = sync::without_target_devices(&settings, &["device-legacy".into()]);
    report.check(
        "只移除被清理的目标引用",
        removed == vec!["device-legacy".to_string()]
            && next.target_device_ids == vec!["device-known".to_string(), "online-peer".to_string()],
        format!("{:?}", next.target_device_ids),
    );
    report.check(
        "设置写入后重读一致（含 receive 键）",
        {
            sqlx::query("INSERT OR REPLACE INTO settings (key,value) VALUES ('notification_sync_desktop_v1',?)")
                .bind(serde_json::to_string(&next).unwrap())
                .execute(&pool)
                .await
                .expect("settings write");
            let stored = sync::desktop_settings(&pool).await;
            stored.target_device_ids == next.target_device_ids
        },
        "",
    );

    // 8. 离线判定：在线/可探测/无地址/未知设备。
    let mut online = peer("phone-online", "192.168.5.10:8888", false);
    report.check("在线设备永不清理", !sync::device_is_offline(Some(&online), false), "");
    online.is_offline = true;
    report.check(
        "已标记离线但探测可达时保留",
        !sync::device_is_offline(Some(&online), true),
        "",
    );
    report.check("探测失败判为离线", sync::device_is_offline(Some(&online), false), "");
    online.addr = String::new();
    report.check(
        "没有地址（本次会话未发现）按离线处理",
        sync::device_is_offline(Some(&online), true),
        "",
    );
    report.check("设备档案里没有该设备按离线处理", sync::device_is_offline(None, false), "");

    // 9. 候选过滤：本机与重复项、非法 ID。
    let candidates = sync::cleanup_candidate_ids(
        vec![" phone ".into(), "ghost-1".into(), "phone".into(), "ghost-1".into()],
        "phone",
    )
    .expect("candidates");
    report.check(
        "本机不进候选、重复项去重",
        candidates == vec!["ghost-1".to_string()],
        format!("{candidates:?}"),
    );
    report.check(
        "非法 ID 直接拒绝",
        sync::cleanup_candidate_ids(vec!["bad id".into()], "phone").is_err(),
        "",
    );

    // 10. 旧数据里“空名”也确实被清空，不再以 ID 呈现。
    let blank: Option<String> = sqlx::query("SELECT device_name FROM notification_history WHERE id='uuid-old'")
        .fetch_one(&pool)
        .await
        .ok()
        .map(|row| row.get("device_name"));
    report.check(
        "旧 UUID 记录的库内名称已清空",
        blank.as_deref() == Some(""),
        format!("{blank:?}"),
    );
    // 11. 统一清理的通知域实现：先删记录再移除目标引用，在线设备与本机保留。
    sqlx::query("INSERT OR REPLACE INTO settings (key,value) VALUES ('notification_sync_desktop_v1',?)")
        .bind(serde_json::to_string(&Settings {
            push_enabled: true,
            receive_enabled: true,
            target_device_ids: vec![
                "keep-online".into(),
                "probe-offline".into(),
                "ghost-only".into(),
            ],
            ..Default::default()
        })
        .unwrap())
        .execute(&pool)
        .await
        .expect("settings seed");
    let online = peer("keep-online", "192.168.5.10:8888", false);
    let offline = peer("probe-offline", "", true);
    let peers = vec![online.clone(), offline.clone()];
    let client = reqwest::Client::new();
    let ids = vec![
        "keep-online".to_string(),
        "probe-offline".to_string(),
        "ghost-only".to_string(),
        "me".to_string(),
    ];
    let candidates = sync::cleanup_candidate_ids(ids, "me").expect("ids");
    let cleanup = sync::clear_notification_devices_for(&pool, &peers, &client, candidates)
        .await
        .expect("notification cleanup");
    let cleared: Vec<String> = cleanup["cleared"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect();
    let kept: Vec<String> = cleanup["kept"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|value| value["peer_id"].as_str().map(str::to_owned))
        .collect();
    let targets: Vec<String> = cleanup["targets_removed"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect();
    report.check(
        "统一清理：在线设备与本机不清理",
        kept == vec!["keep-online".to_string()] && !cleared.contains(&"me".to_string()),
        format!("kept={kept:?} cleared={cleared:?}"),
    );
    report.check(
        "统一清理：离线与未发现设备都按离线处理（含空设备列表场景）",
        cleared.contains(&"probe-offline".to_string()) && cleared.contains(&"ghost-only".to_string()),
        format!("{cleared:?}"),
    );
    report.check(
        "统一清理：记录删除成功后才移除对应推送目标，其余目标保留",
        targets == vec!["probe-offline".to_string(), "ghost-only".to_string()]
            && cleanup["target_error"].is_null(),
        format!("{targets:?}"),
    );
    let remaining: Vec<String> = sqlx::query_scalar("SELECT peer_id FROM notification_device_names")
        .fetch_all(&pool)
        .await
        .unwrap();
    report.check(
        "统一清理：设备名档案只删被清理设备",
        !remaining.contains(&"probe-offline".to_string())
            && !remaining.contains(&"ghost-only".to_string()),
        format!("{remaining:?}"),
    );
    let again = sync::clear_notification_devices_for(
        &pool,
        &peers,
        &client,
        vec!["probe-offline".to_string()],
    )
    .await
    .expect("second cleanup");
    report.check(
        "统一清理：重复执行收敛（记录已无、目标已无引用）",
        again["cleared"].as_array().unwrap().len() == 1
            && again["records_deleted"].as_u64() == Some(0)
            && again["targets_removed"].as_array().unwrap().is_empty(),
        format!("{again}"),
    );

    pool.close().await;
}

fn peer(id: &str, addr: &str, is_offline: bool) -> lanchat::peers::Peer {
    lanchat::peers::Peer {
        id: id.into(),
        name: "设备".into(),
        addr: addr.into(),
        last_seen: 1,
        is_offline,
        available_memory_mb: 0,
        notification_push_enabled: false,
        notification_receive_enabled: true,
        notification_push_target_device_ids: Vec::new(),
    }
}

fn main() {
    let report = Report::default();
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(run(&report));
    let results = report.results.lock().unwrap();
    let passed = results.iter().filter(|(_, ok, _)| *ok).count();
    let failed: Vec<&String> = results.iter().filter(|(_, ok, _)| !ok).map(|(name, _, _)| name).collect();
    println!(
        "SUMMARY {}/{}",
        passed,
        results.len()
    );
    if !failed.is_empty() {
        println!("FAILED: {failed:?}");
        std::process::exit(1);
    }
}