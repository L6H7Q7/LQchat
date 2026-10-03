//! Windows publishers retain Toast objects and restore the UI on activation.
use crate::notification_sync::{self, Notification};
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use windows::{
    core::HSTRING,
    Data::Xml::Dom::XmlDocument,
    Foundation::TypedEventHandler,
    UI::Notifications::{NotificationSetting, ToastNotification, ToastNotificationManager},
};

struct Entry {
    message: Notification,
    tag: String,
    _toast: ToastNotification,
}
struct Publisher {
    generation: u64,
    group: String,
    next: u64,
    entries: VecDeque<Entry>,
}
impl Default for Publisher {
    fn default() -> Self {
        Self {
            generation: 0,
            group: uuid::Uuid::new_v4().simple().to_string()[..16].into(),
            next: 0,
            entries: VecDeque::new(),
        }
    }
}
fn publisher() -> &'static Mutex<Publisher> {
    static STATE: OnceLock<Mutex<Publisher>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(Publisher::default()))
}
pub fn clear() {
    *publisher().lock().unwrap() = Publisher::default();
}
fn xml_text(value: &str) -> String {
    value
        .chars()
        .filter(|c| *c == '\n' || *c == '\r' || *c == '\t' || *c >= ' ')
        .collect::<String>()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

// WinRT invokes activation on its own thread. Restore and focus on Tauri's UI thread.
fn activate_window(payload: Option<serde_json::Value>) {
    use tauri::{Emitter, Manager};
    let Some(handle) = crate::CURRENT_APP_HANDLE
        .get()
        .and_then(|slot| slot.read().ok())
        .and_then(|slot| slot.clone())
    else {
        eprintln!("[Notification] 点击通知时窗口句柄不可用");
        return;
    };
    let ui_handle = handle.clone();
    if let Err(error) = handle.run_on_main_thread(move || {
        if let Some(window) = ui_handle.get_webview_window("main") {
            let result = window.show()
                .and_then(|_| window.unminimize())
                .and_then(|_| window.set_focus());
            if let Err(error) = result {
                eprintln!("[Notification] 唤起窗口失败: {error}");
            }
        }
        if let Some(payload) = payload {
            let _ = ui_handle.emit("synced-notification-tapped", payload);
        } else {
            let _ = ui_handle.emit("open-latest-unread", ());
        }
    }) {
        eprintln!("[Notification] 调度通知点击失败: {error}");
    }
}

fn message_toast_xml(title: &str, body: &str, use_system_sound: bool) -> String {
    let audio = if use_system_sound {
        "<audio src=\"ms-winsoundevent:Notification.Default\"/>"
    } else {
        "<audio silent=\"true\"/>"
    };
    format!(
        "<toast duration=\"short\"><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual>{audio}</toast>",
        xml_text(title), xml_text(body)
    )
}

pub fn show_message(title: &str, body: &str, use_system_sound: bool) -> Result<(), String> {
    // Dropping the COM object immediately after Show can lose its activation handler.
    // Keep a bounded queue, including Toasts that moved into Notification Center.
    static TOASTS: OnceLock<Mutex<VecDeque<ToastNotification>>> = OnceLock::new();
    let result = (|| -> windows::core::Result<()> {
        let doc = XmlDocument::new()?;
        doc.LoadXml(&HSTRING::from(message_toast_xml(title, body, use_system_sound)))?;
        let toast = ToastNotification::CreateToastNotification(&doc)?;
        toast.Activated(&TypedEventHandler::new(|_, _| {
            activate_window(None);
            Ok(())
        }))?;
        let notifier = ToastNotificationManager::CreateToastNotifierWithId(
            &HSTRING::from("com.lanchat.app"),
        )?;
        notifier.Show(&toast)?;
        let mut toasts = TOASTS.get_or_init(|| Mutex::new(VecDeque::new())).lock().unwrap();
        toasts.push_back(toast);
        while toasts.len() > 512 {
            toasts.pop_front();
        }
        Ok(())
    })();
    result.map_err(|error| format!("Windows 系统通知发送失败: {error}"))
}
fn display_title(message: &Notification, source: &str) -> String {
    let is_battery = message.package == "com.lanchat.app"
        && message.notification_key.starts_with("lq-battery-")
        && message.title == "电量提醒";
    let title = if is_battery {
        "电量".to_string()
    } else if message.package == "com.lanchat.app"
        && message.notification_key.starts_with("lq-reminder-")
    {
        message.title.trim().to_string()
    } else {
        let app = message.app_name.trim();
        let title = message.title.trim();
        if app.is_empty() {
            title.to_string()
        } else if title.is_empty() || title == app {
            app.to_string()
        } else if title.starts_with(&format!("{app} · ")) {
            title.to_string()
        } else {
            format!("{app} · {title}")
        }
    };
    let source = source.trim();
    if source.is_empty() || title == source || title.starts_with(&format!("{source} · ")) {
        title
    } else {
        format!("{source} · {title}")
    }
}
// Toast images need a local file. This bounded image-only cache contains no notification text.
fn toast_xml(
    message: &Notification,
    source: &str,
    image: Option<&Path>,
    record_id: &str,
) -> String {
    let image = image
        .and_then(|p| reqwest::Url::from_file_path(p).ok())
        .map(|uri| {
            format!(
                "<image src=\"{}\" hint-removeMargin=\"true\"/>",
                xml_text(uri.as_str())
            )
        })
        .unwrap_or_default();
    let time = chrono::DateTime::from_timestamp_millis(message.post_time)
        .map(|t| t.with_timezone(&chrono::Local).format("%H:%M").to_string())
        .unwrap_or_default();
    let title = display_title(message, source);
    format!(concat!("<toast launch=\"--notification-activation history:{}:{}\"><visual><binding template=\"ToastGeneric\"><group>",
        "<subgroup hint-weight=\"1\">{}<text hint-style=\"caption\" hint-align=\"center\" hint-wrap=\"true\" hint-maxLines=\"2\">{}</text></subgroup>",
        "<subgroup hint-weight=\"4\"><text hint-style=\"body\" hint-wrap=\"true\" hint-maxLines=\"2\">{}</text>",
        "<text hint-wrap=\"true\" hint-maxLines=\"4\">{}</text><text hint-style=\"captionSubtle\" hint-align=\"right\">{}</text></subgroup>",
        "</group><text placement=\"attribution\">来自 {}</text></binding></visual></toast>"),
        xml_text(record_id), xml_text(&message.source_device_id), image, xml_text(&message.app_name), xml_text(&title), xml_text(&message.text), time, xml_text(source))
}
fn publish(
    message: &Notification,
    source: &str,
    record_id: &str,
    image: Option<&Path>,
) -> Result<bool, String> {
    let status = crate::core_runtime::CoreRuntime::global().status();
    if status.state != crate::core_events::CoreState::Running {
        return Err("stopped".into());
    }
    static IDENTITY: OnceLock<Result<(), String>> = OnceLock::new();
    IDENTITY
        .get_or_init(crate::commands::ensure_windows_notification_identity)
        .clone()?;
    let notifier =
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from("com.lanchat.app"))
            .map_err(|_| "notifier_unavailable")?;
    if notifier.Setting().map_err(|_| "permission_unknown")? != NotificationSetting::Enabled {
        return Err("permission_blocked".into());
    }
    let mut state = publisher().lock().unwrap();
    if state.generation != status.generation {
        *state = Publisher {
            generation: status.generation,
            ..Publisher::default()
        };
    }
    let index = state
        .entries
        .iter()
        .position(|e| e.message.key() == message.key());
    if let Some(i) = index {
        if state.entries[i].message.same_content(message) {
            let e = state.entries.remove(i).unwrap();
            state.entries.push_back(e);
            return Ok(false);
        }
    }
    let tag = if let Some(i) = index {
        state.entries[i].tag.clone()
    } else {
        state.next = state.next.checked_add(1).ok_or("tag_exhausted")?;
        let tag = state.next.to_string();
        if tag.len() > 16 {
            return Err("tag_exhausted".into());
        }
        tag
    };
    let result = (|| -> windows::core::Result<ToastNotification> {
        let doc = XmlDocument::new()?;
        doc.LoadXml(&HSTRING::from(toast_xml(message, source, image, record_id)))?;
        let toast = ToastNotification::CreateToastNotification(&doc)?;
        let activation_record = record_id.to_string();
        let activation_source = message.source_device_id.clone();
        toast.Activated(&TypedEventHandler::new(move |_, _| {
            activate_window(Some(serde_json::json!({
                "recordId":activation_record,"sourceDeviceId":activation_source
            })));
            Ok(())
        }))?;
        toast.SetGroup(&HSTRING::from(&state.group))?;
        toast.SetTag(&HSTRING::from(&tag))?;
        notifier.Show(&toast)?;
        Ok(toast)
    })();
    let toast = result.map_err(|_| "system_notification_failed")?;
    if let Some(i) = index {
        state.entries.remove(i);
    }
    state.entries.push_back(Entry {
        message: message.clone(),
        tag,
        _toast: toast,
    });
    while state.entries.len() > 512 {
        state.entries.pop_front();
    }
    Ok(true)
}
pub async fn receive(payload: &serde_json::Value, pool: &sqlx::Pool<sqlx::Sqlite>) {
    let id = payload["request_id"].as_str().unwrap_or("");
    if !notification_sync::pending_active(id) {
        return;
    }
    let result = if notification_sync::receive_enabled(pool).await
        && notification_sync::pending_active(id)
    {
        match serde_json::from_value::<Notification>(payload["notification"].clone()) {
            Ok(message) => {
                let image = crate::notification_history::cached_icon_path(
                    pool,
                    message.app_icon.as_deref(),
                )
                .await;
                publish(
                    &message,
                    payload["source_name"]
                        .as_str()
                        .unwrap_or(&message.source_device_id),
                    payload["history_record_id"].as_str().unwrap_or(""),
                    image.as_deref(),
                )
            }
            Err(_) => Err("invalid".into()),
        }
    } else {
        Err("disabled".into())
    };
    let (success, changed, reason) = match result {
        Ok(changed) => (true, changed, None),
        Err(reason) => (false, false, Some(reason)),
    };
    notification_sync::complete(id, success, changed, reason);
}

