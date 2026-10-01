use crate::utils::generate_random_name;
use sqlx::{sqlite::SqlitePool, Pool, Sqlite};
use std::path::PathBuf;

pub const CHAT_RETENTION_SECS: i64 = 7 * 24 * 60 * 60;
pub const CHAT_RETENTION_SWEEP_SECS: u64 = 24 * 60 * 60;
#[cfg(windows)]
const WINDOWS_PORTABLE_DOWNLOAD_SENTINEL: &str = "$LQCHAT_PORTABLE/downloads";

#[cfg(feature = "desktop")]
use tauri::AppHandle;
#[cfg(all(feature = "desktop", not(windows)))]
use tauri::Manager;

pub struct DbState {
    pub pool: Pool<Sqlite>,
}

pub async fn get_username(pool: &sqlx::Pool<sqlx::Sqlite>) -> Result<String, String> {
    let res: (String,) = sqlx::query_as("SELECT value FROM settings WHERE key = 'username'")
        .fetch_one(pool)
        .await
        .map_err(|e| {
            eprintln!("[DB] read username failed: {}", e);
            e.to_string()
        })?;
    Ok(res.0)
}

pub async fn get_user_id(pool: &sqlx::Pool<sqlx::Sqlite>) -> Result<String, String> {
    let res: Result<(String,), _> =
        sqlx::query_as("SELECT value FROM settings WHERE key = 'user_id'")
            .fetch_one(pool)
            .await;

    match res {
        Ok((id,)) => Ok(id),
        Err(_) => {
            // 如果没有 user_id,生成一个并保存
            let user_id = uuid::Uuid::new_v4().to_string();
            println!("[DB] 生成并保存新的用户 ID: {}", user_id);

            sqlx::query("INSERT INTO settings (key, value) VALUES ('user_id', ?)")
                .bind(&user_id)
                .execute(pool)
                .await
                .map_err(|e| e.to_string())?;

            Ok(user_id)
        }
    }
}

pub async fn update_username(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    new_name: String,
) -> Result<(), String> {
    println!("[DB] updating username to: {}", new_name);

    // 验证用户名不为空
    if new_name.trim().is_empty() {
        return Err("username cannot be empty".to_string());
    }

    // 验证用户名长度
    if new_name.len() > 50 {
        return Err("username too long (max 50 chars)".to_string());
    }

    sqlx::query("UPDATE settings SET value = ? WHERE key = 'username'")
        .bind(new_name.trim())
        .execute(pool)
        .await
        .map_err(|e| {
            println!("[DB] update failed: {}", e);
            e.to_string()
        })?;

    println!("[DB] username updated successfully");
    Ok(())
}

// 获取下载路径
pub async fn get_download_path(pool: &sqlx::Pool<sqlx::Sqlite>) -> Result<String, String> {
    let res: Result<(String,), _> =
        sqlx::query_as("SELECT value FROM settings WHERE key = 'download_path'")
            .fetch_one(pool)
            .await;

    match res {
        Ok((path,)) => {
            #[cfg(windows)]
            if path == WINDOWS_PORTABLE_DOWNLOAD_SENTINEL {
                return crate::config_file::windows_portable_download_dir()
                    .map(|directory| directory.to_string_lossy().to_string());
            }
            Ok(path)
        }
        Err(_) => {
            // 如果没有设置，返回默认路径
            if cfg!(target_os = "android") {
                Ok(std::env::temp_dir()
                    .join("lanchat_received_files")
                    .to_string_lossy()
                    .to_string())
            } else if cfg!(windows) {
                #[cfg(windows)]
                return crate::config_file::windows_portable_download_dir()
                    .map(|path| path.to_string_lossy().to_string());
                #[cfg(not(windows))]
                unreachable!();
            } else {
                let home_dir = dirs::home_dir().ok_or("cannot get home directory")?;
                let default_path = home_dir.join("Downloads").join("LQChat");
                Ok(default_path.to_string_lossy().to_string())
            }
        }
    }
}

/// 接收文件实际落盘的目录。Android 的 `download_path` 允许保存 SAF tree URI，
/// 因此网络接收始终先写入应用专属的 staging 目录，再由 ContentResolver 导出。
pub async fn get_receive_directory(pool: &sqlx::Pool<sqlx::Sqlite>) -> Result<String, String> {
    if cfg!(target_os = "android") {
        let result: Result<(String,), _> =
            sqlx::query_as("SELECT value FROM settings WHERE key = 'download_staging_path'")
                .fetch_one(pool)
                .await;
        return result
            .map(|(path,)| path)
            .map_err(|error| error.to_string());
    }

    get_download_path(pool).await
}

// 更新下载路径
pub async fn update_download_path(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    new_path: String,
) -> Result<(), String> {
    println!("[DB] updating download path to: {}", new_path);

    // 验证路径不为空
    if new_path.trim().is_empty() {
        return Err("path cannot be empty".to_string());
    }

    let trimmed = new_path.trim();

    // SAF tree URI 不是文件系统路径，目录存在性与写权限由 Android
    // ContentResolver 和持久化 URI 授权负责。
    if !trimmed.starts_with("content://") {
        if let Err(e) = std::fs::create_dir_all(trimmed) {
            return Err(format!("cannot create directory: {}", e));
        }
    }

    #[cfg(windows)]
    let stored_path = {
        let portable_default = crate::config_file::windows_portable_download_dir()?;
        if std::path::Path::new(trimmed) == portable_default {
            WINDOWS_PORTABLE_DOWNLOAD_SENTINEL
        } else {
            trimmed
        }
    };
    #[cfg(not(windows))]
    let stored_path = trimmed;

    sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES ('download_path', ?)")
        .bind(stored_path)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

    println!("[DB] download path updated successfully");
    Ok(())
}

// 为 Tauri 桌面端初始化数据库
#[cfg(feature = "desktop")]
pub async fn init_db(app_handle: &AppHandle) -> Result<Pool<Sqlite>, sqlx::Error> {
    #[cfg(windows)]
    let app_dir = crate::config_file::windows_portable_data_dir()
        .map_err(|error| sqlx::Error::Io(std::io::Error::other(error)))?;
    #[cfg(not(windows))]
    let app_dir = app_handle.path().app_data_dir().expect("读取路径失败");
    #[cfg(windows)]
    let _ = app_handle;
    init_db_with_path(app_dir).await
}

// 为 Web 端初始化数据库（自动匹配平台标准路径）
pub async fn init_db_standalone(custom_path: Option<PathBuf>) -> Result<Pool<Sqlite>, sqlx::Error> {
    let app_dir = if let Some(path) = custom_path {
        path
    } else {
        // Windows: C:\Users\用户名\AppData\Roaming\com.lanchat.app
        // Linux: /home/用户名/.local/share/com.lanchat.app
        // macOS: /Users/用户名/Library/Application Support/com.lanchat.app
        dirs::data_dir()
            .map(|p| p.join("com.lanchat.app"))
            .unwrap_or_else(|| {
                // 如果实在拿不到系统路径（极罕见），回退到当前目录下的 data 文件夹
                eprintln!("[DB] 无法获取系统数据目录，回退到本地路径");
                PathBuf::from(".").join("data")
            })
    };

    init_db_with_path(app_dir).await
}

