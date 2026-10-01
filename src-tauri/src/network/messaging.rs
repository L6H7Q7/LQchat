// 消息发送和接收模块
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use crate::core_events::{CoreEvent, CoreEventBus};

/// One request to the existing endpoint. The caller owns the total deadline.
pub async fn send_notification_once(
    addr: &str,
    notification: &crate::notification_sync::Notification,
) -> Result<(), String> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .map_err(|_| "connect_failed")?;
    socket
        .send(Message::Text(
            serde_json::to_string(notification).map_err(|_| "invalid")?,
        ))
        .await
        .map_err(|_| "send_failed")?;
    while let Some(frame) = socket.next().await {
        match frame.map_err(|_| "receive_failed")? {
            Message::Text(text) if text.len() <= crate::notification_sync::MAX_FRAME => {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                    if value["msg_type"] == "notification_result"
                        && value["event_id"] == notification.event_id
                        && value["target_device_id"] == notification.target_device_id
                    {
                        return if value["status"] == "success" {
                            Ok(())
                        } else {
                            Err("rejected".into())
                        };
                    }
                }
            }
            Message::Ping(payload) => {
                socket
                    .send(Message::Pong(payload))
                    .await
                    .map_err(|_| "send_failed")?;
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    Err("disconnected".into())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextMessage {
    pub msg_type: String,  // "text"
    pub from_id: String,   // 发送者 UUID
    pub from_name: String, // 发送者名字
    pub content: String,   // 消息内容
    pub timestamp: u64,    // Unix 时间戳
}

// 握手协议消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandshakeMessage {
    pub protocol: String, // "handshake"
    pub action: String,   // "ready_to_receive" 或 "ack"
    pub from_id: String,  // 发送者 UUID
}

// 发送握手消息（询问对方是否准备好接收）
pub async fn send_handshake(peer_addr: &str, from_id: String, action: &str) -> Result<(), String> {
    let handshake = HandshakeMessage {
        protocol: "handshake".to_string(),
        action: action.to_string(),
        from_id,
    };

    let json = serde_json::to_string(&handshake).map_err(|e| format!("序列化失败: {}", e))?;

    // 尝试通过 WebSocket 发送
    let ws_url = format!("ws://{}/ws", peer_addr);

    match tokio_tungstenite::connect_async(&ws_url).await {
        Ok((mut ws_stream, _)) => {
            use futures_util::SinkExt;
            use tokio_tungstenite::tungstenite::protocol::Message as WsMessage;

            ws_stream
                .send(WsMessage::Text(json))
                .await
                .map_err(|e| format!("发送握手失败: {}", e))?;

            let _ = ws_stream.close(None).await;
            Ok(())
        }
        Err(e) => {
            eprintln!("[Messaging] 握手 WebSocket 连接失败: {}, 尝试 TCP", e);
            send_handshake_via_tcp(peer_addr, handshake).await
        }
    }
}

// 通过 TCP 发送握手消息
async fn send_handshake_via_tcp(
    peer_addr: &str,
    handshake: HandshakeMessage,
) -> Result<(), String> {
    use tokio::net::TcpStream;

    let mut stream = TcpStream::connect(peer_addr)
        .await
        .map_err(|e| format!("TCP 连接失败: {}", e))?;

    let json = serde_json::to_string(&handshake).map_err(|e| format!("序列化失败: {}", e))?;

    let len = json.len() as u32;
    stream
        .write_all(&len.to_be_bytes())
        .await
        .map_err(|e| format!("发送长度失败: {}", e))?;

    stream
        .write_all(json.as_bytes())
        .await
        .map_err(|e| format!("发送握手失败: {}", e))?;

    Ok(())
}

// 发送文本消息
pub async fn send_text_message(
    peer_addr: &str,
    from_id: String,
    from_name: String,
    content: String,
) -> Result<(), String> {
    println!("[Messaging] 正在连接到 {}...", peer_addr);

    // 构造消息
    let message = TextMessage {
        msg_type: "text".to_string(),
        from_id,
        from_name,
        content,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    };

    // 序列化为 JSON
    let json = serde_json::to_string(&message).map_err(|e| format!("序列化失败: {}", e))?;

    // 尝试通过 WebSocket 发送
    let ws_url = format!("ws://{}/ws", peer_addr);

    match tokio_tungstenite::connect_async(&ws_url).await {
        Ok((mut ws_stream, _)) => {
            println!("[Messaging] WebSocket 连接成功");

            use futures_util::SinkExt;
            use tokio_tungstenite::tungstenite::protocol::Message as WsMessage;

            ws_stream
                .send(WsMessage::Text(json))
                .await
                .map_err(|e| format!("发送失败: {}", e))?;

            // 优雅地关闭连接
            let _ = ws_stream.close(None).await;

            println!("[Messaging] 消息发送成功");
            Ok(())
        }
        Err(e) => {
            eprintln!("[Messaging] WebSocket 连接失败: {}, 尝试 TCP", e);
            // 回退到 TCP
            send_via_tcp(peer_addr, message).await
        }
    }
}

// 通过 TCP 发送(回退方案)
async fn send_via_tcp(peer_addr: &str, message: TextMessage) -> Result<(), String> {
    use tokio::net::TcpStream;

    let mut stream = TcpStream::connect(peer_addr)
        .await
        .map_err(|e| format!("TCP 连接失败: {}", e))?;

    let json = serde_json::to_string(&message).map_err(|e| format!("序列化失败: {}", e))?;

    // 发送消息长度(4字节)
    let len = json.len() as u32;
    stream
        .write_all(&len.to_be_bytes())
        .await
        .map_err(|e| format!("发送长度失败: {}", e))?;

    // 发送消息内容
    stream
        .write_all(json.as_bytes())
        .await
        .map_err(|e| format!("发送消息失败: {}", e))?;

    println!("[Messaging] TCP 消息发送成功");
    Ok(())
}

/// 发送任意 JSON 负载到对方的 WebSocket（用于 file_offer / file_request 等自定义协议）
pub async fn send_json_via_ws(peer_addr: &str, json: &str) -> Result<(), String> {
    let ws_url = format!("ws://{}/ws", peer_addr);
    match tokio_tungstenite::connect_async(&ws_url).await {
        Ok((mut ws_stream, _)) => {
            use futures_util::SinkExt;
            use tokio_tungstenite::tungstenite::protocol::Message as WsMessage;
            ws_stream
                .send(WsMessage::Text(json.to_string().into()))
                .await
                .map_err(|e| format!("发送 WS 消息失败: {}", e))?;
            let _ = ws_stream.close(None).await;
            Ok(())
        }
        Err(e) => {
            eprintln!("[Messaging] 发送 JSON via WS 失败: {}", e);
            Err(format!("WS 连接失败: {}", e))
        }
    }
}

// 启动消息接收服务器
pub async fn start_message_server(
    port: u16,
    db_pool: sqlx::Pool<sqlx::Sqlite>,
    #[allow(unused_variables)] event_bus: CoreEventBus,
    cancellation: CancellationToken,
) -> Result<(), String> {
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port))
        .await
        .map_err(|error| format!("无法绑定消息服务器端口: {error}"))?;

    println!("[Messaging] 消息服务器启动在端口 {}", port);

    loop {
        let accepted = tokio::select! {
            _ = cancellation.cancelled() => return Ok(()),
            accepted = listener.accept() => accepted,
        };
        match accepted {
            Ok((stream, addr)) => {
                println!("[Messaging] 收到来自 {} 的连接", addr);

                let pool = db_pool.clone();
                #[cfg(feature = "desktop")]
                let connection_bus = event_bus.clone();

                tokio::spawn(async move {
                    #[cfg(feature = "desktop")]
                    let result = handle_message_connection(stream, pool, connection_bus).await;

                    #[cfg(feature = "web")]
                    let result = handle_message_connection(stream, pool).await;

                    if let Err(e) = result {
                        eprintln!("[Messaging] 处理消息失败: {}", e);
                    }
                });
            }
            Err(e) => {
                eprintln!("[Messaging] 接受连接失败: {}", e);
            }
        }
    }
}

