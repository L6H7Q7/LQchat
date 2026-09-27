use axum::extract::ws::{Message, WebSocket};
use axum::{
    body::Body,
    extract::{ConnectInfo, Json, Multipart, Path, Query, State, WebSocketUpgrade},
    http::{header, Response, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use futures_util::{SinkExt, StreamExt};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::fs;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tower_http::cors::{Any, CorsLayer};

use crate::core_events::CoreEventBus;
use crate::peers::PeerManager;

// 全局媒体 Token（仅 Android 使用）
static MEDIA_TOKEN: Mutex<String> = Mutex::new(String::new());

#[derive(RustEmbed)]
#[folder = "../src/"]
struct Asset;

#[derive(Serialize)]
struct NameResponse {
    name: String,
}

#[derive(Deserialize)]
struct UpdateNameRequest {
    name: String,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Deserialize)]
struct SendMessageRequest {
    peer_id: String,
    peer_addr: String,
    content: String,
}

#[derive(Deserialize)]
struct PeerIdRequest {
    peer_id: String,
}

// Web 服务器的状态
#[derive(Clone)]
pub struct AppState {
    pub pool: Pool<Sqlite>,
    pub peer_manager: Arc<PeerManager>,
    pub media_token: String,
    pub ws_broadcast: broadcast::Sender<String>,
    pub event_bus: CoreEventBus,
}

pub async fn start_server(
    port: u16,
    _udp_port: u16,
    pool: Pool<Sqlite>,
    peer_manager: Arc<PeerManager>,
    event_bus: CoreEventBus,
    cancellation: CancellationToken,
) -> Result<(), String> {
    let listener = bind_server(port).await.map_err(|error| error.to_string())?;
    run_server(listener, pool, peer_manager, event_bus, cancellation).await
}

pub async fn bind_server(port: u16) -> std::io::Result<tokio::net::TcpListener> {
    tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await
}

pub async fn run_server(
    listener: tokio::net::TcpListener,
    pool: Pool<Sqlite>,
    peer_manager: Arc<PeerManager>,
    event_bus: CoreEventBus,
    cancellation: CancellationToken,
) -> Result<(), String> {
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    let media_token = uuid::Uuid::new_v4().to_string();
    println!("[Web Server] 媒体访问 Token: {}", media_token);

    // 将 token 存入全局，供 Tauri command 读取（仅 Android）
    #[cfg(target_os = "android")]
    {
        let mut guard = MEDIA_TOKEN
            .lock()
            .map_err(|_| "媒体 Token 锁已损坏".to_string())?;
        *guard = media_token.clone();
    }

    let (ws_broadcast, _) = broadcast::channel::<String>(128);

    let state = Arc::new(AppState {
        pool,
        peer_manager,
        media_token,
        ws_broadcast,
        event_bus,
    });

    // 配置 CORS - 允许所有来源（局域网内部使用）
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any)
        .allow_credentials(false); // 明确设置不需要凭证

    let app = Router::new()
        .route("/", get(serve_index))
        .route("/*path", get(serve_assets))
        .route("/api/get_my_name", get(get_name_http))
        .route("/api/get_my_id", get(get_id_http))
        .route("/api/update_my_name", post(update_name_http))
        .route("/api/get_settings", get(get_settings_http))
        .route("/api/update_settings", post(update_settings_http))
        .route("/api/get_language", get(get_language_http))
        .route("/api/set_language", post(set_language_http))
        .route("/api/get_custom_peers", get(get_custom_peers_http))
        .route("/api/add_custom_peer", post(add_custom_peer_http))
        .route("/api/remove_custom_peer", post(remove_custom_peer_http))
        .route("/api/get_peers", get(get_peers_http))
        .route("/api/send_message", post(send_message_http))
        .route("/api/chat_history/:peer_id", get(get_chat_history_http))
        .route("/api/upload", post(upload_file_http))
        .route("/api/upload_progress", post(upload_progress_http))
        .route("/api/accept_file/:file_id", post(accept_file_http))
        .route("/api/download/:file_id", get(download_file_http))
        .route("/api/create_upload_record", post(create_upload_record_http))
        .route("/api/update_upload_status", post(update_upload_status_http))
        .route("/api/mark_upload_complete", post(mark_upload_complete_http))
        .route("/api/delete_upload_record", post(delete_upload_record_http))
        .route("/api/clear_chat_history", post(clear_chat_history_http))
        .route("/api/delete_user", post(delete_user_http))
        .route("/api/delete_messages", post(delete_messages_http))
        .route("/api/get_theme_list", get(get_theme_list_http))
        .route("/api/get_theme_css/:theme_name", get(get_theme_css_http))
        .route("/api/save_current_theme", post(save_current_theme_http))
        .route("/api/get_current_theme", get(get_current_theme_http))
        .route("/api/auto_download", get(auto_download_http))
        .route("/api/offer_file", post(offer_file_http))
        .route("/api/start_send", post(start_send_http))
        .route("/api/request_file", post(request_file_http))
        .route("/api/media", get(serve_media_http))
        .route("/ws", get(websocket_handler))
        .layer(cors)
        .layer(axum::extract::DefaultBodyLimit::disable()) // 无限制
        .with_state(state)
        .into_make_service_with_connect_info::<SocketAddr>();

    println!("[Web Server] 启动在端口 {} (无文件大小限制)", port);
    axum::serve(listener, app)
        .with_graceful_shutdown(cancellation.cancelled_owned())
        .await
        .map_err(|error| error.to_string())
}

async fn get_name_http(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    println!("[Web Server] 收到获取用户名请求");

    match crate::db::get_username(&state.pool).await {
        Ok(name) => Json(NameResponse { name }).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("读取用户名失败: {}", e),
            }),
        )
            .into_response(),
    }
}

async fn get_id_http(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    println!("[Web Server] 收到获取用户 ID 请求");

    match crate::db::get_user_id(&state.pool).await {
        Ok(id) => Json(serde_json::json!({ "id": id })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("读取用户 ID 失败: {}", e),
            }),
        )
            .into_response(),
    }
}

async fn get_settings_http(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    println!("[Web Server] 收到获取设置请求");

    let download_path = crate::db::get_download_path(&state.pool)
        .await
        .unwrap_or_else(|_| {
            #[cfg(windows)]
            let path = crate::config_file::windows_portable_download_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("downloads"));
            #[cfg(not(windows))]
            let path = std::env::temp_dir().join("lanchat_downloads");
            path.to_string_lossy().to_string()
        });

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

    Json(serde_json::json!({
        "download_path": download_path,
        "port": port,
        "db_path": db_path,
        "auto_download": auto_download,
        "close_to_tray": close_to_tray,
        "start_minimized": start_minimized,
    }))
    .into_response()
}

#[derive(Deserialize)]
struct UpdateSettingsRequest {
    download_path: Option<String>,
    port: Option<String>,
    db_path: Option<String>,
    auto_download: Option<bool>,
    close_to_tray: Option<bool>,
    start_minimized: Option<bool>,
}

async fn update_settings_http(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<UpdateSettingsRequest>,
) -> impl IntoResponse {
    println!("[Web Server] 收到更新设置请求");

    if let Some(ref path) = payload.download_path {
        if let Err(e) = crate::db::update_download_path(&state.pool, path.clone()).await {
            return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: e })).into_response();
        }
    }
    if let Some(ref p) = payload.port {
        let port_num: u16 = match p.parse() {
            Ok(n) if n > 0 => n,
            _ => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "invalid port".to_string(),
                    }),
                )
                    .into_response();
            }
        };
        if let Err(e) = crate::config_file::save_port_to_config(port_num) {
            return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: e })).into_response();
        }
    }
    if let Some(path) = payload.db_path {
        if !cfg!(target_os = "android") {
            let mut cfg = crate::config_file::read_config();
            if path.is_empty() {
                cfg.db_path = None;
            } else {
                cfg.db_path = Some(path);
            }
            if let Err(e) = crate::config_file::write_config(&cfg) {
                return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: e })).into_response();
            }
        }
    }
    if let Some(enabled) = payload.auto_download {
        let _ = crate::db::set_auto_download(&state.pool, enabled).await;
    }
    if let Some(enabled) = payload.close_to_tray {
        if !cfg!(target_os = "android") {
            if let Err(e) = crate::config_file::save_close_to_tray_to_config(enabled) {
                return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: e })).into_response();
            }
        }
    }
    if let Some(enabled) = payload.start_minimized {
        if !cfg!(target_os = "android") {
            if let Err(e) = crate::config_file::save_start_minimized_to_config(enabled) {
                return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: e })).into_response();
            }
        }
    }

    Json(serde_json::json!({ "success": true })).into_response()
}

async fn get_language_http() -> impl IntoResponse {
    Json(serde_json::json!({
        "lang": crate::config_file::get_lang_from_config().unwrap_or_else(|| "auto".to_string())
    }))
}

#[derive(Deserialize)]
struct SetLanguageRequest {
    lang: String,
}

async fn set_language_http(Json(payload): Json<SetLanguageRequest>) -> impl IntoResponse {
    match crate::config_file::save_lang_to_config(&payload.lang) {
        Ok(()) => Json(serde_json::json!({ "success": true })).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct CustomPeerRequest {
    peer: String,
}

async fn get_custom_peers_http(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let peers = crate::db::get_custom_peers(&state.pool).await;
    Json(serde_json::json!({ "peers": peers })).into_response()
}

async fn add_custom_peer_http(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CustomPeerRequest>,
) -> impl IntoResponse {
    println!("[Web Server] 添加自定义 IP: {}", payload.peer);
    if let Err(e) = crate::db::add_custom_peer(&state.pool, &payload.peer).await {
        return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: e })).into_response();
    }
    Json(serde_json::json!({ "success": true })).into_response()
}

async fn remove_custom_peer_http(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CustomPeerRequest>,
) -> impl IntoResponse {
    println!("[Web Server] 删除自定义 IP: {}", payload.peer);
    if let Err(e) = crate::db::remove_custom_peer(&state.pool, &payload.peer).await {
        return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: e })).into_response();
    }
    Json(serde_json::json!({ "success": true })).into_response()
}

// ── 自动下载状态 ──
async fn auto_download_http(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let enabled = crate::db::get_auto_download(&state.pool).await;
    Json(serde_json::json!({"enabled": enabled}))
}

// ── 本端前端发送 file_offer（POST /api/offer_file） ──
#[derive(Deserialize)]
struct OfferFileRequest {
    peer_addr: String,
    file_name: String,
    file_size: u64,
    sender_msg_id: i64,
}

async fn offer_file_http(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<OfferFileRequest>,
) -> impl IntoResponse {
    println!(
        "[Web Server] file_offer: {} -> {}",
        payload.file_name, payload.peer_addr
    );
    let my_id = crate::db::get_user_id(&state.pool)
        .await
        .unwrap_or_default();
    let my_name = crate::db::get_username(&state.pool)
        .await
        .unwrap_or_default();
    let offer = serde_json::json!({
        "msg_type": "file_offer",
        "from_id": my_id,
        "from_name": my_name,
        "file_name": payload.file_name,
        "file_size": payload.file_size,
        "sender_msg_id": payload.sender_msg_id,
    });
    match crate::network::messaging::send_json_via_ws(&payload.peer_addr, &offer.to_string()).await
    {
        Ok(_) => Json(serde_json::json!({"success": true})).into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "error": format!("发送 file_offer 失败: {}", e)
            })),
        )
            .into_response(),
    }
}

// ── 对方请求开始发送文件（POST /api/start_send） ──
#[derive(Deserialize)]
struct StartSendRequest {
    sender_msg_id: i64,
    receiver_addr: String,
}