fn default_download_dir(app_dir: &std::path::Path) -> PathBuf {
    if cfg!(target_os = "android") {
        // Android 10+ 的分区存储不允许 targetSdk 36 应用直接用 std::fs 写公共 Download。
        // 接收文件保存在应用专属持久目录，并继续由 LANChat 提供下载和预览。
        app_dir.join("received_files")
    } else if cfg!(windows) {
        #[cfg(windows)]
        return crate::config_file::windows_portable_download_dir()
            .unwrap_or_else(|_| app_dir.join("downloads"));
        #[cfg(not(windows))]
        unreachable!();
    } else {
        let home_dir = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
        home_dir.join("Downloads").join("LQChat")
    }
}

// 通用的数据库初始化逻辑
pub async fn init_db_with_path(app_dir: PathBuf) -> Result<Pool<Sqlite>, sqlx::Error> {
    println!("[DB] 数据库路径: {:?}", app_dir);

    // 确保目录一定存在
    if !app_dir.exists() {
        std::fs::create_dir_all(&app_dir).map_err(sqlx::Error::Io)?;
    }

    #[cfg(windows)]
    let db_path = app_dir.join(crate::config_file::WINDOWS_DATABASE_FILE_NAME);
    #[cfg(not(windows))]
    let db_path = app_dir.join("lanchat.db");
    let db_url = format!("sqlite:{}", db_path.to_string_lossy());

    // 检查文件是否存在，如果不存在，手动创建空文件
    if !db_path.exists() {
        std::fs::File::create(&db_path).map_err(sqlx::Error::Io)?;
    }

    let pool = SqlitePool::connect(&db_url).await?;

    // 创建表结构
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            sender_id TEXT,
            receiver_id TEXT,
            content TEXT,
            msg_type TEXT,
            timestamp INTEGER,
            file_path TEXT,
            file_status TEXT,
            stored_at INTEGER NOT NULL DEFAULT (unixepoch())
        )",
    )
    .execute(&pool)
    .await?;

    // 数据库迁移：为现有的messages表添加receiver_id字段（如果不存在）
    let _ = sqlx::query("ALTER TABLE messages ADD COLUMN receiver_id TEXT")
        .execute(&pool)
        .await; // 忽略错误，因为字段可能已经存在

    // 数据库迁移：为现有的messages表添加file_size字段（如果不存在）
    let _ = sqlx::query("ALTER TABLE messages ADD COLUMN file_size INTEGER")
        .execute(&pool)
        .await;

    // 数据库迁移：为现有的messages表添加status字段（如果不存在）
    let _ = sqlx::query("ALTER TABLE messages ADD COLUMN status TEXT DEFAULT 'sent'")
        .execute(&pool)
        .await;

    // 数据库迁移：为现有的messages表添加sender_msg_id字段（对方的消息ID，用于手动下载回执）
    let _ = sqlx::query("ALTER TABLE messages ADD COLUMN sender_msg_id TEXT")
        .execute(&pool)
        .await;

    // Chat retention uses the local insertion time instead of the sender supplied timestamp.
    // Older databases are backfilled once, while a trigger covers every existing INSERT path.
    let _ = sqlx::query("ALTER TABLE messages ADD COLUMN stored_at INTEGER")
        .execute(&pool)
        .await;
    sqlx::query(
        "UPDATE messages
         SET stored_at = COALESCE(timestamp, unixepoch())
         WHERE stored_at IS NULL",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "CREATE TRIGGER IF NOT EXISTS messages_set_stored_at
         AFTER INSERT ON messages
         WHEN NEW.stored_at IS NULL
         BEGIN
             UPDATE messages SET stored_at = unixepoch() WHERE id = NEW.id;
         END",
    )
    .execute(&pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_messages_stored_at ON messages(stored_at)")
        .execute(&pool)
        .await?;

    if cfg!(target_os = "android") {
        cleanup_expired_chat_messages(&pool)
            .await
            .map_err(sqlx::Error::Protocol)?;
    }

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT
        )",
    )
    .execute(&pool)
    .await?;

    // 创建 users 表存储历史用户
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS users (
            id TEXT PRIMARY KEY,
            name TEXT,
            addr TEXT,
            last_seen INTEGER,
            is_offline INTEGER DEFAULT 0,
            available_memory_mb INTEGER DEFAULT 0
        )",
    )
    .execute(&pool)
    .await?;

    // 创建 persisted_uris 表追踪已持久化的 content URI 权限
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS persisted_uris (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            uri TEXT NOT NULL,
            msg_id INTEGER NOT NULL,
            created_at INTEGER NOT NULL
        )",
    )
    .execute(&pool)
    .await?;

    // Notification history is a separate seven-day store and never participates in chat resend.
    crate::notification_history::initialize(&pool).await?;

    // 初始化配置 (如果没有用户名则生成一个)
    let user_exists = sqlx::query("SELECT value FROM settings WHERE key = 'username'")
        .fetch_optional(&pool)
        .await?;

    if user_exists.is_none() {
        let random_name = generate_random_name();
        println!("[DB] 生成随机用户名: {}", random_name);

        // 生成唯一的 UUID
        let user_id = uuid::Uuid::new_v4().to_string();
        println!("[DB] 生成用户 ID: {}", user_id);

        sqlx::query("INSERT INTO settings (key, value) VALUES ('username', ?)")
            .bind(random_name)
            .execute(&pool)
            .await?;

        sqlx::query("INSERT INTO settings (key, value) VALUES ('user_id', ?)")
            .bind(user_id)
            .execute(&pool)
            .await?;

        let download_dir = default_download_dir(&app_dir).to_string_lossy().to_string();

        println!("[DB] 设置默认下载路径: {}", download_dir);

        #[cfg(windows)]
        let stored_download_dir = WINDOWS_PORTABLE_DOWNLOAD_SENTINEL;
        #[cfg(not(windows))]
        let stored_download_dir = download_dir;

        sqlx::query("INSERT INTO settings (key, value) VALUES ('download_path', ?)")
            .bind(stored_download_dir)
            .execute(&pool)
            .await?;
    }

    if cfg!(target_os = "android") {
        let private_download_dir = default_download_dir(&app_dir);
        std::fs::create_dir_all(&private_download_dir).map_err(sqlx::Error::Io)?;
        let expected = private_download_dir.to_string_lossy().to_string();
        let current = get_download_path(&pool).await.ok();

        sqlx::query(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('download_staging_path', ?)",
        )
        .bind(&expected)
        .execute(&pool)
        .await?;

        // 旧版本保存的是公共 Download 路径，在 Android 16/targetSdk 36 下不可直接写入。
        // 已由系统文件夹选择器产生的 content URI 必须保留。
        let has_saf_directory = current
            .as_deref()
            .is_some_and(|path| path.starts_with("content://"));
        if !has_saf_directory && current.as_deref() != Some(expected.as_str()) {
            sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES ('download_path', ?)")
                .bind(&expected)
                .execute(&pool)
                .await?;
            println!("[DB] Android 下载目录已迁移到应用专属路径: {}", expected);
        }
    }

    Ok(pool)
}