// 处理单个消息连接 - 桌面端版本
#[cfg(all(feature = "desktop", not(feature = "web")))]
async fn handle_message_connection(
    mut stream: tokio::net::TcpStream,
    db_pool: sqlx::Pool<sqlx::Sqlite>,
    event_bus: CoreEventBus,
) -> Result<(), String> {
    // 读取消息长度(4字节)
    let mut len_bytes = [0u8; 4];
    stream
        .read_exact(&mut len_bytes)
        .await
        .map_err(|e| format!("读取长度失败: {}", e))?;

    let len = u32::from_be_bytes(len_bytes) as usize;

    if len > 1024 * 1024 {
        return Err("消息过大".to_string());
    }

    // 读取消息内容
    let mut buffer = vec![0u8; len];
    stream
        .read_exact(&mut buffer)
        .await
        .map_err(|e| format!("读取消息失败: {}", e))?;

    // 解析 JSON
    let json_str = String::from_utf8(buffer).map_err(|e| format!("UTF-8 解析失败: {}", e))?;

    // 首先尝试解析为握手消息
    if let Ok(handshake) = serde_json::from_str::<HandshakeMessage>(&json_str) {
        println!("[Messaging] 收到握手消息: action={}", handshake.action);

        if handshake.action == "ready_to_receive" {
            // 对方询问我们是否准备好接收，发送 ack 确认
            let my_id = crate::db::get_user_id(&db_pool).await.unwrap_or_default();
            let ack = HandshakeMessage {
                protocol: "handshake".to_string(),
                action: "ack".to_string(),
                from_id: my_id,
            };

            if let Ok(ack_json) = serde_json::to_string(&ack) {
                let len = ack_json.len() as u32;
                let _ = stream.write_all(&len.to_be_bytes()).await;
                let _ = stream.write_all(ack_json.as_bytes()).await;
                println!("[Messaging] 已发送握手 ack");
            }
        }
        return Ok(());
    }

    // 否则尝试解析为文本消息
    let message: TextMessage =
        serde_json::from_str(&json_str).map_err(|e| format!("JSON 解析失败: {}", e))?;

    println!(
        "[Messaging] 收到消息: {} 说: {}",
        message.from_name, message.content
    );

    // 保存到数据库，并接住返回的 msg_id
    let msg_id = crate::db::save_network_message(
        &db_pool,
        &message.from_id,
        &message.content,
        &message.msg_type,
        message.timestamp,
    )
    .await?;

    // 数据库提交成功后才能发布接收事件。
    event_bus.publish_message(serde_json::json!({
        "id": msg_id,
        "msg_type": message.msg_type,
        "from_id": message.from_id,
        "from_name": message.from_name,
        "content": message.content,
        "timestamp": message.timestamp,
    }));

    Ok(())
}

