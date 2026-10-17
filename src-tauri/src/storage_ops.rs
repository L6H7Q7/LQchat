//! 聊天数据清理与下载目录迁移。
//!
//! 这两个操作只处理应用自己管理的文件（应用数据目录里的 outbox/staging 副本，以及
//! 应用自己导出到下载目录的接收文件），绝不删除用户选中用于发送的原始文件或已安装应用。
//!
//! 安全顺序固定为：
//! 1. 先按归一化路径检查共享引用与来源，再决定是否删除；
//! 2. 迁移时“复制/导出 → 核对完整 → 更新记录 → 删除原文件”，
//!    原文件删不掉时登记待清理路径，下次重试；
//! 3. 清理时先删除文件、再删除记录、最后才释放 Android 持久化 URI 权限，
//!    失败一律上报，不谎报成功。

use sqlx::{Pool, Sqlite};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, serde::Serialize)]
pub struct FileFailure {
    pub path: String,
    pub reason: String,
}

/// 清理一条聊天时对应用文件的处理结果。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct FileCleanupReport {
    /// 真正从磁盘删除的文件数。
    pub deleted: usize,
    /// 记录中引用但磁盘上已不存在的文件数。
    pub missing: usize,
    /// 有意保留的文件数（共享引用、用户原始文件、非应用管理路径）。
    pub kept: usize,
    /// 释放的持久化 URI 权限数（Android SAF）。
    pub released: usize,
    /// 删除失败的文件，前端必须据此提示，不能报成功。
    pub failures: Vec<FileFailure>,
}

impl FileCleanupReport {
    pub fn failed(&self) -> bool {
        !self.failures.is_empty()
    }
}

/// 清理结果 + 需要在记录删除之后再释放的 Android 资源。
#[derive(Debug, Clone, Default)]
pub struct CleanupOutcome {
    pub report: FileCleanupReport,
    /// 该聊天缓存的 FD（按 msg_id），记录删除后释放。
    pub fd_msg_ids: Vec<i64>,
    /// 该聊天持有的持久化 URI 权限（msg_id, uri），记录删除后才释放。
    pub persisted_uris: Vec<(i64, String)>,
}

/// “立刻迁移”把已保存文件搬到当前下载位置的结果。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct MigrationReport {
    pub target: String,
    pub migrated: usize,
    pub already_in_target: usize,
    pub skipped: usize,
    /// 上次迁移遗留、本次成功清理掉的原文件数量。
    pub pending_cleaned: usize,
    pub failures: Vec<FileFailure>,
}

impl MigrationReport {
    pub fn failed(&self) -> bool {
        !self.failures.is_empty()
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct PathUsage {
    sent: bool,
    received: bool,
}

/// 统一路径比较口径：解析符号链接、去掉 Windows 扩展前缀、忽略大小写。
pub fn normalize_path(path: &Path) -> String {
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut text = resolved.to_string_lossy().replace('\\', "/");
    if let Some(stripped) = text.strip_prefix("//?/") {
        text = stripped.to_string();
    }
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    }
}

/// `path` 是否位于 `root` 目录之内（含目录本身）。
pub fn path_within(path: &Path, root: &Path) -> bool {
    if path.as_os_str().is_empty() || root.as_os_str().is_empty() {
        return false;
    }
    let target = normalize_path(path);
    let base = normalize_path(root);
    let base = base.trim_end_matches('/');
    target == base || target.starts_with(&format!("{base}/"))
}

/// 当前运行中的应用数据目录；Web 版没有 Tauri 句柄时返回 `None`。
#[cfg(feature = "desktop")]
pub fn current_app_data_dir() -> Option<PathBuf> {
    use tauri::Manager;
    crate::CURRENT_APP_HANDLE
        .get()
        .and_then(|handle| handle.read().ok())
        .and_then(|handle| handle.clone())
        .and_then(|app| app.path().app_data_dir().ok())
}

#[cfg(not(feature = "desktop"))]
pub fn current_app_data_dir() -> Option<PathBuf> {
    None
}

/// 发送时使用的安全文件名：去掉路径分隔符与控制字符，保留合法扩展名与中文名称。
pub fn sanitize_file_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut cleaned = String::new();
    for ch in trimmed.chars() {
        if ch.is_control() || matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
            cleaned.push('_');
        } else {
            cleaned.push(ch);
        }
    }
    let cleaned = cleaned.trim().trim_matches('.').trim().to_string();
    if cleaned.is_empty() {
        return None;
    }
    let limited: String = cleaned.chars().take(120).collect();
    Some(limited)
}

