// src/main.rs
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use clap::Parser;
use lanchat::db;
use lanchat::peers::PeerManager;
use std::sync::Arc;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{TrayIconBuilder, TrayIconEvent},
    Emitter, Manager,
};

#[derive(Parser, Debug)]
struct Args {
    #[arg(short, long)]
    port: Option<u16>,
    #[arg(long)]
    db_path: Option<String>,
    #[arg(long, hide = true)]
    autostart: bool,
    #[arg(long, hide = true)]
    notification_activation: Option<String>,
}

fn initial_main_window_visible(start_minimized: bool, launched_from_notification: bool) -> bool {
    launched_from_notification || !start_minimized
}

fn main() {
    // Workaround: WebKitGTK DMABUF renderer + NVIDIA + Wayland 导致
    // Gdk-Message: Error 71 (protocol error) dispatching to Wayland display.
    // 见 https://v2.tauri.app/develop/debug/linux-graphics/
    #[cfg(target_os = "linux")]
    std::env::set_var("__NV_DISABLE_EXPLICIT_SYNC", "1");

    #[cfg(windows)]
    lanchat::config_file::prepare_windows_portable_layout().expect("无法初始化 LQChat 便携目录");

    #[cfg(windows)]
    if let Err(error) = lanchat::commands::ensure_windows_notification_identity() {
        eprintln!("[Notification] {error}");
    }

    let args = Args::parse();

    let launched_from_notification = args.notification_activation.is_some();

    #[cfg(windows)]
    if let Some(argument) = args.notification_activation.as_deref() {
        let _ = lanchat::notification_sync::store_windows_activation(argument);
    }

    let cli_port = args.port;
    let cli_db_path = args.db_path;
    let start_minimized = lanchat::config_file::get_start_minimized_from_config();
    let main_window_starts_visible =
        initial_main_window_visible(start_minimized, launched_from_notification);

    let mut context = tauri::generate_context!();
    #[cfg(windows)]
    for window in &mut context.config_mut().app.windows {
        if window.label == "main" {
            // Windows WebView must be created manually so its absolute data directory can
            // stay beside LQChat.exe instead of falling back to LocalAppData.
            window.create = false;
        }
    }

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(
            move |app, args, _cwd| {
                if args.iter().any(|arg| arg == "--autostart") {
                    return;
                }
                let mut launched_from_notification = false;
                #[cfg(windows)]
                {
                    if let Some(argument) = args
                        .windows(2)
                        .find(|pair| pair[0] == "--notification-activation")
                        .map(|pair| pair[1].as_str())
                    {
                        if let Some(payload) =
                            lanchat::notification_sync::store_windows_activation(argument)
                        {
                            let _ = app.emit("synced-notification-tapped", payload);
                        }
                        launched_from_notification = true;
                    }
                }
                // 双击启动也遵循“启动时自动缩小”；通知点击仍必须打开窗口。
                if start_minimized && !launched_from_notification {
                    return;
                }
                // 当尝试启动第二个实例时，显示已存在的窗口
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                    let _ = window.unminimize();
                }
            },
        ))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_sql::Builder::default().build())
        .plugin(tauri_plugin_opener::init());

    #[cfg(windows)]
    let builder = builder.plugin(tauri_plugin_autostart::init(
        tauri_plugin_autostart::MacosLauncher::LaunchAgent,
        Some(vec!["--autostart"]),
    ));

    #[cfg(any(windows, target_os = "macos", target_os = "android"))]
    let builder = builder.plugin(tauri_plugin_notification::init());

    builder
        .invoke_handler(tauri::generate_handler![
            lanchat::notification_sync::notification_settings,
            lanchat::notification_sync::notification_records,
            lanchat::notification_sync::notification_record,
            lanchat::notification_sync::notification_pending_activation,
            lanchat::notification_sync::notification_action,
            lanchat::notification_sync::notification_test,
            lanchat::commands::close_android_fd,
            lanchat::commands::get_my_name,
            lanchat::commands::get_my_id,
            lanchat::commands::update_my_name,
            lanchat::commands::get_peers,
            lanchat::commands::send_message,
            lanchat::commands::get_chat_history,
            lanchat::commands::get_chat_history_with_offset,
            lanchat::commands::send_file,
            lanchat::commands::get_settings,
            lanchat::commands::update_settings,
            lanchat::commands::get_language,
            lanchat::commands::set_language,
            lanchat::commands::get_theme_list,
            lanchat::commands::get_theme_css,
            lanchat::commands::save_current_theme,
            lanchat::commands::get_current_theme,
            lanchat::commands::get_default_download_path,
            lanchat::commands::request_storage_permission,
            lanchat::commands::save_file_message,
            lanchat::commands::open_file_location,
            lanchat::commands::set_android_shared_files,
            lanchat::commands::get_android_shared_files,
            lanchat::commands::clear_android_shared_files,
            lanchat::commands::send_file_from_fd,
            lanchat::commands::share_file_to_other_app,
            lanchat::commands::open_file_in_android,
            lanchat::commands::get_android_file_state,
            lanchat::commands::get_media_token,
            lanchat::commands::read_clipboard_files,
            lanchat::commands::delete_messages,
            lanchat::commands::clear_chat_history,
            lanchat::commands::delete_user_complete,
            lanchat::commands::clear_offline_peers,
            lanchat::commands::get_custom_peers,
            lanchat::commands::add_custom_peer,
            lanchat::commands::remove_custom_peer,
            lanchat::commands::show_notification,
            lanchat::commands::clear_notification,
            lanchat::commands::request_file,
            lanchat::commands::get_notifications_enabled,
            lanchat::commands::set_notifications_enabled,
            lanchat::commands::get_notification_sound_enabled,
            lanchat::commands::set_notification_sound_enabled,
            lanchat::commands::get_background_receive_state,
            lanchat::commands::get_background_runtime_settings,
            lanchat::commands::custom_reminder_settings,
            lanchat::commands::set_background_runtime_settings,
            lanchat::commands::retry_background_service,
            lanchat::commands::stop_background_receive_and_exit,
            lanchat::commands::get_battery_optimization_state,
            lanchat::commands::open_battery_optimization_settings,
            lanchat::commands::get_notification_permission_state,
            lanchat::commands::open_saf_picker,
            lanchat::commands::open_saf_multi_picker,
            lanchat::commands::load_android_media_images,
            lanchat::commands::load_android_apps,
            lanchat::commands::get_local_device_info,
            lanchat::commands::start_tray_flash,
            lanchat::commands::stop_tray_flash,
            lanchat::commands::request_permission_on_android,
            lanchat::commands::get_autostart_enabled,
            lanchat::commands::set_autostart_enabled,
        ])
        .setup(move |app| {
            #[cfg(windows)]
            {
                let window_config = app
                    .config()
                    .app
                    .windows
                    .iter()
                    .find(|window| window.label == "main")
                    .cloned()
                    .ok_or_else(|| std::io::Error::other("缺少 LQChat 主窗口配置"))?;
                let webview_dir = lanchat::config_file::windows_portable_webview_dir()
                    .map_err(std::io::Error::other)?;
                tauri::WebviewWindowBuilder::from_config(app.handle(), &window_config)?
                    // A minimized launch must create the native window hidden. Hiding it after
                    // build() is too late and briefly flashes an unpainted frame on Windows.
                    .visible(main_window_starts_visible)
                    .data_directory(webview_dir)
                    .build()?;
            }

            let handle = app.handle().clone();

            // 获取主窗口并设置关闭事件处理
            if let Some(window) = app.get_webview_window("main") {
                if !main_window_starts_visible {
                    let _ = window.hide();
                }
                let window_clone = window.clone();
                window.on_window_event(move |event| {
                    match event {
                        tauri::WindowEvent::CloseRequested { api, .. } => {
                            if lanchat::config_file::get_close_to_tray_from_config() {
                                // 开关开启：阻止默认关闭并隐藏到托盘。
                                api.prevent_close();
                                let _ = window_clone.hide();
                            } else {
                                // 开关关闭：结束窗口、托盘和后台服务。
                                window_clone.app_handle().exit(0);
                            }
                        }
                        tauri::WindowEvent::Focused(true) => {
                            // Windows 原生通知被点击并聚焦窗口时，跳到最近未读会话。
                            let _ = window_clone.emit("open-latest-unread", ());
                        }
                        _ => {}
                    }
                });
            }

            // 先初始化 DB，再创建托盘（托盘菜单需要读 DB）
            let db_dir = cli_db_path.or_else(|| {
                let cfg = lanchat::config_file::read_config();
                cfg.db_path.as_ref().map(|p| {
                    lanchat::config_file::resolve_db_dir(p)
                        .to_string_lossy()
                        .to_string()
                })
            });

            let pool = tauri::async_runtime::block_on(async {
                if let Some(dir) = db_dir {
                    lanchat::db::init_db_standalone(Some(std::path::PathBuf::from(dir)))
                        .await
                        .expect("DB error")
                } else {
                    db::init_db(&handle).await.expect("DB error")
                }
            });

            let my_name = tauri::async_runtime::block_on(async {
                db::get_username(&pool)
                    .await
                    .unwrap_or_else(|_| "Unknown".into())
            });

            let my_id = tauri::async_runtime::block_on(async {
                db::get_user_id(&pool).await.expect("无法获取或生成用户 ID")
            });

            let port: u16 = cli_port
                .unwrap_or_else(|| lanchat::config_file::get_port_from_config().unwrap_or(8888));

            let notif_enabled = tauri::async_runtime::block_on(async {
                lanchat::db::get_notifications_enabled(&pool).await
            });

            handle.manage(db::DbState { pool: pool.clone() });
            handle.manage(lanchat::commands::TrayFlashState::default());
            println!("[Main] 我的用户名: {}", my_name);
            println!("[Main] 我的 ID: {}", my_id);
            println!("[Main] 服务端口: {}", port);

            // 读取语言设置用于托盘菜单
            let tray_lang = lanchat::config_file::get_lang_from_config()
                .or_else(|| {
                    std::env::var("LANG").ok().map(|l| {
                        if l.starts_with("zh") {
                            "zh".to_string()
                        } else {
                            "en".to_string()
                        }
                    })
                })
                .unwrap_or_else(|| "zh".to_string());
            let (show_text, notif_text, quit_text) = if tray_lang == "en" {
                ("Show Window", "Enable Notifications", "Quit")
            } else {
                ("显示窗口", "开启通知", "退出")
            };

            // 创建托盘菜单（现在 DB 已就绪）
            let show_item = MenuItem::with_id(app, "show", show_text, true, None::<&str>)?;
            let toggle_notif = tauri::menu::CheckMenuItem::with_id(
                app,
                "toggle_notif",
                notif_text,
                true,
                notif_enabled,
                None::<&str>,
            )?;
            let toggle_notif_clone = toggle_notif.clone();
            let notif_enabled_atomic =
                std::sync::Arc::new(std::sync::atomic::AtomicBool::new(notif_enabled));
            let quit_item = MenuItem::with_id(app, "quit", quit_text, true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_item, &toggle_notif, &quit_item])?;

            // 注册托盘菜单项到状态（供 set_language 热更新）
            #[cfg(all(feature = "desktop", not(target_os = "android")))]
            handle.manage(lanchat::commands::TrayMenuItems {
                show_item: show_item.clone(),
                toggle_notif: toggle_notif.clone(),
                quit_item: quit_item.clone(),
            });

            // 创建托盘图标
            let _tray = TrayIconBuilder::with_id("main")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("LQChat")
                .on_menu_event(move |app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                        }
                        let _ = app.emit("open-latest-unread", ());
                    }
                    "toggle_notif" => {
                        let current =
                            notif_enabled_atomic.load(std::sync::atomic::Ordering::Relaxed);
                        let new_state = !current;
                        notif_enabled_atomic.store(new_state, std::sync::atomic::Ordering::Relaxed);
                        let _ = toggle_notif_clone.set_checked(new_state);
                        // 持久化到 DB
                        let pool = {
                            let state = app.state::<lanchat::db::DbState>();
                            state.pool.clone()
                        };
                        tauri::async_runtime::block_on(async {
                            let _ = lanchat::db::set_notifications_enabled(&pool, new_state).await;
                        });
                        // 通知 JS 刷新缓存
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.emit("notifications-changed", new_state);
                        }
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button,
                        button_state,
                        ..
                    } = event
                    {
                        if button == tauri::tray::MouseButton::Left
                            && button_state == tauri::tray::MouseButtonState::Up
                        {
                            let app = tray.app_handle();
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.unminimize();
                                let _ = window.set_focus();
                            }
                            let _ = app.emit("open-latest-unread", ());
                        }
                    }
                })
                .build(app)?;

            // 创建 PeerManager 并启动服务（DB 已就绪，无需再次 block_on）
            let peer_manager = Arc::new(PeerManager::new());
            let _ = tauri::async_runtime::block_on(async {
                if let Err(e) = peer_manager.load_from_db(&pool).await {
                    eprintln!("[Main] 加载历史用户失败: {}", e);
                }
                Ok::<(), ()>(())
            });
            #[cfg(windows)]
            peer_manager.enable_windows_persistence(pool.clone());
            handle.manage(lanchat::commands::PeerState {
                manager: peer_manager.clone(),
            });
            handle.manage(lanchat::commands::AndroidShareState::new());

            // 桌面应用生命周期持有同一 CoreRuntime；网络层不直接持有 AppHandle。
            let core = lanchat::core_runtime::CoreRuntime::global();
            core.prepare(pool.clone(), peer_manager.clone());
            tauri::async_runtime::block_on(async {
                let event_bus = core.event_bus();
                let mut ui_events = event_bus.subscribe();
                let ui_handle = handle.clone();
                let notification_pool = pool.clone();
                tokio::spawn(async move {
                    while let Ok(event) = ui_events.recv().await {
                        lanchat::notification_sync::on_state_event(&event);
                        #[cfg(windows)]
                        if let lanchat::core_events::CoreEvent::NotificationReceived(payload) =
                            &event
                        {
                            lanchat::notification_windows::receive(payload, &notification_pool)
                                .await;
                        }
                        if let Some((name, payload)) = event.ui_event() {
                            let _ = ui_handle.emit(name, payload);
                        }
                        #[cfg(windows)]
                        if let Some((title, body)) =
                            lanchat::commands::windows_notification_for_core_event(&event)
                        {
                            let notifications_enabled =
                                lanchat::db::get_notifications_enabled(&notification_pool).await;
                            let uses_embedded_sound =
                                lanchat::commands::windows_notification_sound_for_core_event(
                                    &event,
                                );
                            if notifications_enabled
                                && uses_embedded_sound
                                && lanchat::db::get_notification_sound_enabled(&notification_pool)
                                    .await
                            {
                                if let Err(error) =
                                    lanchat::commands::play_windows_notification_sound()
                                {
                                    eprintln!("[NotificationSound] {error}");
                                }
                            }
                            let window_focused = ui_handle
                                .get_webview_window("main")
                                .and_then(|window| window.is_focused().ok())
                                .unwrap_or(false);
                            if !window_focused && notifications_enabled {
                                let result = if uses_embedded_sound {
                                    lanchat::commands::show_windows_system_notification_silent(
                                        &title, &body,
                                    )
                                } else {
                                    lanchat::commands::show_windows_system_notification(
                                        &title, &body,
                                    )
                                };
                                match result {
                                    Ok(()) => println!("[Notification] Windows 系统通知已发送"),
                                    Err(error) => eprintln!("[Notification] {error}"),
                                }
                            }
                        }
                    }
                });
            });
            core.start_with_port(std::path::PathBuf::new(), Some(port))
                .map_err(std::io::Error::other)?;

            Ok(())
        })
        .build(context)
        .expect("error while building tauri application")
        .run(|_app, event| {
            #[cfg(windows)]
            if matches!(event, tauri::RunEvent::ExitRequested { .. }) {
                let report = lanchat::core_runtime::CoreRuntime::global().stop_report();
                if !report.ok {
                    eprintln!(
                        "[Shutdown] {}",
                        report.error_message.as_deref().unwrap_or("核心停止失败")
                    );
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::initial_main_window_visible;

    #[test]
    fn minimized_setting_hides_the_initial_window_for_all_regular_launches() {
        assert!(!initial_main_window_visible(true, false));
    }

    #[test]
    fn disabled_minimized_setting_shows_the_initial_window() {
        assert!(initial_main_window_visible(false, false));
    }

    #[test]
    fn notification_activation_always_shows_the_window() {
        assert!(initial_main_window_visible(true, true));
    }
}