/// Delete chat rows older than seven local days. File contents and Android URI permissions
/// intentionally remain untouched; pending and failed rows follow the same retention rule.
pub async fn cleanup_expired_chat_messages(pool: &sqlx::Pool<sqlx::Sqlite>) -> Result<u64, String> {
    let cutoff = chrono::Utc::now().timestamp() - CHAT_RETENTION_SECS;
    let result = sqlx::query(
        "DELETE FROM messages
         WHERE COALESCE(stored_at, timestamp, 0) < ?",
    )
    .bind(cutoff)
    .execute(pool)
    .await
    .map_err(|error| format!("清理过期聊天记录失败: {error}"))?;
    let removed = result.rows_affected();
    if removed > 0 {
        println!("[DB] 已清理 {removed} 条超过 7 天的聊天记录");
    }
    Ok(removed)
}

pub async fn run_chat_retention(
    pool: sqlx::Pool<sqlx::Sqlite>,
    cancellation: tokio_util::sync::CancellationToken,
) -> Result<(), String> {
    let mut interval =
        tokio::time::interval(std::time::Duration::from_secs(CHAT_RETENTION_SWEEP_SECS));
    // Initialization already performs a sweep, so the first interval tick is skipped.
    interval.tick().await;
    loop {
        tokio::select! {
            _ = cancellation.cancelled() => return Ok(()),
            _ = interval.tick() => {
                if let Err(error) = cleanup_expired_chat_messages(&pool).await {
                    eprintln!("[DB] {error}; 下个清理周期将重试");
                }
            }
        }
    }
}

// ==================== 文件相关的数据库函数 ====================