/// 该聊天引用的文件路径及方向，以及被其他聊天引用的归一化路径集合。
async fn chat_file_usage(
    pool: &Pool<Sqlite>,
    my_id: &str,
    peer_id: &str,
) -> Result<(BTreeMap<String, PathUsage>, HashSet<String>), String> {
    let scope = "(sender_id = 'me' AND receiver_id = ?1) \
        OR (sender_id = ?2 AND (receiver_id = ?3 OR receiver_id IS NULL))";
    let rows: Vec<(String, Option<String>)> = sqlx::query_as(&format!(
        "SELECT file_path, sender_id FROM messages \
         WHERE file_path IS NOT NULL AND file_path <> '' AND ({scope})"
    ))
    .bind(peer_id)
    .bind(peer_id)
    .bind(my_id)
    .fetch_all(pool)
    .await
    .map_err(|error| format!("读取聊天文件记录失败: {error}"))?;

    let mut mine: BTreeMap<String, PathUsage> = BTreeMap::new();
    for (path, sender) in rows {
        let usage = mine.entry(path).or_default();
        if sender.as_deref() == Some("me") {
            usage.sent = true;
        } else {
            usage.received = true;
        }
    }

    let others: Vec<(String,)> = sqlx::query_as(&format!(
        "SELECT DISTINCT file_path FROM messages \
         WHERE file_path IS NOT NULL AND file_path <> '' AND NOT ({scope})"
    ))
    .bind(peer_id)
    .bind(peer_id)
    .bind(my_id)
    .fetch_all(pool)
    .await
    .map_err(|error| format!("读取共享文件引用失败: {error}"))?;

    let shared = others
        .into_iter()
        .map(|(path,)| {
            if path.starts_with("content://") || path.starts_with("fd:") {
                path
            } else {
                normalize_path(Path::new(&path))
            }
        })
        .collect();

    Ok((mine, shared))
}

/// content:// 文档是否由应用创建（应用管理副本），用于区分用户选来发送的原始文档。
///
/// 判定依据（任一成立即可）：
/// 1. 该 URI 在“应用导出/迁移产生”的精确凭据列表里（含旧下载目录迁移产生的副本）；
/// 2. 本机收到、非发送源，且位于应用用过的下载目录（download_trees 历史）之下。
pub fn content_uri_is_app_owned(
    path: &str,
    sent: bool,
    received: bool,
    trees: &[String],
    exported: &[String],
) -> bool {
    if exported.iter().any(|uri| uri == path) {
        return true;
    }
    received
        && !sent
        && trees
            .iter()
            .any(|tree| !tree.is_empty() && path.starts_with(tree.as_str()))
}

fn normalized_key(path: &str) -> String {
    if path.starts_with("content://") || path.starts_with("fd:") {
        path.to_string()
    } else {
        normalize_path(Path::new(path))
    }
}

/// 读取当前设置的接收目录与下载位置，供清理/迁移使用。
pub async fn current_storage_paths(pool: &Pool<Sqlite>) -> (Option<PathBuf>, Option<PathBuf>) {
    let receive = crate::db::get_receive_directory(pool)
        .await
        .ok()
        .map(PathBuf::from);
    let download = crate::db::get_download_path(pool)
        .await
        .ok()
        .map(PathBuf::from);
    (receive, download)
}

/// 清除某条聊天对应的本机文件。
///
/// - 共享路径（其他聊天仍引用）一律保留；
/// - 发送源文件（不在应用数据目录里的）一律保留；
/// - 只有“本机收到”且位于接收/下载目录、或位于应用数据目录的文件才删除；
/// - content:// 文档仅在本机收到且位于应用用过的下载目录下时才删除。
pub async fn cleanup_chat_files(
    pool: &Pool<Sqlite>,
    my_id: &str,
    peer_id: &str,
    app_data_dir: Option<&Path>,
    receive_dir: Option<&Path>,
    download_dir: Option<&Path>,
) -> Result<CleanupOutcome, String> {
    let (mine, shared) = chat_file_usage(pool, my_id, peer_id).await?;
    let trees = crate::db::get_download_trees(pool).await;
    let exported = crate::db::get_exported_uris(pool).await;
    let outbox_dir = app_data_dir.map(|dir| dir.join("outbox"));
    let mut outcome = CleanupOutcome::default();

    let msg_ids: Vec<(i64,)> = sqlx::query_as(&format!(
        "SELECT id FROM messages \
         WHERE (sender_id = 'me' AND receiver_id = ?1) \
            OR (sender_id = ?2 AND (receiver_id = ?3 OR receiver_id IS NULL))"
    ))
    .bind(peer_id)
    .bind(peer_id)
    .bind(my_id)
    .fetch_all(pool)
    .await
    .map_err(|error| format!("读取聊天消息 ID 失败: {error}"))?;
    outcome.fd_msg_ids = msg_ids.into_iter().map(|(id,)| id).collect();

    let persisted: Vec<(i64, String)> = sqlx::query_as(&format!(
        "SELECT p.msg_id, p.uri FROM persisted_uris p WHERE p.msg_id IN \
         (SELECT id FROM messages WHERE (sender_id = 'me' AND receiver_id = ?1) \
            OR (sender_id = ?2 AND (receiver_id = ?3 OR receiver_id IS NULL)))"
    ))
    .bind(peer_id)
    .bind(peer_id)
    .bind(my_id)
    .fetch_all(pool)
    .await
    .map_err(|error| format!("读取持久化 URI 记录失败: {error}"))?;

    for (path, usage) in &mine {
        if shared.contains(&normalized_key(path)) {
            outcome.report.kept += 1;
            continue;
        }
        if path.starts_with("fd:") {
            continue;
        }
        if path.starts_with("content://") {
            cleanup_content_uri(path, *usage, &trees, &exported, &mut outcome.report);
            continue;
        }
        if path.starts_with("http://") || path.starts_with("https://") {
            outcome.report.kept += 1;
            continue;
        }

        let file = Path::new(path);
        let in_app_data = app_data_dir.map(|dir| path_within(file, dir)).unwrap_or(false);
        let in_outbox = outbox_dir
            .as_deref()
            .map(|dir| path_within(file, dir))
            .unwrap_or(false);
        let in_receive = receive_dir.map(|dir| path_within(file, dir)).unwrap_or(false);
        let in_download = download_dir.map(|dir| path_within(file, dir)).unwrap_or(false);
        // 应用管理副本判定（与迁移保持一致）：
        //   1. outbox 里由应用自己创建的发送副本；
        //   2. 本机收到的接收文件（应用数据目录 / 接收目录 / 下载目录内）。
        // 其余情形都是“发送源”：即使它位于应用 private 目录（如用户自己写入或分享进来的原文件），
        // 也不是应用创建的副本，必须保留。
        let deletable = in_outbox || (usage.received && (in_app_data || in_receive || in_download));
        if !deletable {
            outcome.report.kept += 1;
            continue;
        }
        if !file.exists() {
            outcome.report.missing += 1;
            continue;
        }
        match tokio::fs::remove_file(file).await {
            Ok(()) => outcome.report.deleted += 1,
            Err(error) => outcome.report.failures.push(FileFailure {
                path: path.clone(),
                reason: format!("无法删除文件: {error}"),
            }),
        }
    }

    // 记录删除之后才释放权限：共享 URI 不释放，避免影响其他聊天。
    outcome.persisted_uris = persisted
        .into_iter()
        .filter(|(_, uri)| !shared.contains(uri))
        .collect();

    Ok(outcome)
}

