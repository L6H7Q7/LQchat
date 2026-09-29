// commands.rs - Tauri 命令（桌面端和移动端共享）
use std::sync::atomic::AtomicBool;
#[cfg(not(target_os = "android"))]
use std::sync::atomic::Ordering;

#[cfg(feature = "desktop")]
use crate::db::DbState;

#[cfg(feature = "desktop")]
use crate::peers::{Peer, PeerManager};

#[cfg(feature = "desktop")]
use std::sync::Arc;

#[cfg(feature = "desktop")]
use tauri::{AppHandle, Emitter, Manager, State};

// 用于管理 PeerManager 的状态
#[cfg(feature = "desktop")]
pub struct PeerState {
    pub manager: Arc<PeerManager>,
}

impl PeerState {
    /// Android 的网络核心可能早于重建后的 Activity/UI 存在。此时 Tauri
    /// 状态中保存的 manager 只能作为兜底，命令必须优先读取正在运行的
    /// CoreRuntime 资源，避免实时事件与轮询分别使用两套在线状态。
    pub fn active_manager(&self) -> Arc<PeerManager> {
        #[cfg(target_os = "android")]
        if let Some((_, manager)) = crate::core_runtime::CoreRuntime::global().shared_resources() {
            return manager;
        }

        self.manager.clone()
    }
}

/// 托盘闪烁状态（仅桌面端）
pub struct TrayFlashState {
    pub is_flashing: Arc<AtomicBool>,
}

impl Default for TrayFlashState {
    fn default() -> Self {
        Self {
            is_flashing: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// 托盘菜单项（桌面端），用于语言热更新
#[cfg(all(feature = "desktop", not(target_os = "android")))]
pub struct TrayMenuItems {
    pub show_item: tauri::menu::MenuItem<tauri::Wry>,
    pub toggle_notif: tauri::menu::CheckMenuItem<tauri::Wry>,
    pub quit_item: tauri::menu::MenuItem<tauri::Wry>,
}

#[cfg(feature = "desktop")]

/// 每次确认一个 10 MB 分块后更新文件进度。
fn calculate_optimal_chunk_size(_file_size: usize) -> usize {
    crate::network::messaging::FILE_TRANSFER_CHUNK_SIZE
}

/// 统一的文件上传实现
/// 接受一个实现了 AsyncRead 的文件对象
async fn upload_file_internal<R: tokio::io::AsyncRead + Unpin>(
    app: &tauri::AppHandle,
    state: &State<'_, DbState>,
    _peer_state: Option<&State<'_, PeerState>>,
    peer_id: String,
    peer_addr: String,
    file_name: String,
    file_size: usize,
    file_path_for_db: String,
    mut file: R,
    is_online: bool,
    pre_saved_msg_id: Option<i64>, // 消息已提前保存时传入（如 Android FD 缓存场景），不再新建
) -> Result<serde_json::Value, String> {
    if !is_online {
        return Err("对方不在线".to_string());
    }
    let overall_status = "sent".to_string();
    let file_status = "uploading".to_string();

    let message_id = if let Some(existing_id) = pre_saved_msg_id {
        Some(existing_id)
    } else {
        crate::db::save_file_message(
            &state.pool,
            peer_id.clone(),
            file_name.clone(),
            file_size,
            file_path_for_db.clone(),
            file_status,
            overall_status,
        )
        .await
        .ok()
    };

    if let Some(id) = message_id {
        let _ = app.emit("file_upload_started", serde_json::json!({
            "sender_msg_id": id, "peer_id": peer_id,
        }));
    }

    // 统一修正 fd: 路径为 fd:{msg_id}（媒体服务器按 msg_id 查 FD 缓存）
    if let Some(mid) = message_id {
        if file_path_for_db.starts_with("fd:") {
            let corrected = format!("fd:{}", mid);
            if let Err(e) = crate::db::update_file_path_by_id(&state.pool, mid, &corrected).await {
                eprintln!("[Command] 修正 FD 路径失败: {}", e);
            }
        }
    }

    // 获取自己的 ID（发送者 ID）
    let my_id = crate::db::get_user_id(&state.pool).await?;

    let adjusted_chunk_size = calculate_optimal_chunk_size(file_size);

    let total_chunks = (file_size + adjusted_chunk_size - 1) / adjusted_chunk_size;

    println!(
        "[Command] 开始分块上传: 文件大小={}, 分块大小={}, 总分块数={}",
        file_size, adjusted_chunk_size, total_chunks
    );

    let client = crate::network::lan_http_client(Some(std::time::Duration::from_secs(300)))?;

    let upload_url = format!("http://{}/api/upload", peer_addr);

    let mut offset = 0;
    let mut chunk_index = 0;
    let start_time = std::time::Instant::now();

    while offset < file_size {
        // 读取分块
        let expected_len = adjusted_chunk_size.min(file_size - offset);
        let mut buf = vec![0u8; expected_len];
        let mut bytes_read = 0;

        while bytes_read < expected_len {
            let n = match tokio::io::AsyncReadExt::read(&mut file, &mut buf[bytes_read..]).await {
                Ok(n) => n,
                Err(e) => {
                    if let Some(id) = message_id {
                        let _ = crate::db::update_file_status_by_id(&state.pool, id, "failed").await;
                    }
                    let _ = app.emit("upload_progress", serde_json::json!({
                        "sender_msg_id": message_id, "transfer_status": "failed",
                        "transferred": offset, "total": file_size,
                    }));
                    return Err(format!("读取文件失败: {}", e));
                }
            };

            if n == 0 {
                break;
            }

            bytes_read += n;
        }

        if bytes_read != expected_len {
            if let Some(id) = message_id {
                let _ = crate::db::update_file_status_by_id(&state.pool, id, "failed").await;
            }
            let _ = app.emit("upload_progress", serde_json::json!({
                "sender_msg_id": message_id, "transfer_status": "failed",
                "transferred": offset, "total": file_size,
            }));
            return Err(format!("文件读取长度不符: 已读取 {bytes_read}/{expected_len} 字节"));
        }

        let n = bytes_read;

        // 在最后一块发出前确认流已结束，避免接收端先标记完成而发送端随后发现多余数据。
        if offset + n == file_size {
            let mut extra = [0u8; 1];
            match tokio::io::AsyncReadExt::read(&mut file, &mut extra).await {
                Ok(0) => {}
                Ok(_) => {
                    if let Some(id) = message_id {
                        let _ = crate::db::update_file_status_by_id(&state.pool, id, "failed").await;
                    }
                    let _ = app.emit("upload_progress", serde_json::json!({
                        "sender_msg_id": message_id, "transfer_status": "failed",
                        "transferred": offset, "total": file_size,
                    }));
                    return Err("文件实际大小超过分享方提供的大小".to_string());
                }
                Err(error) => {
                    if let Some(id) = message_id {
                        let _ = crate::db::update_file_status_by_id(&state.pool, id, "failed").await;
                    }
                    let _ = app.emit("upload_progress", serde_json::json!({
                        "sender_msg_id": message_id, "transfer_status": "failed",
                        "transferred": offset, "total": file_size,
                    }));
                    return Err(format!("确认文件结尾失败: {error}"));
                }
            }
        }

        // 构造 multipart 请求
        // 计算当前累计速度（发送给接收端直接展示）
        let speed_mb_s = if chunk_index > 0 {
            let elapsed = start_time.elapsed().as_secs_f64();
            if elapsed > 0.0 {
                (offset as f64 / (1024.0 * 1024.0)) / elapsed
            } else {
                0.0
            }
        } else {
            0.0
        };

        let sender_msg_id_str = message_id.map(|id| id.to_string()).unwrap_or_default();

        let form = reqwest::multipart::Form::new()
            .text("peer_id", my_id.clone())
            .text("file_name", file_name.clone())
            .text("file_size", file_size.to_string())
            .text("chunk_index", chunk_index.to_string())
            .text("chunk_total", total_chunks.to_string())
            .text("sender_msg_id", sender_msg_id_str)
            .text("speed_mb_s", format!("{:.1}", speed_mb_s))
            .part(
                "chunk",
                reqwest::multipart::Part::bytes(buf.clone())
                    .mime_str("application/octet-stream")
                    .map_err(|e| format!("设置 MIME 类型失败: {}", e))?,
            );

        println!(
            "[Command] 上传分块 {}/{}, 大小: {} 字节",
            chunk_index + 1,
            total_chunks,
            n
        );

        let response = match client.post(&upload_url).multipart(form).send().await {
            Ok(response) => response,
            Err(e) => {
                if let Some(id) = message_id {
                    let _ = crate::db::update_file_status_by_id(&state.pool, id, "chunk_failed").await;
                }
                let _ = app.emit("upload_progress", serde_json::json!({
                    "sender_msg_id": message_id, "transfer_status": "chunk_failed",
                    "transferred": offset, "total": file_size,
                }));
                return Err(format!("上传分块失败: {}", e));
            }
        };

        if !response.status().is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "无法读取错误信息".to_string());
            eprintln!("[Command] ✗ 上传分块失败: {}", error_text);

            if let Some(id) = message_id {
                let _ = crate::db::update_file_status_by_id(&state.pool, id, "chunk_failed").await;
            }
            let _ = app.emit("upload_progress", serde_json::json!({
                "sender_msg_id": message_id, "transfer_status": "chunk_failed",
                "transferred": offset, "total": file_size,
            }));

            return Err(format!("上传分块失败: {}", error_text));
        }

        // 检查是否秒传命中（接收端已有完整文件，所有分块都检查）
        let resp_text = response.text().await.unwrap_or_default();
        if let Ok(resp_json) = serde_json::from_str::<serde_json::Value>(&resp_text) {
            if resp_json.get("status").and_then(|s| s.as_str()) == Some("already_exists") {
                println!("[Command] ✓ 秒传命中，接收端已有完整文件，停止上传");
                if let Some(id) = message_id {
                    let _ = crate::db::update_file_status_by_id(&state.pool, id, "sent").await;
                }
                return Ok(serde_json::json!({
                    "success": true,
                    "file_name": file_name,
                    "file_size": file_size,
                    "instant_transfer": true,
                }));
            }
        }

        // 如果 body 已在 already_exists 检查中被消耗，后续需要时补充
        // 非秒传情况不需要响应 body

        offset += n;
        chunk_index += 1;

        // 打印进度
        let elapsed = start_time.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            let speed = offset as f64 / (1024.0 * 1024.0) / elapsed;
            println!(
                "[Command] 已上传: {} MB, 速度: {:.2} MB/s",
                offset / (1024 * 1024),
                speed
            );
            let _ = app.emit(
                "upload_progress",
                serde_json::json!({
                    "file_name": file_name.clone(),
                    "speed_mb_s": speed,
                    "sender_msg_id": message_id,
                    "transferred": offset,
                    "total": file_size,
                }),
            );
        }
    }

