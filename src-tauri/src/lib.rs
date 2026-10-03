// lib.rs
#[cfg(feature = "desktop")]
pub mod commands;

#[cfg(feature = "desktop")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(feature = "desktop")]
use std::sync::{Arc, Mutex, OnceLock, RwLock};
#[cfg(feature = "desktop")]
use tauri::AppHandle;

/// 可替换的当前 UI 句柄，仅供 Activity 相关桥接使用；网络核心不持有它。
#[cfg(feature = "desktop")]
pub static CURRENT_APP_HANDLE: OnceLock<RwLock<Option<AppHandle>>> = OnceLock::new();
#[cfg(feature = "desktop")]
struct UiEventBridge {
    cancelled: Arc<AtomicBool>,
    thread: std::thread::JoinHandle<()>,
}

#[cfg(feature = "desktop")]
static UI_EVENT_BRIDGE: OnceLock<Mutex<Option<UiEventBridge>>> = OnceLock::new();

#[cfg(feature = "desktop")]
fn ui_event_bridge_slot() -> &'static Mutex<Option<UiEventBridge>> {
    UI_EVENT_BRIDGE.get_or_init(|| Mutex::new(None))
}

#[cfg(feature = "desktop")]
pub fn stop_ui_event_bridge() {
    let existing = ui_event_bridge_slot()
        .lock()
        .ok()
        .and_then(|mut slot| slot.take());
    if let Some(existing) = existing {
        existing.cancelled.store(true, Ordering::Release);
        core_runtime::CoreRuntime::global().event_bus().publish(
            core_events::CoreEvent::CoreStateChanged(core_runtime::CoreRuntime::global().status()),
        );
        let _ = existing.thread.join();
    }
}