fn cleanup_content_uri(
    path: &str,
    usage: PathUsage,
    trees: &[String],
    exported: &[String],
    report: &mut FileCleanupReport,
) {
    // 只有应用自己创建/导出的文档才删除；用户选来发送的原始文档一律保留。
    let app_owned = content_uri_is_app_owned(path, usage.sent, usage.received, trees, exported);
    if !app_owned {
        report.kept += 1;
        return;
    }
    #[cfg(target_os = "android")]
    {
        match crate::android_fd::delete_exported_file(path) {
            Ok(()) => report.deleted += 1,
            Err(error) => report.failures.push(FileFailure {
                path: path.to_string(),
                reason: format!("无法删除系统目录中的文件: {error}"),
            }),
        }
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = (usage, trees, exported, path);
        report.kept += 1;
    }
}

/// 记录删除之后调用：释放该聊天占用的 FD 缓存与持久化 URI 权限。
pub async fn finalize_cleanup(pool: &Pool<Sqlite>, outcome: &CleanupOutcome) {
    for msg_id in &outcome.fd_msg_ids {
        #[cfg(target_os = "android")]
        crate::android_fd::remove_cached_fd(*msg_id);
        #[cfg(not(target_os = "android"))]
        let _ = msg_id;
    }
    for (msg_id, uri) in &outcome.persisted_uris {
        #[cfg(target_os = "android")]
        let _ = crate::android_fd::AndroidFile::release_persistable_uri_permission(uri);
        #[cfg(not(target_os = "android"))]
        let _ = uri;
        let _ = crate::db::remove_persisted_uri_by_msg_id(pool, *msg_id).await;
    }
}

/// 清除一条聊天的统一入口：清理文件并返回需要在记录删除后释放的资源。
pub async fn cleanup_peer_chat_files(
    pool: &Pool<Sqlite>,
    my_id: &str,
    peer_id: &str,
) -> Result<CleanupOutcome, String> {
    let app_data = current_app_data_dir();
    let (receive, download) = current_storage_paths(pool).await;
    cleanup_chat_files(
        pool,
        my_id,
        peer_id,
        app_data.as_deref(),
        receive.as_deref(),
        download.as_deref(),
    )
    .await
}

/// 所有记录的“发送/接收”方向（按原始路径聚合），供迁移判断来源。
async fn all_file_usage(pool: &Pool<Sqlite>) -> Result<BTreeMap<String, PathUsage>, String> {
    let rows: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT file_path, sender_id FROM messages WHERE file_path IS NOT NULL AND file_path <> ''",
    )
    .fetch_all(pool)
    .await
    .map_err(|error| format!("读取已保存文件记录失败: {error}"))?;
    let mut usage: BTreeMap<String, PathUsage> = BTreeMap::new();
    for (path, sender) in rows {
        let entry = usage.entry(path).or_default();
        if sender.as_deref() == Some("me") {
            entry.sent = true;
        } else {
            entry.received = true;
        }
    }
    Ok(usage)
}

fn file_name_of(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("LQChat-file")
        .to_string()
}

fn sha256_of(path: &Path) -> Result<Vec<u8>, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|error| format!("无法读取文件: {error}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| format!("读取文件失败: {error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_vec())
}