    if offset != file_size {
        if let Some(id) = message_id {
            let _ = crate::db::update_file_status_by_id(&state.pool, id, "failed").await;
        }
        let _ = app.emit("upload_progress", serde_json::json!({
            "sender_msg_id": message_id, "transfer_status": "failed",
            "transferred": offset, "total": file_size,
        }));
        return Err(format!("文件未完整发送: {offset}/{file_size}"));
    }
    let total_time = start_time.elapsed().as_secs_f64();
    let avg_speed = file_size as f64 / (1024.0 * 1024.0) / total_time;
    println!(
        "[Command] ✓ 文件上传完成，耗时: {:.2}s, 平均速度: {:.2} MB/s",
        total_time, avg_speed
    );

    // 更新数据库状态为 "sent"
    if let Some(id) = message_id {
        if let Err(e) = crate::db::update_file_status_by_id(&state.pool, id, "sent").await {
            eprintln!("[Command] ⚠ 更新数据库状态失败: {}", e);
        } else {
            println!("[Command] ✓ 文件状态已更新为 sent");
        }
    }

    Ok(serde_json::json!({
        "success": true,
        "file_name": file_name,
        "file_size": file_size,
    }))
}

#[cfg(target_os = "android")]
use std::os::unix::io::FromRawFd;

#[tauri::command]
pub async fn close_android_fd(#[allow(unused_variables)] fd: i32) {
    #[cfg(target_os = "android")]
    {
        if fd >= 0 {
            // 在 Rust 中，使用 from_raw_fd 接管 FD 的所有权。
            // 当这个 block 结束时，_file 会被 drop，Rust 会自动安全地调用底层的 close(fd)
            unsafe {
                let _file = std::fs::File::from_raw_fd(fd);
            }
            println!("[Rust] 成功释放被取消的共享文件描述符: {}", fd);
        }
    }
}

#[tauri::command]
pub async fn get_my_name(state: State<'_, DbState>) -> Result<String, String> {
    let name = crate::db::get_username(&state.pool).await?;
    #[cfg(target_os = "android")]
    if let Err(error) = call_android_activity_string_arg("setLocalDeviceName", &name) {
        eprintln!("[Command] 同步 Android 本机设备名失败: {error}");
    }
    Ok(name)
}

#[tauri::command]
pub async fn get_my_id(state: State<'_, DbState>) -> Result<String, String> {
    crate::db::get_user_id(&state.pool).await
}

#[tauri::command]
pub async fn get_settings(state: State<'_, DbState>) -> Result<serde_json::Value, String> {
    let download_path = crate::db::get_download_path(&state.pool).await?;
    #[cfg(target_os = "android")]
    let port = crate::db::get_port(&state.pool)
        .await
        .map(|p| p.to_string())
        .unwrap_or_else(|| "8888".to_string());
    #[cfg(not(target_os = "android"))]
    let port = crate::config_file::get_port_from_config()
        .map(|p| p.to_string())
        .unwrap_or_else(|| "8888".to_string());
    let cfg = crate::config_file::read_config();
    let close_to_tray = cfg.close_to_tray.unwrap_or(true);
    let start_minimized = cfg.start_minimized.unwrap_or(true);
    let db_path = cfg
        .db_path
        .unwrap_or_else(crate::config_file::get_default_db_path);
    let auto_download = crate::db::get_auto_download(&state.pool).await;

    Ok(serde_json::json!({
        "download_path": download_path,
        "port": port,
        "db_path": db_path,
        "auto_download": auto_download,
        "close_to_tray": close_to_tray,
        "start_minimized": start_minimized,
    }))
}

#[tauri::command]
pub async fn update_settings(
    state: State<'_, DbState>,
    download_path: Option<String>,
    port: Option<String>,
    db_path: Option<String>,
    auto_download: Option<bool>,
    close_to_tray: Option<bool>,
    start_minimized: Option<bool>,
) -> Result<(), String> {
    if let Some(path) = download_path {
        crate::db::update_download_path(&state.pool, path).await?;
    }
    if let Some(ref p) = port {
        let port_num: u16 = p.parse().map_err(|_| "Invalid port".to_string())?;
        #[cfg(target_os = "android")]
        {
            crate::db::set_port(&state.pool, port_num).await?;
        }
        #[cfg(not(target_os = "android"))]
        {
            crate::config_file::save_port_to_config(port_num)?;
        }
    }
    if let Some(path) = db_path {
        if !cfg!(target_os = "android") {
            let mut cfg = crate::config_file::read_config();
            if path.is_empty() {
                cfg.db_path = None;
            } else {
                cfg.db_path = Some(path);
            }
            crate::config_file::write_config(&cfg)?;
        }
    }
    if let Some(enabled) = auto_download {
        crate::db::set_auto_download(&state.pool, enabled).await?;
    }
    if let Some(_enabled) = close_to_tray {
        #[cfg(not(target_os = "android"))]
        crate::config_file::save_close_to_tray_to_config(_enabled)?;
    }
    if let Some(_enabled) = start_minimized {
        #[cfg(not(target_os = "android"))]
        crate::config_file::save_start_minimized_to_config(_enabled)?;
    }

    Ok(())
}

#[tauri::command]
pub fn get_language() -> Result<String, String> {
    Ok(crate::config_file::get_lang_from_config().unwrap_or_else(|| "auto".to_string()))
}

