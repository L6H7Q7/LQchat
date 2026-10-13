//! UI settings use the already-initialized Activity. NLS never calls this bridge.
use jni::{
    objects::{JObject, JString, JValue},
    sys::jboolean,
    JNIEnv, JavaVM,
};

pub fn settings(input: serde_json::Value) -> Result<serde_json::Value, String> {
    let context = ndk_context::android_context();
    let vm = unsafe { JavaVM::from_raw(context.vm().cast()) }.map_err(|_| "JNI 未就绪")?;
    let mut env = vm.attach_current_thread().map_err(|_| "JNI 未就绪")?;
    // Borrow the existing Activity reference; never delete the runtime-owned global.
    let activity = unsafe { JObject::from_raw(context.context().cast()) };
    let arg = JObject::from(
        env.new_string(input.to_string())
            .map_err(|_| "设置参数无效")?,
    );
    let value = env.call_method(
        &activity,
        "notificationSettings",
        "(Ljava/lang/String;)Ljava/lang/String;",
        &[JValue::Object(&arg)],
    );
    let value = match value {
        Ok(value) => value.l().map_err(|_| "设置结果无效")?,
        Err(_) => {
            let _ = env.exception_clear();
            return Err("Android 设置操作失败".into());
        }
    };
    let value = JString::from(value);
    let text: String = env.get_string(&value).map_err(|_| "设置结果无效")?.into();
    let result: serde_json::Value = serde_json::from_str(&text).map_err(|_| "设置结果无效")?;
    if let Some(error) = result["error"].as_str() {
        return Err(error.into());
    }
    Ok(result)
}

#[no_mangle]
pub extern "system" fn Java_com_lanchat_app_NotificationSyncNative_send(
    mut env: JNIEnv,
    _: JObject,
    data: JString,
) {
    let Ok(data) = env.get_string(&data) else {
        return;
    };
    let data: String = data.into();
    if data.len() > 64 * 1024 {
        return;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&data) else {
        return;
    };
    let (Ok(notification), Ok(settings)) = (
        serde_json::from_value(value["notification"].clone()),
        serde_json::from_value(value["settings"].clone()),
    ) else {
        return;
    };
    crate::notification_sync::dispatch(notification, settings);
}
#[no_mangle]
pub extern "system" fn Java_com_lanchat_app_NotificationSyncNative_recordRoute(
    mut env: JNIEnv,
    _: JObject,
    data: JString,
) {
    let Ok(data) = env.get_string(&data) else {
        return;
    };
    let data: String = data.into();
    if data.len() > 4096 {
        return;
    }
    if let Ok(event) = serde_json::from_str(&data) {
        crate::notification_sync::dispatch_route_history(event);
    }
}

#[no_mangle]
pub extern "system" fn Java_com_lanchat_app_NotificationSyncNative_localHistoryChanged(
    _: JNIEnv, _: JObject,
) {
    crate::core_runtime::CoreRuntime::global().event_bus()
        .publish(crate::core_events::CoreEvent::NotificationRecordsChanged);
}

#[no_mangle]
pub extern "system" fn Java_com_lanchat_app_NotificationSyncNative_pending(
    mut env: JNIEnv,
    _: JObject,
    id: JString,
) -> jboolean {
    let Ok(id) = env.get_string(&id) else {
        return 0;
    };
    crate::notification_sync::pending_active(id.to_string_lossy().as_ref()) as jboolean
}
#[no_mangle]
pub extern "system" fn Java_com_lanchat_app_NotificationSyncNative_complete(
    mut env: JNIEnv,
    _: JObject,
    id: JString,
    success: jboolean,
    changed: jboolean,
    reason: JString,
) {
    if let Ok(id) = env.get_string(&id) {
        let reason = env
            .get_string(&reason)
            .ok()
            .map(|value| value.to_string_lossy().into_owned())
            .filter(|value| !value.is_empty());
        crate::notification_sync::complete(
            id.to_string_lossy().as_ref(),
            success != 0,
            changed != 0,
            reason,
        );
    }
}