/// 复制后核对大小与 SHA-256；任何不一致都说明目标不完整，立即删除目标并报错。
async fn copy_verified(source: &Path, destination: &Path) -> Result<u64, String> {
    if destination.exists() {
        return Err("目标文件已存在".to_string());
    }
    tokio::fs::copy(source, destination)
        .await
        .map_err(|error| format!("复制文件失败: {error}"))?;
    let check = (|| -> Result<u64, String> {
        let source_size = std::fs::metadata(source)
            .map_err(|error| format!("读取原文件失败: {error}"))?
            .len();
        let copied_size = std::fs::metadata(destination)
            .map_err(|error| format!("读取目标文件失败: {error}"))?
            .len();
        if source_size != copied_size {
            return Err(format!("目标文件不完整: {copied_size}/{source_size} 字节"));
        }
        if sha256_of(source)? != sha256_of(destination)? {
            return Err("目标文件校验值与原文件不一致".to_string());
        }
        Ok(copied_size)
    })();
    if check.is_err() {
        let _ = tokio::fs::remove_file(destination).await;
    }
    check
}

fn unique_target_path(directory: &Path, name: &str) -> PathBuf {
    let candidate = directory.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("LQChat-file");
    let extension = path.extension().and_then(|value| value.to_str());
    for index in 1..10_000 {
        let next = match extension {
            Some(extension) => directory.join(format!("{stem} ({index}).{extension}")),
            None => directory.join(format!("{stem} ({index})")),
        };
        if !next.exists() {
            return next;
        }
    }
    directory.join(format!("{stem}-{}{}", uuid::Uuid::new_v4(), match extension {
        Some(extension) => format!(".{extension}"),
        None => String::new(),
    }))
}

const IN_FLIGHT_STATUSES: [&str; 7] = [
    "uploading",
    "downloading",
    "saving",
    "offering",
    "retrying",
    "pending",
    "offered",
];