async fn start_send_http(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<StartSendRequest>,
) -> impl IntoResponse {
    println!(
        "[手动下载] HTTP请求开始发送文件: msg_id={}, 接收端={}",
        payload.sender_msg_id, payload.receiver_addr
    );

    // 查询发送端 DB 中的文件记录
    match crate::db::get_sender_file_by_msg_id(&state.pool, payload.sender_msg_id).await {
        Ok((file_path, file_name, file_size)) => {
            if file_path.is_empty() || !std::path::Path::new(&file_path).exists() {
                if file_path.is_empty() {
                    // Web 端：文件在浏览器中 → 通知浏览器开始上传
                    let start_evt = serde_json::json!({
                        "msg_type": "start_upload",
                        "sender_msg_id": payload.sender_msg_id,
                        "file_name": file_name,
                        "file_size": file_size,
                        "receiver_addr": payload.receiver_addr,
                    });
                    let _ = state.ws_broadcast.send(start_evt.to_string());
                    state.event_bus.publish_message(start_evt);
                    // 通知发送端前端显示上传中
                    let upload_notice = serde_json::json!({
                        "msg_type": "file_status_update",
                        "sender_msg_id": payload.sender_msg_id,
                        "file_status": "retrying",
                    });
                    let _ = state.ws_broadcast.send(upload_notice.to_string());
                    state.event_bus.publish_message(upload_notice);
                    return (
                        StatusCode::OK,
                        Json(serde_json::json!({
                            "success": true, "status": "notifying_browser"
                        })),
                    )
                        .into_response();
                }
                // 文件丢失 → 通知接收端
                let notice = serde_json::json!({
                    "msg_type": "file_not_found",
                    "sender_msg_id": payload.sender_msg_id,
                });
                let _ = crate::network::messaging::send_json_via_ws(
                    &payload.receiver_addr,
                    &notice.to_string(),
                )
                .await;
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({
                        "error": "文件不存在",
                        "not_found": true,
                    })),
                )
                    .into_response();
            }

            // 文件存在，开始上传
            let file = match tokio::fs::File::open(&file_path).await {
                Ok(f) => f,
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({
                            "error": format!("打开文件失败: {}", e)
                        })),
                    )
                        .into_response();
                }
            };

            // 通知发送端前端显示上传中
            let upload_notice = serde_json::json!({
                "msg_type": "file_status_update",
                "sender_msg_id": payload.sender_msg_id,
                "file_status": "retrying",
            });
            let _ = state.ws_broadcast.send(upload_notice.to_string());
            state.event_bus.publish_message(upload_notice);

            // 调用同步的上传函数
            let _ = state.pool.clone();
            let pool = state.pool.clone();
            let receiver_addr = payload.receiver_addr.clone();
            let fname = file_name.clone();
            let fsize = file_size as usize;
            let fpath = file_path.clone();

            let sm_id = payload.sender_msg_id;
            let ws_tx = state.ws_broadcast.clone();
            let event_bus = state.event_bus.clone();
            tokio::spawn(async move {
                let completed = upload_to_receiver(
                    &pool,
                    &receiver_addr,
                    &fname,
                    fsize,
                    &fpath,
                    file,
                    sm_id,
                    event_bus.clone(),
                )
                .await;
                // 上传完成 → 更新发送端 DB 状态
                let _ = crate::db::update_file_status_by_id(&pool, sm_id, if completed { "sent" } else { "failed" }).await;
                // 通知发送端前端更新 UI
                let update = serde_json::json!({
                    "msg_type": "file_status_update",
                    "sender_msg_id": sm_id,
                    "file_status": if completed { "sent" } else { "failed" },
                });
                let _ = ws_tx.send(update.to_string());
                event_bus.publish_message(update);
            });

            (
                StatusCode::OK,
                Json(serde_json::json!({"success": true, "status": "sending"})),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": format!("查询文件记录失败: {}", e),
                "not_found": true,
            })),
        )
            .into_response(),
    }
}

// 非 Tauri 环境下的文件上传（web 端用）
async fn upload_to_receiver(
    _pool: &sqlx::Pool<sqlx::Sqlite>,
    _receiver_addr: &str,
    _file_name: &str,
    _file_size: usize,
    _file_path: &str,
    file: tokio::fs::File,
    _sender_msg_id: i64,
    event_bus: CoreEventBus,
) -> bool {
    // 获取自己的 ID
    let my_id = crate::db::get_user_id(_pool).await.unwrap_or_default();

    let file_size = _file_size;
    let file_name = _file_name.to_string();
    let peer_addr = _receiver_addr.to_string();

    // 分块上传到接收端
    let chunk_size = crate::network::messaging::FILE_TRANSFER_CHUNK_SIZE;
    let total_chunks = (file_size + chunk_size - 1) / chunk_size;

    let mut reader = tokio::io::BufReader::new(file);
    let mut chunk_index = 0usize;
    let mut offset: usize = 0;
    let start_time = std::time::Instant::now();
    let client = match crate::network::lan_http_client(None) {
        Ok(client) => client,
        Err(error) => {
            eprintln!("[WebServer] 无法创建局域网 HTTP 客户端: {error}");
            return false;
        }
    };
    let upload_url = format!("http://{}/api/upload", peer_addr);

    loop {
        let mut buf = vec![0u8; chunk_size];
        let mut bytes_read = 0usize;
        while bytes_read < chunk_size {
            let n = match tokio::io::AsyncReadExt::read(&mut reader, &mut buf[bytes_read..]).await {
                Ok(0) => break,
                Ok(n) => n,
                Err(error) => {
                    eprintln!("[WebServer] 读取文件失败: {error}");
                    return false;
                }
            };
            bytes_read += n;
            if n == 0 {
                break;
            }
        }
        if bytes_read == 0 {
            break;
        }
        buf.truncate(bytes_read);

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

        let form = reqwest::multipart::Form::new()
            .text("peer_id", my_id.clone())
            .text("file_name", file_name.clone())
            .text("file_size", file_size.to_string())
            .text("chunk_index", chunk_index.to_string())
            .text("chunk_total", total_chunks.to_string())
            .text("sender_msg_id", _sender_msg_id.to_string())
            .text("speed_mb_s", format!("{:.1}", speed_mb_s))
            .part(
                "chunk",
                reqwest::multipart::Part::bytes(buf)
                    .mime_str("application/octet-stream")
                    .unwrap(),
            );

        match client.post(&upload_url).multipart(form).send().await {
            Ok(resp) if resp.status().is_success() => {
                println!("[WebServer] ✓ 分块 {}/{}", chunk_index + 1, total_chunks);
            }
            result => {
                eprintln!("[WebServer] 上传分块失败: {:?}", result.err());
                event_bus.publish(crate::core_events::CoreEvent::FileTransferProgress(
                    serde_json::json!({"sender_msg_id": _sender_msg_id, "transfer_status": "failed", "transferred": offset, "total": file_size}),
                ));
                return false;
            }
        }
        offset += bytes_read;
        chunk_index += 1;

        // 发送进度到发送端前端
        let elapsed = start_time.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            let speed = offset as f64 / (1024.0 * 1024.0) / elapsed;
            event_bus.publish(crate::core_events::CoreEvent::FileTransferProgress(
                serde_json::json!({
                    "file_name": _file_name,
                    "speed_mb_s": speed,
                    "sender_msg_id": _sender_msg_id,
                    "transferred": offset,
                    "total": file_size,
                }),
            ));
        }
    }

    if offset != file_size {
        eprintln!("[WebServer] 文件未完整发送: {offset}/{file_size}");
        return false;
    }
    println!("[WebServer] ✓ 文件上传完成: {}", file_name);
    true
}

// ── 接收端通过本地服务器发送 file_request（POST /api/request_file） ──
#[derive(Deserialize)]
struct RequestFilePayload {
    sender_addr: String,
    sender_msg_id: i64,
}

async fn request_file_http(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<RequestFilePayload>,
) -> impl IntoResponse {
    println!(
        "[手动下载] 接收端请求文件: msg_id={}, 发送端地址={}",
        payload.sender_msg_id, payload.sender_addr
    );

    let my_id = crate::db::get_user_id(&state.pool)
        .await
        .unwrap_or_default();

    let req_msg = serde_json::json!({
        "msg_type": "file_request",
        "sender_msg_id": payload.sender_msg_id,
        "from_id": my_id,
    });

    match crate::network::messaging::send_json_via_ws(&payload.sender_addr, &req_msg.to_string())
        .await
    {
        Ok(_) => Json(serde_json::json!({"success": true})).into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "error": format!("发送 file_request 失败: {}", e)
            })),
        )
            .into_response(),
    }
}

async fn update_name_http(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<UpdateNameRequest>,
) -> impl IntoResponse {
    println!("[Web Server] 收到改名请求: {}", payload.name);

    // 使用数据库的更新函数（包含验证逻辑）
    match crate::db::update_username(&state.pool, payload.name.clone()).await {
        Ok(_) => {
            // 数据库更新后，定时广播线程会自动使用新名称
            println!("[Web Server] 用户名已更新，广播线程将使用新名称");

            Json(NameResponse { name: payload.name }).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: e })).into_response(),
    }
}

async fn get_peers_http(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // 不打印日志,避免刷屏
    let peers = state.peer_manager.get_all_peers();
    Json(peers).into_response()
}

async fn serve_index() -> impl IntoResponse {
    serve_assets(axum::extract::Path("index.html".to_string())).await
}

async fn serve_assets(axum::extract::Path(path): axum::extract::Path<String>) -> impl IntoResponse {
    match Asset::get(&path) {
        Some(content) => {
            let mime = mime_guess::from_path(&path).first_or_octet_stream();
            Response::builder()
                .header(header::CONTENT_TYPE, mime.as_ref())
                .body(Body::from(content.data))
                .unwrap()
        }
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("404"))
            .unwrap(),
    }
}

async fn send_message_http(
    State(state): State<Arc<AppState>>,
    Json(mut payload): Json<SendMessageRequest>,
) -> impl IntoResponse {
    println!("[Web Server] 收到发送消息请求");

    // 获取自己的信息
    let my_id = match crate::db::get_user_id(&state.pool).await {
        Ok(id) => id,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse { error: e }),
            )
                .into_response();
        }
    };

    let my_name = match crate::db::get_username(&state.pool).await {
        Ok(name) => name,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse { error: e }),
            )
                .into_response();
        }
    };

    // 检查用户状态的同时，执行后端 IP 二次校验
    let peers = state.peer_manager.get_all_peers();
    let mut is_online = false;

    if let Some(p) = peers.iter().find(|p| p.id == payload.peer_id) {
        if !p.is_offline {
            is_online = true;
            // 发现 IP 不一致，强行改写前端的请求载荷！
            if p.addr != payload.peer_addr {
                println!(
                    "[Web Server] 🛡️ 拦截到过期 Web 请求 IP，后端强行纠正: {} -> {}",
                    payload.peer_addr, p.addr
                );
                payload.peer_addr = p.addr.clone();
            }
        }
    }

    if is_online {
        // 用户在线，尝试发送（这也是一种网络探测）
        match crate::network::messaging::send_text_message(
            &payload.peer_addr,
            my_id,
            my_name,
            payload.content.clone(),
        )
        .await
        {
            Ok(_) => {
                // 发送成功，保存到数据库(标记为已发送)
                if let Err(e) = crate::db::save_text_message_with_status(
                    &state.pool,
                    payload.peer_id,
                    payload.content,
                    "sent".to_string(),
                )
                .await
                {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse { error: e }),
                    )
                        .into_response();
                }
            }
            Err(e) => {
                // 发送失败（探测到实际已离线或网络故障，比如 IP 刚变但心跳还没发）
                eprintln!(
                    "[Web Server] 发送失败(网络探测): {}. 消息将转入挂起队列。",
                    e
                );

                // 1. 立即更新 Web Server 内存中的状态，标记为离线
                state.peer_manager.force_mark_offline(&payload.peer_id);

                // 2. 保存为挂起状态 (pending)
                if let Err(db_e) = crate::db::save_text_message_with_status(
                    &state.pool,
                    payload.peer_id.clone(),
                    payload.content,
                    "pending".to_string(),
                )
                .await
                {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse { error: db_e }),
                    )
                        .into_response();
                }
            }
        }
    } else {
        // 用户本来就在离线记录中，直接保存为挂起状态
        println!(
            "[Web Server] 用户 {} 离线，消息保存为挂起状态",
            payload.peer_id
        );
        if let Err(e) = crate::db::save_text_message_with_status(
            &state.pool,
            payload.peer_id,
            payload.content,
            "pending".to_string(),
        )
        .await
        {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse { error: e }),
            )
                .into_response();
        }
    }

    Json(serde_json::json!({ "success": true })).into_response()
}