#[tauri::command]
#[allow(unused_variables)]
pub fn set_language(lang: String, app: tauri::AppHandle) -> Result<(), String> {
    crate::config_file::save_lang_to_config(&lang)?;

    // 桌面端托盘菜单热更新
    #[cfg(all(feature = "desktop", not(target_os = "android")))]
    if let Some(state) = app.try_state::<TrayMenuItems>() {
        if lang == "en" {
            let _ = state.show_item.set_text("Show Window");
            let _ = state.toggle_notif.set_text("Enable Notifications");
            let _ = state.quit_item.set_text("Quit");
        } else {
            let _ = state.show_item.set_text("显示窗口");
            let _ = state.toggle_notif.set_text("开启通知");
            let _ = state.quit_item.set_text("退出");
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn update_my_name(state: State<'_, DbState>, new_name: String) -> Result<String, String> {
    // 更新数据库
    crate::db::update_username(&state.pool, new_name.clone()).await?;

    let saved_name = crate::db::get_username(&state.pool).await?;
    #[cfg(target_os = "android")]
    if let Err(error) = call_android_activity_string_arg("setLocalDeviceName", &saved_name) {
        eprintln!("[Command] 同步 Android 本机设备名失败: {error}");
    }

    // 数据库更新后，定时广播线程会自动使用新名称
    println!("[Command] 用户名已更新，广播线程将使用新名称");

    // 返回更新后的名字
    Ok(saved_name)
}

#[tauri::command]
pub async fn get_peers(state: State<'_, PeerState>) -> Result<Vec<Peer>, String> {
    Ok(state.active_manager().get_all_peers())
}

#[tauri::command]
pub async fn send_message(
    state: State<'_, DbState>,
    peer_state: State<'_, PeerState>,
    peer_id: String,
    mut peer_addr: String,
    content: String,
) -> Result<(), String> {
    println!("[Command] 收到发送消息请求: 发送给 {}", peer_id);
    let my_id = crate::db::get_user_id(&state.pool).await?;
    let my_name = crate::db::get_username(&state.pool).await?;

    // 提取状态时，顺便把后端内存里最新的 IP 拿出来
    let (is_offline_now, peer_name, backend_addr) = {
        let peers = peer_state.active_manager().get_all_peers();
        if let Some(p) = peers.iter().find(|p| p.id == peer_id) {
            (p.is_offline, p.name.clone(), Some(p.addr.clone()))
        } else {
            (true, "未知".into(), None)
        }
    };

    // 后端终极校验：如果前端传来的 IP 和后端最新的不一致，强制纠正！
    if let Some(latest_addr) = backend_addr {
        if latest_addr != peer_addr {
            println!(
                "[Command] 🛡️ 拦截到过期 IP，后端强行纠正: {} -> {}",
                peer_addr, latest_addr
            );
            peer_addr = latest_addr;
        }
    }

    if is_offline_now {
        return Err("对方不在线".to_string());
    }

    // 3. 尝试发送（这同时也是一种探测）
    println!("[Command] 尝试发送消息给 {}({})...", peer_name, peer_addr);
    match crate::network::messaging::send_text_message(&peer_addr, my_id, my_name, content.clone())
        .await
    {
        Ok(_) => {
            // 发送成功
            crate::db::save_text_message_with_status(
                &state.pool,
                peer_id,
                content,
                "sent".to_string(),
            )
            .await?;
        }
        Err(e) => {
            // 发送失败（探测到实际已离线或网络故障）
            eprintln!("[Command] 发送失败(网络探测): {}", e);

            // 立即更新本地状态，避免下次发送再次空转
            peer_state.active_manager().force_mark_offline(&peer_id);

            return Err("对方不在线".to_string());
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn get_chat_history(
    state: State<'_, DbState>,
    peer_id: String,
) -> Result<Vec<serde_json::Value>, String> {
    crate::network::messaging::get_chat_history(&state.pool, &peer_id, 10).await
}

#[tauri::command]
pub async fn get_chat_history_with_offset(
    state: State<'_, DbState>,
    peer_id: String,
    limit: i32,
    offset: i32,
) -> Result<Vec<serde_json::Value>, String> {
    crate::network::messaging::get_chat_history_with_offset(&state.pool, &peer_id, limit, offset)
        .await
}

#[tauri::command]
pub async fn send_file(
    app: tauri::AppHandle,
    state: State<'_, DbState>,
    peer_state: State<'_, PeerState>,
    peer_id: String,
    mut peer_addr: String,
    file_path: String,
) -> Result<serde_json::Value, String> {
    println!(
        "[Command] 收到发送文件请求: {} -> {} ({})",
        file_path, peer_addr, peer_id
    );

    // 处理 file:// URI 格式
    let actual_path = if file_path.starts_with("file://") {
        let path_without_prefix = &file_path[7..];
        urlencoding::decode(path_without_prefix)
            .map_err(|e| format!("解码 URI 失败: {}", e))?
            .to_string()
    } else {
        file_path.clone()
    };

    println!("[Command] 实际文件路径: {}", actual_path);

    // ── 提取文件元数据（不打开文件） ──
    let (file_name, file_size) = if actual_path.starts_with("content://") {
        #[cfg(target_os = "android")]
        {
            use crate::android_fd::AndroidFile;
            let (name, size) =
                AndroidFile::query_content_uri_info(&actual_path).unwrap_or_default();
            let name = if name.is_empty() {
                let raw_seg = actual_path
                    .split('/')
                    .last()
                    .and_then(|s| urlencoding::decode(s).ok())
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                let name = if let Some(idx) = raw_seg.rfind(':') {
                    raw_seg[idx + 1..].to_string()
                } else {
                    raw_seg
                };
                if !name.contains('.') || name.is_empty() {
                    format!("file_{}.dat", chrono::Utc::now().timestamp())
                } else {
                    name
                }
            } else {
                name
            };
            (name, size as usize)
        }
        #[cfg(not(target_os = "android"))]
        {
            return Err("content:// URI 仅在 Android 上支持".to_string());
        }
    } else {
        let name = std::path::Path::new(&actual_path)
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("无效的文件名")?
            .to_string();
        let metadata =
            std::fs::metadata(&actual_path).map_err(|e| format!("读取文件信息失败: {}", e))?;
        (name, metadata.len() as usize)
    };

    println!("[Command] 文件: {}, 大小: {} 字节", file_name, file_size);

    // ── 检查接收端是否离线 ──
    let is_offline_now = {
        let peers = peer_state.active_manager().get_all_peers();
        peers
            .iter()
            .find(|p| p.id == peer_id)
            .map(|p| p.is_offline)
            .unwrap_or(true)
    };

    if is_offline_now {
        return Err("对方不在线".to_string());
    }

    // ── 检查接收端的 auto_download 设置 ──
    let auto_enabled = {
        let auto_dl_url = format!("http://{}/api/auto_download", peer_addr);
        match crate::network::lan_http_client(None) {
            Ok(client) => match client.get(&auto_dl_url).send().await {
                Ok(resp) => {
                    if let Ok(data) = resp.json::<serde_json::Value>().await {
                        data.get("enabled")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(true)
                    } else {
                        true
                    }
                }
                Err(_) => true, // 连不上时默认 ON（保底走现有上传流程）
            },
            Err(_) => true,
        }
    };

    if !auto_enabled {
        // ── 自动下载关闭：先发 file_offer，不上传 ──
        let msg_id = crate::db::save_file_message(
            &state.pool,
            peer_id.clone(),
            file_name.clone(),
            file_size,
            actual_path.clone(),
            "offering".to_string(),
            "sent".to_string(),
        )
        .await
        .ok();

        // ── Android: 持久化 content URI 权限（auto OFF 场景） ──
        #[cfg(target_os = "android")]
        if actual_path.starts_with("content://") {
            use crate::android_fd::AndroidFile;
            let sender_msg_id_val = msg_id.unwrap_or(0);

            // 1. 如果系统配额将满，FIFO 淘汰最旧的
            let max_limit = if AndroidFile::get_api_level().unwrap_or(30) >= 30 {
                500
            } else {
                120
            };
            let sys_count = AndroidFile::get_persisted_uri_count().unwrap_or(0);
            let need_free = (sys_count as i64) - (max_limit as i64 - 1);
            if need_free > 0 {
                for _ in 0..need_free {
                    match crate::db::get_oldest_persisted_uri(&state.pool)
                        .await
                        .unwrap_or(None)
                    {
                        Some((_id, oldest_uri, _oldest_msg_id)) => {
                            let _ = AndroidFile::release_persistable_uri_permission(&oldest_uri);
                            let _ = crate::db::remove_persisted_uri(&state.pool, &oldest_uri).await;
                            println!("[Command] FIFO 淘汰持久化 URI: {}", oldest_uri);
                        }
                        None => break,
                    }
                }
            }

            // 2. 尝试持久化当前 URI
            match AndroidFile::take_persistable_uri_permission(&actual_path) {
                Ok(_) => {
                    let _ =
                        crate::db::add_persisted_uri(&state.pool, &actual_path, sender_msg_id_val)
                            .await;
                    println!("[Command] ✓ content URI 权限已持久化: {}", actual_path);
                }
                Err(e) => {
                    // 持久化失败（例如 Tauri dialog 未加 FLAG_GRANT_PERSISTABLE_URI_PERMISSION）
                    // 降级方案：趁临时权限还在，立刻提取 FD 入缓存
                    println!("[Command] ⚠ URI 不支持持久化，使用 FD 缓存兜底: {}", e);
                    if let Ok(af) = AndroidFile::from_content_uri(&actual_path) {
                        let raw_fd = af.into_raw_fd();
                        crate::android_fd::cache_fd_for_msg(
                            sender_msg_id_val,
                            raw_fd,
                            file_name.clone(),
                            file_size as u64,
                        );
                        // 更新 DB 文件路径为 "fd:{msg_id}" 标记，让 file_request 走 FD 缓存
                        let _ = crate::db::update_file_path_by_id(
                            &state.pool,
                            sender_msg_id_val,
                            &format!("fd:{}", sender_msg_id_val),
                        )
                        .await;
                        println!(
                            "[Command] ✓ FD 已缓存作为降级: msg_id={}",
                            sender_msg_id_val
                        );
                    }
                }
            }
        }

        let my_id = crate::db::get_user_id(&state.pool)
            .await
            .unwrap_or_default();
        let my_name = crate::db::get_username(&state.pool)
            .await
            .unwrap_or_default();
        let sender_msg_id = msg_id.unwrap_or(0);

        // 通过 WS 向接收端发送 file_offer
        let offer = serde_json::json!({
            "msg_type": "file_offer",
            "from_id": my_id,
            "from_name": my_name,
            "file_name": file_name,
            "file_size": file_size,
            "sender_msg_id": sender_msg_id,
        });
        let _ = crate::network::messaging::send_json_via_ws(&peer_addr, &offer.to_string()).await;

        return Ok(serde_json::json!({
            "success": true,
            "status": "offered",
            "msg_id": sender_msg_id,
            "file_name": file_name,
        }));
    }

    // ── 自动下载开启：现有上传流程 ──
    // 检测是否是 Android content URI
    if actual_path.starts_with("content://") {
        #[cfg(target_os = "android")]
        {
            println!("[Command] 检测到 Android content URI，使用 FD 方式");
            use crate::android_fd::AndroidFile;

            let android_file = AndroidFile::from_content_uri(&actual_path)?;
            let std_file = android_file.into_file();
            let file = tokio::fs::File::from_std(std_file);

            let peer_state = app.try_state::<PeerState>();
            let (is_online, backend_addr) = peer_state
                .as_ref()
                .map(|s| {
                    if let Some(p) = s
                        .active_manager()
                        .get_all_peers()
                        .iter()
                        .find(|p| p.id == peer_id)
                    {
                        (!p.is_offline, Some(p.addr.clone()))
                    } else {
                        (false, None)
                    }
                })
                .unwrap_or((true, None));

            if let Some(latest_addr) = backend_addr {
                if latest_addr != peer_addr {
                    peer_addr = latest_addr;
                }
            }
            return upload_file_internal(
                &app,
                &state,
                peer_state.as_ref(),
                peer_id,
                peer_addr,
                file_name,
                file_size,
                actual_path,
                file,
                is_online,
                None,
            )
            .await;
        }
        #[cfg(not(target_os = "android"))]
        {
            return Err("content:// URI 仅在 Android 上支持".to_string());
        }
    }

    // 普通文件路径
    let file = tokio::fs::File::open(&actual_path)
        .await
        .map_err(|e| format!("打开文件失败: {}", e))?;

    let peer_state = app.try_state::<PeerState>();
    let (is_online, backend_addr) = peer_state
        .as_ref()
        .map(|s| {
            if let Some(p) = s
                .active_manager()
                .get_all_peers()
                .iter()
                .find(|p| p.id == peer_id)
            {
                (!p.is_offline, Some(p.addr.clone()))
            } else {
                (false, None)
            }
        })
        .unwrap_or((true, None));

    if let Some(latest_addr) = backend_addr {
        if latest_addr != peer_addr {
            peer_addr = latest_addr;
        }
    }
    upload_file_internal(
        &app,
        &state,
        peer_state.as_ref(),
        peer_id,
        peer_addr,
        file_name,
        file_size,
        actual_path,
        file,
        is_online,
        None,
    )
    .await
}

#[tauri::command]
pub async fn get_theme_list() -> Result<Vec<serde_json::Value>, String> {
    let mut themes = vec![
        serde_json::json!({
            "name": "default",
            "display_name": "Default",
            "is_custom": false,
            "is_builtin": true
        }),
        serde_json::json!({
            "name": "vscode",
            "display_name": "VSCode",
            "is_custom": false,
            "is_builtin": true
        }),
    ];

    // 检查自定义主题目录
    let theme_dir = crate::config_file::custom_theme_dir()?;

    if theme_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&theme_dir) {
            for entry in entries {
                if let Ok(entry) = entry {
                    let path = entry.path();
                    if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("css") {
                        if let Some(file_name) = path.file_stem().and_then(|s| s.to_str()) {
                            themes.push(serde_json::json!({
                                "name": file_name,
                                "display_name": file_name,
                                "is_custom": true,
                                "is_builtin": false,
                                "path": path.to_string_lossy()
                            }));
                        }
                    }
                }
            }
        }
    }

    println!("[Command] 找到 {} 个主题", themes.len());
    Ok(themes)
}

#[tauri::command]
pub async fn get_theme_css(theme_name: String) -> Result<String, String> {
    if theme_name == "default" {
        return Ok(String::new()); // 默认主题返回空字符串
    }

    // 检查是否是内置主题
    if theme_name == "vscode" {
        // 从嵌入的资源中读取 vscode.css
        let css_content = include_str!("../../src/css/vscode.css");
        println!(
            "[Command] 加载内置主题: vscode ({} 字节)",
            css_content.len()
        );
        return Ok(css_content.to_string());
    }

    // 自定义主题从用户目录读取
    let theme_path = crate::config_file::custom_theme_dir()?.join(format!("{}.css", theme_name));

    if !theme_path.exists() {
        return Err(format!("主题文件不存在: {}", theme_path.display()));
    }

    let css_content =
        std::fs::read_to_string(&theme_path).map_err(|e| format!("读取主题文件失败: {}", e))?;

    println!(
        "[Command] 成功读取主题文件: {} ({} 字节)",
        theme_path.display(),
        css_content.len()
    );
    Ok(css_content)
}

#[tauri::command]
pub async fn save_current_theme(
    state: State<'_, DbState>,
    theme_name: String,
) -> Result<(), String> {
    // 保存当前主题到数据库
    crate::db::save_current_theme(&state.pool, theme_name.clone()).await?;

    println!("[Command] 主题设置已保存: {}", theme_name);
    Ok(())
}

#[tauri::command]
pub async fn get_current_theme(state: State<'_, DbState>) -> Result<String, String> {
    let result = crate::db::get_current_theme(&state.pool).await?;

    let theme = result.unwrap_or_else(|| "default".to_string());
    println!("[Command] 当前主题: {}", theme);
    Ok(theme)
}

#[tauri::command]
pub async fn get_default_download_path(_state: State<'_, DbState>) -> Result<String, String> {
    if cfg!(target_os = "android") {
        let download_path = crate::db::get_download_path(&_state.pool).await?;
        println!("[Command] Android 下载目标: {}", download_path);
        Ok(download_path)
    } else {
        #[cfg(windows)]
        let download_path = crate::config_file::windows_portable_download_dir()?;
        #[cfg(not(windows))]
        let download_path = dirs::home_dir()
            .ok_or("无法获取用户主目录")?
            .join("Downloads")
            .join("LQChat");
        println!("[Command] 默认下载路径: {}", download_path.display());
        Ok(download_path.to_string_lossy().to_string())
    }
}

#[tauri::command]
pub async fn request_storage_permission() -> Result<bool, String> {
    #[cfg(target_os = "android")]
    {
        crate::android_fd::AndroidFile::trigger_download_directory_picker_jni()?;
        println!("[Command] 已打开 Android 系统下载文件夹选择器");
        return Ok(true);
    }

    #[cfg(not(target_os = "android"))]
    {
        // 桌面端不需要权限
        Ok(true)
    }
}

#[tauri::command]
pub async fn save_file_message(
    state: State<'_, DbState>,
    peer_id: String,
    file_name: String,
    file_size: usize,
    file_path: String,
    status: String,
) -> Result<i64, String> {
    println!(
        "[Command] 文件: {}, 大小: {}, 状态: {}",
        file_name, file_size, status
    );

    // 使用数据库层的函数
    crate::db::save_file_message(
        &state.pool,
        peer_id,
        file_name,
        file_size,
        file_path,
        status,
        "sent".to_string(),
    )
    .await
}
#[cfg(feature = "desktop")]
#[tauri::command]
#[cfg_attr(target_os = "android", allow(unused_variables))]
pub async fn open_file_location(app: tauri::AppHandle, file_path: String) -> Result<(), String> {
    #[cfg(not(target_os = "android"))]
    {
        use tauri_plugin_opener::OpenerExt;

        println!("[Command] 打开文件位置: {}", file_path);

        app.opener()
            .reveal_item_in_dir(&file_path)
            .map_err(|e| format!("打开文件位置失败: {}", e))?;

        println!("[Command] ✓ 文件位置已打开");
    }

    // Android 不支持"在文件管理器中显示位置"
    Ok(())
}

#[cfg(feature = "desktop")]
#[tauri::command]
pub async fn set_android_shared_files(
    app: tauri::AppHandle,
    files: Vec<serde_json::Value>,
) -> Result<(), String> {
    println!(
        "[Command] set_android_shared_files 被调用，文件数: {}",
        files.len()
    );

    #[cfg(target_os = "android")]
    {
        use tauri::Manager;

        if let Some(share_state) = app.try_state::<AndroidShareState>() {
            share_state.set_files(files);
            println!("[Command] 文件已保存到状态");
            return Ok(());
        }

        println!("[Command] 没有找到分享状态");
        Err("分享状态未初始化".to_string())
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, files);
        Err("此功能仅在 Android 上可用".to_string())
    }
}

#[tauri::command]
pub async fn get_android_shared_files(
    app: tauri::AppHandle,
) -> Result<Vec<serde_json::Value>, String> {
    println!("[Command] get_android_shared_files 被调用");

    #[cfg(target_os = "android")]
    {
        let _ = app;
        let json = call_android_activity_string("takeAndroidSharedFiles")?;
        serde_json::from_str(&json).map_err(|error| format!("解析 Android 分享文件失败: {error}"))
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Err("此功能仅在 Android 上可用".to_string())
    }
}

#[tauri::command]
pub async fn clear_android_shared_files(app: tauri::AppHandle) -> Result<(), String> {
    println!("[Command] clear_android_shared_files 被调用");

    #[cfg(target_os = "android")]
    {
        use tauri::Manager;

        if let Some(share_state) = app.try_state::<AndroidShareState>() {
            share_state.clear_files();
            println!("[Command] 已清除分享文件");
        }

        Ok(())
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Err("此功能仅在 Android 上可用".to_string())
    }
}

// Android 分享状态管理
#[cfg(all(feature = "desktop", target_os = "android"))]
pub struct AndroidShareState {
    files: std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
}

#[cfg(target_os = "android")]
impl AndroidShareState {
    pub fn new() -> Self {
        Self {
            files: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    pub fn set_files(&self, files: Vec<serde_json::Value>) {
        if let Ok(mut f) = self.files.lock() {
            *f = files;
            println!("[AndroidShareState] 已设置 {} 个文件", f.len());
        }
    }

    pub fn get_files(&self) -> Vec<serde_json::Value> {
        if let Ok(f) = self.files.lock() {
            println!("[AndroidShareState] 获取 {} 个文件", f.len());
            f.clone()
        } else {
            Vec::new()
        }
    }

    pub fn clear_files(&self) {
        if let Ok(mut f) = self.files.lock() {
            f.clear();
            println!("[AndroidShareState] 已清除文件");
        }
    }
}

#[cfg(all(feature = "desktop", not(target_os = "android")))]
pub struct AndroidShareState;

#[cfg(all(feature = "desktop", not(target_os = "android")))]
impl AndroidShareState {
    pub fn new() -> Self {
        Self
    }
}

#[tauri::command]
pub async fn open_saf_picker() -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        crate::android_fd::AndroidFile::trigger_saf_picker_jni()
    }
    #[cfg(not(target_os = "android"))]
    {
        Err("该功能仅在 Android 端可用".to_string())
    }
}

#[tauri::command]
pub async fn open_saf_multi_picker() -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        crate::android_fd::AndroidFile::trigger_saf_multi_picker_jni()
    }
    #[cfg(not(target_os = "android"))]
    {
        Err("该功能仅在 Android 端可用".to_string())
    }
}

#[tauri::command]
pub async fn load_android_media_images() -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        crate::android_fd::AndroidFile::trigger_media_images_jni()
    }
    #[cfg(not(target_os = "android"))]
    {
        Err("该功能仅在 Android 端可用".to_string())
    }
}