/// 保存文件消息到数据库
pub async fn save_file_message(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    peer_id: String,
    file_name: String,
    file_size: usize,
    file_path: String,
    file_status: String,
    overall_status: String,
) -> Result<i64, String> {
    println!(
        "[DB] 保存文件消息: 文件={}, 大小={}, 状态={}",
        file_name, file_size, file_status
    );
    // 检查是否存在
    let existing = sqlx::query_as::<_, (i64,)>(
        "SELECT id FROM messages WHERE receiver_id = ? AND content = ? AND msg_type = 'file' AND file_status = 'uploading' ORDER BY id DESC LIMIT 1"
    )
    .bind(&peer_id).bind(&file_name).fetch_optional(pool).await.map_err(|e| e.to_string())?;

    if let Some((msg_id,)) = existing {
        sqlx::query("UPDATE messages SET file_path = ?, file_status = ?, status = ? WHERE id = ?") // <--- 【修改 2】更新 status
            .bind(&file_path)
            .bind(&file_status)
            .bind(&overall_status)
            .bind(msg_id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(msg_id)
    } else {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // <--- 【修改 3】把 status 写入 INSERT
        let result = sqlx::query(
            "INSERT INTO messages (sender_id, receiver_id, content, msg_type, timestamp, file_path, file_status, file_size, sender_msg_id, status) VALUES ('me', ?, ?, 'file', ?, ?, ?, ?, '0', ?)"
        )
        .bind(&peer_id).bind(&file_name).bind(timestamp).bind(&file_path)
        .bind(&file_status).bind(file_size as i64).bind(&overall_status)
        .execute(pool).await.map_err(|e| e.to_string())?;

        let new_id = result.last_insert_rowid();
        // 更新 sender_msg_id 为实际 msg_id
        let _ = sqlx::query("UPDATE messages SET sender_msg_id = ? WHERE id = ?")
            .bind(new_id.to_string())
            .bind(new_id)
            .execute(pool)
            .await;
        println!(
            "[DB] 文件消息保存完成, id={}, sender_msg_id={}",
            new_id, new_id
        );
        Ok(new_id)
    }
}

/// 获取下载中的文件（根据发送者ID）
pub async fn get_downloading_file(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    sender_id: &str,
) -> Result<Option<String>, String> {
    let row = sqlx::query("SELECT content FROM messages WHERE sender_id = ? AND msg_type = 'file' AND file_status = 'downloading' ORDER BY id DESC LIMIT 1")
        .bind(sender_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("查询文件失败: {}", e))?;

    if let Some(row) = row {
        use sqlx::Row;
        let file_name: String = row.get("content");
        Ok(Some(file_name))
    } else {
        Ok(None)
    }
}

/// 按 sender_msg_id 查询当前正在下载的文件名（多文件并发隔离）
pub async fn get_downloading_file_by_sender_msg_id(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    sender_msg_id: &str,
) -> Result<Option<String>, String> {
    let row = sqlx::query("SELECT content FROM messages WHERE sender_msg_id = ? AND msg_type = 'file' AND file_status = 'downloading' ORDER BY id DESC LIMIT 1")
        .bind(sender_msg_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("查询文件失败: {}", e))?;

    if let Some(row) = row {
        use sqlx::Row;
        let file_name: String = row.get("content");
        Ok(Some(file_name))
    } else {
        Ok(None)
    }
}

/// 更新文件状态（从 downloading 到 accepted）
pub async fn update_file_status(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    file_name: &str,
    new_status: &str,
) -> Result<(), String> {
    println!("[DB] 更新文件状态: {} -> {}", file_name, new_status);

    sqlx::query(
        "UPDATE messages SET file_status = ? WHERE content = ? AND msg_type = 'file' AND file_status = 'downloading'"
    )
    .bind(new_status)
    .bind(file_name)
    .execute(pool)
    .await
    .map_err(|e| format!("更新文件状态失败: {}", e))?;

    println!("[DB] 文件状态已更新");
    Ok(())
}

/// 创建文件接收记录（Web端发送文件时）
pub async fn create_upload_record(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    receiver_id: String,
    file_name: String,
    file_size: u64,
    timestamp: i64,
    file_status: String,
    overall_status: String,
) -> Result<i64, String> {
    println!(
        "[DB] 创建上传记录: 接收者={}, 文件={}, 状态={}",
        receiver_id, file_name, overall_status
    );

    let new_id: i64 = {
        let result = sqlx::query(
            "INSERT INTO messages (sender_id, receiver_id, content, msg_type, timestamp, file_path, file_status, file_size, sender_msg_id, status) 
             VALUES ('me', ?, ?, 'file', ?, '', ?, ?, '0', ?)"
        )
        .bind(&receiver_id)
        .bind(&file_name)
        .bind(timestamp)
        .bind(&file_status)
        .bind(file_size as i64)
        .bind(&overall_status)
        .execute(pool)
        .await
        .map_err(|e| format!("创建记录失败: {}", e))?;
        result.last_insert_rowid()
    };
    // 更新 sender_msg_id 为实际 msg_id
    let _ = sqlx::query("UPDATE messages SET sender_msg_id = ? WHERE id = ?")
        .bind(new_id.to_string())
        .bind(new_id)
        .execute(pool)
        .await;
    println!(
        "[DB] 上传记录创建完成, id={}, sender_msg_id={}",
        new_id, new_id
    );
    Ok(new_id)
}

/// 更新上传状态
pub async fn update_upload_status(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    file_name: String,
    status: String,
) -> Result<(), String> {
    println!(
        "[DB] Web前端确认发送完毕，强制同步更新状态: {} -> {}",
        file_name, status
    );
    // 只要前端说传完了，不管后端之前以为它是 pending 还是 uploading，统统强制标为已发送！
    sqlx::query(
        "UPDATE messages SET file_status = ?, status = 'sent' WHERE sender_id = 'me' AND content = ?"
    )
    .bind(&status)
    .bind(&file_name)
    .execute(pool)
    .await
    .map_err(|e| format!("更新状态失败: {}", e))?;

    println!("[DB] 上传状态已强制更新为 sent，移出补发队列");
    Ok(())
}

/// 删除上传记录（上传失败时）
pub async fn delete_upload_record(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    file_name: String,
    timestamp: i64,
) -> Result<(), String> {
    println!("[DB] 删除上传记录: {}", file_name);

    sqlx::query(
        "DELETE FROM messages WHERE sender_id = 'me' AND content = ? AND timestamp = ? AND file_status = 'uploading'"
    )
    .bind(&file_name)
    .bind(timestamp)
    .execute(pool)
    .await
    .map_err(|e| format!("删除记录失败: {}", e))?;

    println!("[DB] 上传记录已删除");
    Ok(())
}

/// 更新文件状态（通过消息ID）
pub async fn update_file_status_by_id(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    msg_id: i64,
    new_status: &str,
) -> Result<(), String> {
    println!("[DB] 更新文件状态（ID: {}）: -> {}", msg_id, new_status);

    sqlx::query("UPDATE messages SET file_status = ? WHERE id = ?")
        .bind(new_status)
        .bind(msg_id)
        .execute(pool)
        .await
        .map_err(|e| format!("更新文件状态失败: {}", e))?;

    println!("[DB] 文件状态已更新");
    Ok(())
}

/// 删除消息（通过消息ID）
pub async fn delete_message_by_id(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    msg_id: i64,
) -> Result<(), String> {
    println!("[DB] 删除消息: ID {}", msg_id);

    sqlx::query("DELETE FROM messages WHERE id = ?")
        .bind(msg_id)
        .execute(pool)
        .await
        .map_err(|e| format!("删除消息失败: {}", e))?;

    println!("[DB] 消息已删除");
    Ok(())
}

/// 保存文本消息
pub async fn save_text_message(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    receiver_id: String,
    content: String,
) -> Result<(), String> {
    println!(
        "[DB] 保存文本消息: 接收者={}, 内容长度={}",
        receiver_id,
        content.len()
    );

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    sqlx::query(
        "INSERT INTO messages (sender_id, receiver_id, content, msg_type, timestamp) VALUES ('me', ?, ?, 'text', ?)"
    )
    .bind(&receiver_id)
    .bind(&content)
    .bind(timestamp)
    .execute(pool)
    .await
    .map_err(|e| format!("保存消息失败: {}", e))?;

    println!("[DB] 文本消息已保存");
    Ok(())
}

/// 保存当前主题
pub async fn save_current_theme(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    theme_name: String,
) -> Result<(), String> {
    println!("[DB] 保存当前主题: {}", theme_name);

    sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES ('current_theme', ?)")
        .bind(&theme_name)
        .execute(pool)
        .await
        .map_err(|e| format!("保存主题失败: {}", e))?;

    println!("[DB] 主题已保存");
    Ok(())
}

/// 获取当前主题
pub async fn get_current_theme(pool: &sqlx::Pool<sqlx::Sqlite>) -> Result<Option<String>, String> {
    let result =
        sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = 'current_theme'")
            .fetch_optional(pool)
            .await
            .map_err(|e| format!("查询主题失败: {}", e))?;

    Ok(result)
}

/// 保存接收到的文本消息（来自其他对等体）
pub async fn save_received_text_message(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    sender_id: String,
    content: String,
    msg_type: String,
    timestamp: i64,
) -> Result<i64, String> {
    println!(
        "[DB] 保存接收到的文本消息: 发送者={}, 内容长度={}",
        sender_id,
        content.len()
    );

    let result = sqlx::query(
        "INSERT INTO messages (sender_id, content, msg_type, timestamp) VALUES (?, ?, ?, ?)",
    )
    .bind(&sender_id)
    .bind(&content)
    .bind(&msg_type)
    .bind(timestamp)
    .execute(pool)
    .await
    .map_err(|e| format!("保存消息失败: {}", e))?;

    let msg_id = result.last_insert_rowid();
    println!("[DB] 接收到的消息已保存, ID: {}", msg_id);
    Ok(msg_id)
}

/// 更新文件状态（通过文件路径）
pub async fn update_file_status_by_path(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    old_path: &str,
    new_path: &str,
    new_status: &str,
) -> Result<(), String> {
    println!(
        "[DB] 更新文件状态（路径）: {} -> {}, 状态: {}",
        old_path, new_path, new_status
    );

    sqlx::query("UPDATE messages SET file_status = ?, file_path = ? WHERE file_path = ?")
        .bind(new_status)
        .bind(new_path)
        .bind(old_path)
        .execute(pool)
        .await
        .map_err(|e| format!("更新文件状态失败: {}", e))?;

    println!("[DB] 文件状态已更新");
    Ok(())
}

/// 获取所有文件消息（用于调试）
pub async fn get_all_file_messages(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    limit: i64,
) -> Result<Vec<(i64, String, String, String, String)>, String> {
    println!("[DB] 获取所有文件消息（限制: {}）", limit);

    let rows = sqlx::query_as::<_, (i64, String, String, String, String)>(
        "SELECT id, sender_id, content, file_path, file_status FROM messages WHERE msg_type = 'file' ORDER BY id DESC LIMIT ?"
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("查询文件消息失败: {}", e))?;

    println!("[DB] 找到 {} 条文件消息", rows.len());
    Ok(rows)
}

/// 查询待接收的文件（通过文件路径模糊匹配）
pub async fn get_pending_file_by_path(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    path_pattern: &str,
) -> Result<Option<(String, String)>, String> {
    println!("[DB] 查询待接收文件: 路径模式={}", path_pattern);

    let row = sqlx::query_as::<_, (String, String)>(
        "SELECT file_path, content FROM messages
         WHERE file_path LIKE ?
           AND file_status = 'pending'
           AND COALESCE(stored_at, timestamp, 0) >= unixepoch() - 604800",
    )
    .bind(path_pattern)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("查询文件失败: {}", e))?;

    if let Some((path, name)) = &row {
        println!("[DB] 找到待接收文件: {} ({})", name, path);
    } else {
        println!("[DB] 未找到待接收文件");
    }

    Ok(row)
}