async fn get_chat_history_http(
    State(state): State<Arc<AppState>>,
    Path(peer_id): Path<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    // 从查询参数获取 limit 和 offset
    let limit = params
        .get("limit")
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(10);

    let offset = params
        .get("offset")
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(0);

    match crate::network::messaging::get_chat_history_with_offset(
        &state.pool,
        &peer_id,
        limit,
        offset,
    )
    .await
    {
        Ok(messages) => Json(serde_json::json!({ "messages": messages })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error: e }),
        )
            .into_response(),
    }
}

// WebSocket 处理器
async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> axum::response::Response {
    ws.on_upgrade(|socket| handle_websocket(socket, state))
}

// 处理 WebSocket 连接
async fn handle_websocket(socket: WebSocket, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();

    println!("[WebSocket] 新的 WebSocket 连接");

    // 订阅广播频道并转发给此 WebSocket 客户端
    let mut broadcast_rx: broadcast::Receiver<String> = state.ws_broadcast.subscribe();
    let (direct_tx, mut direct_rx) = tokio::sync::mpsc::channel::<String>(1);
    let forward_handle = tokio::spawn(async move {
        loop {
            let broadcast_message = tokio::select! {
                direct = direct_rx.recv() => {
                    let Some(direct) = direct else { break; };
                    if sender.send(Message::Text(direct)).await.is_err() { break; }
                    continue;
                }
                message = broadcast_rx.recv() => message,
            };
            match broadcast_message {
                Ok(msg) => {
                    if sender.send(Message::Text(msg.into())).await.is_err() {
                        break;
                    }
                }
                Err(_) => {
                    // Lagged（滞后）或 Closed（不会发生）：继续接收
                    continue;
                }
            }
        }
    });

    // 接收消息
    while let Some(msg) = receiver.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                // Do not log raw frames: notification bodies may contain private data.

                match serde_json::from_str::<serde_json::Value>(&text) {
                    Ok(val) => {
                        let notification_kind = val.get("msg_type").and_then(|v| v.as_str());
                        if notification_kind == Some("notification") {
                            let result = crate::notification_sync::receive(
                                val,
                                text.len(),
                                &state.pool,
                                &state.peer_manager,
                            )
                            .await;
                            if direct_tx.send(result.to_string()).await.is_err() {
                                break;
                            }
                            continue;
                        }
                        if notification_kind == Some("notification_result") {
                            continue;
                        }
                        let stream_id = val
                            .get("stream_id")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        let stream_final = val
                            .get("stream_final")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(true);
                        let msg_type = val
                            .get("msg_type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("text")
                            .to_string();

                        if let Some(ref sid) = stream_id {
                            // ── 流式消息 ──
                            // 构建广播消息（保留原字段，统一注入 is_streaming）
                            let mut broadcast_val = val.clone();
                            if let Some(obj) = broadcast_val.as_object_mut() {
                                obj.insert(
                                    "is_streaming".to_string(),
                                    serde_json::json!(!stream_final),
                                );
                                obj.remove("stream_final");
                            }
                            let broadcast_msg = broadcast_val.to_string();

                            // 流式消息不存 DB（除了 text 类型的 stream_final）
                            if stream_final && msg_type == "text" {
                                if let Ok(message) = serde_json::from_value::<
                                    crate::network::messaging::TextMessage,
                                >(val.clone())
                                {
                                    match save_message_to_db(&state.pool, &message).await {
                                        Ok(msg_id) => {
                                            println!(
                                                "[WebSocket] 流式完成: {} (id={})",
                                                message
                                                    .content
                                                    .chars()
                                                    .take(40)
                                                    .collect::<String>(),
                                                msg_id
                                            );
                                            // 重新广播（带 id）
                                            let mut final_val = val.clone();
                                            if let Some(obj) = final_val.as_object_mut() {
                                                obj.insert(
                                                    "id".to_string(),
                                                    serde_json::json!(msg_id),
                                                );
                                                obj.insert(
                                                    "is_streaming".to_string(),
                                                    serde_json::json!(false),
                                                );
                                                obj.remove("stream_final");
                                            }
                                            let final_msg = final_val.to_string();
                                            let _ = state.ws_broadcast.send(final_msg);
                                            state.event_bus.publish_message(final_val);
                                        }
                                        Err(e) => {
                                            eprintln!("[WebSocket] 保存流式最终消息失败: {}", e)
                                        }
                                    }
                                }
                            } else {
                                // 非 final 或非 text 类型：直接广播
                                println!("[WebSocket] 流式块 ({}, stream={})", msg_type, sid);
                                let _ = state.ws_broadcast.send(broadcast_msg.clone());
                                // 流式中间块尚未形成数据库提交，不发布 CoreEvent。
                            }
                        } else if msg_type == "file_offer" {
                            // ── 收到文件邀请 ──
                            let file_name = val
                                .get("file_name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown");
                            let file_size =
                                val.get("file_size").and_then(|v| v.as_u64()).unwrap_or(0);
                            let from_id = val.get("from_id").and_then(|v| v.as_str()).unwrap_or("");
                            let sender_msg_id = val
                                .get("sender_msg_id")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(0);
                            let from_name = val
                                .get("from_name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();

                            let auto_dl = crate::db::get_auto_download(&state.pool).await;
                            if auto_dl {
                                // 自动下载开启 → 通知发送端开始上传
                                let accept = serde_json::json!({
                                    "msg_type": "file_accept",
                                    "sender_msg_id": sender_msg_id,
                                });
                                // 需要知道对方的地址来回复，用 from_id 查找 peer
                                let peer_addr = state
                                    .peer_manager
                                    .get_all_peers()
                                    .iter()
                                    .find(|p| p.id == from_id)
                                    .map(|p| p.addr.clone());
                                if let Some(addr) = peer_addr {
                                    let _ = crate::network::messaging::send_json_via_ws(
                                        &addr,
                                        &accept.to_string(),
                                    )
                                    .await;
                                }
                            } else {
                                // 自动下载关闭 → 创建 offered 记录并通知前端
                                let timestamp = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap()
                                    .as_secs()
                                    as i64;
                                if let Ok(msg_id) = crate::db::create_offered_file_record(
                                    &state.pool,
                                    from_id,
                                    file_name,
                                    file_size,
                                    &sender_msg_id.to_string(),
                                    timestamp,
                                )
                                .await
                                {
                                    let broadcast_msg = serde_json::json!({
                                        "id": msg_id,
                                        "from_id": from_id,
                                        "from_name": from_name,
                                        "content": file_name,
                                        "msg_type": "file",
                                        "file_status": "offered",
                                        "file_size": file_size,
                                        "sender_msg_id": sender_msg_id,
                                        "timestamp": timestamp,
                                    });
                                    let _ = state.ws_broadcast.send(broadcast_msg.to_string());
                                    state.event_bus.publish_message(broadcast_msg);
                                }
                            }
                        } else if msg_type == "file_accept" {
                            // ── 对方确认接受（auto_download=ON），开始上传 ──
                            let sender_msg_id = val
                                .get("sender_msg_id")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(0);
                            eprintln!(
                                "[WebSocket] 收到 file_accept for msg_id={}, 开始上传",
                                sender_msg_id
                            );
                            // 更新 DB 状态
                            let _ = crate::db::update_file_status_by_id(
                                &state.pool,
                                sender_msg_id,
                                "uploading",
                            )
                            .await;
                        } else if msg_type == "file_request" {
                            // ── 对方请求开始发送文件（手动下载） ──
                            let sender_msg_id = val
                                .get("sender_msg_id")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(0);
                            let from_id = val.get("from_id").and_then(|v| v.as_str()).unwrap_or("");
                            println!(
                                "[手动下载] 发送端收到下载请求: msg_id={}, from={}",
                                sender_msg_id, from_id
                            );

                            // 从 peer manager 查找对方地址
                            let receiver_addr = state
                                .peer_manager
                                .get_all_peers()
                                .iter()
                                .find(|p| p.id == from_id)
                                .map(|p| p.addr.clone())
                                .unwrap_or_default();

                            if receiver_addr.is_empty() {
                                eprintln!("[WebSocket] file_request: 找不到对方地址");
                                break;
                            }

                            // 查询发送端文件记录
                            match crate::db::get_sender_file_by_msg_id(&state.pool, sender_msg_id)
                                .await
                            {
                                Ok((file_path, fname, fsize)) => {
                                    // 立即更新为 uploading，前端显示「上传中」
                                    let _ = crate::db::update_file_status_by_id(
                                        &state.pool,
                                        sender_msg_id,
                                        "retrying",
                                    )
                                    .await;
                                    let update = serde_json::json!({
                                        "msg_type": "file_status_update",
                                        "sender_msg_id": sender_msg_id,
                                        "file_status": "retrying",
                                    });
                                    let _ = state.ws_broadcast.send(update.to_string());
                                    state.event_bus.publish_message(update);

                                    if file_path.is_empty() {
                                        // ── Web 端：文件在浏览器中，通知浏览器上传 ──
                                        println!("[手动下载] Web端文件在浏览器中，通知浏览器上传: msg_id={}", sender_msg_id);
                                        let start_evt = serde_json::json!({
                                            "msg_type": "start_upload",
                                            "sender_msg_id": sender_msg_id,
                                            "file_name": fname,
                                            "file_size": fsize,
                                            "receiver_addr": receiver_addr,
                                        });
                                        let _ = state.ws_broadcast.send(start_evt.to_string());
                                        state.event_bus.publish_message(start_evt);
                                    } else if file_path.starts_with("content://") {
                                        // ── Android SAF 文件（URI 已持久化权限） ──
                                        println!("[手动下载] Android content URI 文件: msg_id={}, uri={}", sender_msg_id, file_path);
                                        #[cfg(target_os = "android")]
                                        {
                                            use crate::android_fd::AndroidFile;
                                            match AndroidFile::from_content_uri(&file_path) {
                                                Ok(af) => {
                                                    let file =
                                                        tokio::fs::File::from_std(af.into_file());
                                                    let pool = state.pool.clone();
                                                    let raddr = receiver_addr.clone();
                                                    let ws_tx = state.ws_broadcast.clone();
                                                    let event_bus = state.event_bus.clone();
                                                    let fname2 = fname.clone();
                                                    tokio::spawn(async move {
                                                        let completed = upload_to_receiver(
                                                            &pool,
                                                            &raddr,
                                                            &fname2,
                                                            fsize as usize,
                                                            &file_path,
                                                            file,
                                                            sender_msg_id,
                                                            event_bus.clone(),
                                                        )
                                                        .await;
                                                        let _ =
                                                            crate::db::update_file_status_by_id(
                                                                &pool,
                                                                sender_msg_id,
                                                                if completed { "sent" } else { "failed" },
                                                            )
                                                            .await;
                                                        let update = serde_json::json!({
                                                            "msg_type": "file_status_update",
                                                            "sender_msg_id": sender_msg_id,
                                                            "file_status": if completed { "sent" } else { "failed" },
                                                        });
                                                        let _ = ws_tx.send(update.to_string());
                                                        event_bus.publish_message(update);
                                                    });
                                                }
                                                Err(e) => {
                                                    eprintln!(
                                                        "[手动下载] content URI 打开失败: {}",
                                                        e
                                                    );
                                                    let notice = serde_json::json!({
                                                        "msg_type": "file_not_found",
                                                        "sender_msg_id": sender_msg_id,
                                                    });
                                                    let _ = crate::network::messaging::send_json_via_ws(
                                                        &receiver_addr,
                                                        &notice.to_string(),
                                                    ).await;
                                                }
                                            }
                                        }
                                        #[cfg(not(target_os = "android"))]
                                        {
                                            let notice = serde_json::json!({
                                                "msg_type": "file_not_found",
                                                "sender_msg_id": sender_msg_id,
                                            });
                                            let _ = crate::network::messaging::send_json_via_ws(
                                                &receiver_addr,
                                                &notice.to_string(),
                                            )
                                            .await;
                                        }
                                    } else if file_path.starts_with("fd:") {
                                        // ── Android Share Intent 文件（FD 缓存） ──
                                        println!(
                                            "[手动下载] Share Intent FD 缓存文件: msg_id={}",
                                            sender_msg_id
                                        );
                                        #[cfg(target_os = "android")]
                                        {
                                            match crate::android_fd::duplicate_cached_file(
                                                sender_msg_id,
                                            ) {
                                                Some((file, cached_name, cached_size)) => {
                                                    let pool = state.pool.clone();
                                                    let raddr = receiver_addr.clone();
                                                    let ws_tx = state.ws_broadcast.clone();
                                                    let event_bus = state.event_bus.clone();
                                                    let fname2 = cached_name;
                                                    tokio::spawn(async move {
                                                        let completed = upload_to_receiver(
                                                            &pool,
                                                            &raddr,
                                                            &fname2,
                                                            cached_size as usize,
                                                            &file_path,
                                                            file,
                                                            sender_msg_id,
                                                            event_bus.clone(),
                                                        )
                                                        .await;
                                                        let _ =
                                                            crate::db::update_file_status_by_id(
                                                                &pool,
                                                                sender_msg_id,
                                                                if completed { "sent" } else { "failed" },
                                                            )
                                                            .await;
                                                        let update = serde_json::json!({
                                                            "msg_type": "file_status_update",
                                                            "sender_msg_id": sender_msg_id,
                                                            "file_status": if completed { "sent" } else { "failed" },
                                                        });
                                                        let _ = ws_tx.send(update.to_string());
                                                        event_bus.publish_message(update);
                                                    });
                                                }
                                                None => {
                                                    eprintln!("[手动下载] FD 缓存未命中 (app 可能已被杀): msg_id={}", sender_msg_id);
                                                    let notice = serde_json::json!({
                                                        "msg_type": "file_not_found",
                                                        "sender_msg_id": sender_msg_id,
                                                    });
                                                    let _ = crate::network::messaging::send_json_via_ws(
                                                        &receiver_addr,
                                                        &notice.to_string(),
                                                    ).await;
                                                }
                                            }
                                        }
                                        #[cfg(not(target_os = "android"))]
                                        {
                                            let notice = serde_json::json!({
                                                "msg_type": "file_not_found",
                                                "sender_msg_id": sender_msg_id,
                                            });
                                            let _ = crate::network::messaging::send_json_via_ws(
                                                &receiver_addr,
                                                &notice.to_string(),
                                            )
                                            .await;
                                        }
                                    } else if !std::path::Path::new(&file_path).exists() {
                                        // ── 普通文件不存在 ──
                                        println!(
                                            "[手动下载] 文件已丢失: msg_id={}, path={}",
                                            sender_msg_id, file_path
                                        );
                                        let notice = serde_json::json!({
                                            "msg_type": "file_not_found",
                                            "sender_msg_id": sender_msg_id,
                                        });
                                        let _ = crate::network::messaging::send_json_via_ws(
                                            &receiver_addr,
                                            &notice.to_string(),
                                        )
                                        .await;
                                    } else {
                                        // ── 普通文件存在，开始上传 ──
                                        println!("[手动下载] 文件存在，开始上传: msg_id={}, file={}, to={}", sender_msg_id, file_path, receiver_addr);
                                        match tokio::fs::File::open(&file_path).await {
                                            Ok(file) => {
                                                let pool = state.pool.clone();
                                                let raddr = receiver_addr.clone();
                                                let ws_tx = state.ws_broadcast.clone();
                                                let event_bus = state.event_bus.clone();
                                                let fname2 = fname.clone();
                                                tokio::spawn(async move {
                                                    let completed = upload_to_receiver(
                                                        &pool,
                                                        &raddr,
                                                        &fname2,
                                                        fsize as usize,
                                                        &file_path,
                                                        file,
                                                        sender_msg_id,
                                                        event_bus.clone(),
                                                    )
                                                    .await;
                                                    // 上传完成 → 更新发送端 DB 状态
                                                    let _ = crate::db::update_file_status_by_id(
                                                        &pool,
                                                        sender_msg_id,
                                                        if completed { "sent" } else { "failed" },
                                                    )
                                                    .await;
                                                    // 通知发送端前端更新 UI
                                                    let update = serde_json::json!({
                                                        "msg_type": "file_status_update",
                                                        "sender_msg_id": sender_msg_id,
                                                        "file_status": if completed { "sent" } else { "failed" },
                                                    });
                                                    let _ = ws_tx.send(update.to_string());
                                                    event_bus.publish_message(update);
                                                });
                                            }
                                            Err(_) => {
                                                println!("[手动下载] 打开文件失败(打开时错误): msg_id={}, path={}", sender_msg_id, file_path);
                                                let notice = serde_json::json!({
                                                    "msg_type": "file_not_found",
                                                    "sender_msg_id": sender_msg_id,
                                                });
                                                let _ =
                                                    crate::network::messaging::send_json_via_ws(
                                                        &receiver_addr,
                                                        &notice.to_string(),
                                                    )
                                                    .await;
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    println!(
                                        "[手动下载] 查询文件记录失败: msg_id={}, err={}",
                                        sender_msg_id, e
                                    );
                                    let notice = serde_json::json!({
                                        "msg_type": "file_not_found",
                                        "sender_msg_id": sender_msg_id,
                                    });
                                    let _ = crate::network::messaging::send_json_via_ws(
                                        &receiver_addr,
                                        &notice.to_string(),
                                    )
                                    .await;
                                }
                            }
                        } else if msg_type == "file_not_found" {
                            // ── 发送端告知文件已丢失 ──
                            let sender_msg_id = val
                                .get("sender_msg_id")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(0);
                            println!("[WebSocket] 收到 file_not_found: msg_id={}", sender_msg_id);
                            let _ = crate::db::update_file_status_by_sender_msg_id(
                                &state.pool,
                                &sender_msg_id.to_string(),
                                "invalid",
                            )
                            .await;

                            // 通知前端更新状态
                            let update = serde_json::json!({
                                "msg_type": "file_status_update",
                                "sender_msg_id": sender_msg_id,
                                "file_status": "invalid",
                            });
                            let _ = state.ws_broadcast.send(update.to_string());
                            state.event_bus.publish_message(update);
                        } else {
                            // ── 非流式消息：尝试作为 TextMessage 存 DB ──
                            if let Ok(message) = serde_json::from_value::<
                                crate::network::messaging::TextMessage,
                            >(val.clone())
                            {
                                match save_message_to_db(&state.pool, &message).await {
                                    Ok(msg_id) => {
                                        println!(
                                            "[WebSocket] 消息已保存: {} 说: {} (id={})",
                                            message.from_name, message.content, msg_id
                                        );
                                        let broadcast_value = serde_json::json!({
                                            "id": msg_id,
                                            "from_id": message.from_id,
                                            "from_name": message.from_name,
                                            "content": message.content,
                                            "timestamp": message.timestamp,
                                            "msg_type": message.msg_type,
                                        });
                                        let _ =
                                            state.ws_broadcast.send(broadcast_value.to_string());
                                        // save_message_to_db 已提交后才发布接收事件。
                                        state.event_bus.publish_message(broadcast_value);
                                    }
                                    Err(e) => eprintln!("[WebSocket] 保存消息失败: {}", e),
                                }
                            } else {
                                eprintln!(
                                    "[WebSocket] 无法解析消息: {}",
                                    &text[..text.len().min(100)]
                                );
                            }
                        }
                    }
                    Err(e) => eprintln!("[WebSocket] JSON 解析失败: {}", e),
                }
            }
            Ok(Message::Close(_)) => {
                println!("[WebSocket] 连接关闭");
                break;
            }
            Err(e) => {
                eprintln!("[WebSocket] 错误: {}", e);
                break;
            }
            _ => {}
        }
    }

    forward_handle.abort();
}

// 保存消息到数据库
async fn save_message_to_db(
    pool: &Pool<Sqlite>,
    message: &crate::network::messaging::TextMessage,
) -> Result<i64, String> {
    crate::db::save_received_text_message(
        pool,
        message.from_id.clone(),
        message.content.clone(),
        message.msg_type.clone(),
        message.timestamp as i64,
    )
    .await
}

async fn notify_receive_failure(state: &AppState, sender_id: &str, sender_msg_id: &str) {
    if sender_msg_id.is_empty() { return; }
    let _ = crate::db::mark_received_file_failed(&state.pool, sender_id, sender_msg_id).await;
    let event = serde_json::json!({
        "msg_type": "file_status_update", "sender_msg_id": sender_msg_id,
        "file_status": "failed", "direction": "received",
    });
    let _ = state.ws_broadcast.send(event.to_string());
    state.event_bus.publish_message(event);
}

#[derive(Clone, Serialize, Deserialize)]
struct UploadProgressAck {
    sender_msg_id: i64,
    receiver_id: String,
    received: u64,
    total: u64,
    speed_mb_s: f64,
}

// 接收端的块内进度走独立通道；发送端仍以 /api/upload 的成功响应作为分块完成确认。
async fn upload_progress_http(
    State(state): State<Arc<AppState>>,
    Json(progress): Json<UploadProgressAck>,
) -> impl IntoResponse {
    if progress.total == 0 || progress.received > progress.total || !progress.speed_mb_s.is_finite() {
        return StatusCode::BAD_REQUEST;
    }
    let record = sqlx::query_as::<_, (Option<String>, Option<i64>, Option<String>)>(
        "SELECT receiver_id, file_size, file_status FROM messages \
         WHERE id = ? AND sender_id = 'me' AND msg_type = 'file'",
    )
    .bind(progress.sender_msg_id)
    .fetch_optional(&state.pool)
    .await;
    let Ok(Some((receiver_id, file_size, file_status))) = record else {
        return StatusCode::NOT_FOUND;
    };
    if receiver_id.as_deref() != Some(progress.receiver_id.as_str())
        || file_size != i64::try_from(progress.total).ok()
        || !matches!(file_status.as_deref(), Some("uploading" | "retrying" | "offering"))
    {
        return StatusCode::CONFLICT;
    }

    // 接收中最多显示 99.x%；最终 100% 仍由最后一块的保存成功响应决定。
    let acknowledged = progress.received.min(progress.total.saturating_sub(1));
    let update = serde_json::json!({
        "sender_msg_id": progress.sender_msg_id,
        "transferred": acknowledged,
        "total": progress.total,
        "speed_mb_s": progress.speed_mb_s,
    });
    state.event_bus.publish(crate::core_events::CoreEvent::FileTransferProgress(update.clone()));
    let mut browser_update = update;
    browser_update["msg_type"] = serde_json::json!("file_upload_progress");
    let _ = state.ws_broadcast.send(browser_update.to_string());
    StatusCode::NO_CONTENT
}

async fn upload_progress_relay(
    state: &AppState,
    sender_id: &str,
    sender_msg_id: &str,
) -> Option<tokio::sync::mpsc::Sender<UploadProgressAck>> {
    let sender_msg_id = sender_msg_id.parse::<i64>().ok()?;
    let sender_addr = state.peer_manager.get_all_peers().into_iter()
        .find(|peer| peer.id == sender_id && !peer.is_offline)?.addr;
    let receiver_id = crate::db::get_user_id(&state.pool).await.ok()?;
    let client = crate::network::lan_http_client(Some(std::time::Duration::from_secs(2))).ok()?;
    let url = format!("http://{sender_addr}/api/upload_progress");
    let (tx, mut rx) = tokio::sync::mpsc::channel::<UploadProgressAck>(2);
    tokio::spawn(async move {
        while let Some(mut progress) = rx.recv().await {
            progress.sender_msg_id = sender_msg_id;
            progress.receiver_id = receiver_id.clone();
            let _ = client.post(&url).json(&progress).send().await;
        }
    });
    Some(tx)
}

async fn upload_file_http(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    println!("[Web Server] 收到文件上传请求");
    let request_started = std::time::Instant::now();

    let mut sender_id = String::new();
    let mut file_name = String::new();
    let mut file_size: u64 = 0;
    let mut chunk_index: usize = 0;
    let mut chunk_total: usize = 0;
    let mut chunk_data: Option<Vec<u8>> = None;
    // 第一块先落到独立 staging 文件；后续块可直接追加到已有的 .downloading 文件。
    let mut streamed_chunk: Option<(std::path::PathBuf, u64, bool)> = None;
    let mut sender_msg_id = String::new();
    let mut speed_mb_s: f64 = 0.0;

    // 获取下载目录
    let download_dir = get_download_dir(&state.pool).await;
    if let Err(e) = fs::create_dir_all(&download_dir).await {
        eprintln!("[Web Server] 创建目录失败: {}", e);
    }

    // 解析 multipart 字段
    println!("[Web Server] 开始解析 multipart 字段");
    loop {
        let mut field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(e) => {
                if let Some((path, _, true)) = &streamed_chunk {
                    let _ = fs::remove_file(path).await;
                }
                notify_receive_failure(&state, &sender_id, &sender_msg_id).await;
                return (StatusCode::BAD_REQUEST,
                    Json(ErrorResponse { error: format!("解析上传请求失败: {e}") })).into_response();
            }
        };
        let field_name = field.name().map(|s| s.to_string()).unwrap_or_default();

        match field_name.as_str() {
            "peer_id" => {
                if let Ok(text) = field.text().await {
                    sender_id = text;
                    println!("[Web Server] sender_id (发送者): {}", sender_id);
                }
            }
            "file_name" => {
                if let Ok(text) = field.text().await {
                    file_name = text;
                    println!("[Web Server] 文件名: {}", file_name);
                }
            }
            "file_size" => {
                if let Ok(text) = field.text().await {
                    file_size = text.parse().unwrap_or(0);
                    println!("[Web Server] 文件总大小: {}", file_size);
                }
            }
            "chunk_index" => {
                if let Ok(text) = field.text().await {
                    chunk_index = text.parse().unwrap_or(0);
                }
            }
            "chunk_total" => {
                if let Ok(text) = field.text().await {
                    chunk_total = text.parse().unwrap_or(0);
                    println!("[Web Server] 分块信息: {}/{}", chunk_index + 1, chunk_total);
                }
            }
            "sender_msg_id" => {
                if let Ok(text) = field.text().await {
                    sender_msg_id = text;
                }
            }
            "speed_mb_s" => {
                if let Ok(text) = field.text().await {
                    speed_mb_s = text.parse().unwrap_or(0.0);
                }
            }
            "chunk" => {
                let stream_path = if chunk_index == 0 && !file_name.is_empty() {
                    let candidate = download_dir.join(&file_name);
                    let downloading = download_dir.join(format!("{}.downloading", file_name));
                    let already_exists = candidate.exists() && !downloading.exists()
                        && fs::metadata(&candidate).await.map(|meta| meta.len() == file_size).unwrap_or(false);
                    if already_exists { None } else {
                        Some((download_dir.join(format!(".lqchat-{}.part", uuid::Uuid::new_v4())), true))
                    }
                } else if chunk_index > 0 {
                    let target = if !sender_msg_id.is_empty() {
                        crate::db::get_downloading_file_by_sender_msg_id(&state.pool, &sender_msg_id).await
                    } else {
                        crate::db::get_downloading_file(&state.pool, &sender_id).await
                    };
                    match target {
                        Ok(Some(name)) => Some((download_dir.join(format!("{}.downloading", name)), false)),
                        Ok(None) => None, // 旧版协议的秒传完成分支仍由下方处理。
                        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ErrorResponse { error: format!("查询接收文件失败: {e}") })).into_response(),
                    }
                } else {
                    None
                };

                if let Some((path, first_chunk)) = stream_path {
                    let prior_bytes = if first_chunk { 0 } else {
                        match fs::metadata(&path).await {
                            Ok(meta) => meta.len(),
                            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR,
                                Json(ErrorResponse { error: format!("读取临时文件失败: {e}") })).into_response(),
                        }
                    };
                    let file = if first_chunk {
                        fs::OpenOptions::new().create_new(true).write(true).open(&path).await
                    } else {
                        fs::OpenOptions::new().append(true).open(&path).await
                    };
                    let mut writer = match file {
                        Ok(file) => file,
                        Err(e) => {
                            notify_receive_failure(&state, &sender_id, &sender_msg_id).await;
                            return (StatusCode::INTERNAL_SERVER_ERROR,
                                Json(ErrorResponse { error: format!("打开临时文件失败: {e}") })).into_response();
                        }
                    };
                    let relay = upload_progress_relay(&state, &sender_id, &sender_msg_id).await;
                    let mut written = 0u64;
                    let mut last_update = std::time::Instant::now() - std::time::Duration::from_millis(200);
                    loop {
                        let next = match field.chunk().await {
                            Ok(next) => next,
                            Err(e) => {
                                if !first_chunk { let _ = writer.set_len(prior_bytes).await; }
                                drop(writer);
                                if first_chunk { let _ = fs::remove_file(&path).await; }
                                notify_receive_failure(&state, &sender_id, &sender_msg_id).await;
                                return (StatusCode::BAD_REQUEST,
                                    Json(ErrorResponse { error: format!("读取分块失败: {e}") })).into_response();
                            }
                        };
                        let Some(bytes) = next else { break; };
                        if let Err(e) = tokio::io::AsyncWriteExt::write_all(&mut writer, &bytes).await {
                            if !first_chunk { let _ = writer.set_len(prior_bytes).await; }
                            drop(writer);
                            if first_chunk { let _ = fs::remove_file(&path).await; }
                            notify_receive_failure(&state, &sender_id, &sender_msg_id).await;
                            return (StatusCode::INTERNAL_SERVER_ERROR,
                                Json(ErrorResponse { error: format!("写入分块失败: {e}") })).into_response();
                        }
                        written += bytes.len() as u64;
                        if last_update.elapsed() >= std::time::Duration::from_millis(150) {
                            let received = prior_bytes + written;
                            let speed = written as f64 / (1024.0 * 1024.0)
                                / request_started.elapsed().as_secs_f64().max(0.001);
                            let progress = serde_json::json!({
                                "msg_type": "file_download_progress", "sender_msg_id": sender_msg_id,
                                "received": received, "total": file_size, "speed_mb_s": speed,
                            });
                            let _ = state.ws_broadcast.send(progress.to_string());
                            state.event_bus.publish_message(progress);
                            if let Some(tx) = &relay {
                                let _ = tx.try_send(UploadProgressAck {
                                    sender_msg_id: 0, receiver_id: String::new(),
                                    received, total: file_size, speed_mb_s: speed,
                                });
                            }
                            last_update = std::time::Instant::now();
                        }
                    }
                    if let Err(e) = tokio::io::AsyncWriteExt::flush(&mut writer).await {
                        if !first_chunk { let _ = writer.set_len(prior_bytes).await; }
                        drop(writer);
                        if first_chunk { let _ = fs::remove_file(&path).await; }
                        notify_receive_failure(&state, &sender_id, &sender_msg_id).await;
                        return (StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ErrorResponse { error: format!("刷新分块失败: {e}") })).into_response();
                    }
                    streamed_chunk = Some((path, written, first_chunk));
                } else {
                    // 秒传兼容路径：读取但不写入新文件；解析错误不能误当成成功。
                    let mut data = Vec::new();
                    loop {
                        match field.chunk().await {
                            Ok(Some(bytes)) => data.extend_from_slice(&bytes),
                            Ok(None) => break,
                            Err(e) => return (StatusCode::BAD_REQUEST,
                                Json(ErrorResponse { error: format!("读取分块失败: {e}") })).into_response(),
                        }
                    }
                    chunk_data = Some(data);
                }
            }
            _ => {
                println!("[Web Server] 忽略未知字段: {}", field_name);
            }
        }
    }

    // 验证必需字段
    // 第一块需要所有字段，后续块只需要 chunk_index 和 chunk
    println!(
        "[Web Server] 验证字段: file_name={}, chunk_index={}, chunk_total={}, has_chunk={}",
        file_name,
        chunk_index,
        chunk_total,
        chunk_data.is_some() || streamed_chunk.is_some()
    );

    if chunk_data.is_none() && streamed_chunk.is_none() {
        eprintln!("[Web Server] ✗ 缺少 chunk 数据");
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "缺少 chunk 数据".to_string(),
            }),
        )
            .into_response();
    }

    if chunk_index == 0 && file_name.is_empty() {
        eprintln!("[Web Server] ✗ 第一块缺少 file_name");
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "第一块缺少 file_name".to_string(),
            }),
        )
            .into_response();
    }

    let chunk_data = chunk_data.unwrap_or_default();
    let chunk_len = streamed_chunk.as_ref().map(|(_, len, _)| *len as usize).unwrap_or(chunk_data.len());
    // 当前 10 MiB 协议必须完整收到每块；兼容旧客户端不同分块大小时仍按旧规则处理。
    let standard_chunk = crate::network::messaging::FILE_TRANSFER_CHUNK_SIZE as u64;
    let standard_total = file_size.saturating_add(standard_chunk - 1) / standard_chunk;
    if file_size > 0 && chunk_total as u64 == standard_total {
        let expected = standard_chunk.min(file_size.saturating_sub((chunk_index as u64).saturating_mul(standard_chunk)));
        if chunk_index >= chunk_total || chunk_len as u64 != expected {
            if let Some((path, _, true)) = &streamed_chunk {
                let _ = fs::remove_file(path).await;
            }
            notify_receive_failure(&state, &sender_id, &sender_msg_id).await;
            return (StatusCode::BAD_REQUEST,
                Json(ErrorResponse { error: "分块长度不完整".to_string() })).into_response();
        }
    }

    // ── 第一块：智能命名协商 ──────────────────────────────────────────────
    // 最终写入的文件名（可能因重命名而与 file_name 不同）
    let final_file_name: String;

    if chunk_index == 0 {
        // 拆分主文件名和扩展名
        let (stem, ext) = {
            let p = std::path::Path::new(&file_name);
            let s = p
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(&file_name)
                .to_string();
            let e = p
                .extension()
                .and_then(|s| s.to_str())
                .map(|e| format!(".{}", e))
                .unwrap_or_default();
            (s, e)
        };

        // 检查目标路径是否存在完整文件（无 .downloading 后缀）
        let candidate = download_dir.join(&file_name);
        let downloading_path = download_dir.join(format!("{}.downloading", file_name));

        if candidate.exists() && !downloading_path.exists() {
            // 目标文件存在且完整，比较大小
            let existing_size = tokio::fs::metadata(&candidate)
                .await
                .map(|m| m.len())
                .unwrap_or(0);

            if existing_size == file_size {
                // 大小完全相同 → 秒传：直接复用已有文件，写入数据库记录
                if let Some((path, _, true)) = &streamed_chunk {
                    let _ = fs::remove_file(path).await;
                }
                println!(
                    "[Web Server] ✓ 秒传命中: {:?} (大小相同: {} 字节)",
                    candidate, file_size
                );

                // 秒传也必须提交真实的最终保存地址，不能丢弃 SAF 导出返回的
                // content URI。导出失败时保留 staging 文件并明确标记 save_failed。
                let (stored_path, stored_status, save_error) =
                    resolve_received_file_destination(&state.pool, &candidate, &file_name).await;

                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64;

                // 秒传路径：先检查 offered 记录
                let updated_offered = crate::db::find_and_update_offered_record(
                    &state.pool,
                    &sender_id,
                    &file_name,
                    &stored_path,
                )
                .await
                .unwrap_or(None);

                if let Some((existing_id, _)) = updated_offered {
                    // 立即标记为 accepted（文件已完整）
                    if let Err(error) = crate::db::update_file_status_by_id(
                        &state.pool,
                        existing_id,
                        &stored_status,
                    )
                    .await
                    {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ErrorResponse {
                                error: format!("秒传状态提交失败: {error}"),
                            }),
                        )
                            .into_response();
                    }
                    println!(
                        "[Web Server] ✓ 秒传: 已更新 offered 记录(ID={}) 为 {}，路径={}",
                        existing_id, stored_status, stored_path
                    );

                    let sender_name = state
                        .peer_manager
                        .get_all_peers()
                        .iter()
                        .find(|p| p.id == sender_id)
                        .map(|p| p.name.clone())
                        .unwrap_or_default();

                    // 通知前端替换消息元素（完整 file 消息，触发图片渲染）
                    let full_msg = serde_json::json!({
                        "id": existing_id,
                        "from_id": sender_id,
                        "from_name": sender_name,
                        "content": file_name,
                        "msg_type": "file",
                        "file_status": stored_status,
                        "file_path": stored_path,
                        "file_save_error": save_error,
                        "file_size": file_size,
                        "file_id": file_name,
                        "file_name": file_name,
                        "sender_msg_id": sender_msg_id,
                        "timestamp": std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
                    });
                    let _ = state.ws_broadcast.send(full_msg.to_string());
                    state.event_bus.publish_message(full_msg);
                } else {
                    match crate::db::create_received_file_record(
                        &state.pool,
                        sender_id.clone(),
                        file_name.clone(),
                        stored_path.clone(),
                        file_size,
                        timestamp,
                        &sender_msg_id,
                    )
                    .await
                    {
                        Ok(msg_id) => {
                            // 立即标记为 accepted（文件已完整）
                            if let Err(error) = crate::db::update_file_status_by_id(
                                &state.pool,
                                msg_id,
                                &stored_status,
                            )
                            .await
                            {
                                return (
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    Json(ErrorResponse {
                                        error: format!("秒传状态提交失败: {error}"),
                                    }),
                                )
                                    .into_response();
                            }
                            println!(
                                "[Web Server] ✓ 秒传记录已创建并标记为 {}，ID: {}，路径={}",
                                stored_status, msg_id, stored_path
                            );
                            let sender_name = state
                                .peer_manager
                                .get_all_peers()
                                .iter()
                                .find(|p| p.id == sender_id)
                                .map(|p| p.name.clone())
                                .unwrap_or_default();
                            // 通知前端替换消息元素（完整 file 消息，触发图片渲染）
                            let full_msg = serde_json::json!({
                                "id": msg_id,
                                "from_id": sender_id,
                                "from_name": sender_name,
                                "content": file_name,
                                "msg_type": "file",
                                "file_status": stored_status,
                                "file_path": stored_path,
                                "file_save_error": save_error,
                                "file_size": file_size,
                                "file_id": file_name,
                                "file_name": file_name,
                                "sender_msg_id": sender_msg_id,
                                "timestamp": std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
                            });
                            let _ = state.ws_broadcast.send(full_msg.to_string());
                            state.event_bus.publish_message(full_msg);
                        }
                        Err(e) => {
                            eprintln!("[Web Server] ✗ 秒传记录创建失败: {}", e);
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(ErrorResponse {
                                    error: format!("秒传记录创建失败: {}", e),
                                }),
                            )
                                .into_response();
                        }
                    }
                }

                println!("[Web Server] ========== 秒传完成，通知发送端停止 ==========");
                return Json(serde_json::json!({
                    "status": "already_exists",
                    "file_name": file_name,
                    "file_size": file_size,
                    "message": "文件已存在且完整，秒传成功",
                }))
                .into_response();
            } else {
                // 大小不同 → 冲突，需要重命名
                println!(
                    "[Web Server] 同名文件大小不同 (已有: {}, 新: {})，触发重命名",
                    existing_size, file_size
                );
            }
        }

        // 需要找一个不冲突的文件名（原名冲突 或 存在 .downloading 残留）
        let mut resolved_name = file_name.clone();
        if candidate.exists() || downloading_path.exists() {
            let mut i = 1usize;
            loop {
                let candidate_name = format!("{}({}){}", stem, i, ext);
                let candidate_path = download_dir.join(&candidate_name);
                let candidate_dl = download_dir.join(format!("{}.downloading", candidate_name));
                if !candidate_path.exists() && !candidate_dl.exists() {
                    resolved_name = candidate_name;
                    println!("[Web Server] 重命名为: {}", resolved_name);
                    break;
                }
                i += 1;
                if i > 9999 {
                    // 极端情况兜底
                    resolved_name = format!("{}_{}{}", stem, chrono::Utc::now().timestamp(), ext);
                    break;
                }
            }
        }

        final_file_name = resolved_name;
    } else {
        // 后续块：从数据库按 sender_msg_id 查询正在下载的文件名（多文件并发隔离）
        let target = if !sender_msg_id.is_empty() {
            crate::db::get_downloading_file_by_sender_msg_id(&state.pool, &sender_msg_id).await
        } else {
            crate::db::get_downloading_file(&state.pool, &sender_id).await
        };
        if file_name.is_empty() {
            match target {
                Ok(Some(name)) => {
                    final_file_name = name;
                    println!(
                        "[Web Server] 后续块从数据库查询到文件名: {}",
                        final_file_name
                    );
                }
                Ok(None) => {
                    // DB 中没有 downloading 记录 → 文件可能已秒传完成
                    // 返回 success 让发送端停止上传
                    println!(
                        "[Web Server] 后续块无可下载记录 (sender_msg_id={})，假定已完成，通知发送端停止",
                        sender_msg_id
                    );
                    return Json(serde_json::json!({
                        "status": "already_exists",
                        "file_name": file_name,
                        "file_size": file_size,
                    }))
                    .into_response();
                }
                Err(e) => {
                    eprintln!("[Web Server] ✗ 数据库查询失败: {}", e);
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            error: format!("数据库查询失败: {}", e),
                        }),
                    )
                        .into_response();
                }
            }
        } else {
            // 发送端传来了 file_name，但后续块应以数据库中的 resolved 名为准
            match target {
                Ok(Some(name)) => final_file_name = name,
                Ok(None) => {
                    // 无可下载记录 → 文件已秒传完成，通知发送端停止
                    return Json(serde_json::json!({
                        "status": "already_exists",
                        "file_name": file_name,
                        "file_size": file_size,
                    }))
                    .into_response();
                }
                _ => final_file_name = file_name.clone(),
            }
        }
    }

    // 临时文件路径（写入期间使用 .downloading 后缀）
    let temp_path = download_dir.join(format!("{}.downloading", final_file_name));
    let final_path = download_dir.join(&final_file_name);

    // 第一块：创建/截断临时文件（不触碰已有的完整文件）
    if chunk_index == 0 {
        println!("[Web Server] 创建临时文件: {:?}", temp_path);
        let _ = fs::remove_file(&temp_path).await;
    }

    if let Some((stream_path, _, first_chunk)) = &streamed_chunk {
        if *first_chunk {
            if let Err(e) = fs::rename(stream_path, &temp_path).await {
                notify_receive_failure(&state, &sender_id, &sender_msg_id).await;
                return (StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse { error: format!("移动接收分块失败: {e}") })).into_response();
            }
        } else if stream_path != &temp_path {
            notify_receive_failure(&state, &sender_id, &sender_msg_id).await;
            return (StatusCode::CONFLICT,
                Json(ErrorResponse { error: "接收文件路径变化".to_string() })).into_response();
        }
    } else {
        // 秒传兼容路径才需要把内存中的块写入临时文件。
        let file = match fs::OpenOptions::new().create(true).append(true).open(&temp_path).await {
            Ok(file) => file,
            Err(e) => {
                notify_receive_failure(&state, &sender_id, &sender_msg_id).await;
                return (StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse { error: format!("打开文件失败: {e}") })).into_response();
            }
        };
        let mut writer = tokio::io::BufWriter::new(file);
        if let Err(e) = tokio::io::AsyncWriteExt::write_all(&mut writer, &chunk_data).await {
            notify_receive_failure(&state, &sender_id, &sender_msg_id).await;
            return (StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse { error: format!("写入文件失败: {e}") })).into_response();
        }
        if let Err(e) = tokio::io::AsyncWriteExt::flush(&mut writer).await {
            notify_receive_failure(&state, &sender_id, &sender_msg_id).await;
            return (StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse { error: format!("刷新文件失败: {e}") })).into_response();
        }
    }

    println!(
        "[Web Server] ✓ 分块 {}/{} 已写入临时文件，大小: {} 字节",
        chunk_index + 1,
        chunk_total,
        chunk_len
    );

    // 第一块时创建/更新数据库记录，并广播初始消息给前端
    if chunk_index == 0 {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let file_path = final_path.to_str().unwrap_or("").to_string();

        // 先检查是否存在 offered 记录（手动下载场景），更新它而非新建
        let updated_offered = crate::db::find_and_update_offered_record(
            &state.pool,
            &sender_id,
            &final_file_name,
            &file_path,
        )
        .await
        .unwrap_or(None);

        let record_id = if let Some((found_id, _)) = updated_offered {
            println!("[Web Server] ✓ 已更新 offered 记录为 downloading，无需新建");
            found_id
        } else {
            // 没有 offered 记录 → 新建接收记录（含 sender_msg_id，用于 DOM 查找）
            match crate::db::create_received_file_record(
                &state.pool,
                sender_id.clone(),
                final_file_name.clone(),
                file_path,
                file_size,
                timestamp,
                &sender_msg_id,
            )
            .await
            {
                Ok(msg_id) => {
                    println!(
                        "[Web Server] ✓ 文件消息已创建，ID: {}, 最终名: {}",
                        msg_id, final_file_name
                    );
                    msg_id
                }
                Err(e) => {
                    eprintln!("[Web Server] ✗ 创建文件消息失败: {}", e);
                    0
                }
            }
        };

        // 查询发送者名称
        let sender_name = state
            .peer_manager
            .get_all_peers()
            .iter()
            .find(|p| p.id == sender_id)
            .map(|p| p.name.clone())
            .unwrap_or_default();

        // 广播初始消息（使前端创建 DOM 元素，data-sender-msg-id 供进度查找）
        if record_id > 0 && !sender_msg_id.is_empty() {
            let init_msg = serde_json::json!({
                "id": record_id,
                "from_id": sender_id,
                "from_name": sender_name,
                "content": final_file_name,
                "msg_type": "file",
                "file_status": "downloading",
                "file_size": file_size,
                "file_name": final_file_name,
                "sender_msg_id": sender_msg_id,
                "timestamp": timestamp,
            });
            let _ = state.ws_broadcast.send(init_msg.to_string());
            state.event_bus.publish_message(init_msg);
        }
    }

    // 元数据确认写入后才公布累计进度；每次重传第一块都会截断临时文件。
    let written_size = tokio::fs::metadata(&temp_path)
        .await
        .map(|m| m.len())
        .unwrap_or(0);
    // 广播接收端累计写入量。
    if !sender_msg_id.is_empty() {
        // 包含 multipart 接收和落盘耗时；首块也能显示实际吞吐。
        let chunk_speed_mb_s = chunk_len as f64
            / (1024.0 * 1024.0)
            / request_started.elapsed().as_secs_f64().max(0.001);
        let progress_msg = serde_json::json!({
            "msg_type": "file_download_progress",
            "sender_msg_id": sender_msg_id,
            "speed_mb_s": if chunk_speed_mb_s.is_finite() { chunk_speed_mb_s } else { speed_mb_s },
            "received": written_size,
            "total": file_size,
        });
        let _ = state.ws_broadcast.send(progress_msg.to_string());
        state.event_bus.publish_message(progress_msg);
    }

    // 检查是否是最后一块
    let temp_size = written_size;

    if temp_size >= file_size && file_size > 0 {
        // 最后一块写完：将临时文件重命名为最终文件（原子操作，绕过 Android 覆盖限制）
        match tokio::fs::rename(&temp_path, &final_path).await {
            Ok(_) => {
                println!(
                    "[Web Server] ✓ 临时文件已重命名为最终文件: {:?}",
                    final_path
                );

                let (stored_path, stored_status, save_error) =
                    resolve_received_file_destination(&state.pool, &final_path, &final_file_name)
                        .await;
                let final_path_string = final_path.to_string_lossy().into_owned();

                match crate::db::update_file_status_by_path(
                    &state.pool,
                    &final_path_string,
                    &stored_path,
                    &stored_status,
                )
                .await
                {
                    Ok(_) => {
                        println!(
                            "[Web Server] ✓ 文件状态已更新为 {}, 路径={}",
                            stored_status, stored_path
                        );

                        // 通知前端刷新（广播完整消息对象，让前端重新渲染）
                        let msg_id = crate::db::get_latest_msg_id_by_file(
                            &state.pool,
                            &sender_id,
                            &final_file_name,
                        )
                        .await
                        .unwrap_or(0);
                        let sender_msg_id = crate::db::get_sender_msg_id_by_file(
                            &state.pool,
                            &sender_id,
                            &final_file_name,
                        )
                        .await
                        .unwrap_or_default();
                        let file_id = std::path::Path::new(&final_file_name)
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or(&final_file_name)
                            .to_string();
                        let sender_name = state
                            .peer_manager
                            .get_all_peers()
                            .iter()
                            .find(|p| p.id == sender_id)
                            .map(|p| p.name.clone())
                            .unwrap_or_default();
                        let full_msg = serde_json::json!({
                            "id": msg_id,
                            "from_id": sender_id,
                            "from_name": sender_name,
                            "content": final_file_name,
                            "msg_type": "file",
                            "file_status": stored_status,
                            "file_path": stored_path,
                            "file_save_error": save_error,
                            "file_size": file_size,
                            "file_id": file_id,
                            "file_name": final_file_name,
                            "sender_msg_id": sender_msg_id,
                            "timestamp": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
                        });
                        let _ = state.ws_broadcast.send(full_msg.to_string());
                        // update_received_file_complete 已提交后才发布文件完成事件。
                        state.event_bus.publish(
                            crate::core_events::CoreEvent::FileTransferCompleted(full_msg),
                        );
                    }
                    Err(e) => {
                        eprintln!("[Web Server] ✗ 更新文件状态失败: {}", e);
                        notify_receive_failure(&state, &sender_id, &sender_msg_id).await;
                        return (StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ErrorResponse { error: format!("更新文件状态失败: {}", e) })).into_response();
                    },
                }
            }
            Err(e) => {
                eprintln!("[Web Server] ✗ 重命名临时文件失败: {}", e);
                notify_receive_failure(&state, &sender_id, &sender_msg_id).await;
                return (StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse { error: format!("保存文件失败: {}", e) })).into_response();
            }
        }
    }

    println!("[Web Server] ========== 文件上传处理完成 ==========");
    Json(serde_json::json!({
        "status": "success",
        "file_name": final_file_name,
        "file_size": file_size,
        "chunk_index": chunk_index,
        "chunk_total": chunk_total,
    }))
    .into_response()
}