/// 把当前已保存的应用管理文件搬到新的下载位置。
///
/// 顺序固定为：复制/导出 → 核对目标完整 → 更新记录引用 → 删除原文件。
/// 目标不完整时不会删除原文件；原文件删不掉时登记到待清理列表，下次迁移重试。
pub async fn migrate_saved_files(
    pool: &Pool<Sqlite>,
    app_data_dir: Option<&Path>,
    receive_dir: Option<&Path>,
    download_path: &str,
) -> MigrationReport {
    let mut report = MigrationReport {
        target: download_path.to_string(),
        ..Default::default()
    };

    let target = download_path.trim();
    if target.is_empty() {
        report.failures.push(FileFailure {
            path: String::new(),
            reason: "尚未设置下载位置".to_string(),
        });
        return report;
    }
    let target_is_tree = target.starts_with("content://");
    if target_is_tree {
        // 迁移目标也是一个“应用用过的下载目录”，后续清理据此判断来源。
        let _ = crate::db::remember_download_tree(pool, target).await;
    }
    let target_dir = if target_is_tree {
        None
    } else {
        Some(PathBuf::from(target))
    };
    if let Some(directory) = target_dir.as_deref() {
        if let Err(error) = tokio::fs::create_dir_all(directory).await {
            report.failures.push(FileFailure {
                path: target.to_string(),
                reason: format!("无法创建下载目录: {error}"),
            });
            return report;
        }
    }

    // 先重试上次遗留的原文件清理。
    for leftover in crate::db::get_pending_cleanup(pool).await {
        let candidate = PathBuf::from(&leftover);
        let managed = app_data_dir.map(|dir| path_within(&candidate, dir)).unwrap_or(false)
            || receive_dir.map(|dir| path_within(&candidate, dir)).unwrap_or(false);
        if !candidate.exists() || !managed {
            let _ = crate::db::remove_pending_cleanup(pool, &leftover).await;
            continue;
        }
        match tokio::fs::remove_file(&candidate).await {
            Ok(()) => {
                let _ = crate::db::remove_pending_cleanup(pool, &leftover).await;
                report.pending_cleaned += 1;
            }
            Err(error) => report.failures.push(FileFailure {
                path: leftover.clone(),
                reason: format!("上次迁移遗留的原文件仍未删除: {error}"),
            }),
        }
    }

    let rows: Vec<(String, Option<String>)> = match sqlx::query_as(
        "SELECT DISTINCT file_path, file_status FROM messages WHERE file_path IS NOT NULL AND file_path <> ''",
    )
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(error) => {
            report.failures.push(FileFailure {
                path: String::new(),
                reason: format!("读取已保存文件失败: {error}"),
            });
            return report;
        }
    };

    let in_flight: HashSet<String> = rows
        .iter()
        .filter(|(_, status)| {
            status
                .as_deref()
                .map(|value| IN_FLIGHT_STATUSES.contains(&value))
                .unwrap_or(false)
        })
        .map(|(path, _)| normalized_key(path))
        .collect();
    let trees = crate::db::get_download_trees(pool).await;
    let exported = crate::db::get_exported_uris(pool).await;
    let usage_map = match all_file_usage(pool).await {
        Ok(map) => map,
        Err(error) => {
            report.failures.push(FileFailure {
                path: String::new(),
                reason: error,
            });
            return report;
        }
    };
    // 只有应用自己创建的 outbox 副本才可以整体迁移；用户放进私有目录的文件不算。
    let outbox_dir = app_data_dir.map(|dir| dir.join("outbox"));
    let mut seen: HashSet<String> = HashSet::new();

    for (path, status) in rows {
        if !seen.insert(path.clone()) {
            continue;
        }
        if path.starts_with("fd:") {
            report.skipped += 1;
            continue;
        }
        if in_flight.contains(&normalized_key(&path))
            || status
                .as_deref()
                .map(|value| IN_FLIGHT_STATUSES.contains(&value))
                .unwrap_or(false)
        {
            // 正在传输的文件不动，避免打断发送/接收。
            report.skipped += 1;
            continue;
        }
        if path.starts_with("content://") {
            let usage = usage_map.get(&path).copied().unwrap_or_default();
            // 只迁移应用自己导出/迁移产生的文档；用户选来发送的 content:// 原始文档不动。
            if !content_uri_is_app_owned(&path, usage.sent, usage.received, &trees, &exported) {
                report.skipped += 1;
                continue;
            }
            migrate_content_uri(
                pool,
                &path,
                target,
                target_dir.as_deref(),
                receive_dir,
                &mut report,
            )
            .await;
            continue;
        }
        if path.starts_with("http://") || path.starts_with("https://") {
            report.skipped += 1;
            continue;
        }

        let source = PathBuf::from(&path);
        let usage = usage_map.get(&path).copied().unwrap_or_default();
        let in_outbox = outbox_dir
            .as_deref()
            .map(|dir| path_within(&source, dir))
            .unwrap_or(false);
        let in_receive = receive_dir
            .map(|dir| path_within(&source, dir))
            .unwrap_or(false);
        // 应用管理副本：outbox 里自己创建的发送副本，或接收目录里的接收文件。
        // 仅作为“发送源”的普通文件（例如用户放进私有目录的原图）不迁移、不删除。
        let managed = in_outbox || (in_receive && !(usage.sent && !usage.received));
        if !managed || !source.is_file() {
            report.skipped += 1;
            continue;
        }
        if let Some(directory) = target_dir.as_deref() {
            if path_within(&source, directory) {
                report.already_in_target += 1;
                continue;
            }
        }

        let name = file_name_of(&source);

        if target_is_tree {
            #[cfg(target_os = "android")]
            {
                match export_verified(&source, target, &name) {
                    Ok(new_uri) => {
                        let _ = crate::db::remember_exported_uri(pool, &new_uri).await;
                        if let Err(error) =
                            crate::db::update_file_paths_by_old_path(pool, &path, &new_uri).await
                        {
                            report.failures.push(FileFailure {
                                path: path.clone(),
                                reason: format!("更新记录失败: {error}"),
                            });
                            continue;
                        }
                        match tokio::fs::remove_file(&source).await {
                            Ok(()) => report.migrated += 1,
                            Err(error) => {
                                let _ = crate::db::add_pending_cleanup(pool, &path).await;
                                report.failures.push(FileFailure {
                                    path: path.clone(),
                                    reason: format!(
                                        "新位置已保存，原文件暂未删除（已登记重试）: {error}"
                                    ),
                                });
                            }
                        }
                    }
                    Err(error) => report.failures.push(FileFailure {
                        path: path.clone(),
                        reason: error,
                    }),
                }
            }
            #[cfg(not(target_os = "android"))]
            {
                report.failures.push(FileFailure {
                    path: path.clone(),
                    reason: "当前平台不支持迁移到系统文件夹".to_string(),
                });
            }
            continue;
        }

        let directory = match target_dir.as_deref() {
            Some(directory) => directory,
            None => continue,
        };
        let destination = unique_target_path(directory, &name);
        match copy_verified(&source, &destination).await {
            Ok(_) => {
                let new_path = destination.to_string_lossy().to_string();
                if let Err(error) =
                    crate::db::update_file_paths_by_old_path(pool, &path, &new_path).await
                {
                    let _ = tokio::fs::remove_file(&destination).await;
                    report.failures.push(FileFailure {
                        path: path.clone(),
                        reason: format!("更新记录失败: {error}"),
                    });
                    continue;
                }
                match tokio::fs::remove_file(&source).await {
                    Ok(()) => report.migrated += 1,
                    Err(error) => {
                        let _ = crate::db::add_pending_cleanup(pool, &path).await;
                        report.failures.push(FileFailure {
                            path: path.clone(),
                            reason: format!("新位置已保存，原文件暂未删除（已登记重试）: {error}"),
                        });
                    }
                }
            }
            Err(error) => report.failures.push(FileFailure {
                path: path.clone(),
                reason: error,
            }),
        }
    }

    report
}