#[tauri::command]
pub async fn load_android_apps() -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        crate::android_fd::AndroidFile::trigger_installed_apps_jni()
    }
    #[cfg(not(target_os = "android"))]
    {
        Err("该功能仅在 Android 端可用".to_string())
    }
}

#[tauri::command]
pub fn get_local_device_info() -> serde_json::Value {
    let ip = std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|socket| {
            socket.connect("8.8.8.8:80")?;
            socket.local_addr()
        })
        .map(|addr| addr.ip().to_string())
        .unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = crate::config_file::get_port_from_config().unwrap_or(8888);
    serde_json::json!({ "ip": ip, "port": port })
}

#[cfg(target_os = "android")]
async fn persist_android_fd(
    app: &tauri::AppHandle,
    fd: i32,
    file_name: &str,
) -> Result<String, String> {
    use crate::android_fd::AndroidFile;
    let safe_name: String = file_name
        .chars()
        .map(|ch| {
            if matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                ch
            }
        })
        .collect();
    let outbox = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("无法定位应用数据目录: {}", e))?
        .join("outbox");
    tokio::fs::create_dir_all(&outbox)
        .await
        .map_err(|e| format!("创建离线附件目录失败: {}", e))?;
    let target = outbox.join(format!("{}_{}", uuid::Uuid::new_v4(), safe_name));
    let source = AndroidFile::from_fd(fd)?.into_file();
    let mut source = tokio::fs::File::from_std(source);
    let mut destination = tokio::fs::File::create(&target)
        .await
        .map_err(|e| format!("创建离线附件副本失败: {}", e))?;
    tokio::io::copy(&mut source, &mut destination)
        .await
        .map_err(|e| format!("保存离线附件失败: {}", e))?;
    Ok(target.to_string_lossy().to_string())
}