// 接受文件（手动接收模式）
#[derive(Deserialize)]
struct AcceptFileRequest {
    save_path: Option<String>,
}

async fn accept_file_http(
    State(state): State<Arc<AppState>>,
    Path(file_id): Path<String>,
    Json(payload): Json<AcceptFileRequest>,
) -> impl IntoResponse {
    println!("[Web Server] ========== 开始处理文件接收 ==========");
    println!("[Web Server] 收到接受文件请求: file_id={}", file_id);
    println!("[Web Server] save_path: {:?}", payload.save_path);

    // 先列出所有文件消息，方便调试
    println!("[Web Server] 查询所有文件消息...");
    if let Ok(rows) = crate::db::get_all_file_messages(&state.pool, 10).await {
        for (id, sender, content, path, status) in rows {
            println!(
                "[Web Server]   ID={}, sender={}, content={}, path={}, status={}",
                id, sender, content, path, status
            );
        }
    }

    // 从数据库查询文件信息 - 使用 file_path LIKE 模糊匹配
    let query_pattern = format!("%{}%", file_id);
    println!("[Web Server] 查询模式: {}", query_pattern);

    let row = match crate::db::get_pending_file_by_path(&state.pool, &query_pattern).await {
        Ok(Some((path, name))) => {
            println!("[Web Server] ✓ 找到匹配的 pending 文件记录");
            (path, name)
        }
        Ok(None) => {
            println!("[Web Server] ✗ 未找到匹配的 pending 文件");
            println!("[Web Server] 可能原因:");
            println!("[Web Server]   1. file_id 不匹配任何文件路径");
            println!("[Web Server]   2. 文件状态不是 'pending'");
            println!("[Web Server]   3. 文件已被接收或删除");
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("文件不存在或已接收 (file_id={})", file_id),
                }),
            )
                .into_response();
        }
        Err(e) => {
            println!("[Web Server] ✗ 数据库查询失败: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("查询文件失败: {}", e),
                }),
            )
                .into_response();
        }
    };

    let temp_path = row.0;
    let file_name = row.1;

    // 检查文件是否存在（可能还在上传中）
    if !std::path::Path::new(&temp_path).exists() {
        println!("[Web Server] ⏳ 文件还在下载中");
        return (
            StatusCode::ACCEPTED, // 202 表示请求已接受但还在处理中
            Json(serde_json::json!({
                "downloading": true,
                "message": "文件正在下载中，请稍候..."
            })),
        )
            .into_response();
    }

    // Android 的公开目标可能是 content URI，实际接收/移动仍在应用专属 staging 中完成。
    #[cfg(target_os = "android")]
    let final_path = get_download_dir(&state.pool).await.join(&file_name);
    #[cfg(not(target_os = "android"))]
    let final_path = if let Some(path) = payload.save_path {
        std::path::PathBuf::from(path).join(&file_name)
    } else {
        get_download_dir(&state.pool).await.join(&file_name)
    };

    println!("[Web Server] 最终路径: {:?}", final_path);

    // 确保目标目录存在
    if let Some(parent) = final_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            println!("[Web Server] ✗ 创建目录失败: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("创建目录失败: {}", e),
                }),
            )
                .into_response();
        }
    }

    // 移动文件 - 使用复制+删除来支持跨文件系统
    println!("[Web Server] 开始移动文件...");
    if let Err(e) = std::fs::rename(&temp_path, &final_path) {
        // rename 失败（可能是跨文件系统），尝试复制+删除
        println!("[Web Server] rename 失败 ({}), 尝试复制+删除", e);

        if let Err(e) = std::fs::copy(&temp_path, &final_path) {
            println!("[Web Server] ✗ 复制文件失败: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("复制文件失败: {}", e),
                }),
            )
                .into_response();
        }

        // 复制成功后删除临时文件
        if let Err(e) = std::fs::remove_file(&temp_path) {
            println!("[Web Server] ⚠ 删除临时文件失败: {}", e);
            // 不返回错误，因为文件已经复制成功了
        }

        println!("[Web Server] ✓ 文件已复制到目标位置");
    } else {
        println!("[Web Server] ✓ 文件已移动到目标位置");
    }

    let (stored_path, stored_status, save_error) =
        resolve_received_file_destination(&state.pool, &final_path, &file_name).await;

    // 更新数据库状态
    if let Err(e) =
        crate::db::update_file_status_by_path(&state.pool, &temp_path, &stored_path, &stored_status)
            .await
    {
        println!("[Web Server] ✗ 更新数据库失败: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("更新数据库失败: {}", e),
            }),
        )
            .into_response();
    }

    println!(
        "[Web Server] ✓ 文件已接受，状态={}, 保存位置={}",
        stored_status, stored_path
    );
    Json(serde_json::json!({
        "success": true,
        "path": stored_path,
        "file_status": stored_status,
        "file_save_error": save_error,
    }))
    .into_response()
}