/// 创建接收文件记录（接收来自其他对等体的文件）
pub async fn create_received_file_record(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    sender_id: String,
    file_name: String,
    file_path: String,
    file_size: u64,
    timestamp: i64,
    sender_msg_id: &str, // 发送端的 DB row ID，用于进度 DOM 查找
) -> Result<i64, String> {
    // 获取当前用户ID作为接收者
    let my_id = get_user_id(pool).await?;

    let result = sqlx::query(
        "INSERT INTO messages (sender_id, receiver_id, content, msg_type, timestamp, file_path, file_status, file_size, sender_msg_id) VALUES (?, ?, ?, 'file', ?, ?, 'downloading', ?, ?)"
    )
    .bind(&sender_id)
    .bind(&my_id)
    .bind(&file_name)
    .bind(timestamp)
    .bind(&file_path)
    .bind(file_size as i64)
    .bind(sender_msg_id)
    .execute(pool)
    .await
    .map_err(|e| format!("创建记录失败: {}", e))?;

    let msg_id = result.last_insert_rowid();
    println!(
        "[DB] ✓ 接收文件记录已创建，ID: {}, 文件: {}, 状态: downloading, sender_msg_id: {}",
        msg_id, file_name, sender_msg_id
    );
    Ok(msg_id)
}

/// 批量删除消息（通过消息ID列表）
pub async fn delete_messages_by_ids(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    msg_ids: Vec<i64>,
) -> Result<(), String> {
    if msg_ids.is_empty() {
        return Ok(());
    }

    println!("[DB] 批量删除消息: {} 条", msg_ids.len());

    // 构建 IN 子句的占位符
    let placeholders = msg_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let query_str = format!("DELETE FROM messages WHERE id IN ({})", placeholders);

    let mut query = sqlx::query(&query_str);
    for id in msg_ids {
        query = query.bind(id);
    }

    query
        .execute(pool)
        .await
        .map_err(|e| format!("批量删除消息失败: {}", e))?;

    println!("[DB] 消息已批量删除");
    Ok(())
}

// ==================== 历史用户管理函数 ====================

/// 保存或更新用户到历史用户表
pub async fn save_or_update_user(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    id: String,
    name: String,
    addr: String,
    is_offline: bool,
    available_memory_mb: u64,
) -> Result<(), String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    sqlx::query(
        "INSERT OR REPLACE INTO users (id, name, addr, last_seen, is_offline, available_memory_mb) VALUES (?, ?, ?, ?, ?, ?)"
    )
    .bind(&id)
    .bind(&name)
    .bind(&addr)
    .bind(now)
    .bind(if is_offline { 1 } else { 0 })
    .bind(available_memory_mb as i64)
    .execute(pool)
    .await
    .map_err(|e| format!("保存用户失败: {}", e))?;

    Ok(())
}

/// 获取所有历史用户（包括离线的）
pub async fn get_all_users(
    pool: &sqlx::Pool<sqlx::Sqlite>,
) -> Result<Vec<(String, String, String, i64, bool, u64)>, String> {
    let rows = sqlx::query_as::<_, (String, String, String, i64, i64, i64)>(
        "SELECT id, name, addr, last_seen, is_offline, available_memory_mb FROM users ORDER BY last_seen DESC"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("查询用户失败: {}", e))?;

    Ok(rows
        .into_iter()
        .map(|(id, name, addr, last_seen, is_offline, mem)| {
            (id, name, addr, last_seen, is_offline != 0, mem as u64)
        })
        .collect())
}

/// 获取所有挂起的消息（发送给离线用户的消息）
pub async fn get_pending_messages(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    receiver_id: &str,
) -> Result<Vec<(i64, String, String, i64, Option<String>, Option<i64>)>, String> {
    let rows = sqlx::query_as::<_, (i64, String, String, i64, Option<String>, Option<i64>)>(
        "SELECT id, content, msg_type, timestamp, file_path, file_size
         FROM messages
         WHERE receiver_id = ?
           AND status = 'pending'
           AND COALESCE(stored_at, timestamp, 0) >= unixepoch() - 604800
         ORDER BY timestamp ASC",
    )
    .bind(receiver_id)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("查询挂起消息失败: {}", e))?;

    Ok(rows)
}

/// 标记消息为已发送
pub async fn mark_message_as_sent(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    msg_id: i64,
) -> Result<(), String> {
    sqlx::query("UPDATE messages SET status = 'sent' WHERE id = ?")
        .bind(msg_id)
        .execute(pool)
        .await
        .map_err(|e| format!("更新消息状态失败: {}", e))?;

    Ok(())
}

/// 标记多条消息为已发送
pub async fn mark_messages_as_sent(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    msg_ids: Vec<i64>,
) -> Result<(), String> {
    if msg_ids.is_empty() {
        return Ok(());
    }

    let placeholders = msg_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let query_str = format!(
        "UPDATE messages SET status = 'sent' WHERE id IN ({})",
        placeholders
    );

    let mut query = sqlx::query(&query_str);
    for id in msg_ids {
        query = query.bind(id);
    }

    query
        .execute(pool)
        .await
        .map_err(|e| format!("批量更新消息状态失败: {}", e))?;

    Ok(())
}

/// 保存文本消息（支持挂起状态）
pub async fn save_text_message_with_status(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    receiver_id: String,
    content: String,
    status: String,
) -> Result<i64, String> {
    println!(
        "[DB] 保存文本消息: 接收者={}, 内容长度={}, 状态={}",
        receiver_id,
        content.len(),
        status
    );

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let result = sqlx::query(
        "INSERT INTO messages (sender_id, receiver_id, content, msg_type, timestamp, status) VALUES ('me', ?, ?, 'text', ?, ?)"
    )
    .bind(&receiver_id)
    .bind(&content)
    .bind(timestamp)
    .bind(&status)
    .execute(pool)
    .await
    .map_err(|e| format!("保存消息失败: {}", e))?;

    let msg_id = result.last_insert_rowid();
    println!("[DB] 文本消息已保存，ID: {}, 状态: {}", msg_id, status);
    Ok(msg_id)
}