#[cfg(test)]
mod tests {
    #[test]
    fn message_toast_escapes_content_and_preserves_sound_setting() {
        let xml = super::message_toast_xml("<标题>", "A&B\u{0}", false);
        assert!(xml.contains("<text>&lt;标题&gt;</text><text>A&amp;B</text>"));
        assert!(xml.contains("<audio silent=\"true\"/>"));
        let audible = super::message_toast_xml("标题", "消息", true);
        assert!(audible.contains("ms-winsoundevent:Notification.Default"));
        assert!(!audible.contains("silent="));
    }
    fn sample() -> crate::notification_sync::Notification {
        crate::notification_sync::Notification {
            msg_type: "notification".into(),
            event_id: "event-1".into(),
            source_device_id: "phone-a".into(),
            target_device_id: "pc".into(),
            package: "app.chat".into(),
            app_name: "微信".into(),
            app_icon: None,
            title: "张三".into(),
            text: "你好".into(),
            notification_key: "key-1".into(),
            post_time: 1,
        }
    }

    #[test]
    fn notification_content_cannot_inject_toast_xml() {
        assert_eq!(super::xml_text("<&\"'\u{0}>"), "&lt;&amp;&quot;&apos;&gt;");
    }

    #[test]
    fn displayed_title_starts_with_source_and_avoids_battery_duplication() {
        let mut message = sample();
        assert_eq!(
            super::display_title(&message, " IQOO "),
            "IQOO · 微信 · 张三"
        );
        assert_eq!(super::display_title(&message, "  "), "微信 · 张三");

        message.title.clear();
        assert_eq!(super::display_title(&message, "IQOO"), "IQOO · 微信");
        message.title = "微信".into();
        assert_eq!(super::display_title(&message, "IQOO"), "IQOO · 微信");
        message.title = "微信 · 张三".into();
        assert_eq!(super::display_title(&message, "IQOO"), "IQOO · 微信 · 张三");

        message.package = "com.lanchat.app".into();
        message.title = "电量提醒".into();
        message.notification_key = "lq-battery-50-123-0".into();
        assert_eq!(super::display_title(&message, "IQOO"), "IQOO · 电量");
        message.app_name = "LQChat".into();
        message.title = "收菜".into();
        message.notification_key = "lq-reminder-example-123-1".into();
        assert_eq!(super::display_title(&message, "IQOO"), "IQOO · 收菜");
        assert_eq!(super::display_title(&message, "  "), "收菜");
    }
}