// 下载文件
async fn download_file_http(
    State(state): State<Arc<AppState>>,
    Path(file_id): Path<String>,
) -> impl IntoResponse {
    println!("[Web Server] 下载文件请求: {}", file_id);

    // 首先尝试从数据库查询文件路径
    // 通过 file_path 的文件名匹配
    let file_path_result = sqlx::query_scalar::<_, String>(
        "SELECT file_path FROM messages 
         WHERE file_path IS NOT NULL 
         AND (file_path LIKE ? OR content = ?)
         ORDER BY timestamp DESC LIMIT 1",
    )
    .bind(format!("%/{}", file_id)) // 匹配路径末尾的文件名
    .bind(&file_id)
    .fetch_optional(&state.pool)
    .await;

    // A received SAF document is already identified by the existing message lookup.
    // Read only that stored URI through the existing permission-aware Android bridge;
    // never treat a content URI as a filesystem path or accept an arbitrary URI here.
    #[cfg(target_os = "android")]
    if let Ok(Some(uri)) = &file_path_result {
        if uri.starts_with("content://") {
            let file = match crate::android_fd::AndroidFile::from_content_uri(uri) {
                Ok(file) => tokio::fs::File::from_std(file.into_file()),
                Err(_) => {
                    return Response::builder()
                        .status(StatusCode::FORBIDDEN)
                        .body(Body::from("文件不可访问或目录权限已失效"))
                        .unwrap();
                }
            };
            return Response::builder()
                .header(
                    header::CONTENT_TYPE,
                    mime_guess::from_path(uri).first_or_octet_stream().as_ref(),
                )
                .header(header::CONTENT_DISPOSITION, "inline")
                .body(Body::from_stream(tokio_util::io::ReaderStream::new(file)))
                .unwrap();
        }
    }

    let file_path = match file_path_result {
        Ok(Some(path)) => {
            println!("[Web Server] 从数据库找到文件路径: {}", path);
            std::path::PathBuf::from(path)
        }
        _ => {
            println!("[Web Server] 数据库中未找到，尝试从下载目录查找");
            // 没有找到记录，尝试从下载目录查找
            let download_dir = get_download_dir(&state.pool).await;
            download_dir.join(&file_id)
        }
    };

    println!("[Web Server] 尝试读取文件: {}", file_path.display());

    match fs::read(&file_path).await {
        Ok(data) => {
            // 根据文件扩展名设置 Content-Type
            let content_type = if file_id.ends_with(".jpg") || file_id.ends_with(".jpeg") {
                "image/jpeg"
            } else if file_id.ends_with(".png") {
                "image/png"
            } else if file_id.ends_with(".gif") {
                "image/gif"
            } else if file_id.ends_with(".webp") {
                "image/webp"
            } else {
                "application/octet-stream"
            };

            println!("[Web Server] 文件读取成功，大小: {} bytes", data.len());

            Response::builder()
                .header(header::CONTENT_TYPE, content_type)
                .header(
                    header::CONTENT_DISPOSITION,
                    format!("inline; filename=\"{}\"", file_id),
                )
                .body(Body::from(data))
                .unwrap()
        }
        Err(e) => {
            eprintln!("[Web Server] 文件不存在: {} - {}", file_path.display(), e);
            Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from(format!(
                    "文件不存在: {} - {}",
                    file_path.display(),
                    e
                )))
                .unwrap()
        }
    }
}