/// 查询聊天历史（带偏移量，用于懒加载）
pub async fn get_chat_history_with_offset(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    peer_id: &str,
    limit: i32,
    offset: i32,
) -> Result<Vec<crate::models::Message>, String> {
    // 获取当前用户ID
    let my_id = get_user_id(pool).await?;

    // 【核心修复】：在两处 SELECT 中加上了 status 字段
    let messages = sqlx::query_as::<_, crate::models::Message>(
        "SELECT id, sender_id, receiver_id, content, msg_type, timestamp, file_path, file_status, file_size, sender_msg_id, status 
         FROM (
            SELECT id, sender_id, receiver_id, content, msg_type, timestamp, file_path, file_status, file_size, sender_msg_id, status 
            FROM messages 
            WHERE COALESCE(stored_at, timestamp, 0) >= unixepoch() - 604800 AND (
                (sender_id = ? AND receiver_id = ?) OR 
                (sender_id = ? AND (receiver_id = ? OR receiver_id IS NULL)) OR
                (sender_id = 'me' AND receiver_id = ?)
            )
            ORDER BY timestamp DESC 
            LIMIT ? OFFSET ?
         ) 
         ORDER BY timestamp ASC",
    )
    .bind(&my_id)
    .bind(peer_id)
    .bind(peer_id)
    .bind(&my_id)
    .bind(peer_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("查询历史失败: {}", e))?;

    Ok(messages)
}

/// 专门用于保存从网络接收到的文本消息
pub async fn save_network_message(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    from_id: &str,
    content: &str,
    msg_type: &str,
    timestamp: u64,
) -> Result<i64, String> {
    // 获取当前用户ID作为接收者
    let my_id = get_user_id(pool).await?;

    let result = sqlx::query(
        "INSERT INTO messages (sender_id, receiver_id, content, msg_type, timestamp) VALUES (?, ?, ?, ?, ?)"
    )
    .bind(from_id)
    .bind(&my_id)
    .bind(content)
    .bind(msg_type)
    .bind(timestamp as i64)
    .execute(pool)
    .await
    .map_err(|e| format!("保存网络消息失败: {}", e))?;

    println!("[DB] 接收自网络的消息已保存到数据库");

    // 返回生成的数据库行 ID
    Ok(result.last_insert_rowid())
}

/// 根据发送者和文件名获取最新的一条消息 ID
pub async fn get_latest_msg_id_by_file(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    sender_id: &str,
    file_name: &str,
) -> Option<i64> {
    // 尝试最多 3 次查询，每次间隔 50ms，应对写入延迟
    for _ in 0..3 {
        let res = sqlx::query_scalar::<_, i64>(
            "SELECT id FROM messages WHERE sender_id = ? AND content = ? ORDER BY id DESC LIMIT 1",
        )
        .bind(sender_id)
        .bind(file_name)
        .fetch_optional(pool)
        .await
        .unwrap_or(None);

        if res.is_some() {
            return res;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }
    None
}

/// 查询消息的 sender_msg_id（用于 file_status_update 广播）
pub async fn get_sender_msg_id_by_file(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    sender_id: &str,
    file_name: &str,
) -> Result<String, String> {
    let result = sqlx::query_scalar::<_, String>(
        "SELECT sender_msg_id FROM messages WHERE sender_id = ? AND content = ? AND sender_msg_id IS NOT NULL AND sender_msg_id != '' ORDER BY id DESC LIMIT 1"
    )
    .bind(sender_id)
    .bind(file_name)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("查询 sender_msg_id 失败: {}", e))?;

    Ok(result.unwrap_or_default())
}

/// 清空与某个用户的聊天记录
pub async fn clear_chat_history(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    my_id: &str,
    peer_id: &str,
) -> Result<(), String> {
    println!("[DB] 清空与用户 {} 的聊天记录", peer_id);
    sqlx::query(
        "DELETE FROM messages WHERE 
        (sender_id = 'me' AND receiver_id = ?) OR 
        (sender_id = ? AND (receiver_id = ? OR receiver_id IS NULL))",
    )
    .bind(peer_id)
    .bind(peer_id)
    .bind(my_id)
    .execute(pool)
    .await
    .map_err(|e| format!("清空聊天记录失败: {}", e))?;
    Ok(())
}

/// 删除用户及其所有聊天记录
pub async fn delete_user_and_history(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    my_id: &str,
    peer_id: &str,
) -> Result<(), String> {
    // 1. 删除消息
    clear_chat_history(pool, my_id, peer_id).await?;
    // 2. 从用户表删除
    sqlx::query("DELETE FROM users WHERE id = ?")
        .bind(peer_id)
        .execute(pool)
        .await
        .map_err(|e| format!("删除用户失败: {}", e))?;
    println!("[DB] 用户 {} 及其记录已彻底删除", peer_id);
    Ok(())
}

// ── 自定义 IP ──

const CUSTOM_PEER_KEY_PREFIX: &str = "custom_peer_";

/// 获取所有自定义 IP
pub async fn get_custom_peers(pool: &sqlx::Pool<sqlx::Sqlite>) -> Vec<String> {
    let pattern = format!("{}%", CUSTOM_PEER_KEY_PREFIX);
    match sqlx::query_as::<_, (String,)>("SELECT value FROM settings WHERE key LIKE ?")
        .bind(&pattern)
        .fetch_all(pool)
        .await
    {
        Ok(rows) => rows.into_iter().map(|r| r.0).collect(),
        Err(e) => {
            eprintln!("[DB] 读取自定义 IP 失败: {}", e);
            vec![]
        }
    }
}

/// 添加自定义 IP
pub async fn add_custom_peer(pool: &sqlx::Pool<sqlx::Sqlite>, peer: &str) -> Result<(), String> {
    let key = format!("{}{}", CUSTOM_PEER_KEY_PREFIX, peer);
    sqlx::query("INSERT OR IGNORE INTO settings (key, value) VALUES (?, ?)")
        .bind(&key)
        .bind(peer)
        .execute(pool)
        .await
        .map_err(|e| format!("添加自定义 IP 失败: {}", e))?;
    println!("[DB] 已添加自定义 IP: {}", peer);
    Ok(())
}

/// 删除自定义 IP
pub async fn remove_custom_peer(pool: &sqlx::Pool<sqlx::Sqlite>, peer: &str) -> Result<(), String> {
    let key = format!("{}{}", CUSTOM_PEER_KEY_PREFIX, peer);
    sqlx::query("DELETE FROM settings WHERE key = ?")
        .bind(&key)
        .execute(pool)
        .await
        .map_err(|e| format!("删除自定义 IP 失败: {}", e))?;
    println!("[DB] 已删除自定义 IP: {}", peer);
    Ok(())
}

// ── 通知开关 ──────────────────────────────────────────────

/// 获取通知开关状态（默认开启）
pub async fn get_notifications_enabled(pool: &sqlx::Pool<sqlx::Sqlite>) -> bool {
    let res = sqlx::query_as::<_, (String,)>(
        "SELECT value FROM settings WHERE key = 'notifications_enabled'",
    )
    .fetch_one(pool)
    .await;

    match res {
        Ok((val,)) => val == "true",
        Err(_) => true, // 默认开启
    }
}

/// 设置通知开关状态
pub async fn set_notifications_enabled(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    enabled: bool,
) -> Result<(), String> {
    let val = if enabled { "true" } else { "false" };
    sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES ('notifications_enabled', ?)")
        .bind(val)
        .execute(pool)
        .await
        .map_err(|e| format!("保存通知设置失败: {}", e))?;
    println!("[DB] 通知状态已设置为: {}", val);
    Ok(())
}