// Web 端的消息处理 - 使用 CoreEventBus，不依赖 UI 生命周期
#[cfg(all(feature = "web", not(feature = "desktop")))]
async fn handle_message_connection(
    mut stream: tokio::net::TcpStream,
    db_pool: sqlx::Pool<sqlx::Sqlite>,
) -> Result<(), String> {
    // 读取消息长度(4字节)
    let mut len_bytes = [0u8; 4];
    stream
        .read_exact(&mut len_bytes)
        .await
        .map_err(|e| format!("读取长度失败: {}", e))?;

    let len = u32::from_be_bytes(len_bytes) as usize;

    if len > 1024 * 1024 {
        return Err("消息过大".to_string());
    }

    // 读取消息内容
    let mut buffer = vec![0u8; len];
    stream
        .read_exact(&mut buffer)
        .await
        .map_err(|e| format!("读取消息失败: {}", e))?;

    // 解析 JSON
    let json_str = String::from_utf8(buffer).map_err(|e| format!("UTF-8 解析失败: {}", e))?;

    // 首先尝试解析为握手消息
    if let Ok(handshake) = serde_json::from_str::<HandshakeMessage>(&json_str) {
        println!("[Messaging] 收到握手消息: action={}", handshake.action);

        if handshake.action == "ready_to_receive" {
            // 对方询问我们是否准备好接收，发送 ack 确认
            let my_id = crate::db::get_user_id(&db_pool).await.unwrap_or_default();
            let ack = HandshakeMessage {
                protocol: "handshake".to_string(),
                action: "ack".to_string(),
                from_id: my_id,
            };

            if let Ok(ack_json) = serde_json::to_string(&ack) {
                let len = ack_json.len() as u32;
                let _ = stream.write_all(&len.to_be_bytes()).await;
                let _ = stream.write_all(ack_json.as_bytes()).await;
                println!("[Messaging] 已发送握手 ack");
            }
        }
        return Ok(());
    }

    // 否则尝试解析为文本消息
    let message: TextMessage =
        serde_json::from_str(&json_str).map_err(|e| format!("JSON 解析失败: {}", e))?;

    println!(
        "[Messaging] 收到消息: {} 说: {}",
        message.from_name, message.content
    );

    // 保存到数据库
    let _msg_id = crate::db::save_network_message(
        &db_pool,
        &message.from_id,
        &message.content,
        &message.msg_type,
        message.timestamp,
    )
    .await?;

    // Web 端暂时只保存,不通知前端(前端会轮询)

    Ok(())
}