// 获取下载目录
async fn get_download_dir(pool: &Pool<Sqlite>) -> std::path::PathBuf {
    // Android 返回应用专属 staging 目录；桌面端返回用户配置的下载目录。
    match crate::db::get_receive_directory(pool).await {
        Ok(path) => std::path::PathBuf::from(path),
        Err(_) => {
            // 默认路径
            #[cfg(windows)]
            let path = crate::config_file::windows_portable_download_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("downloads"));
            #[cfg(not(windows))]
            let path = std::env::temp_dir().join("lanchat_downloads");
            path
        }
    }
}

async fn export_to_configured_android_directory(
    pool: &Pool<Sqlite>,
    source_path: &std::path::Path,
    file_name: &str,
) -> Result<Option<String>, String> {
    #[cfg(target_os = "android")]
    {
        let tree_uri = crate::db::get_download_path(pool).await?;
        if !tree_uri.starts_with("content://") {
            return Ok(None);
        }

        let source_path = source_path
            .to_str()
            .ok_or_else(|| "接收文件路径不是有效 UTF-8".to_string())?
            .to_string();
        let file_name = file_name.to_string();
        let exported_uri = tokio::task::spawn_blocking(move || {
            crate::android_fd::export_received_file(&tree_uri, &source_path, &file_name)
        })
        .await
        .map_err(|error| format!("Android 文件导出任务失败: {error}"))??;
        println!("[Web Server] ✓ 文件已导出到 Android 系统目录: {exported_uri}");
        Ok(Some(exported_uri))
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = (pool, source_path, file_name);
        Ok(None)
    }
}