/// 仅从设备列表移除档案；聊天记录保留，设备重新发现后仍可查看。
pub async fn delete_peer_profile(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    peer_id: &str,
) -> Result<(), String> {
    sqlx::query("DELETE FROM users WHERE id = ?")
        .bind(peer_id)
        .execute(pool)
        .await
        .map_err(|e| format!("清除设备失败: {e}"))?;
    Ok(())
}

/// 获取消息与文件通知音效开关状态（默认开启）。
pub async fn get_notification_sound_enabled(pool: &sqlx::Pool<sqlx::Sqlite>) -> bool {
    let res = sqlx::query_as::<_, (String,)>(
        "SELECT value FROM settings WHERE key = 'notification_sound_enabled'",
    )
    .fetch_one(pool)
    .await;

    match res {
        Ok((val,)) => val == "true",
        Err(_) => true,
    }
}

/// 设置消息与文件通知音效开关状态。
pub async fn set_notification_sound_enabled(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    enabled: bool,
) -> Result<(), String> {
    let val = if enabled { "true" } else { "false" };
    sqlx::query(
        "INSERT OR REPLACE INTO settings (key, value) VALUES ('notification_sound_enabled', ?)",
    )
    .bind(val)
    .execute(pool)
    .await
    .map_err(|e| format!("保存通知音效设置失败: {e}"))?;
    println!("[DB] 通知音效状态已设置为: {val}");
    Ok(())
}

// ── 自动下载开关 ──────────────────────────────────────────────

/// 获取自动下载开关状态（默认开启）
pub async fn get_auto_download(pool: &sqlx::Pool<sqlx::Sqlite>) -> bool {
    let res =
        sqlx::query_as::<_, (String,)>("SELECT value FROM settings WHERE key = 'auto_download'")
            .fetch_one(pool)
            .await;

    match res {
        Ok((val,)) => val == "true",
        Err(_) => true, // 默认开启
    }
}

/// 设置自动下载开关
pub async fn set_auto_download(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    enabled: bool,
) -> Result<(), String> {
    let val = if enabled { "true" } else { "false" };
    sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES ('auto_download', ?)")
        .bind(val)
        .execute(pool)
        .await
        .map_err(|e| format!("保存自动下载设置失败: {}", e))?;
    println!("[DB] 自动下载已设置为: {}", val);
    Ok(())
}

/// 获取端口（仅 Android 端使用，其他平台走 config.json）
pub async fn get_port(pool: &sqlx::Pool<sqlx::Sqlite>) -> Option<u16> {
    let res = sqlx::query_as::<_, (String,)>("SELECT value FROM settings WHERE key = 'port'")
        .fetch_one(pool)
        .await;

    match res {
        Ok((val,)) => val.parse::<u16>().ok(),
        Err(_) => None,
    }
}

/// 设置端口（仅 Android 端使用）
pub async fn set_port(pool: &sqlx::Pool<sqlx::Sqlite>, port: u16) -> Result<(), String> {
    sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES ('port', ?)")
        .bind(port.to_string())
        .execute(pool)
        .await
        .map_err(|e| format!("保存端口设置失败: {}", e))?;
    println!("[DB] 端口已设置为: {}", port);
    Ok(())
}

// ── 手动下载（file_offer / file_request） ───────────────────────

/// 当收到 file_offer 且 auto_download=OFF 时，创建 "offered" 状态的记录
pub async fn create_offered_file_record(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    sender_id: &str,
    file_name: &str,
    file_size: u64,
    sender_msg_id: &str,
    timestamp: i64,
) -> Result<i64, String> {
    let my_id = get_user_id(pool).await?;
    let result = sqlx::query(
        "INSERT INTO messages (sender_id, receiver_id, content, msg_type, timestamp, file_status, file_size, sender_msg_id) \
         VALUES (?, ?, ?, 'file', ?, 'offered', ?, ?)"
    )
    .bind(sender_id)
    .bind(&my_id)
    .bind(file_name)
    .bind(timestamp)
    .bind(file_size as i64)
    .bind(sender_msg_id)
    .execute(pool)
    .await
    .map_err(|e| format!("创建 offered 记录失败: {}", e))?;
    Ok(result.last_insert_rowid())
}

/// 当收到文件上传时，查找并更新已存在的 offered 记录为 downloading
/// 返回 (被更新的记录ID, sender_msg_id)（如果找到并更新），否则返回 None
pub async fn find_and_update_offered_record(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    sender_id: &str,
    file_name: &str,
    file_path: &str,
    file_size: u64,
) -> Result<Option<(i64, String)>, String> {
    // 查找 sender_id + file_name 匹配且 file_status = 'offered' 的记录，获取 id 和 sender_msg_id
    let result = sqlx::query_as::<_, (i64, String)>(
        "SELECT id, sender_msg_id FROM messages WHERE sender_id = ? AND content = ? AND file_status = 'offered' LIMIT 1"
    )
    .bind(sender_id)
    .bind(file_name)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("查询 offered 记录失败: {}", e))?;

    if let Some((msg_id, sender_msg_id)) = result {
        sqlx::query("UPDATE messages SET file_status = 'downloading', file_path = ?, file_size = ? WHERE id = ?")
            .bind(file_path)
            .bind(i64::try_from(file_size).map_err(|_| "文件过大".to_string())?)
            .bind(msg_id)
            .execute(pool)
            .await
            .map_err(|e| format!("更新 offered 为 downloading 失败: {}", e))?;
        println!(
            "[DB] 已更新 offered 记录(ID={}, sender_msg_id={}) 为 downloading，路径: {}",
            msg_id, sender_msg_id, file_path
        );
        Ok(Some((msg_id, sender_msg_id)))
    } else {
        Ok(None)
    }
}

/// 通过 sender_msg_id 更新 file_status
pub async fn update_file_status_by_sender_msg_id(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    sender_msg_id: &str,
    status: &str,
) -> Result<(), String> {
    sqlx::query("UPDATE messages SET file_status = ? WHERE sender_msg_id = ?")
        .bind(status)
        .bind(sender_msg_id)
        .execute(pool)
        .await
        .map_err(|e| format!("更新文件状态失败: {}", e))?;
    Ok(())
}

pub async fn mark_received_file_failed(
    pool: &sqlx::Pool<sqlx::Sqlite>, sender_id: &str, sender_msg_id: &str,
) -> Result<(), String> {
    sqlx::query("UPDATE messages SET file_status = 'failed' WHERE msg_type = 'file' AND sender_id = ? AND sender_msg_id = ?")
        .bind(sender_id).bind(sender_msg_id).execute(pool).await
        .map_err(|e| format!("更新接收文件状态失败: {e}"))?;
    Ok(())
}