#[tauri::command]
#[allow(unused_variables)]
pub async fn send_file_from_fd(
    app: tauri::AppHandle,
    state: State<'_, DbState>,
    peer_state: State<'_, PeerState>,
    #[allow(non_snake_case)] peerId: String,
    #[allow(non_snake_case)] peerAddr: String,
    #[allow(non_snake_case)] fileName: String,
    #[allow(non_snake_case)] fileSize: usize,
    fd: i32,
    #[allow(non_snake_case)] originalUri: Option<String>,
) -> Result<serde_json::Value, String> {
    println!(
        "[Command] 收到从 FD 发送文件请求: fd={}, name={}, size={}, uri={:?}, to={}",
        fd, fileName, fileSize, originalUri, peerAddr
    );

    #[cfg(target_os = "android")]
    {
        use crate::android_fd::AndroidFile;
        use std::os::unix::io::IntoRawFd;

        // ── 检查接收端是否离线 ──
        let is_offline_now = {
            let peers = peer_state.active_manager().get_all_peers();
            peers
                .iter()
                .find(|p| p.id == peerId)
                .map(|p| p.is_offline)
                .unwrap_or(true)
        };

        if is_offline_now {
            // JNI 已将原始 FD 的所有权交给 Rust；拒绝发送时也必须关闭。
            drop(AndroidFile::from_fd(fd)?);
            return Err("对方不在线".to_string());
        }

        // ── 1. 检查接收端的 auto_download 设置 ──
        let auto_enabled = {
            let auto_dl_url = format!("http://{}/api/auto_download", peerAddr);
            match crate::network::lan_http_client(None) {
                Ok(client) => match client.get(&auto_dl_url).send().await {
                    Ok(resp) => {
                        if let Ok(data) = resp.json::<serde_json::Value>().await {
                            data.get("enabled")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(true)
                        } else {
                            true
                        }
                    }
                    Err(_) => true,
                },
                Err(_) => true,
            }
        };

        if !peer_state.active_manager().get_all_peers().iter()
            .any(|peer| peer.id == peerId && !peer.is_offline)
        {
            drop(AndroidFile::from_fd(fd)?);
            return Err("对方不在线".to_string());
        }

        if !auto_enabled {
            // ── 自动下载关闭：先持久化，再发 file_offer ──
            let persisted_path = persist_android_fd(&app, fd, &fileName).await?;
            let sender_msg_id = crate::db::save_file_message(
                &state.pool,
                peerId.clone(),
                fileName.clone(),
                fileSize,
                persisted_path,
                "offering".to_string(),
                "sent".to_string(),
            )
            .await
            .map_err(|e| format!("创建发送记录失败: {}", e))?;

            let my_id = crate::db::get_user_id(&state.pool)
                .await
                .unwrap_or_default();
            let my_name = crate::db::get_username(&state.pool)
                .await
                .unwrap_or_default();

            // 通过 WS 向接收端发送 file_offer
            let offer = serde_json::json!({
                "msg_type": "file_offer",
                "from_id": my_id,
                "from_name": my_name,
                "file_name": fileName,
                "file_size": fileSize,
                "sender_msg_id": sender_msg_id,
            });
            let _ =
                crate::network::messaging::send_json_via_ws(&peerAddr, &offer.to_string()).await;

            return Ok(serde_json::json!({
                "success": true,
                "status": "offered",
                "msg_id": sender_msg_id,
                "file_name": fileName,
            }));
        }

        // ── 自动下载开启：先保存消息 + 缓存 FD，再上传 ──
        let peer_state = app.try_state::<PeerState>();
        let (is_online, backend_addr) = peer_state
            .as_ref()
            .map(|s| {
                if let Some(p) = s
                    .active_manager()
                    .get_all_peers()
                    .iter()
                    .find(|p| p.id == peerId)
                {
                    (!p.is_offline, Some(p.addr.clone()))
                } else {
                    (false, None)
                }
            })
            .unwrap_or((true, None));

        let peer_addr = if let Some(latest_addr) = backend_addr {
            if latest_addr != peerAddr {
                println!(
                    "[Command] 🛡️ 拦截到过期文件传输 IP，后端强行纠正: {} -> {}",
                    peerAddr, latest_addr
                );
                latest_addr
            } else {
                peerAddr
            }
        } else {
            peerAddr
        };

        if !is_online {
            return Err("对方不在线".to_string());
        }

        // 保存消息 + 缓存 FD，再上传
        let overall_status = "sent";
        let file_path = originalUri.clone().unwrap_or_else(|| format!("fd:{}", fd));
        let android_file = AndroidFile::from_fd(fd)?;
        let std_file = android_file.into_file();
        // try_clone/dup 与原 FD 共用读取位置。文件卡片的可读性检查会 seek(0)，
        // 因此上传必须从分享 URI 独立打开，不能从缓存 FD 克隆。
        let source_uri = originalUri
            .as_deref()
            .filter(|uri| uri.starts_with("content://"))
            .ok_or("分享文件缺少可独立打开的 content URI")?;
        let upload_file = AndroidFile::from_content_uri(source_uri)
            .map_err(|error| format!("无法独立打开分享文件: {error}"))?
            .into_file();
        if let Ok(metadata) = upload_file.metadata() {
            if metadata.len() > 0 && metadata.len() != fileSize as u64 {
                return Err(format!(
                    "分享文件大小不一致: 声明 {fileSize} 字节，实际 {} 字节",
                    metadata.len()
                ));
            }
        }
        let file = tokio::fs::File::from_std(upload_file);

        // 先存消息获取 msg_id
        let msg_id = crate::db::save_file_message(
            &state.pool,
            peerId.clone(),
            fileName.clone(),
            fileSize,
            file_path,
            "uploading".to_string(),
            overall_status.to_string(),
        )
        .await
        .map_err(|e| format!("保存消息失败: {}", e))?;

        // 缓存 FD（用 msg_id 作为 key，供媒体服务器 /api/media 读取）
        let raw_fd = std_file.into_raw_fd();
        crate::android_fd::cache_fd_for_msg(msg_id, raw_fd, fileName.clone(), fileSize as u64);

        // 修正 DB 路径为 fd:{msg_id}
        let corrected = format!("fd:{}", msg_id);
        if let Err(e) = crate::db::update_file_path_by_id(&state.pool, msg_id, &corrected).await {
            eprintln!("[Command] 修正 FD 路径失败: {}", e);
        }

        upload_file_internal(
            &app,
            &state,
            peer_state.as_ref(),
            peerId,
            peer_addr,
            fileName.clone(),
            fileSize,
            format!("fd:{}", msg_id),
            file,
            is_online,
            Some(msg_id),
        )
        .await
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = (
            app,
            state,
            peerId,
            peerAddr,
            fileName,
            fileSize,
            fd,
            originalUri,
        );
        Err("此功能仅在 Android 上可用".to_string())
    }
}

#[cfg(target_os = "android")]
fn normalize_android_external_file_path(file_path: &str) -> String {
    if let Some(msg_id) = file_path.strip_prefix("fd:") {
        let msg_id_i64: i64 = msg_id.parse().unwrap_or(0);
        let file_name = crate::android_fd::get_cached_file_name(msg_id_i64)
            .unwrap_or_else(|| "file".to_string());
        format!("content://com.lanchat.app.fdprovider/{msg_id}/{file_name}")
    } else {
        file_path.to_string()
    }
}

#[cfg(target_os = "android")]
fn call_android_file_method(method: &str, file_path: &str) -> Result<String, String> {
    use jni::objects::JValue;

    let context = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(context.vm().cast()) }
        .map_err(|e| format!("获取 JavaVM 失败: {}", e))?;

    let mut env = vm
        .attach_current_thread()
        .map_err(|e| format!("附加线程失败: {}", e))?;

    let activity = unsafe { jni::objects::JObject::from_raw(context.context().cast()) };

    let file_path_jstring = env
        .new_string(file_path)
        .map_err(|e| format!("创建字符串失败: {}", e))?;

    let result = env
        .call_method(
            activity,
            method,
            "(Ljava/lang/String;)Ljava/lang/String;",
            &[JValue::Object(&file_path_jstring)],
        )
        .map_err(|e| format!("调用 {method} 失败: {e}"))?
        .l()
        .map_err(|e| format!("读取 {method} 返回值失败: {e}"))?;
    let result = jni::objects::JString::from(result);
    let result = env
        .get_string(&result)
        .map_err(|e| format!("解析 {method} 返回值失败: {e}"))?
        .to_string_lossy()
        .into_owned();

    Ok(result)
}

#[cfg(target_os = "android")]
fn call_android_file_action(method: &str, file_path: &str) -> Result<(), String> {
    let result = call_android_file_method(method, file_path)?;
    if let Some(error) = result.strip_prefix("ERROR:") {
        Err(error.to_string())
    } else if result == "OK" {
        Ok(())
    } else {
        Err(format!("{method} 返回未知结果: {result}"))
    }
}

// 分享文件到其他应用（仅 Android）
#[cfg(target_os = "android")]
#[tauri::command]
pub async fn share_file_to_other_app(
    #[allow(non_snake_case)] filePath: String,
) -> Result<(), String> {
    let final_path = normalize_android_external_file_path(&filePath);
    println!("[Command] 准备分享文件到其他应用: {}", final_path);

    call_android_file_action("shareFile", &final_path)
}

// 非 Android 平台的空实现
#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn share_file_to_other_app(
    #[allow(non_snake_case)] filePath: String,
) -> Result<(), String> {
    let _ = filePath;
    Err("此功能仅在 Android 上可用".to_string())
}

// 用对应应用打开文件（仅 Android）
#[cfg(target_os = "android")]
#[tauri::command]
pub async fn open_file_in_android(#[allow(non_snake_case)] filePath: String) -> Result<(), String> {
    let final_path = normalize_android_external_file_path(&filePath);
    println!("[Command] 准备打开文件: {}", final_path);
    call_android_file_action("openFile", &final_path)
}