/// 将接收完成的 staging 文件映射到聊天记录最终应保存的地址。
/// Android SAF 导出成功时必须持久化系统返回的 content URI；导出失败时保留
/// staging 文件作为可打开/可分享的兜底，避免传输成功却丢失访问入口。
async fn resolve_received_file_destination(
    pool: &Pool<Sqlite>,
    staging_path: &std::path::Path,
    file_name: &str,
) -> (String, String, Option<String>) {
    let staging_path_string = staging_path.to_string_lossy().into_owned();
    match export_to_configured_android_directory(pool, staging_path, file_name).await {
        Ok(Some(exported_uri)) => (exported_uri, "accepted".to_string(), None),
        Ok(None) => (staging_path_string, "accepted".to_string(), None),
        Err(error) => {
            eprintln!(
                "[Web Server] 文件已接收，但导出到配置目录失败；保留 staging 文件: {}",
                error
            );
            (staging_path_string, "save_failed".to_string(), Some(error))
        }
    }
}

// 创建上传记录（Web 端发送文件时）
#[derive(Deserialize)]
struct CreateUploadRecordRequest {
    file_name: String,
    file_size: u64,
    timestamp: i64,
    receiver_id: String,
    auto_download: Option<bool>,
}

async fn create_upload_record_http(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateUploadRecordRequest>,
) -> impl IntoResponse {
    let is_online = state
        .peer_manager
        .get_active_peers()
        .iter()
        .any(|p| p.id == payload.receiver_id);

    let auto_dl = payload.auto_download.unwrap_or(true);
    let (file_status, overall_status) = if is_online {
        if auto_dl {
            ("uploading".to_string(), "sent".to_string())
        } else {
            ("offering".to_string(), "sent".to_string())
        }
    } else {
        ("pending".to_string(), "pending".to_string())
    };

    match crate::db::create_upload_record(
        &state.pool,
        payload.receiver_id.clone(),
        payload.file_name.clone(),
        payload.file_size,
        payload.timestamp,
        file_status.clone(),
        overall_status.clone(),
    )
    .await
    {
        Ok(msg_id) => Json(serde_json::json!({
            "success": true,
            "msg_id": msg_id,
            "is_online": is_online,
            "file_status": file_status,
            "status": overall_status,
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        )
            .into_response(),
    }
}

// 更新上传状态
#[derive(Deserialize)]
struct UpdateUploadStatusRequest {
    file_name: String,
    status: String,
}

async fn update_upload_status_http(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<UpdateUploadStatusRequest>,
) -> impl IntoResponse {
    println!(
        "[Web Server] 更新上传状态: {} -> {}",
        payload.file_name, payload.status
    );

    match crate::db::update_upload_status(
        &state.pool,
        payload.file_name.clone(),
        payload.status.clone(),
    )
    .await
    {
        Ok(_) => {
            println!("[Web Server] ✓ 上传状态已更新");
            Json(serde_json::json!({ "success": true })).into_response()
        }
        Err(e) => {
            eprintln!("[Web Server] ✗ 更新上传状态失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("更新状态失败: {}", e),
                }),
            )
                .into_response()
        }
    }
}