/// 旧位置在系统文件夹（SAF）里的文件：先导出到新位置，核对大小后再更新记录、删除旧文档。
async fn migrate_content_uri(
    pool: &Pool<Sqlite>,
    path: &str,
    target: &str,
    target_dir: Option<&Path>,
    staging_dir: Option<&Path>,
    report: &mut MigrationReport,
) {
    #[cfg(target_os = "android")]
    {
        let raw_name = path
            .rsplit('/')
            .next()
            .and_then(|segment| urlencoding::decode(segment).ok())
            .map(|decoded| decoded.to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "LQChat-file".to_string());
        let name = raw_name.rsplit(':').next().unwrap_or(&raw_name).to_string();

        let staged = match stage_content_uri(staging_dir, path, &name).await {
            Ok(path) => path,
            Err(error) => {
                report.failures.push(FileFailure {
                    path: path.to_string(),
                    reason: error,
                });
                return;
            }
        };
        let result = if target.starts_with("content://") {
            export_verified(&staged, target, &name)
        } else if let Some(directory) = target_dir {
            let destination = unique_target_path(directory, &name);
            match copy_verified(&staged, &destination).await {
                Ok(_) => Ok(destination.to_string_lossy().to_string()),
                Err(error) => Err(error),
            }
        } else {
            Err("下载位置无效".to_string())
        };
        let _ = tokio::fs::remove_file(&staged).await;
        match result {
            Ok(new_path) => {
                if new_path.starts_with("content://") {
                    let _ = crate::db::remember_exported_uri(pool, &new_path).await;
                }
                if let Err(error) =
                    crate::db::update_file_paths_by_old_path(pool, path, &new_path).await
                {
                    report.failures.push(FileFailure {
                        path: path.to_string(),
                        reason: format!("更新记录失败: {error}"),
                    });
                    return;
                }
                match crate::android_fd::delete_exported_file(path) {
                    Ok(()) => report.migrated += 1,
                    Err(error) => report.failures.push(FileFailure {
                        path: path.to_string(),
                        reason: format!("新位置已保存，但旧位置的文件未能删除: {error}"),
                    }),
                }
            }
            Err(error) => report.failures.push(FileFailure {
                path: path.to_string(),
                reason: error,
            }),
        }
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = (pool, target, target_dir, staging_dir, path);
        report.skipped += 1;
    }
}

#[cfg(target_os = "android")]
async fn stage_content_uri(
    staging_dir: Option<&Path>,
    uri: &str,
    name: &str,
) -> Result<PathBuf, String> {
    let staging = staging_dir
        .map(|path| path.to_path_buf())
        .ok_or_else(|| "尚未确定应用专属暂存目录".to_string())?;
    tokio::fs::create_dir_all(&staging)
        .await
        .map_err(|error| format!("创建临时目录失败: {error}"))?;
    let target = staging.join(format!("migrate-{}-{name}", uuid::Uuid::new_v4()));
    let uri = uri.to_string();
    let destination = target.clone();
    tokio::task::spawn_blocking(move || {
        crate::android_fd::copy_content_uri_to_file(&uri, &destination)
    })
    .await
    .map_err(|error| format!("复制系统文件任务失败: {error}"))??;
    Ok(target)
}