#[cfg(target_os = "android")]
#[tauri::command]
pub async fn get_android_file_state(
    #[allow(non_snake_case)] filePath: String,
) -> Result<String, String> {
    let final_path = normalize_android_external_file_path(&filePath);
    call_android_file_method("getFileState", &final_path)
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn get_android_file_state(
    #[allow(non_snake_case)] filePath: String,
) -> Result<String, String> {
    let _ = filePath;
    Ok("NOT_APPLICABLE".to_string())
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn open_file_in_android(#[allow(non_snake_case)] filePath: String) -> Result<(), String> {
    let _ = filePath;
    Err("此功能仅在 Android 上可用".to_string())
}

/// 获取媒体代理 Token（前端用于构造 /api/media 请求 URL）
#[tauri::command]
pub async fn get_media_token() -> String {
    crate::web_server::get_media_token()
}

// 读取剪贴板中的文件路径（桌面端）
#[cfg(feature = "desktop")]
#[tauri::command]
pub async fn read_clipboard_files() -> Result<Vec<String>, String> {
    println!("[Command] 读取剪贴板文件");

    #[cfg(all(not(target_os = "android"), feature = "clipboard-rs"))]
    {
        use clipboard_rs::common::RustImage;
        // 1. 优先尝试 Wayland (仅 Linux 桌面端)
        #[cfg(all(target_os = "linux", feature = "wl-clipboard-rs"))]
        {
            if let Ok(files) = try_read_wayland_clipboard().await {
                if !files.is_empty() {
                    println!("[Command] ✓ 通过 Wayland 读取到 {} 个文件", files.len());
                    return Ok(files);
                }
            }
        }

        // 2. Fallback: 使用 clipboard-rs
        println!("[Command] 尝试使用 clipboard-rs");
        use clipboard_rs::{Clipboard, ClipboardContext};

        let ctx = ClipboardContext::new().map_err(|e| format!("创建剪贴板上下文失败: {}", e))?;

        // 优先尝试读取本地文件路径
        if let Ok(files) = ctx.get_files() {
            if !files.is_empty() {
                println!(
                    "[Command] ✓ 通过 clipboard-rs 读取到 {} 个文件",
                    files.len()
                );
                return Ok(files);
            }
        }

        // 如果没有文件，尝试读取纯图片(截图)
        if let Ok(img) = ctx.get_image() {
            println!("[Command] 发现剪贴板图片，尝试生成临时文件...");

            let temp_dir = std::env::temp_dir();
            // 生成唯一文件名
            let file_name = format!(
                "screenshot_{}.png",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
            );
            let temp_path = temp_dir.join(file_name);
            let path_str = temp_path.to_string_lossy().to_string();

            // 借助 clipboard-rs 的 RustImageData 的 save_to_path 写入磁盘
            if img.save_to_path(&path_str).is_ok() {
                println!("[Command] ✓ 图片已保存至临时路径: {}", path_str);
                return Ok(vec![path_str]);
            }
        }

        Ok(vec![])
    }

    #[cfg(not(all(not(target_os = "android"), feature = "clipboard-rs")))]
    {
        Err("剪贴板功能不可用".to_string())
    }
}

// Wayland 剪贴板读取（仅 Linux 桌面端）
#[cfg(all(feature = "desktop", target_os = "linux", feature = "wl-clipboard-rs"))]
async fn try_read_wayland_clipboard() -> Result<Vec<String>, String> {
    use std::io::Read;
    use wl_clipboard_rs::paste::{get_contents, ClipboardType, MimeType, Seat};

    println!("[Command] 尝试通过 Wayland 读取剪贴板");
    // 1. 尝试读取 text/uri-list MIME 类型（文件列表）
    let uri_result = get_contents(
        ClipboardType::Regular,
        Seat::Unspecified,
        MimeType::Specific("text/uri-list"),
    );
    if let Ok((mut pipe, _)) = uri_result {
        let mut contents = String::new();
        if pipe.read_to_string(&mut contents).is_ok() {
            let files: Vec<String> = contents
                .lines()
                .filter(|line| !line.is_empty() && line.starts_with("file://"))
                .map(|line| line.trim_start_matches("file://").to_string()) // 去除 file:// 前缀
                .collect();
            if !files.is_empty() {
                return Ok(files);
            }
        }
    }

    // 2. 尝试读取 image/png 类型（截图）
    let img_result = get_contents(
        ClipboardType::Regular,
        Seat::Unspecified,
        MimeType::Specific("image/png"),
    );
    if let Ok((mut pipe, _)) = img_result {
        let mut buffer = Vec::new();
        if pipe.read_to_end(&mut buffer).is_ok() {
            let temp_dir = std::env::temp_dir();
            let file_name = format!(
                "screenshot_{}.png",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
            );
            let temp_path = temp_dir.join(file_name);

            if std::fs::write(&temp_path, &buffer).is_ok() {
                println!("[Command] Wayland: 成功读取截图并保存为临时文件");
                return Ok(vec![temp_path.to_string_lossy().to_string()]);
            }
        }
    }

    Err("剪贴板中既没有文件也没有图片".to_string())
}

// Web 端的空实现
#[cfg(not(feature = "desktop"))]
#[tauri::command]
pub async fn read_clipboard_files() -> Result<Vec<String>, String> {
    Err("此功能仅在桌面端可用".to_string())
}

// Web 端的空实现
#[cfg(not(feature = "desktop"))]
pub struct PeerState;

#[cfg(not(feature = "desktop"))]
pub struct AndroidShareState;

#[cfg(not(feature = "desktop"))]
impl AndroidShareState {
    pub fn new() -> Self {
        Self
    }
}

// 批量删除消息
#[tauri::command]
pub async fn delete_messages(
    #[allow(unused_variables)] app: tauri::AppHandle,
    state: tauri::State<'_, crate::db::DbState>,
    msg_ids: Vec<i64>,
) -> Result<(), String> {
    println!("[Command] 批量删除消息: {:?}", msg_ids);

    // ── 删除前清理关联的资源 ──
    #[cfg(target_os = "android")]
    {
        use crate::android_fd::AndroidFile;
        let outbox = app
            .path()
            .app_data_dir()
            .ok()
            .map(|path| path.join("outbox"));
        for &msg_id in &msg_ids {
            // 释放持久化 URI 权限
            if let Ok(Some(uri)) = crate::db::get_uri_by_msg_id(&state.pool, msg_id).await {
                let _ = AndroidFile::release_persistable_uri_permission(&uri);
                let _ = crate::db::remove_persisted_uri(&state.pool, &uri).await;
                println!("[Command] 释放 URI 权限: msg_id={}, uri={}", msg_id, uri);
            }
            // 清理 FD 缓存
            crate::android_fd::remove_cached_fd(msg_id);

            // 仅删除由 LANChat 自己复制到私有 outbox 的附件副本。
            if let (Some(outbox_dir), Ok((file_path, _, _))) = (
                outbox.as_ref(),
                crate::db::get_sender_file_by_msg_id(&state.pool, msg_id).await,
            ) {
                let candidate = std::path::PathBuf::from(file_path);
                if candidate.starts_with(outbox_dir) && candidate.is_file() {
                    let _ = tokio::fs::remove_file(candidate).await;
                }
            }
        }
    }

    crate::db::delete_messages_by_ids(&state.pool, msg_ids).await
}

#[tauri::command]
pub async fn clear_chat_history(
    state: tauri::State<'_, crate::db::DbState>,
    peer_id: String,
) -> Result<(), String> {
    let my_id = crate::db::get_user_id(&state.pool).await?;
    crate::db::clear_chat_history(&state.pool, &my_id, &peer_id).await
}

#[tauri::command]
pub async fn delete_user_complete(
    state: tauri::State<'_, crate::db::DbState>,
    peer_state: tauri::State<'_, PeerState>, // 必须引入这个来操作内存
    peer_id: String,
) -> Result<(), String> {
    let my_id = crate::db::get_user_id(&state.pool).await?;

    peer_state
        .active_manager()
        .delete_user_and_history(&state.pool, &my_id, &peer_id)
        .await
}

#[tauri::command]
pub async fn clear_offline_peers(
    state: tauri::State<'_, crate::db::DbState>,
    peer_state: tauri::State<'_, PeerState>,
) -> Result<Vec<String>, String> {
    peer_state.active_manager().clear_offline_peer_profiles(&state.pool).await
}

// ── 自定义 IP 命令 ──

#[tauri::command]
pub async fn get_custom_peers(
    state: tauri::State<'_, crate::db::DbState>,
) -> Result<Vec<String>, String> {
    Ok(crate::db::get_custom_peers(&state.pool).await)
}

#[tauri::command]
pub async fn add_custom_peer(
    state: tauri::State<'_, crate::db::DbState>,
    peer: String,
) -> Result<(), String> {
    crate::db::add_custom_peer(&state.pool, &peer).await
}

#[tauri::command]
pub async fn remove_custom_peer(
    state: tauri::State<'_, crate::db::DbState>,
    peer: String,
) -> Result<(), String> {
    crate::db::remove_custom_peer(&state.pool, &peer).await
}

#[tauri::command]
pub async fn request_file(
    state: tauri::State<'_, crate::db::DbState>,
    sender_addr: String,
    sender_msg_id: i64,
) -> Result<(), String> {
    println!(
        "[手动下载] 桌面端接收端请求文件: msg_id={}, 发送端地址={}",
        sender_msg_id, sender_addr
    );
    let my_id = crate::db::get_user_id(&state.pool).await?;
    let req_msg = serde_json::json!({
        "msg_type": "file_request",
        "sender_msg_id": sender_msg_id,
        "from_id": my_id,
    });
    crate::network::messaging::send_json_via_ws(&sender_addr, &req_msg.to_string()).await
}

#[tauri::command]
pub async fn get_notifications_enabled(
    state: tauri::State<'_, crate::db::DbState>,
) -> Result<bool, String> {
    Ok(crate::db::get_notifications_enabled(&state.pool).await)
}

#[tauri::command]
pub async fn set_notifications_enabled(
    state: tauri::State<'_, crate::db::DbState>,
    enabled: bool,
) -> Result<(), String> {
    crate::db::set_notifications_enabled(&state.pool, enabled).await
}

#[tauri::command]
pub async fn get_notification_sound_enabled(
    state: tauri::State<'_, crate::db::DbState>,
) -> Result<bool, String> {
    Ok(crate::db::get_notification_sound_enabled(&state.pool).await)
}

#[tauri::command]
pub async fn set_notification_sound_enabled(
    state: tauri::State<'_, crate::db::DbState>,
    enabled: bool,
) -> Result<(), String> {
    crate::db::set_notification_sound_enabled(&state.pool, enabled).await
}

#[tauri::command]
pub fn get_background_receive_state() -> serde_json::Value {
    let value = serde_json::to_value(crate::core_runtime::CoreRuntime::global().status())
        .unwrap_or_else(|_| serde_json::json!({"state": "ERROR"}));
    #[cfg(windows)]
    let value = {
        let mut value = value;
        if let Some((_, manager)) = crate::core_runtime::CoreRuntime::global().shared_resources() {
            if let Some(store) = manager.persistence() {
                value["peer_persistence"] = serde_json::json!(store.status());
            }
        }
        value
    };
    value
}

#[cfg(target_os = "android")]
fn call_android_activity_void(method: &str) -> Result<(), String> {
    let context = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(context.vm().cast()) }
        .map_err(|error| format!("获取 JavaVM 失败: {error}"))?;
    let mut env = vm
        .attach_current_thread()
        .map_err(|error| format!("附加线程失败: {error}"))?;
    let activity = unsafe { jni::objects::JObject::from_raw(context.context().cast()) };
    env.call_method(activity, method, "()V", &[])
        .map_err(|error| format!("调用 {method} 失败: {error}"))?;
    Ok(())
}

#[cfg(target_os = "android")]
fn call_android_activity_string(method: &str) -> Result<String, String> {
    let context = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(context.vm().cast()) }
        .map_err(|error| format!("获取 JavaVM 失败: {error}"))?;
    let mut env = vm
        .attach_current_thread()
        .map_err(|error| format!("附加线程失败: {error}"))?;
    let activity = unsafe { jni::objects::JObject::from_raw(context.context().cast()) };
    let value = env
        .call_method(activity, method, "()Ljava/lang/String;", &[])
        .map_err(|error| format!("调用 {method} 失败: {error}"))?
        .l()
        .map_err(|error| error.to_string())?;
    let value = jni::objects::JString::from(value);
    env.get_string(&value)
        .map(|value| value.to_string_lossy().into_owned())
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "android")]
fn call_android_activity_string_arg(method: &str, input: &str) -> Result<String, String> {
    let context = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(context.vm().cast()) }
        .map_err(|error| format!("获取 JavaVM 失败: {error}"))?;
    let mut env = vm
        .attach_current_thread()
        .map_err(|error| format!("附加线程失败: {error}"))?;
    let activity = unsafe { jni::objects::JObject::from_raw(context.context().cast()) };
    let arg =
        jni::objects::JObject::from(env.new_string(input).map_err(|error| error.to_string())?);
    let value = env
        .call_method(
            activity,
            method,
            "(Ljava/lang/String;)Ljava/lang/String;",
            &[jni::objects::JValue::Object(&arg)],
        )
        .map_err(|error| format!("调用 {method} 失败: {error}"))?
        .l()
        .map_err(|error| error.to_string())?;
    let value = jni::objects::JString::from(value);
    env.get_string(&value)
        .map(|value| value.to_string_lossy().into_owned())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_background_runtime_settings() -> Result<serde_json::Value, String> {
    #[cfg(target_os = "android")]
    return call_android_activity_string("getBackgroundRuntimeSettings")
        .and_then(|value| serde_json::from_str(&value).map_err(|error| error.to_string()));
    #[cfg(not(target_os = "android"))]
    Ok(serde_json::json!({}))
}