/// 通过 sender_msg_id 查询发送端的文件记录（file_path 等）
pub async fn get_sender_file_by_msg_id(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    msg_id: i64,
) -> Result<(String, String, i64), String> {
    sqlx::query_as::<_, (String, String, i64)>(
        "SELECT file_path, content, file_size FROM messages WHERE id = ? AND sender_id = 'me'",
    )
    .bind(msg_id)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("查询发送端文件记录失败: {}", e))
}

// ═══════════════════════════════════════════════════════════════
// persisted_uris 表 CRUD
// ═══════════════════════════════════════════════════════════════

/// 添加一条持久化 URI 追踪记录
pub async fn add_persisted_uri(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    uri: &str,
    msg_id: i64,
) -> Result<(), String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    sqlx::query("INSERT OR IGNORE INTO persisted_uris (uri, msg_id, created_at) VALUES (?, ?, ?)")
        .bind(uri)
        .bind(msg_id)
        .bind(now)
        .execute(pool)
        .await
        .map_err(|e| format!("添加持久化 URI 记录失败: {}", e))?;

    println!("[DB] 已添加持久化 URI: msg_id={}, uri={}", msg_id, uri);
    Ok(())
}

/// 删除一条持久化 URI 追踪记录
pub async fn remove_persisted_uri(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    uri: &str,
) -> Result<(), String> {
    sqlx::query("DELETE FROM persisted_uris WHERE uri = ?")
        .bind(uri)
        .execute(pool)
        .await
        .map_err(|e| format!("删除持久化 URI 记录失败: {}", e))?;
    Ok(())
}

/// 通过 msg_id 删除持久化 URI 追踪记录
pub async fn remove_persisted_uri_by_msg_id(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    msg_id: i64,
) -> Result<(), String> {
    sqlx::query("DELETE FROM persisted_uris WHERE msg_id = ?")
        .bind(msg_id)
        .execute(pool)
        .await
        .map_err(|e| format!("删除持久化 URI 通过 msg_id 失败: {}", e))?;
    Ok(())
}

/// 查询最早的持久化 URI（用于 FIFO 淘汰）
pub async fn get_oldest_persisted_uri(
    pool: &sqlx::Pool<sqlx::Sqlite>,
) -> Result<Option<(i64, String, i64)>, String> {
    // 返回 (id, uri, msg_id)，按 created_at ASC
    let result = sqlx::query_as::<_, (i64, String, i64)>(
        "SELECT id, uri, msg_id FROM persisted_uris ORDER BY created_at ASC LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("查询最旧持久化 URI 失败: {}", e))?;
    Ok(result)
}

/// 统计当前追踪的持久化 URI 数量
pub async fn count_persisted_uris(pool: &sqlx::Pool<sqlx::Sqlite>) -> Result<i64, String> {
    let result = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM persisted_uris")
        .fetch_one(pool)
        .await
        .map_err(|e| format!("统计持久化 URI 数量失败: {}", e))?;
    Ok(result)
}

/// 通过 URI 查询对应的 msg_id
pub async fn get_msg_id_for_uri(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    uri: &str,
) -> Result<Option<i64>, String> {
    let result = sqlx::query_scalar::<_, i64>("SELECT msg_id FROM persisted_uris WHERE uri = ?")
        .bind(uri)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("通过 URI 查询 msg_id 失败: {}", e))?;
    Ok(result)
}

/// 通过 msg_id 查询对应的 URI
pub async fn get_uri_by_msg_id(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    msg_id: i64,
) -> Result<Option<String>, String> {
    let result = sqlx::query_scalar::<_, String>("SELECT uri FROM persisted_uris WHERE msg_id = ?")
        .bind(msg_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("通过 msg_id 查询 URI 失败: {}", e))?;
    Ok(result)
}

/// 更新消息的 file_path（用于持久化失败时降级为 FD 缓存标记）
pub async fn update_file_path_by_id(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    msg_id: i64,
    file_path: &str,
) -> Result<(), String> {
    sqlx::query("UPDATE messages SET file_path = ? WHERE id = ?")
        .bind(file_path)
        .bind(msg_id)
        .execute(pool)
        .await
        .map_err(|e| format!("更新 file_path 失败: {}", e))?;
    Ok(())
}

#[cfg(all(test, windows))]
mod windows_portable_tests {
    use super::*;

    #[tokio::test]
    async fn new_database_uses_lqchat_name_and_move_safe_download_marker() {
        let directory =
            std::env::temp_dir().join(format!("LQChat-portable-db-test-{}", uuid::Uuid::new_v4()));
        let pool = init_db_with_path(directory.clone()).await.unwrap();

        assert!(directory
            .join(crate::config_file::WINDOWS_DATABASE_FILE_NAME)
            .is_file());
        let stored: String =
            sqlx::query_scalar("SELECT value FROM settings WHERE key = 'download_path'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(stored, WINDOWS_PORTABLE_DOWNLOAD_SENTINEL);
        assert!(get_notification_sound_enabled(&pool).await);
        set_notification_sound_enabled(&pool, false).await.unwrap();
        assert!(!get_notification_sound_enabled(&pool).await);
        assert_eq!(
            get_download_path(&pool).await.unwrap(),
            crate::config_file::windows_portable_download_dir()
                .unwrap()
                .to_string_lossy()
        );

        pool.close().await;
        std::fs::remove_dir_all(directory).unwrap();
    }
}

/// 上传前同步本次打开文件的大小，避免旧媒体库信息进入重试与文件卡片。
pub async fn update_sent_file_size(pool: &sqlx::Pool<sqlx::Sqlite>, msg_id: i64, size: usize) -> Result<(), String> {
    sqlx::query("UPDATE messages SET file_size = ? WHERE id = ? AND sender_id = 'me'")
        .bind(i64::try_from(size).map_err(|_| "文件过大".to_string())?)
        .bind(msg_id).execute(pool).await
        .map_err(|error| format!("更新文件大小失败: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod file_size_tests {
    use super::*;

    #[tokio::test]
    async fn reopened_source_size_updates_sender_and_existing_receiver_offer() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1)
            .connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE messages (id INTEGER PRIMARY KEY, sender_id TEXT, content TEXT, file_size INTEGER, file_path TEXT, file_status TEXT, sender_msg_id TEXT)")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO messages VALUES (1, 'me', 'photo.jpg', 10476447, 'content://photo', 'failed', '1'), (2, 'phone', 'photo.jpg', 10476447, '', 'offered', '1')")
            .execute(&pool).await.unwrap();
        update_sent_file_size(&pool, 1, 2559098).await.unwrap();
        assert_eq!(find_and_update_offered_record(&pool, "phone", "photo.jpg", "/photo.jpg", 2559098).await.unwrap(), Some((2, "1".to_string())));
        let rows: Vec<(i64, String)> = sqlx::query_as("SELECT file_size, file_status FROM messages ORDER BY id")
            .fetch_all(&pool).await.unwrap();
        assert_eq!(rows, vec![(2559098, "failed".to_string()), (2559098, "downloading".to_string())]);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages").fetch_one(&pool).await.unwrap();
        assert_eq!(count, 2);
        pool.close().await;
    }
}