#[cfg(feature = "desktop")]
pub fn start_ui_event_bridge() {
    let Ok(mut slot) = ui_event_bridge_slot().lock() else {
        return;
    };
    if slot.is_some() {
        return;
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    let thread_cancelled = cancelled.clone();
    let mut core_events = core_runtime::CoreRuntime::global().event_bus().subscribe();
    let thread = std::thread::spawn(move || {
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
        else {
            return;
        };
        runtime.block_on(async move {
            while !thread_cancelled.load(Ordering::Acquire) {
                let event = tokio::select! {
                    event = core_events.recv() => event.ok(),
                    _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => None,
                };
                let Some(event) = event else {
                    continue;
                };
                if thread_cancelled.load(Ordering::Acquire) {
                    break;
                }
                let Some((name, payload)) = event.ui_event() else {
                    continue;
                };
                #[cfg(target_os = "android")]
                if !android_service::ui_visible() {
                    continue;
                }
                let Some(ui_handle) = CURRENT_APP_HANDLE
                    .get()
                    .and_then(|handle| handle.read().ok())
                    .and_then(|handle| handle.clone())
                else {
                    continue;
                };
                use tauri::Emitter;
                let _ = ui_handle.emit(name, payload);
            }
        });
    });
    *slot = Some(UiEventBridge { cancelled, thread });
}

pub mod android_fd;
pub mod android_service;
pub mod config_file;
pub mod core_events;
pub mod core_runtime;
pub mod db;
pub mod models;
pub mod file_source;
pub mod network;
#[cfg(target_os = "android")]
pub mod notification_android;
pub mod notification_history;
pub mod notification_icon;
pub mod notification_sync;
#[cfg(all(windows, feature = "desktop"))]
pub mod notification_windows;
#[cfg(windows)]
pub mod peer_persistence;
pub mod peers;
pub mod utils;
pub mod web_server;

// 仅在桌面端编译时包含 Tauri 运行函数
#[cfg(feature = "desktop")]
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    use std::sync::Arc;
    use tauri::Manager;

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_sql::Builder::default().build());

    #[cfg(any(windows, target_os = "macos", target_os = "android"))]
    let builder = builder.plugin(tauri_plugin_notification::init());

    builder
        .invoke_handler(tauri::generate_handler![
            notification_sync::notification_settings,
            notification_sync::notification_records,
            notification_sync::notification_record,
            notification_sync::notification_pending_activation,
            notification_sync::notification_action,
            notification_sync::notification_test,
            commands::close_android_fd,
            commands::get_my_name,
            commands::get_my_id,
            commands::update_my_name,
            commands::get_peers,
            commands::send_message,
            commands::get_chat_history,
            commands::get_chat_history_with_offset,
            commands::send_file,
            commands::get_settings,
            commands::update_settings,
            commands::get_language,
            commands::set_language,
            commands::get_theme_list,
            commands::get_theme_css,
            commands::save_current_theme,
            commands::get_current_theme,
            commands::get_default_download_path,
            commands::request_storage_permission,
            commands::save_file_message,
            commands::open_file_location,
            commands::set_android_shared_files,
            commands::get_android_shared_files,
            commands::clear_android_shared_files,
            commands::send_file_from_fd,
            commands::share_file_to_other_app,
            commands::open_file_in_android,
            commands::get_android_file_state,
            commands::get_media_token,
            commands::delete_messages,
            commands::clear_chat_history,
            commands::request_file,
            commands::delete_user_complete,
            commands::clear_offline_peers,
            commands::get_custom_peers,
            commands::add_custom_peer,
            commands::remove_custom_peer,
            commands::show_notification,
            commands::clear_notification,
            commands::get_notifications_enabled,
            commands::set_notifications_enabled,
            commands::get_notification_sound_enabled,
            commands::set_notification_sound_enabled,
            commands::get_background_receive_state,
            commands::get_background_runtime_settings,
            commands::custom_reminder_settings,
            commands::set_background_runtime_settings,
            commands::retry_background_service,
            commands::stop_background_receive_and_exit,
            commands::get_battery_optimization_state,
            commands::open_battery_optimization_settings,
            commands::get_notification_permission_state,
            commands::request_permission_on_android,
            commands::get_autostart_enabled,
            commands::set_autostart_enabled,
            commands::start_tray_flash,
            commands::stop_tray_flash,
            commands::open_saf_picker,
            commands::open_saf_multi_picker,
            commands::load_android_media_images,
            commands::load_android_apps,
            commands::get_local_device_info,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            *CURRENT_APP_HANDLE
                .get_or_init(|| RwLock::new(None))
                .write()
                .expect("UI handle lock poisoned") = Some(handle.clone());

            tauri::async_runtime::block_on(async move {
                println!("[Lib] 正在初始化数据库...");
                let core = core_runtime::CoreRuntime::global();

                // Activity/UI 可以在同一 Android 进程内重建，而前台 Service
                // 与 CoreRuntime 继续运行。优先复用核心已经持有的数据库与
                // PeerManager，禁止 UI 再创建一套会逐渐过期的设备状态。
                #[cfg(target_os = "android")]
                let existing_core_resources = core.shared_resources();

                #[cfg(target_os = "android")]
                let pool = match existing_core_resources.as_ref() {
                    Some((pool, _)) => {
                        println!("[Lib] 复用运行中 CoreRuntime 的数据库连接");
                        pool.clone()
                    }
                    None => db::init_db(&handle).await.expect("DB error"),
                };
                #[cfg(not(target_os = "android"))]
                let pool = db::init_db(&handle).await.expect("DB error");
                let my_name = db::get_username(&pool)
                    .await
                    .unwrap_or_else(|_| "Unknown".into());

                let my_id = db::get_user_id(&pool).await.expect("无法获取或生成用户 ID");

                // 读端口：Android 端走数据库，其他平台走配置文件
                #[cfg(target_os = "android")]
                let port: u16 = crate::db::get_port(&pool).await.unwrap_or(8888);
                #[cfg(not(target_os = "android"))]
                let port: u16 = crate::config_file::get_port_from_config().unwrap_or(8888);
                println!("[Lib] 服务端口: {}", port);

                handle.manage(db::DbState { pool: pool.clone() });
                println!("[Lib] 我的用户名: {}", my_name);
                println!("[Lib] 我的 ID: {}", my_id);

                #[cfg(target_os = "android")]
                let peer_manager = match existing_core_resources {
                    Some((_, manager)) => {
                        println!("[Lib] 复用运行中 CoreRuntime 的 PeerManager");
                        manager
                    }
                    None => {
                        let manager = Arc::new(peers::PeerManager::new());
                        if let Err(e) = manager.load_from_db(&pool).await {
                            eprintln!("[Lib] 加载历史用户失败: {}", e);
                        }
                        manager
                    }
                };
                #[cfg(not(target_os = "android"))]
                let peer_manager = {
                    let manager = Arc::new(peers::PeerManager::new());
                    if let Err(e) = manager.load_from_db(&pool).await {
                        eprintln!("[Lib] 加载历史用户失败: {}", e);
                    }
                    manager
                };

                // 将 PeerManager 注册到 Tauri 状态管理
                handle.manage(commands::PeerState {
                    manager: peer_manager.clone(),
                });

                core.prepare(pool.clone(), peer_manager.clone());
                start_ui_event_bridge();

                // 注册 Android 分享状态
                handle.manage(commands::AndroidShareState::new());
                handle.manage(commands::TrayFlashState::default());

                #[cfg(not(target_os = "android"))]
                let event_bus = core.event_bus();
                #[cfg(not(target_os = "android"))]
                let cancellation = tokio_util::sync::CancellationToken::new();
                #[cfg(not(target_os = "android"))]
                let id1 = my_id.clone();
                #[cfg(not(target_os = "android"))]
                let name1 = my_name.clone();
                #[cfg(not(target_os = "android"))]
                let peer_manager_clone = peer_manager.clone();
                #[cfg(not(target_os = "android"))]
                let pool_for_discovery = pool.clone();
                #[cfg(not(target_os = "android"))]
                let listener_bus = event_bus.clone();
                #[cfg(not(target_os = "android"))]
                let listener_cancellation = cancellation.child_token();
                #[cfg(not(target_os = "android"))]
                tokio::spawn(async move {
                    println!("[Lib] 开启监听线程...");
                    let _ = network::discovery::start_listening(
                        port,
                        id1,
                        name1,
                        peer_manager_clone,
                        pool_for_discovery,
                        listener_bus,
                        listener_cancellation,
                    )
                    .await;
                });

                #[cfg(not(target_os = "android"))]
                let id2 = my_id.clone();
                #[cfg(not(target_os = "android"))]
                let pool2 = pool.clone();
                #[cfg(not(target_os = "android"))]
                let announcer_cancellation = cancellation.child_token();
                #[cfg(not(target_os = "android"))]
                tokio::spawn(async move {
                    println!("[Lib] 开启广播线程...");
                    let _ = network::discovery::start_announcing(
                        port,
                        id2,
                        pool2,
                        announcer_cancellation,
                    )
                    .await;
                });

                // 启动 HTTP 服务器（用于接收文件和 WebSocket 消息）
                #[cfg(not(target_os = "android"))]
                let pool_clone = pool.clone();
                #[cfg(not(target_os = "android"))]
                let peer_manager_clone = peer_manager.clone();
                #[cfg(not(target_os = "android"))]
                let server_bus = event_bus.clone();
                #[cfg(not(target_os = "android"))]
                let server_cancellation = cancellation.child_token();
                #[cfg(not(target_os = "android"))]
                tokio::spawn(async move {
                    println!("[Lib] 启动 HTTP 服务器在端口 {}...", port);
                    let _ = web_server::start_server(
                        port,
                        port,
                        pool_clone,
                        peer_manager_clone,
                        server_bus,
                        server_cancellation,
                    )
                    .await;
                });
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