#[tauri::command]
pub fn set_background_runtime_settings(
    settings: serde_json::Value,
) -> Result<serde_json::Value, String> {
    #[cfg(target_os = "android")]
    return call_android_activity_string_arg("setBackgroundRuntimeSettings", &settings.to_string())
        .and_then(|value| serde_json::from_str(&value).map_err(|error| error.to_string()));
    #[cfg(not(target_os = "android"))]
    Ok(settings)
}

#[tauri::command]
pub fn retry_background_service() -> Result<(), String> {
    #[cfg(target_os = "android")]
    return call_android_activity_void("retryBackgroundService");
    #[cfg(not(target_os = "android"))]
    Err("仅 Android 支持后台接收服务".to_string())
}

#[tauri::command]
pub fn stop_background_receive_and_exit() -> Result<(), String> {
    #[cfg(target_os = "android")]
    return call_android_activity_void("stopBackgroundReceiveAndExit");
    #[cfg(not(target_os = "android"))]
    Err("仅 Android 支持后台接收服务".to_string())
}

#[tauri::command]
pub fn get_battery_optimization_state() -> Result<String, String> {
    #[cfg(target_os = "android")]
    return call_android_activity_string("getBatteryOptimizationState");
    #[cfg(not(target_os = "android"))]
    Ok("not_applicable".to_string())
}

#[tauri::command]
pub fn open_battery_optimization_settings() -> Result<(), String> {
    #[cfg(target_os = "android")]
    return call_android_activity_void("openBatteryOptimizationSettings");
    #[cfg(not(target_os = "android"))]
    Err("仅 Android 支持电池优化设置".to_string())
}

#[tauri::command]
pub fn get_notification_permission_state() -> Result<String, String> {
    #[cfg(target_os = "android")]
    return call_android_activity_string("getNotificationPermissionState");
    #[cfg(not(target_os = "android"))]
    Ok("not_applicable".to_string())
}

#[cfg(windows)]
const WINDOWS_AUTOSTART_RUN_KEY: &str = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Run";

#[cfg(windows)]
fn windows_autostart_command(executable: &std::path::Path) -> String {
    format!("\"{}\" --autostart", executable.display())
}

#[cfg(windows)]
fn ensure_windows_autostart_command(app_name: &str) -> Result<(), String> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};
    use winreg::RegKey;

    let executable =
        std::env::current_exe().map_err(|error| format!("无法取得 LQChat 程序路径: {error}"))?;
    let command = windows_autostart_command(&executable);
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(WINDOWS_AUTOSTART_RUN_KEY, KEY_SET_VALUE)
        .and_then(|key| key.set_value(app_name, &command))
        .map_err(|error| format!("写入 Windows 开机启动项失败: {error}"))
}

#[tauri::command]
pub fn get_autostart_enabled(app: tauri::AppHandle) -> Result<bool, String> {
    #[cfg(windows)]
    {
        use tauri_plugin_autostart::ManagerExt;
        let enabled = app.autolaunch().is_enabled().map_err(|e| e.to_string())?;
        if enabled {
            // auto-launch 0.5 writes an unquoted executable path. Repair existing entries so
            // portable builds also start correctly from directories such as "Program Files".
            ensure_windows_autostart_command(&app.package_info().name)?;
        }
        return Ok(enabled);
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        Ok(false)
    }
}

#[tauri::command]
pub fn set_autostart_enabled(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        use tauri_plugin_autostart::ManagerExt;
        let manager = app.autolaunch();
        if enabled {
            manager.enable().map_err(|e| e.to_string())?;
            ensure_windows_autostart_command(&app.package_info().name)
        } else {
            manager.disable().map_err(|e| e.to_string())
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (app, enabled);
        Err("Autostart is only available on Windows".to_string())
    }
}

#[cfg(all(test, windows))]
mod autostart_tests {
    use super::windows_autostart_command;
    use std::path::Path;

    #[test]
    fn windows_autostart_quotes_portable_paths_and_starts_hidden() {
        assert_eq!(
            windows_autostart_command(Path::new(r"D:\Program Files\LQChat\LQChat.exe")),
            r#""D:\Program Files\LQChat\LQChat.exe" --autostart"#,
        );
    }
}

#[tauri::command]
pub fn request_permission_on_android(app: tauri::AppHandle) -> Result<String, String> {
    #[cfg(target_os = "android")]
    {
        use tauri_plugin_notification::NotificationExt;
        let result = app
            .notification()
            .request_permission()
            .map_err(|e| e.to_string())?;
        return Ok(format!("{:?}", result));
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Ok("granted".to_string())
    }
}

/// 将 from_id 字符串哈希为固定的正数 i32（用作通知 ID）
#[allow(dead_code)]
fn get_notification_id(from_id: &str) -> i32 {
    if from_id.is_empty() {
        return 0;
    }
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    from_id.hash(&mut hasher);
    (hasher.finish() & 0x7FFFFFFF) as i32
}

#[cfg(windows)]
const WINDOWS_NOTIFICATION_APP_ID: &str = "com.lanchat.app";

/// 便携版 EXE 没有安装器创建的 AppUserModelId。Windows Toast 使用自定义
/// AppUserModelId 前必须先注册显示名称，否则通知可能既不弹出也不进入通知中心。
#[cfg(windows)]
pub fn ensure_windows_notification_identity() -> Result<(), String> {
    use std::path::PathBuf;
    use windows::core::{Interface, GUID, HSTRING};
    use windows::Win32::Foundation::{PROPERTYKEY, RPC_E_CHANGED_MODE};
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
    use windows::Win32::UI::Shell::{
        IShellLinkW, SetCurrentProcessExplicitAppUserModelID, ShellLink,
    };
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    const PKEY_APP_USER_MODEL_ID: PROPERTYKEY = PROPERTYKEY {
        fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
        pid: 5,
    };

    let current_user = RegKey::predef(HKEY_CURRENT_USER);
    let path = format!(
        r"Software\Classes\AppUserModelId\{}",
        WINDOWS_NOTIFICATION_APP_ID
    );
    let (key, _) = current_user
        .create_subkey(path)
        .map_err(|error| format!("创建 Windows 通知身份失败: {error}"))?;
    key.set_value("DisplayName", &"LQChat")
        .map_err(|error| format!("写入 Windows 通知显示名称失败: {error}"))?;
    key.set_value("IconBackgroundColor", &"0")
        .map_err(|error| format!("写入 Windows 通知图标背景失败: {error}"))?;

    let app_id = HSTRING::from(WINDOWS_NOTIFICATION_APP_ID);
    unsafe { SetCurrentProcessExplicitAppUserModelID(&app_id) }
        .map_err(|error| format!("设置 Windows 进程通知身份失败: {error}"))?;

    let executable =
        std::env::current_exe().map_err(|error| format!("读取当前 LQChat 路径失败: {error}"))?;
    let working_directory = executable
        .parent()
        .map(PathBuf::from)
        .ok_or_else(|| "LQChat 路径缺少父目录".to_string())?;
    let shortcut_icon = working_directory.join("LQChat.ico");
    let launch_target = &executable;
    let icon_target = if shortcut_icon.is_file() {
        &shortcut_icon
    } else {
        &executable
    };
    let app_data = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| "Windows APPDATA 目录不可用".to_string())?;
    let shortcut_dir = app_data
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs");
    let start_menu_shortcut = shortcut_dir.join("LQChat.lnk");
    let mut shortcuts = vec![start_menu_shortcut.clone()];
    // 只有打包资源存在时才接管桌面快捷方式；开发构建不能覆盖用户桌面入口。
    if shortcut_icon.is_file() {
        if let Some(desktop_dir) = dirs::desktop_dir() {
            shortcuts.push(desktop_dir.join("LQChat.lnk"));
        }
    }
    for shortcut in &shortcuts {
        if let Some(parent) = shortcut.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("创建快捷方式目录失败: {error}"))?;
        }
    }

    let com_result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let should_uninitialize = com_result.is_ok();
    if com_result.is_err() && com_result != RPC_E_CHANGED_MODE {
        return Err(format!("初始化 Windows Shell 失败: {com_result:?}"));
    }

    let create_shortcut = |shortcut: &std::path::Path| -> windows::core::Result<()> {
        unsafe {
            let shell_link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            shell_link.SetPath(&HSTRING::from(launch_target.to_string_lossy().as_ref()))?;
            shell_link.SetWorkingDirectory(&HSTRING::from(
                working_directory.to_string_lossy().as_ref(),
            ))?;
            shell_link.SetDescription(&HSTRING::from("LQChat 局域网聊天"))?;
            shell_link
                .SetIconLocation(&HSTRING::from(icon_target.to_string_lossy().as_ref()), 0)?;
            shell_link.SetShowCmd(windows::Win32::UI::WindowsAndMessaging::SW_SHOWMINNOACTIVE)?;

            let property_store: IPropertyStore = shell_link.cast()?;
            let app_id_value = PROPVARIANT::from(WINDOWS_NOTIFICATION_APP_ID);
            property_store.SetValue(&PKEY_APP_USER_MODEL_ID, &app_id_value)?;
            property_store.Commit()?;

            let persist_file: IPersistFile = shell_link.cast()?;
            persist_file.Save(&HSTRING::from(shortcut.to_string_lossy().as_ref()), true)?;
            Ok(())
        }
    };

    let create_result = shortcuts
        .iter()
        .try_for_each(|shortcut| create_shortcut(shortcut));

    if should_uninitialize {
        unsafe { CoUninitialize() };
    }
    create_result.map_err(|error| format!("创建 LQChat 快捷方式失败: {error}"))?;
    let legacy_shortcut = shortcut_dir.join("LANChat.lnk");
    if legacy_shortcut != start_menu_shortcut && legacy_shortcut.is_file() {
        let _ = std::fs::remove_file(legacy_shortcut);
    }
    Ok(())
}

#[cfg(windows)]
pub fn show_windows_system_notification(title: &str, body: &str) -> Result<(), String> {
    show_windows_system_notification_with_sound(title, body, true)
}

#[cfg(windows)]
pub fn show_windows_system_notification_silent(title: &str, body: &str) -> Result<(), String> {
    show_windows_system_notification_with_sound(title, body, false)
}

#[cfg(windows)]
fn show_windows_system_notification_with_sound(
    title: &str,
    body: &str,
    use_system_sound: bool,
) -> Result<(), String> {
    ensure_windows_notification_identity()?;
    use tauri_winrt_notification::{Duration, Sound, Toast};
    let toast = Toast::new(WINDOWS_NOTIFICATION_APP_ID)
        .title(title)
        .text1(body)
        .duration(Duration::Short);
    let toast = if use_system_sound {
        toast.sound(Some(Sound::Default))
    } else {
        toast.sound(None)
    };
    toast
        .show()
        .map_err(|error| format!("Windows 系统通知发送失败: {error}"))
}