// 查询聊天历史（支持分页）
pub async fn get_chat_history(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    peer_id: &str,
    limit: i32,
) -> Result<Vec<serde_json::Value>, String> {
    get_chat_history_with_offset(pool, peer_id, limit, 0).await
}

// 查询聊天历史（带偏移量，用于懒加载）
pub async fn get_chat_history_with_offset(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    peer_id: &str,
    limit: i32,
    offset: i32,
) -> Result<Vec<serde_json::Value>, String> {
    // 底层 SQL 交给 db.rs 处理，这里只负责序列化 JSON
    let messages = crate::db::get_chat_history_with_offset(pool, peer_id, limit, offset).await?;

    // 转换为 MessageResponse 并序列化为 JSON
    let responses: Vec<serde_json::Value> = messages
        .into_iter()
        .map(|msg| {
            let response = crate::models::MessageResponse::from(msg);
            serde_json::to_value(response).unwrap_or(serde_json::json!({}))
        })
        .collect();

    Ok(responses)
}

// ======================================
// 补发挂起的文件（后台静默文件传输）
// ======================================
pub const FILE_TRANSFER_CHUNK_SIZE: usize = 10 * 1024 * 1024;

async fn resend_file_background(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    my_id: &str,
    peer_addr: &str,
    file_name: &str,
    file_path: &str,
    file_size: i64,
    msg_id: i64,
    event_bus: &CoreEventBus,
) -> Result<(), String> {
    println!("[Messaging] 正在后台补发文件: {}", file_name);

    // 区分 Android content URI 和普通文件路径
    #[cfg(target_os = "android")]
    let mut file = if file_path.starts_with("content://") {
        println!("[Messaging] 检测到 content URI，调用 Android 专用 FD 获取机制");
        let android_file = crate::android_fd::AndroidFile::from_content_uri(file_path)
            .map_err(|e| format!("无法从 Content URI 获取文件: {}", e))?;
        tokio::fs::File::from_std(android_file.into_file())
    } else if file_path.starts_with("fd:") {
        // 解析 msg_id 并从 FD 缓存克隆
        let msg_id_str = &file_path["fd:".len()..];
        let msg_id: i64 = msg_id_str
            .parse()
            .map_err(|_| format!("无效的 fd: 路径: {}", file_path))?;
        match crate::android_fd::duplicate_cached_file(msg_id) {
            Some((file, _, _)) => file,
            None => return Err(format!("FD 缓存未命中: msg_id={}", msg_id)),
        }
    } else {
        tokio::fs::File::open(file_path)
            .await
            .map_err(|e| format!("无法打开文件: {}", e))?
    };

    #[cfg(not(target_os = "android"))]
    let mut file = tokio::fs::File::open(file_path)
        .await
        .map_err(|e| format!("无法打开文件: {}", e))?;

    let client = crate::network::lan_http_client(Some(std::time::Duration::from_secs(300)))?;

    let file_size = crate::file_source::size_from_metadata(&file.metadata().await
        .map_err(|error| format!("读取补发文件信息失败: {error}"))?,
        usize::try_from(file_size).unwrap_or(0))? as i64;
    crate::db::update_sent_file_size(pool, msg_id, file_size as usize).await?;
    let upload_url = format!("http://{}/api/upload", peer_addr);
    let chunk_size = FILE_TRANSFER_CHUNK_SIZE as i64;
    let total_chunks = (file_size + chunk_size - 1) / chunk_size;
    let mut offset = 0;
    let mut chunk_index = 0;
    let start_time = std::time::Instant::now();

    while offset < file_size {
        let expected = chunk_size.min(file_size - offset) as usize;
        let buf = crate::file_source::read_upload_chunk(
            &mut file, expected, offset + expected as i64 == file_size,
        ).await?;
        let bytes_read = buf.len();

        let form = reqwest::multipart::Form::new()
            .text("peer_id", my_id.to_string())
            .text("file_name", file_name.to_string())
            .text("file_size", file_size.to_string())
            .text("chunk_index", chunk_index.to_string())
            .text("chunk_total", total_chunks.to_string())
            .text("sender_msg_id", msg_id.to_string())
            .text("speed_mb_s", if offset > 0 {
                format!("{:.1}", offset as f64 / (1024.0 * 1024.0) / start_time.elapsed().as_secs_f64().max(0.001))
            } else { "0".to_string() })
            .part(
                "chunk",
                reqwest::multipart::Part::bytes(buf)
                    .mime_str("application/octet-stream")
                    .unwrap(),
            );

        let resp = client
            .post(&upload_url)
            .multipart(form)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("HTTP Error: {}", resp.status()));
        }

        offset += bytes_read as i64;
        chunk_index += 1;
        let elapsed = start_time.elapsed().as_secs_f64().max(0.001);
        event_bus.publish(CoreEvent::FileTransferProgress(serde_json::json!({
            "sender_msg_id": msg_id, "transferred": offset, "total": file_size,
            "speed_mb_s": offset as f64 / (1024.0 * 1024.0) / elapsed,
        })));
        if chunk_index % 5 == 0 {
            println!(
                "[Messaging] 文件 {} 补发中: {}/{} MB",
                file_name,
                offset / (1024 * 1024),
                file_size / (1024 * 1024)
            );
        }
    }
    if offset != file_size {
        return Err(format!("文件未完整补发: {offset}/{file_size}"));
    }
    Ok(())
}