// 秒传完成标记（web 端发送者收到 already_exists 后更新本地状态）
#[derive(Deserialize)]
struct MarkUploadCompleteRequest {
    msg_id: i64,
    status: String,
}

async fn mark_upload_complete_http(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<MarkUploadCompleteRequest>,
) -> impl IntoResponse {
    println!(
        "[Web Server] 标记上传完成: msg_id={}, status={}",
        payload.msg_id, payload.status
    );

    match crate::db::update_file_status_by_id(&state.pool, payload.msg_id, &payload.status).await {
        Ok(_) => Json(serde_json::json!({"success": true})).into_response(),
        Err(e) => {
            eprintln!("[Web Server] ✗ 标记上传完成失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("标记失败: {}", e),
                }),
            )
                .into_response()
        }
    }
}

// 删除上传记录（上传失败时）
#[derive(Deserialize)]
struct DeleteUploadRecordRequest {
    file_name: String,
    timestamp: i64,
}

async fn delete_upload_record_http(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<DeleteUploadRecordRequest>,
) -> impl IntoResponse {
    println!("[Web Server] 删除上传记录: {}", payload.file_name);

    match crate::db::delete_upload_record(&state.pool, payload.file_name.clone(), payload.timestamp)
        .await
    {
        Ok(_) => {
            println!("[Web Server] ✓ 上传记录已删除");
            Json(serde_json::json!({ "success": true })).into_response()
        }
        Err(e) => {
            eprintln!("[Web Server] ✗ 删除上传记录失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("删除记录失败: {}", e),
                }),
            )
                .into_response()
        }
    }
}

// 主题相关的 HTTP 处理函数
async fn get_theme_list_http() -> impl IntoResponse {
    println!("[Web Server] 收到获取主题列表请求");

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
    if let Ok(theme_dir) = crate::config_file::custom_theme_dir() {
        if theme_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&theme_dir) {
                for entry in entries {
                    if let Ok(entry) = entry {
                        let path = entry.path();
                        if path.is_file()
                            && path.extension().and_then(|s| s.to_str()) == Some("css")
                        {
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
    }

    println!("[Web Server] 找到 {} 个主题", themes.len());
    Json(themes).into_response()
}

async fn get_theme_css_http(Path(theme_name): Path<String>) -> impl IntoResponse {
    println!("[Web Server] 收到获取主题CSS请求: {}", theme_name);

    if theme_name == "default" {
        return Response::builder()
            .header(header::CONTENT_TYPE, "text/css")
            .body(Body::from(""))
            .unwrap()
            .into_response();
    }

    // 检查是否是内置主题 vscode
    if theme_name == "vscode" {
        // 从嵌入的资源中读取 vscode.css
        if let Some(content) = Asset::get("css/vscode.css") {
            let css_content = String::from_utf8_lossy(&content.data).to_string();
            println!(
                "[Web Server] 加载内置主题: vscode ({} 字节)",
                css_content.len()
            );
            return Response::builder()
                .header(header::CONTENT_TYPE, "text/css")
                .body(Body::from(css_content))
                .unwrap()
                .into_response();
        } else {
            eprintln!("[Web Server] 内置主题 vscode.css 未找到");
        }
    }

    // 自定义主题从用户目录读取
    if let Ok(theme_dir) = crate::config_file::custom_theme_dir() {
        let theme_path = theme_dir.join(format!("{}.css", theme_name));

        if theme_path.exists() {
            match std::fs::read_to_string(&theme_path) {
                Ok(css_content) => {
                    println!(
                        "[Web Server] 成功读取主题文件: {} ({} 字节)",
                        theme_path.display(),
                        css_content.len()
                    );
                    return Response::builder()
                        .header(header::CONTENT_TYPE, "text/css")
                        .body(Body::from(css_content))
                        .unwrap()
                        .into_response();
                }
                Err(e) => {
                    eprintln!("[Web Server] 读取主题文件失败: {}", e);
                }
            }
        }
    }

    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: format!("主题文件不存在: {}", theme_name),
        }),
    )
        .into_response()
}

#[derive(Deserialize)]
struct SaveThemeRequest {
    theme_name: String,
}

async fn save_current_theme_http(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SaveThemeRequest>,
) -> impl IntoResponse {
    println!("[Web Server] 收到保存主题请求: {}", req.theme_name);

    match crate::db::save_current_theme(&state.pool, req.theme_name.clone()).await {
        Ok(_) => {
            println!("[Web Server] 主题设置已保存: {}", req.theme_name);
            Json(serde_json::json!({"success": true})).into_response()
        }
        Err(e) => {
            eprintln!("[Web Server] 保存主题设置失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("保存主题设置失败: {}", e),
                }),
            )
                .into_response()
        }
    }
}

async fn get_current_theme_http(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    println!("[Web Server] 收到获取当前主题请求");

    match crate::db::get_current_theme(&state.pool).await {
        Ok(result) => {
            let theme = result.unwrap_or_else(|| "default".to_string());
            println!("[Web Server] 当前主题: {}", theme);
            Json(serde_json::json!({"theme": theme})).into_response()
        }
        Err(e) => {
            eprintln!("[Web Server] 查询主题设置失败: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("查询主题设置失败: {}", e),
                }),
            )
                .into_response()
        }
    }
}

// 批量删除消息
async fn delete_messages_http(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let msg_ids: Vec<i64> = match payload.get("msg_ids").and_then(|v| v.as_array()) {
        Some(arr) => arr.iter().filter_map(|v| v.as_i64()).collect(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "缺少 msg_ids 参数".to_string(),
                }),
            )
                .into_response();
        }
    };

    match crate::db::delete_messages_by_ids(&state.pool, msg_ids).await {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "success": true }))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error: e }),
        )
            .into_response(),
    }
}

async fn clear_chat_history_http(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<PeerIdRequest>,
) -> impl IntoResponse {
    println!(
        "[Web Server] 收到清空聊天记录请求: peer_id={}",
        payload.peer_id
    );

    // 获取自己的 ID
    let my_id = match crate::db::get_user_id(&state.pool).await {
        Ok(id) => id,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse { error: e }),
            )
                .into_response()
        }
    };

    match crate::db::clear_chat_history(&state.pool, &my_id, &payload.peer_id).await {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "success": true }))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error: e }),
        )
            .into_response(),
    }
}

async fn delete_user_http(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<PeerIdRequest>,
) -> impl IntoResponse {
    println!("[Web Server] 收到删除用户请求: peer_id={}", payload.peer_id);

    // 获取自己的 ID
    let my_id = match crate::db::get_user_id(&state.pool).await {
        Ok(id) => id,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse { error: e }),
            )
                .into_response()
        }
    };

    match state
        .peer_manager
        .delete_user_and_history(&state.pool, &my_id, &payload.peer_id)
        .await
    {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "success": true }))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error: e }),
        )
            .into_response(),
    }
}

/// 获取媒体访问 Token（供 Tauri command 读取后传给前端）
pub fn get_media_token() -> String {
    MEDIA_TOKEN.lock().unwrap().clone()
}

/// 媒体代理请求参数
#[derive(Deserialize)]
#[allow(dead_code)]
struct MediaQuery {
    uri: String,
    token: String,
}

/// GET /api/media?uri=<content_uri>&token=<token>
/// 仅允许本机（127.0.0.1）访问，并校验 token
#[cfg(target_os = "android")]
async fn serve_media_http(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
    Query(params): Query<MediaQuery>,
) -> impl IntoResponse {
    // 防线 A：IP 白名单，只允许本机回环地址
    if !addr.ip().is_loopback() {
        println!("[Media] 拒绝非本机请求: {}", addr.ip());
        return (StatusCode::FORBIDDEN, "Forbidden").into_response();
    }

    // 防线 B：Token 校验
    if params.token != state.media_token {
        println!("[Media] Token 校验失败");
        return (StatusCode::FORBIDDEN, "Invalid token").into_response();
    }

    println!("[Media] 请求媒体: {}", params.uri);

    // 获取 MIME 类型（用于 Content-Type header）
    let mime = mime_guess::from_path(params.uri.split('/').last().unwrap_or("file"))
        .first_or_octet_stream();

    // 支持 fd: 路径：从 FD 缓存获取文件句柄
    let tokio_file: tokio::fs::File;
    if params.uri.starts_with("fd:") {
        let msg_id_str = &params.uri["fd:".len()..];
        match msg_id_str.parse::<i64>() {
            Ok(msg_id) => match crate::android_fd::duplicate_cached_file(msg_id) {
                Some((f, _name, _size)) => {
                    tokio_file = f;
                }
                None => {
                    println!("[Media] FD 缓存未命中: msg_id={}", msg_id);
                    return (StatusCode::GONE, "FD cache miss").into_response();
                }
            },
            Err(_) => {
                println!("[Media] 无效的 fd: 路径: {}", params.uri);
                return (StatusCode::BAD_REQUEST, "Invalid fd: path").into_response();
            }
        }
    } else {
        // content:// URI：通过 JNI 获取 FD
        use crate::android_fd::AndroidFile;
        let android_file = match AndroidFile::from_content_uri(&params.uri) {
            Ok(f) => f,
            Err(e) => {
                println!("[Media] 获取 FD 失败（权限过期或文件不存在）: {}", e);
                return (StatusCode::FORBIDDEN, e).into_response();
            }
        };
        let std_file = android_file.into_file();
        tokio_file = tokio::fs::File::from_std(std_file);
    }

    // 流式返回
    let stream = tokio_util::io::ReaderStream::new(tokio_file);
    let body = Body::from_stream(stream);

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime.as_ref())
        .header(header::CACHE_CONTROL, "private, max-age=3600")
        .body(body)
        .unwrap()
        .into_response()
}

/// 非 Android 平台的空实现
#[cfg(not(target_os = "android"))]
async fn serve_media_http(
    ConnectInfo(_addr): ConnectInfo<SocketAddr>,
    State(_state): State<Arc<AppState>>,
    Query(_params): Query<MediaQuery>,
) -> impl IntoResponse {
    (StatusCode::NOT_FOUND, "仅 Android 支持").into_response()
}