#[cfg(windows)]
const WINDOWS_NOTIFICATION_SOUND: &[u8] = include_bytes!("../assets/lqchat_notification.wav");

#[cfg(windows)]
static LAST_WINDOWS_NOTIFICATION_SOUND: std::sync::OnceLock<
    std::sync::Mutex<Option<std::time::Instant>>,
> = std::sync::OnceLock::new();

#[cfg(windows)]
pub fn windows_notification_sound_for_core_event(event: &crate::core_events::CoreEvent) -> bool {
    use crate::core_events::CoreEvent;
    let payload = match event {
        CoreEvent::MessageReceived(payload)
        | CoreEvent::FileOfferReceived(payload)
        | CoreEvent::FileTransferCompleted(payload) => payload,
        _ => return false,
    };
    payload
        .get("from_id")
        .and_then(|value| value.as_str())
        .is_some_and(|from_id| !from_id.is_empty() && from_id != "me")
}

/// Plays the embedded message/file chime at most once in an 800 ms burst.
#[cfg(windows)]
pub fn play_windows_notification_sound() -> Result<bool, String> {
    use windows::core::PCWSTR;
    use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};

    let now = std::time::Instant::now();
    let mut last = LAST_WINDOWS_NOTIFICATION_SOUND
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .map_err(|_| "通知音效状态锁已损坏".to_string())?;
    if last.is_some_and(|played| now.duration_since(played) < std::time::Duration::from_millis(800))
    {
        return Ok(false);
    }
    *last = Some(now);

    // PlaySound treats pszSound as a WAVE memory image when SND_MEMORY is set.
    // The bytes are static, so they remain valid for asynchronous playback.
    let played = unsafe {
        PlaySoundW(
            PCWSTR(WINDOWS_NOTIFICATION_SOUND.as_ptr().cast()),
            None,
            SND_ASYNC | SND_MEMORY | SND_NODEFAULT,
        )
    }
    .as_bool();
    if played {
        Ok(true)
    } else {
        *last = None;
        Err("Windows 通知音效播放失败".to_string())
    }
}

#[cfg(windows)]
pub fn windows_notification_for_core_event(
    event: &crate::core_events::CoreEvent,
) -> Option<(String, String)> {
    use crate::core_events::CoreEvent;

    let (payload, kind) = match event {
        CoreEvent::MessageReceived(payload) => (payload, "message"),
        CoreEvent::FileOfferReceived(payload) => (payload, "file_offer"),
        CoreEvent::FileTransferCompleted(payload) => (payload, "file_completed"),
        CoreEvent::CoreError(status) => {
            return Some((
                "LQChat 后台接收发生异常".to_string(),
                status
                    .last_error_message
                    .clone()
                    .unwrap_or_else(|| "点击打开并重试".to_string()),
            ));
        }
        _ => return None,
    };

    let from_id = payload.get("from_id").and_then(|value| value.as_str())?;
    if from_id.is_empty() || from_id == "me" {
        return None;
    }
    let title = payload
        .get("from_name")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("局域网设备")
        .to_string();
    let file_name = payload
        .get("file_name")
        .or_else(|| payload.get("content"))
        .and_then(|value| value.as_str())
        .unwrap_or("未知文件");
    let body = match kind {
        "file_offer" => format!("请求发送文件：{file_name}"),
        "file_completed" => format!("文件接收完成：{file_name}"),
        _ if payload.get("msg_type").and_then(|value| value.as_str()) == Some("image") => {
            "[图片]".to_string()
        }
        _ => payload
            .get("content")
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("收到一条新消息")
            .chars()
            .take(100)
            .collect(),
    };
    Some((title, body))
}

#[tauri::command]
pub fn show_notification(
    _app: tauri::AppHandle,
    title: String,
    body: String,
    #[allow(unused_variables)] from_id: String,
) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        // Linux 用 notify-send
        let _ = std::process::Command::new("notify-send")
            .arg(&title)
            .arg(&body)
            .spawn();
        return Ok(());
    }

    #[cfg(windows)]
    {
        show_windows_system_notification(&title, &body)?;
        return Ok(());
    }

    #[cfg(any(target_os = "macos", target_os = "android"))]
    {
        // macOS / Android 使用 Tauri 的系统原生通知组件。
        use tauri_plugin_notification::NotificationExt;
        let app = _app;
        let mut builder = app.notification().builder().title(&title).body(&body);
        if !from_id.is_empty() {
            let notif_id = get_notification_id(&from_id);
            builder = builder.id(notif_id);
        }
        builder.show().map_err(|error| error.to_string())?;
        return Ok(());
    }

    #[allow(unreachable_code)]
    Ok(())
}

#[cfg(all(test, windows))]
mod notification_tests {
    use super::{
        windows_notification_for_core_event, windows_notification_sound_for_core_event,
        WINDOWS_NOTIFICATION_SOUND,
    };
    use crate::core_events::CoreEvent;

    #[test]
    fn windows_notification_maps_incoming_text_and_ignores_progress() {
        let incoming = CoreEvent::MessageReceived(serde_json::json!({
            "from_id": "phone-id",
            "from_name": "IQOO",
            "content": "测试消息",
            "msg_type": "text"
        }));
        assert_eq!(
            windows_notification_for_core_event(&incoming),
            Some(("IQOO".to_string(), "测试消息".to_string()))
        );

        let progress = CoreEvent::FileTransferProgress(serde_json::json!({
            "from_id": "phone-id",
            "file_name": "demo.apk"
        }));
        assert_eq!(windows_notification_for_core_event(&progress), None);
        assert!(windows_notification_sound_for_core_event(&incoming));
        assert!(!windows_notification_sound_for_core_event(&progress));

        let outgoing = CoreEvent::FileTransferCompleted(serde_json::json!({
            "from_id": "me",
            "file_name": "demo.apk"
        }));
        assert!(!windows_notification_sound_for_core_event(&outgoing));
        assert_eq!(&WINDOWS_NOTIFICATION_SOUND[..4], b"RIFF");
        assert_eq!(&WINDOWS_NOTIFICATION_SOUND[8..12], b"WAVE");
    }
}

/// 清除某个用户的所有通知（按 from_id）
#[tauri::command]
pub fn clear_notification(_app: tauri::AppHandle, #[allow(unused_variables)] from_id: String) {
    #[cfg(any(target_os = "macos", target_os = "android"))]
    {
        if from_id.is_empty() {
            return;
        }
        use tauri_plugin_notification::NotificationExt;
        let notif_id = get_notification_id(&from_id);
        let _ = _app.notification().cancel(vec![notif_id]);
    }
    // Linux 的 notify-send 没有统一的按会话撤销接口。
}

// ═══════════════════════════════════════════════════════════════
// 托盘图标闪烁（桌面端 real，Android 空实现）
// ═══════════════════════════════════════════════════════════════

#[cfg(not(target_os = "android"))]
const ICON_NORMAL: &[u8] = include_bytes!("../icons/32x32.png");

/// 保留原图标的透明轮廓，仅降低可见像素亮度，供托盘闪烁使用。
#[cfg(not(target_os = "android"))]
fn dim_tray_icon_rgba(rgba: &[u8]) -> Vec<u8> {
    let mut dimmed = rgba.to_vec();
    for pixel in dimmed.chunks_exact_mut(4) {
        pixel[0] = ((pixel[0] as u16 * 45) / 100) as u8;
        pixel[1] = ((pixel[1] as u16 * 45) / 100) as u8;
        pixel[2] = ((pixel[2] as u16 * 45) / 100) as u8;
        // Alpha 保持不变，避免 Windows 将透明区域渲染成黑色方框。
    }
    dimmed
}

/// 开始托盘闪烁
#[tauri::command]
pub fn start_tray_flash(
    #[allow(unused_variables)] app: AppHandle,
    #[allow(unused_variables)] state: State<'_, TrayFlashState>,
) {
    #[cfg(not(target_os = "android"))]
    {
        use tauri::image::Image;

        println!("[TrayFlash] start_tray_flash 被调用");
        if state.is_flashing.load(Ordering::Relaxed) {
            println!("[TrayFlash] 已在闪烁中，跳过");
            return;
        }
        state.is_flashing.store(true, Ordering::Relaxed);
        println!("[TrayFlash] 开始闪烁");

        let flashing = state.is_flashing.clone();
        let app = app.clone();

        std::thread::spawn(move || {
            let normal_img = match Image::from_bytes(ICON_NORMAL) {
                Ok(img) => img,
                Err(e) => {
                    eprintln!("[TrayFlash] 无法加载正常图标: {}", e);
                    return;
                }
            };
            let dimmed_img = Image::new_owned(
                dim_tray_icon_rgba(normal_img.rgba()),
                normal_img.width(),
                normal_img.height(),
            );

            let mut toggle = false;
            while flashing.load(Ordering::Relaxed) {
                if let Some(tray) = app.tray_by_id("main") {
                    let icon = if toggle { &normal_img } else { &dimmed_img };
                    let _ = tray.set_icon(Some(icon.clone() as tauri::image::Image));
                } else {
                    eprintln!("[TrayFlash] 找不到托盘 'main'");
                }
                toggle = !toggle;
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            // 停止闪烁，恢复为正常图标
            println!("[TrayFlash] 停止闪烁，恢复图标");
            if let Some(tray) = app.tray_by_id("main") {
                let _ = tray.set_icon(Some(normal_img as tauri::image::Image));
            }
        });
    }
    #[cfg(target_os = "android")]
    println!("[TrayFlash] Android 无系统托盘，忽略 start_tray_flash");
}

/// 停止托盘闪烁
#[tauri::command]
pub fn stop_tray_flash(#[allow(unused_variables)] state: State<'_, TrayFlashState>) {
    #[cfg(not(target_os = "android"))]
    {
        println!("[TrayFlash] stop_tray_flash 被调用");
        state.is_flashing.store(false, Ordering::Relaxed);
    }
    #[cfg(target_os = "android")]
    println!("[TrayFlash] Android 无系统托盘，忽略 stop_tray_flash");
}

#[cfg(all(test, not(target_os = "android")))]
mod tray_icon_tests {
    use super::dim_tray_icon_rgba;

    #[test]
    fn dimmed_tray_icon_keeps_alpha_and_scales_rgb() {
        let rgba = [200, 100, 50, 128, 0, 0, 0, 0];

        assert_eq!(dim_tray_icon_rgba(&rgba), [90, 45, 22, 128, 0, 0, 0, 0]);
    }
}