// 补发挂起的消息给上线的用户
pub async fn resend_pending_messages(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    peer_id: &str,
    peer_addr: &str,
    event_bus: CoreEventBus,
) -> Result<(), String> {
    println!("[Messaging] 检查用户 {} 的挂起消息...", peer_id);

    let pending_messages = crate::db::get_pending_messages(pool, peer_id).await?;
    if pending_messages.is_empty() {
        println!("[Messaging] 用户 {} 没有挂起消息", peer_id);
        return Ok(());
    }

    println!(
        "[Messaging] 发现 {} 条挂起消息，开始握手...",
        pending_messages.len()
    );
    let my_id = crate::db::get_user_id(pool).await?;

    match send_handshake(peer_addr, my_id.clone(), "ready_to_receive").await {
        Ok(_) => {
            println!("[UDP] ✓ 握手请求已发送");
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            let mut msg_ids_to_mark = Vec::new();

            for (msg_id, content, msg_type, _timestamp, file_path, file_size) in pending_messages {
                if msg_type == "text" {
                    // 补发文本消息
                    match send_text_message(
                        peer_addr,
                        my_id.clone(),
                        "System".to_string(),
                        content.clone(),
                    )
                    .await
                    {
                        Ok(_) => {
                            println!("[UDP] ✓ 文本消息 {} 补发成功", msg_id);
                            msg_ids_to_mark.push(msg_id);
                        }
                        Err(e) => {
                            eprintln!("[UDP] ✗ 文本消息 {} 补发失败: {}", msg_id, e);
                        }
                    }
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                } else if msg_type == "file" {
                    match (file_path.as_ref(), file_size) {
                        (Some(path), Some(size)) if !path.trim().is_empty() => {
                            // 检查接收端 auto_download 设置
                            let auto_dl_url = format!("http://{}/api/auto_download", peer_addr);
                            let auto_enabled = match crate::network::lan_http_client(None) {
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
                            };

                            if auto_enabled {
                                // 自动下载开启 → 直接上传（原行为）
                                let _ = crate::db::update_file_status_by_id(pool, msg_id, "retrying").await;
                                event_bus.publish_message(serde_json::json!({
                                    "msg_type": "file_status_update", "sender_msg_id": msg_id,
                                    "file_status": "retrying", "total": size,
                                }));
                                match resend_file_background(
                                    pool,
                                    &my_id, peer_addr, &content, path, size, msg_id, &event_bus,
                                )
                                .await
                                {
                                    Ok(_) => {
                                        println!("[UDP] ✓ 文件消息 {} 补发成功", msg_id);
                                        let _ = crate::db::update_file_status_by_id(pool, msg_id, "sent").await;
                                        event_bus.publish_message(serde_json::json!({
                                            "msg_type": "file_status_update", "sender_msg_id": msg_id,
                                            "file_status": "sent",
                                        }));
                                        msg_ids_to_mark.push(msg_id);
                                    }
                                    Err(e) => {
                                        eprintln!("[UDP] ✗ 文件消息 {} 补发失败: {}", msg_id, e);
                                        let _ = crate::db::update_file_status_by_id(pool, msg_id, "failed").await;
                                        event_bus.publish_message(serde_json::json!({
                                            "msg_type": "file_status_update", "sender_msg_id": msg_id,
                                            "file_status": "failed",
                                        }));
                                    }
                                }
                            } else {
                                // 自动下载关闭 → 发 file_offer，不直接上传
                                let my_name =
                                    crate::db::get_username(pool).await.unwrap_or_default();
                                let offer = serde_json::json!({
                                    "msg_type": "file_offer",
                                    "from_id": my_id,
                                    "from_name": my_name,
                                    "file_name": content,
                                    "file_size": size,
                                    "sender_msg_id": msg_id,
                                });
                                let ws_url = format!("ws://{}/ws", peer_addr);
                                if let Ok((mut ws_stream, _)) =
                                    tokio_tungstenite::connect_async(&ws_url).await
                                {
                                    use futures_util::SinkExt;
                                    use tokio_tungstenite::tungstenite::protocol::Message as WsMessage;
                                    let _ =
                                        ws_stream.send(WsMessage::Text(offer.to_string())).await;
                                    let _ = ws_stream.close(None).await;
                                    // 更新发送端 status 为 offering，前端显示「等待对方接收」
                                    let _ = crate::db::update_file_status_by_id(
                                        pool, msg_id, "offering",
                                    )
                                    .await;
                                    event_bus.publish_message(serde_json::json!({
                                        "msg_type": "file_status_update",
                                        "sender_msg_id": msg_id,
                                        "file_status": "offering",
                                    }));
                                    println!(
                                        "[UDP] ✓ 文件消息 {} 已发出 file_offer，等待手动接收",
                                        msg_id
                                    );
                                    msg_ids_to_mark.push(msg_id);
                                } else {
                                    eprintln!(
                                        "[UDP] ✗ 文件消息 {} file_offer 发送失败（无法连接 WS）",
                                        msg_id
                                    );
                                }
                            }
                        }
                        _ => {
                            if let Some(size) = file_size {
                                // 有文件大小但无路径 → Web端发送的离线文件，浏览器持有 File 引用
                                // 尝试发送 file_offer 给接收端，后续 file_request → start_upload 由 WS handler 处理
                                let my_name =
                                    crate::db::get_username(pool).await.unwrap_or_default();
                                let offer = serde_json::json!({
                                    "msg_type": "file_offer",
                                    "from_id": my_id,
                                    "from_name": my_name,
                                    "file_name": content,
                                    "file_size": size,
                                    "sender_msg_id": msg_id,
                                });
                                let ws_url = format!("ws://{}/ws", peer_addr);
                                if let Ok((mut ws_stream, _)) =
                                    tokio_tungstenite::connect_async(&ws_url).await
                                {
                                    use futures_util::SinkExt;
                                    use tokio_tungstenite::tungstenite::protocol::Message as WsMessage;
                                    let _ =
                                        ws_stream.send(WsMessage::Text(offer.to_string())).await;
                                    let _ = ws_stream.close(None).await;
                                    // 更新发送端 status 为 offering，前端显示「等待对方接收」
                                    let _ = crate::db::update_file_status_by_id(
                                        pool, msg_id, "offering",
                                    )
                                    .await;
                                    event_bus.publish_message(serde_json::json!({
                                        "msg_type": "file_status_update",
                                        "sender_msg_id": msg_id,
                                        "file_status": "offering",
                                    }));
                                    println!(
                                        "[UDP] ✓ Web端文件消息 {} 已发出 file_offer，等待手动接收",
                                        msg_id
                                    );
                                    msg_ids_to_mark.push(msg_id);
                                } else {
                                    eprintln!(
                                        "[UDP] ✗ Web端文件消息 {} file_offer 发送失败（无法连接 WS），保留挂起状态",
                                        msg_id
                                    );
                                }
                            } else {
                                // 既无路径也无大小 → 数据异常，移出挂起队列避免死循环
                                eprintln!("[UDP] ⚠ 跳过无法补发的文件消息 {}: 路径缺失且无文件大小(数据异常)。将其移出挂起队列。", msg_id);
                                msg_ids_to_mark.push(msg_id);
                            }
                        }
                    }
                }
            }

            if !msg_ids_to_mark.is_empty() {
                crate::db::mark_messages_as_sent(pool, msg_ids_to_mark).await?;
                event_bus.publish(CoreEvent::MessagesResent {
                    peer_id: peer_id.to_string(),
                });
            }
        }
        Err(e) => eprintln!("[Messaging] 握手失败: {}", e),
    }
    Ok(())
}