#[cfg(target_os = "android")]
fn export_verified(source: &Path, tree_uri: &str, name: &str) -> Result<String, String> {
    let source_path = source
        .to_str()
        .ok_or_else(|| "原文件路径不是有效 UTF-8".to_string())?;
    let expected = std::fs::metadata(source)
        .map_err(|error| format!("读取原文件失败: {error}"))?
        .len();
    let uri = crate::android_fd::export_received_file(tree_uri, source_path, name)?;
    let (_, copied) = crate::android_fd::AndroidFile::query_content_uri_info(&uri)?;
    if copied != expected {
        let _ = crate::android_fd::delete_exported_file(&uri);
        return Err(format!("目标文件不完整: {copied}/{expected} 字节"));
    }
    Ok(uri)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn fixture_pool(root: &Path) -> Pool<Sqlite> {
        crate::db::init_db_with_path(root.to_path_buf())
            .await
            .expect("初始化测试数据库失败")
    }

    fn temp_dir(label: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("lqchat-storage-{label}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    async fn insert_message(pool: &Pool<Sqlite>, sender: &str, receiver: &str, path: &Path) {
        sqlx::query(
            "INSERT INTO messages (sender_id, receiver_id, content, msg_type, timestamp, file_path, file_status)
             VALUES (?, ?, 'file.bin', 'file', unixepoch(), ?, 'accepted')",
        )
        .bind(sender)
        .bind(receiver)
        .bind(path.to_string_lossy().to_string())
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn migration_moves_app_managed_file_and_updates_records() {
        let root = temp_dir("migrate-app-data");
        let staging = root.join("staging");
        let target = temp_dir("migrate-target");
        std::fs::create_dir_all(&staging).unwrap();
        let source = staging.join("report.bin");
        std::fs::write(&source, b"lqchat-migration-payload").unwrap();

        let pool = fixture_pool(&root).await;
        insert_message(&pool, "phone", "me", &source).await;

        let report =
            migrate_saved_files(&pool, Some(&root), Some(&staging), &target.to_string_lossy()).await;

        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert_eq!(report.migrated, 1);
        assert!(!source.exists(), "原文件应在核对目标后被删除");
        let moved = target.join("report.bin");
        assert_eq!(std::fs::read(&moved).unwrap(), b"lqchat-migration-payload");

        let stored: String = sqlx::query_scalar("SELECT file_path FROM messages LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(PathBuf::from(&stored), moved);

        pool.close().await;
        std::fs::remove_dir_all(&root).unwrap();
        std::fs::remove_dir_all(&target).unwrap();
    }

    #[tokio::test]
    async fn migration_keeps_both_files_on_name_conflict_and_never_overwrites() {
        let root = temp_dir("migrate-conflict-app");
        let staging = root.join("staging");
        let target = temp_dir("migrate-conflict-target");
        std::fs::create_dir_all(&staging).unwrap();
        let source = staging.join("photo.png");
        std::fs::write(&source, b"new-content").unwrap();
        std::fs::write(target.join("photo.png"), b"existing-content").unwrap();

        let pool = fixture_pool(&root).await;
        insert_message(&pool, "phone", "me", &source).await;

        let report =
            migrate_saved_files(&pool, Some(&root), Some(&staging), &target.to_string_lossy()).await;

        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert_eq!(report.migrated, 1);
        assert_eq!(std::fs::read(target.join("photo.png")).unwrap(), b"existing-content");
        assert_eq!(std::fs::read(target.join("photo (1).png")).unwrap(), b"new-content");
        assert!(!source.exists());
        let stored: String = sqlx::query_scalar("SELECT file_path FROM messages LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(stored.ends_with("photo (1).png"));

        pool.close().await;
        std::fs::remove_dir_all(&root).unwrap();
        std::fs::remove_dir_all(&target).unwrap();
    }

    #[tokio::test]
    async fn migration_failure_keeps_source_and_records_for_retry() {
        let root = temp_dir("migrate-fail-app");
        let staging = root.join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        let source = staging.join("keep.bin");
        std::fs::write(&source, b"must-not-be-lost").unwrap();
        // 目标位置指向一个已存在的文件，无法创建目录，迁移必须整体失败且保留原文件。
        let blocked = root.join("blocked-target");
        std::fs::write(&blocked, b"x").unwrap();

        let pool = fixture_pool(&root).await;
        insert_message(&pool, "phone", "me", &source).await;

        let report =
            migrate_saved_files(&pool, Some(&root), Some(&staging), &blocked.to_string_lossy()).await;

        assert!(!report.failures.is_empty());
        assert_eq!(report.migrated, 0);
        assert!(source.exists(), "迁移失败时绝不能删除原文件");
        let stored: String = sqlx::query_scalar("SELECT file_path FROM messages LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(PathBuf::from(stored), source);

        pool.close().await;
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn migration_retries_registered_pending_cleanup() {
        let root = temp_dir("migrate-pending-app");
        let staging = root.join("staging");
        let target = temp_dir("migrate-pending-target");
        std::fs::create_dir_all(&staging).unwrap();
        let leftover = staging.join("leftover.bin");
        std::fs::write(&leftover, b"leftover").unwrap();
        std::fs::write(target.join("leftover.bin"), b"leftover").unwrap();

        let pool = fixture_pool(&root).await;
        crate::db::add_pending_cleanup(&pool, &leftover.to_string_lossy())
            .await
            .unwrap();

        let report =
            migrate_saved_files(&pool, Some(&root), Some(&staging), &target.to_string_lossy()).await;

        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert_eq!(report.pending_cleaned, 1);
        assert!(!leftover.exists(), "登记过待清理的原文件应被删除");
        assert!(crate::db::get_pending_cleanup(&pool).await.is_empty());

        pool.close().await;
        std::fs::remove_dir_all(&root).unwrap();
        std::fs::remove_dir_all(&target).unwrap();
    }

    #[tokio::test]
    async fn cleanup_removes_received_files_but_keeps_send_sources_and_shared_paths() {
        let root = temp_dir("cleanup-app");
        let staging = root.join("staging");
        let download = temp_dir("cleanup-download");
        std::fs::create_dir_all(&staging).unwrap();

        let received = download.join("received.apk");
        std::fs::write(&received, b"received").unwrap();
        let sent_source = temp_dir("cleanup-user-source");
        let user_file = sent_source.join("original.apk");
        std::fs::write(&user_file, b"user-source").unwrap();
        let shared = staging.join("shared.bin");
        std::fs::write(&shared, b"shared").unwrap();

        let pool = fixture_pool(&root).await;
        insert_message(&pool, "iqoo", "me", &received).await;
        insert_message(&pool, "me", "iqoo", &user_file).await;
        insert_message(&pool, "iqoo", "me", &shared).await;
        // 另一条聊天也在引用同一个文件，必须保留。
        insert_message(&pool, "other", "me", &shared).await;

        let outcome =
            cleanup_chat_files(&pool, "me", "iqoo", Some(&root), Some(&staging), Some(&download))
                .await
                .unwrap();

        assert!(outcome.report.failures.is_empty(), "{:?}", outcome.report.failures);
        assert!(!received.exists(), "接收文件应被清理");
        assert!(user_file.exists(), "用户选来发送的原始文件不得删除");
        assert!(shared.exists(), "共享路径仍被其他聊天引用时必须保留");
        assert_eq!(outcome.report.deleted, 1);
        assert!(outcome.report.kept >= 2);
        assert!(!outcome.fd_msg_ids.is_empty());

        pool.close().await;
        std::fs::remove_dir_all(&root).unwrap();
        std::fs::remove_dir_all(&download).unwrap();
        std::fs::remove_dir_all(&sent_source).unwrap();
    }

    #[tokio::test]
    async fn cleanup_protects_send_source_inside_download_directory() {
        let root = temp_dir("cleanup-source-in-download");
        let staging = root.join("staging");
        let download = temp_dir("cleanup-source-download");
        std::fs::create_dir_all(&staging).unwrap();
        // 用户正好从 LQChat 的下载目录里挑了一个文件发送：这是发送源，不能删。
        let source_in_download = download.join("picked-by-user.apk");
        std::fs::write(&source_in_download, b"user-picked").unwrap();

        let pool = fixture_pool(&root).await;
        insert_message(&pool, "me", "iqoo", &source_in_download).await;

        let outcome =
            cleanup_chat_files(&pool, "me", "iqoo", Some(&root), Some(&staging), Some(&download))
                .await
                .unwrap();

        assert!(source_in_download.exists(), "发送源即使位于下载目录也必须保留");
        assert_eq!(outcome.report.deleted, 0);

        pool.close().await;
        std::fs::remove_dir_all(&root).unwrap();
        std::fs::remove_dir_all(&download).unwrap();
    }

    #[tokio::test]
    async fn cleanup_detects_shared_reference_after_path_normalization() {
        let root = temp_dir("cleanup-normalized");
        let staging = root.join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        let shared = staging.join("Shared-Case.bin");
        std::fs::write(&shared, b"shared").unwrap();

        let pool = fixture_pool(&root).await;
        insert_message(&pool, "iqoo", "me", &shared).await;
        // 另一条聊天用不同大小写引用同一个文件（Windows 上视为同一路径）。
        let alias = shared.to_string_lossy().to_uppercase();
        sqlx::query(
            "INSERT INTO messages (sender_id, receiver_id, content, msg_type, timestamp, file_path, file_status)
             VALUES ('other', 'me', 'file.bin', 'file', unixepoch(), ?, 'accepted')",
        )
        .bind(&alias)
        .execute(&pool)
        .await
        .unwrap();

        let outcome =
            cleanup_chat_files(&pool, "me", "iqoo", Some(&root), Some(&staging), None)
                .await
                .unwrap();

        assert!(shared.exists(), "大小写/别名相同路径仍被引用时必须保留");
        assert_eq!(outcome.report.deleted, 0);
        assert!(outcome.report.kept >= 1);

        pool.close().await;
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn cleanup_keeps_content_uri_without_export_provenance() {
        let root = temp_dir("cleanup-content-uri");
        let staging = root.join("staging");
        std::fs::create_dir_all(&staging).unwrap();

        let pool = fixture_pool(&root).await;
        sqlx::query(
            "INSERT INTO messages (sender_id, receiver_id, content, msg_type, timestamp, file_path, file_status)
             VALUES ('iqoo', 'me', 'doc.pdf', 'file', unixepoch(), 'content://user/docs/1', 'accepted')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let outcome = cleanup_chat_files(&pool, "me", "iqoo", Some(&root), Some(&staging), None)
            .await
            .unwrap();

        assert!(outcome.report.failures.is_empty());
        assert_eq!(outcome.report.deleted, 0);
        assert_eq!(outcome.report.kept, 1);

        pool.close().await;
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn cleanup_reports_failures_instead_of_success() {
        let root = temp_dir("cleanup-fail-app");
        let staging = root.join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        let download = temp_dir("cleanup-fail-download");
        // 用目录冒充文件路径，删除必然失败。
        let blocked = download.join("blocked.apk");
        std::fs::create_dir_all(&blocked).unwrap();

        let pool = fixture_pool(&root).await;
        insert_message(&pool, "iqoo", "me", &blocked).await;

        let outcome =
            cleanup_chat_files(&pool, "me", "iqoo", Some(&root), Some(&staging), Some(&download))
                .await
                .unwrap();

        assert!(outcome.report.failed());
        assert_eq!(outcome.report.deleted, 0);
        assert!(blocked.exists());

        pool.close().await;
        std::fs::remove_dir_all(&root).unwrap();
        std::fs::remove_dir_all(&download).unwrap();
    }

    #[tokio::test]
    async fn cleanup_surfaces_database_errors_instead_of_empty_success() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        // 缺少 messages 表：查询失败必须报错，而不是当成"没有文件"返回成功。
        let result = cleanup_chat_files(&pool, "me", "iqoo", None, None, None).await;
        assert!(result.is_err());
        pool.close().await;
    }

    #[tokio::test]
    async fn caption_and_path_updates_reach_every_referencing_row() {
        let root = temp_dir("caption-app");
        let pool = fixture_pool(&root).await;
        let path = root.join("shared.bin");
        std::fs::write(&path, b"x").unwrap();
        insert_message(&pool, "me", "iqoo", &path).await;
        insert_message(&pool, "iqoo", "me", &path).await;

        let updated = crate::db::update_file_paths_by_old_path(
            &pool,
            &path.to_string_lossy(),
            "/new/shared.bin",
        )
        .await
        .unwrap();
        assert_eq!(updated, 2);

        crate::db::set_message_caption(&pool, 1, "一起发送的文字").await.unwrap();
        let caption: Option<String> = sqlx::query_scalar("SELECT caption FROM messages WHERE id = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(caption.as_deref(), Some("一起发送的文字"));

        pool.close().await;
        std::fs::remove_dir_all(&root).unwrap();
    }
}
