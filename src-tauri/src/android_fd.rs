// android_fd.rs - Android 文件描述符处理 + URI 持久化权限 + FD 缓存

// ─── Android 实现 ───
#[cfg(target_os = "android")]
use std::os::unix::io::{FromRawFd, RawFd};

// ═══════════════════════════════════════════════════════════════
// FD Cache (全局、进程生命周期)
// 用于 Share Intent 文件：从外部应用分享进来时提前拿到 FD
// 缓存住，供后续 auto_download OFF 延迟上传使用
// ═══════════════════════════════════════════════════════════════

#[cfg(target_os = "android")]
use std::collections::HashMap;
#[cfg(target_os = "android")]
use std::sync::{Mutex, OnceLock};

#[cfg(target_os = "android")]
struct AndroidServiceContext {
    vm: jni::JavaVM,
    context: jni::objects::GlobalRef,
}

#[cfg(target_os = "android")]
fn service_context() -> &'static Mutex<Option<AndroidServiceContext>> {
    static CONTEXT: OnceLock<Mutex<Option<AndroidServiceContext>>> = OnceLock::new();
    CONTEXT.get_or_init(|| Mutex::new(None))
}

/// Register an application Context owned by the foreground service. Unlike
/// ndk_context, this is available after Android recreates only the Service and
/// never launches the Tauri Activity.
#[cfg(target_os = "android")]
pub fn initialize_service_context(
    env: &mut jni::JNIEnv<'_>,
    service: &jni::objects::JObject<'_>,
) -> Result<(), String> {
    let vm = env
        .get_java_vm()
        .map_err(|error| format!("获取前台服务 JavaVM 失败: {error}"))?;
    let application_context = env
        .call_method(
            service,
            "getApplicationContext",
            "()Landroid/content/Context;",
            &[],
        )
        .map_err(|error| format!("获取前台服务 Application Context 失败: {error}"))?
        .l()
        .map_err(|error| format!("读取前台服务 Application Context 失败: {error}"))?;
    if application_context.is_null() {
        return Err("前台服务 Application Context 为空".to_string());
    }
    let context = env
        .new_global_ref(application_context)
        .map_err(|error| format!("保存前台服务 Application Context 失败: {error}"))?;
    let mut slot = service_context()
        .lock()
        .map_err(|_| "前台服务 Android Context 锁已损坏".to_string())?;
    *slot = Some(AndroidServiceContext { vm, context });
    Ok(())
}

#[cfg(target_os = "android")]
type FdCacheEntry = (RawFd, String, u64, String); // (raw_fd, file_name, file_size, source_uri)

#[cfg(target_os = "android")]
const FD_CACHE_MAX: usize = 30;

#[cfg(target_os = "android")]
fn fd_cache() -> &'static Mutex<HashMap<i64, FdCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<i64, FdCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 缓存一个 FD 供后续使用（auto_download OFF 场景）。
/// 在 msg_id 已知后调用，将 FD 与 msg_id 绑定。
/// 超过 FD_CACHE_MAX 则 FIFO 淘汰最老的。
#[cfg(target_os = "android")]
pub fn cache_fd_for_msg(msg_id: i64, fd: RawFd, name: String, size: u64, source_uri: String) {
    let mut cache = fd_cache().lock().unwrap();

    // FIFO: 超出上限则淘汰最老的
    while cache.len() >= FD_CACHE_MAX {
        // HashMap 的迭代顺序是未定义的，但 remove 第一个 entry 是快速清理
        // 近似 FIFO：取任意一条清理（实践中谁先被 iterate 到就清谁）
        let oldest = cache.keys().next().copied();
        if let Some(old_id) = oldest {
            if let Some((old_fd, _, _, _)) = cache.remove(&old_id) {
                // 关闭 FD 释放内核资源
                unsafe { std::fs::File::from_raw_fd(old_fd) };
            }
        } else {
            break;
        }
    }

    if let Some((old_fd, _, _, _)) = cache.insert(msg_id, (fd, name, size, source_uri)) {
        unsafe { std::fs::File::from_raw_fd(old_fd) };
    }
    println!(
        "[AndroidFD] FD 已缓存: msg_id={}, fd={} (cache_size={})",
        msg_id,
        fd,
        cache.len()
    );
}

/// 使用缓存中的 URI 独立打开文件对象，避免共享 FD 游标，
/// 不消耗原始 FD，支持多次重试调用。
/// 返回 (tokio::fs::File, file_name, file_size)。
#[cfg(target_os = "android")]
pub fn duplicate_cached_file(msg_id: i64) -> Option<(tokio::fs::File, String, u64)> {
    use std::mem::ManuallyDrop;
    use std::os::unix::io::AsRawFd;
    // Pin the descriptor while opening a new description through /proc. This
    // preserves temporary-grant caching without reading from a shared dup cursor.
    let (pinned, name, declared, uri) = {
        let cache = fd_cache().lock().unwrap();
        let (fd, name, size, uri) = cache.get(&msg_id)?;
        let borrowed = ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(*fd) });
        let pinned = borrowed.try_clone().ok()?;
        (pinned, name.clone(), *size, uri.clone())
    };
    let result = (|| {
        let source = match std::fs::File::open(format!("/proc/self/fd/{}", pinned.as_raw_fd())) {
            Ok(source) => source,
            Err(_) => AndroidFile::from_content_uri(&uri)?.into_file(),
        };
        let metadata = source.metadata().map_err(|error| error.to_string())?;
        let size = crate::file_source::size_from_metadata(&metadata, declared as usize)?;
        Ok::<_, String>((tokio::fs::File::from_std(source), name, size as u64))
    })();
    match result {
        Ok(source) => Some(source),
        Err(error) => { eprintln!("[AndroidFD] 无法独立打开缓存文件: {error}"); None }
    }
}

/// 从缓存中移除 FD 并关闭底层文件描述符。
/// 在消息删除或传输终结时调用。
#[cfg(target_os = "android")]
pub fn remove_cached_fd(msg_id: i64) {
    let mut cache = fd_cache().lock().unwrap();
    if let Some((fd, _, _, _)) = cache.remove(&msg_id) {
        // drop File → 自动 close(fd)
        unsafe { std::fs::File::from_raw_fd(fd) };
        println!("[AndroidFD] FD 已移除并关闭: msg_id={}, fd={}", msg_id, fd);
    }
}

#[cfg(target_os = "android")]
pub fn cached_fd_count() -> usize {
    fd_cache().lock().unwrap().len()
}

/// 从 FD 缓存中克隆一个原始 FD（用于跨进程分享）。
/// 通过 try_clone (fcntl F_DUPFD_CLOEXEC) 纯内存复制。
/// 返回的 RawFd 所有权转移给调用者，调用者负责关闭。
#[cfg(target_os = "android")]
pub fn clone_fd_for_ipc(msg_id: i64) -> Option<RawFd> {
    use std::io::Seek;
    use std::mem::ManuallyDrop;
    use std::os::unix::io::{FromRawFd, IntoRawFd};

    let cache = fd_cache().lock().unwrap();
    if let Some((raw_fd, _, _, _)) = cache.get(&msg_id) {
        let mut original = ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(*raw_fd) });
        if let Err(e) = original.seek(std::io::SeekFrom::Start(0)) {
            eprintln!("[AndroidFD] 警告: seek(0) 失败: {}", e);
        }
        match original.try_clone() {
            Ok(cloned) => {
                let fd = cloned.into_raw_fd();
                println!(
                    "[AndroidFD] 跨进程分享 FD 克隆成功: msg_id={}, fd={}",
                    msg_id, fd
                );
                Some(fd)
            }
            Err(e) => {
                eprintln!(
                    "[AndroidFD] 跨进程分享 FD try_clone 失败: msg_id={}, err={}",
                    msg_id, e
                );
                None
            }
        }
    } else {
        eprintln!("[AndroidFD] 跨进程分享 FD 缓存未命中: msg_id={}", msg_id);
        None
    }
}

/// 从 FD 缓存中获取文件名（用于构造带文件名的分享 URI）。
#[cfg(target_os = "android")]
pub fn get_cached_file_name(msg_id: i64) -> Option<String> {
    let cache = fd_cache().lock().unwrap();
    cache.get(&msg_id).map(|(_, name, _, _)| name.clone())
}

/// JNI 导出：供 FdContentProvider 调用，获取克隆的 FD
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "system" fn Java_com_lanchat_app_FdContentProvider_nativeGetClonedFd(
    _env: jni::JNIEnv,
    _class: jni::objects::JClass,
    msg_id: jni::sys::jlong,
) -> jni::sys::jint {
    println!(
        "[AndroidFD] FdContentProvider 请求克隆 FD: msg_id={}",
        msg_id
    );
    match clone_fd_for_ipc(msg_id as i64) {
        Some(fd) => fd as jni::sys::jint,
        None => -1,
    }
}
/// 供 Kotlin 端 ContentProvider 查询文件大小
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "system" fn Java_com_lanchat_app_FdContentProvider_nativeGetFileSize(
    mut _env: jni::JNIEnv,
    _class: jni::objects::JClass,
    msg_id: jni::sys::jlong,
) -> jni::sys::jlong {
    let cache = fd_cache().lock().unwrap();
    if let Some((_, _, size, _)) = cache.get(&(msg_id as i64)) {
        *size as jni::sys::jlong
    } else {
        -1
    }
}
#[cfg(target_os = "android")]
pub fn clear_all_cached_fds() {
    let mut cache = fd_cache().lock().unwrap();
    for (_msg_id, (fd, _, _, _)) in cache.drain() {
        unsafe { std::fs::File::from_raw_fd(fd) };
    }
    println!("[AndroidFD] 全部 FD 缓存已清理");
}

// ═══════════════════════════════════════════════════════════════
// AndroidFile 结构体
//   - 从 FD 创建 File
//   - 从 content URI 打开 FD
//   - URI 持久化权限管理
//   - 系统 API level 查询
// ═══════════════════════════════════════════════════════════════

#[cfg(target_os = "android")]
pub struct AndroidFile {
    file: std::fs::File,
}

#[cfg(target_os = "android")]
impl AndroidFile {
    /// 从文件描述符创建 File 对象
    pub fn from_fd(fd: RawFd) -> Result<Self, String> {
        if fd < 0 {
            return Err("无效的文件描述符".to_string());
        }
        println!("[AndroidFD] 从 FD 创建文件对象: fd={}", fd);
        let file = unsafe { std::fs::File::from_raw_fd(fd) };
        Ok(AndroidFile { file })
    }

    /// 获取内部的 File 对象
    pub fn into_file(self) -> std::fs::File {
        self.file
    }

    /// 获取内部的原始文件描述符（所有权转移，调用后 AndroidFile 被消费）
    pub fn into_raw_fd(self) -> RawFd {
        use std::os::unix::io::IntoRawFd;
        let fd = self.file.into_raw_fd();
        println!("[AndroidFD] into_raw_fd: {}", fd);
        fd
    }

    /// 从 content:// URI 获取文件描述符
    pub fn from_content_uri(uri: &str) -> Result<Self, String> {
        println!("[AndroidFD] 尝试从 content URI 获取 FD: {}", uri);

        use jni::objects::{JObject, JValue};
        use jni::JavaVM;

        let ctx = ndk_context::android_context();
        let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }
            .map_err(|e| format!("无法获取 JavaVM: {}", e))?;
        let mut env = vm
            .attach_current_thread()
            .map_err(|e| format!("无法附加到当前线程: {}", e))?;
        let context = unsafe { JObject::from_raw(ctx.context().cast()) };

        let content_resolver = env
            .call_method(
                context,
                "getContentResolver",
                "()Landroid/content/ContentResolver;",
                &[],
            )
            .map_err(|e| format!("无法获取 ContentResolver: {}", e))?
            .l()
            .map_err(|e| format!("无法转换为对象: {}", e))?;

        let uri_string = env
            .new_string(uri)
            .map_err(|e| format!("无法创建 URI 字符串: {}", e))?;
        let uri_obj = env
            .call_static_method(
                "android/net/Uri",
                "parse",
                "(Ljava/lang/String;)Landroid/net/Uri;",
                &[JValue::Object(&JObject::from(uri_string))],
            )
            .map_err(|e| format!("无法解析 URI: {}", e))?
            .l()
            .map_err(|e| format!("无法转换为对象: {}", e))?;

        let mode_string = env
            .new_string("r")
            .map_err(|e| format!("无法创建模式字符串: {}", e))?;
        let pfd_result = env.call_method(
            content_resolver,
            "openFileDescriptor",
            "(Landroid/net/Uri;Ljava/lang/String;)Landroid/os/ParcelFileDescriptor;",
            &[
                JValue::Object(&uri_obj),
                JValue::Object(&JObject::from(mode_string)),
            ],
        );

        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
            return Err(format!("content URI 权限已过期或文件不存在: {}", uri));
        }

        let pfd = pfd_result
            .map_err(|e| format!("无法打开文件描述符: {}", e))?
            .l()
            .map_err(|e| format!("无法转换为对象: {}", e))?;

        if pfd.is_null() {
            return Err(format!("openFileDescriptor 返回 null: {}", uri));
        }

        let fd = env
            .call_method(pfd, "detachFd", "()I", &[])
            .map_err(|e| format!("无法分离文件描述符: {}", e))?
            .i()
            .map_err(|e| format!("无法转换为整数: {}", e))?;

        println!("[AndroidFD] 成功获取文件描述符: fd={}", fd);
        Self::from_fd(fd)
    }

    /// 通过 ContentResolver 查询 content URI 的真实文件名和大小
    pub fn query_content_uri_info(uri: &str) -> Result<(String, u64), String> {
        println!("[AndroidFD] 查询 content URI 文件信息: {}", uri);

        use jni::objects::{JObject, JValue};
        use jni::JavaVM;

        let ctx = ndk_context::android_context();
        let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }
            .map_err(|e| format!("无法获取 JavaVM: {}", e))?;
        let mut env = vm
            .attach_current_thread()
            .map_err(|e| format!("无法附加到当前线程: {}", e))?;
        let context = unsafe { JObject::from_raw(ctx.context().cast()) };

        let content_resolver = env
            .call_method(
                &context,
                "getContentResolver",
                "()Landroid/content/ContentResolver;",
                &[],
            )
            .map_err(|e| format!("无法获取 ContentResolver: {}", e))?
            .l()
            .map_err(|e| format!("转换失败: {}", e))?;

        let uri_jstring = env
            .new_string(uri)
            .map_err(|e| format!("创建字符串失败: {}", e))?;
        let uri_obj = env
            .call_static_method(
                "android/net/Uri",
                "parse",
                "(Ljava/lang/String;)Landroid/net/Uri;",
                &[JValue::Object(&JObject::from(uri_jstring))],
            )
            .map_err(|e| format!("解析 URI 失败: {}", e))?
            .l()
            .map_err(|e| format!("转换失败: {}", e))?;

        let cursor = env.call_method(
            &content_resolver,
            "query",
            "(Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;)Landroid/database/Cursor;",
            &[
                JValue::Object(&uri_obj),
                JValue::Object(&JObject::null()),
                JValue::Object(&JObject::null()),
                JValue::Object(&JObject::null()),
                JValue::Object(&JObject::null()),
            ],
        ).map_err(|e| format!("query 失败: {}", e))?
        .l().map_err(|e| format!("转换失败: {}", e))?;

        if cursor.is_null() {
            return Err("ContentResolver.query 返回 null".to_string());
        }

        let has_row = env
            .call_method(&cursor, "moveToFirst", "()Z", &[])
            .map_err(|e| format!("moveToFirst 失败: {}", e))?
            .z()
            .map_err(|e| format!("转换失败: {}", e))?;

        if !has_row {
            let _ = env.call_method(&cursor, "close", "()V", &[]);
            return Err("Cursor 为空，无法获取文件信息".to_string());
        }

        let display_name_col = {
            let col_name = env
                .new_string("_display_name")
                .map_err(|e| format!("创建字符串失败: {}", e))?;
            env.call_method(
                &cursor,
                "getColumnIndex",
                "(Ljava/lang/String;)I",
                &[JValue::Object(&JObject::from(col_name))],
            )
            .map_err(|e| format!("getColumnIndex 失败: {}", e))?
            .i()
            .map_err(|e| format!("转换失败: {}", e))?
        };

        let size_col = {
            let col_name = env
                .new_string("_size")
                .map_err(|e| format!("创建字符串失败: {}", e))?;
            env.call_method(
                &cursor,
                "getColumnIndex",
                "(Ljava/lang/String;)I",
                &[JValue::Object(&JObject::from(col_name))],
            )
            .map_err(|e| format!("getColumnIndex 失败: {}", e))?
            .i()
            .map_err(|e| format!("转换失败: {}", e))?
        };

        let file_name = if display_name_col >= 0 {
            let name_obj = env
                .call_method(
                    &cursor,
                    "getString",
                    "(I)Ljava/lang/String;",
                    &[JValue::Int(display_name_col)],
                )
                .map_err(|e| format!("getString 失败: {}", e))?
                .l()
                .map_err(|e| format!("转换失败: {}", e))?;

            if name_obj.is_null() {
                String::new()
            } else {
                let jstr = unsafe { jni::objects::JString::from_raw(name_obj.into_raw()) };
                env.get_string(&jstr).map(|s| s.into()).unwrap_or_default()
            }
        } else {
            String::new()
        };

        let file_size = if size_col >= 0 {
            env.call_method(&cursor, "getLong", "(I)J", &[JValue::Int(size_col)])
                .map_err(|e| format!("getLong 失败: {}", e))?
                .j()
                .map_err(|e| format!("转换失败: {}", e))
                .unwrap_or(0) as u64
        } else {
            0u64
        };

        let _ = env.call_method(&cursor, "close", "()V", &[]);

        println!(
            "[AndroidFD] 查询结果: 文件名={}, 大小={}",
            file_name, file_size
        );
        Ok((file_name, file_size))
    }

    // ─── 新方法：URI 持久化权限 ───

    /// 获取 Android API level
    pub fn get_api_level() -> Result<i32, String> {
        use jni::JavaVM;

        let ctx = ndk_context::android_context();
        let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }
            .map_err(|e| format!("无法获取 JavaVM: {}", e))?;
        let mut env = vm
            .attach_current_thread()
            .map_err(|e| format!("无法附加到当前线程: {}", e))?;

        let sdk_int = env
            .get_static_field("android/os/Build$VERSION", "SDK_INT", "I")
            .map_err(|e| format!("获取 SDK_INT 失败: {}", e))?
            .i()
            .map_err(|e| format!("转换为 int 失败: {}", e))?;

        println!("[AndroidFD] API level: {}", sdk_int);
        Ok(sdk_int)
    }

    /// 持久化 content URI 的读取权限。
    /// 调用 ContentResolver.takePersistableUriPermission(uri, FLAG_GRANT_READ_URI_PERMISSION)
    pub fn take_persistable_uri_permission(uri: &str) -> Result<(), String> {
        use jni::objects::{JObject, JValue};
        use jni::JavaVM;

        let ctx = ndk_context::android_context();
        let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }
            .map_err(|e| format!("无法获取 JavaVM: {}", e))?;
        let mut env = vm
            .attach_current_thread()
            .map_err(|e| format!("无法附加到当前线程: {}", e))?;
        let context = unsafe { JObject::from_raw(ctx.context().cast()) };

        let content_resolver = env
            .call_method(
                context,
                "getContentResolver",
                "()Landroid/content/ContentResolver;",
                &[],
            )
            .map_err(|e| format!("无法获取 ContentResolver: {}", e))?
            .l()
            .map_err(|e| format!("无法转换为对象: {}", e))?;

        let uri_string = env
            .new_string(uri)
            .map_err(|e| format!("创建 URI 字符串失败: {}", e))?;
        let uri_obj = env
            .call_static_method(
                "android/net/Uri",
                "parse",
                "(Ljava/lang/String;)Landroid/net/Uri;",
                &[JValue::Object(&JObject::from(uri_string))],
            )
            .map_err(|e| format!("解析 URI 失败: {}", e))?
            .l()
            .map_err(|e| format!("无法转换为对象: {}", e))?;

        // FLAG_GRANT_READ_URI_PERMISSION = 1
        let flags: i32 = 1;

        let call_result = env.call_method(
            content_resolver,
            "takePersistableUriPermission",
            "(Landroid/net/Uri;I)V",
            &[JValue::Object(&uri_obj), JValue::Int(flags)],
        );

        // ★ 必须先检查并清除 Java 异常，再处理 Result
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
            return Err(format!("URI 不支持持久化权限: {}", uri));
        }

        // 此时 JVM 状态干净，安全地处理 JNI Result
        call_result.map_err(|e| format!("takePersistableUriPermission JNI 错误: {}", e))?;

        println!("[AndroidFD] ✓ 持久化 URI 权限: {}", uri);
        Ok(())
    }

    /// 释放 content URI 的持久化权限。
    /// 调用 ContentResolver.releasePersistableUriPermission(uri, FLAG_GRANT_READ_URI_PERMISSION)
    pub fn release_persistable_uri_permission(uri: &str) -> Result<(), String> {
        use jni::objects::{JObject, JValue};
        use jni::JavaVM;

        let ctx = ndk_context::android_context();
        let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }
            .map_err(|e| format!("无法获取 JavaVM: {}", e))?;
        let mut env = vm
            .attach_current_thread()
            .map_err(|e| format!("无法附加到当前线程: {}", e))?;
        let context = unsafe { JObject::from_raw(ctx.context().cast()) };

        let content_resolver = env
            .call_method(
                context,
                "getContentResolver",
                "()Landroid/content/ContentResolver;",
                &[],
            )
            .map_err(|e| format!("无法获取 ContentResolver: {}", e))?
            .l()
            .map_err(|e| format!("无法转换为对象: {}", e))?;

        let uri_string = env
            .new_string(uri)
            .map_err(|e| format!("创建 URI 字符串失败: {}", e))?;
        let uri_obj = env
            .call_static_method(
                "android/net/Uri",
                "parse",
                "(Ljava/lang/String;)Landroid/net/Uri;",
                &[JValue::Object(&JObject::from(uri_string))],
            )
            .map_err(|e| format!("解析 URI 失败: {}", e))?
            .l()
            .map_err(|e| format!("无法转换为对象: {}", e))?;

        let flags: i32 = 1; // FLAG_GRANT_READ_URI_PERMISSION

        let call_result = env.call_method(
            content_resolver,
            "releasePersistableUriPermission",
            "(Landroid/net/Uri;I)V",
            &[JValue::Object(&uri_obj), JValue::Int(flags)],
        );

        // ★ 必须先检查并清除 Java 异常
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
            // 释放失败也 OK，可能这条 URI 已被系统回收
        } else {
            // 无异常时才需要处理 Result（异常已清除，Result 内容无关紧要）
            let _ = call_result;
        }

        println!("[AndroidFD] 释放 URI 权限: {}", uri);
        Ok(())
    }

    /// 查询当前系统记录的持久化 URI 权限数量。
    /// 调用 ContentResolver.getPersistedUriPermissions()
    pub fn get_persisted_uri_count() -> Result<i32, String> {
        use jni::objects::JObject;
        use jni::JavaVM;

        let ctx = ndk_context::android_context();
        let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }
            .map_err(|e| format!("无法获取 JavaVM: {}", e))?;
        let mut env = vm
            .attach_current_thread()
            .map_err(|e| format!("无法附加到当前线程: {}", e))?;
        let context = unsafe { JObject::from_raw(ctx.context().cast()) };

        let content_resolver = env
            .call_method(
                context,
                "getContentResolver",
                "()Landroid/content/ContentResolver;",
                &[],
            )
            .map_err(|e| format!("无法获取 ContentResolver: {}", e))?
            .l()
            .map_err(|e| format!("无法转换为对象: {}", e))?;

        // getPersistedUriPermissions() → java.util.List
        let list = env
            .call_method(
                content_resolver,
                "getPersistedUriPermissions",
                "()Ljava/util/List;",
                &[],
            )
            .map_err(|e| format!("getPersistedUriPermissions 失败: {}", e))?
            .l()
            .map_err(|e| format!("无法转换为 List: {}", e))?;

        // List.size() → int
        let count = env
            .call_method(&list, "size", "()I", &[])
            .map_err(|e| format!("List.size() 失败: {}", e))?
            .i()
            .map_err(|e| format!("转换为 int 失败: {}", e))?;

        println!("[AndroidFD] 系统持久化 URI 权限数量: {}", count);
        Ok(count)
    }

    /// JNI：主动调用 Kotlin 的 launchSafFilePicker 唤起系统文件选择器
    pub fn trigger_saf_picker_jni() -> Result<(), String> {
        use jni::objects::JObject;
        use jni::JavaVM;

        let ctx = ndk_context::android_context();
        let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }
            .map_err(|e| format!("获取 JavaVM 失败: {}", e))?;
        let mut env = vm
            .attach_current_thread()
            .map_err(|e| format!("附加线程失败: {}", e))?;

        let activity = unsafe { JObject::from_raw(ctx.context().cast()) };

        env.call_method(&activity, "launchSafFilePicker", "()V", &[])
            .map_err(|e| format!("JNI 调用 launchSafFilePicker 失败: {}", e))?;

        println!("[AndroidFD] JNI 已触发 launchSafFilePicker");
        Ok(())
    }

    fn trigger_activity_method(method: &str) -> Result<(), String> {
        use jni::objects::JObject;
        use jni::JavaVM;

        let ctx = ndk_context::android_context();
        let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }
            .map_err(|e| format!("获取 JavaVM 失败: {}", e))?;
        let mut env = vm
            .attach_current_thread()
            .map_err(|e| format!("附加线程失败: {}", e))?;
        let activity = unsafe { JObject::from_raw(ctx.context().cast()) };
        env.call_method(&activity, method, "()V", &[])
            .map_err(|e| format!("JNI 调用 {} 失败: {}", method, e))?;
        Ok(())
    }

    pub fn trigger_saf_multi_picker_jni() -> Result<(), String> {
        Self::trigger_activity_method("launchSafMultiFilePicker")
    }

    pub fn trigger_download_directory_picker_jni() -> Result<(), String> {
        Self::trigger_activity_method("launchDownloadDirectoryPicker")
    }

    pub fn trigger_media_images_jni() -> Result<(), String> {
        Self::trigger_activity_method("loadMediaImages")
    }

    pub fn trigger_installed_apps_jni() -> Result<(), String> {
        Self::trigger_activity_method("loadInstalledApps")
    }
}

/// 将应用专属目录中已经完整接收的文件导出到用户通过 SAF 选择的目录。
/// Kotlin 侧负责 DocumentsContract/ContentResolver 操作，Rust 侧只传递稳定的字符串参数。
#[cfg(target_os = "android")]
pub fn export_received_file(
    tree_uri: &str,
    source_path: &str,
    file_name: &str,
) -> Result<String, String> {
    use jni::objects::{JClass, JObject, JString, JValue};
    let context_guard = service_context()
        .lock()
        .map_err(|_| "前台服务 Android Context 锁已损坏".to_string())?;
    let service_context = context_guard
        .as_ref()
        .ok_or_else(|| "前台服务 Android Context 尚未初始化".to_string())?;
    let mut env = service_context
        .vm
        .attach_current_thread()
        .map_err(|error| format!("附加 JNI 线程失败: {error}"))?;

    // FindClass 在由 Rust/Tokio 附加的原生线程上只使用系统 ClassLoader，
    // 无法找到应用 Kotlin 类。必须从 Android Context 取得应用 ClassLoader。
    let class_loader = env
        .call_method(
            service_context.context.as_obj(),
            "getClassLoader",
            "()Ljava/lang/ClassLoader;",
            &[],
        )
        .map_err(|error| format!("获取应用 ClassLoader 失败: {error}"))?
        .l()
        .map_err(|error| format!("读取应用 ClassLoader 失败: {error}"))?;
    let class_name = env
        .new_string("com.lanchat.app.AndroidDownloadStore")
        .map_err(|error| format!("创建 Kotlin 类名失败: {error}"))?;
    let loaded_class = match env.call_method(
        &class_loader,
        "loadClass",
        "(Ljava/lang/String;)Ljava/lang/Class;",
        &[JValue::Object(&JObject::from(class_name))],
    ) {
        Ok(value) => value
            .l()
            .map_err(|error| format!("读取 Kotlin 导出类失败: {error}"))?,
        Err(error) => {
            if env.exception_check().unwrap_or(false) {
                let _ = env.exception_describe();
                let _ = env.exception_clear();
            }
            return Err(format!("加载 Kotlin 文件导出类失败: {error}"));
        }
    };
    let download_store_class = JClass::from(loaded_class);

    let tree_uri = env
        .new_string(tree_uri)
        .map_err(|error| format!("创建目录 URI 参数失败: {error}"))?;
    let source_path = env
        .new_string(source_path)
        .map_err(|error| format!("创建源路径参数失败: {error}"))?;
    let file_name = env
        .new_string(file_name)
        .map_err(|error| format!("创建文件名参数失败: {error}"))?;

    let result_value = match env.call_static_method(
        &download_store_class,
        "exportReceivedFile",
        "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;",
        &[
            JValue::Object(&JObject::from(tree_uri)),
            JValue::Object(&JObject::from(source_path)),
            JValue::Object(&JObject::from(file_name)),
        ],
    ) {
        Ok(value) => value,
        Err(error) => {
            if env.exception_check().unwrap_or(false) {
                let _ = env.exception_describe();
                let _ = env.exception_clear();
            }
            return Err(format!("调用 Android 文件导出失败: {error}"));
        }
    };
    let result = result_value
        .l()
        .map_err(|error| format!("读取 Android 文件导出结果失败: {error}"))?;

    if result.is_null() {
        return Err("Android 文件导出未返回结果".to_string());
    }

    let result = JString::from(result);
    let value: String = env
        .get_string(&result)
        .map_err(|error| format!("解析 Android 文件导出结果失败: {error}"))?
        .into();
    if let Some(message) = value.strip_prefix("ERROR:") {
        Err(message.to_string())
    } else {
        Ok(value)
    }
}

/// JNI 导出：供 Kotlin 在选完文件并提取持久化权限后回调
/// 函数名必须严格匹配 Kotlin 的包名: com_lanchat_app_MainActivity_nativeOnSafFileSelected
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "system" fn Java_com_lanchat_app_MainActivity_nativeOnSafFileSelected(
    mut env: jni::JNIEnv,
    _activity: jni::objects::JObject,
    uri: jni::objects::JString,
    name: jni::objects::JString,
    size: jni::sys::jlong,
) {
    use tauri::Emitter;

    let uri_str: String = match env.get_string(&uri) {
        Ok(s) => s.into(),
        Err(e) => {
            eprintln!("[JNI-Callback] 获取 URI 失败: {}", e);
            return;
        }
    };
    let name_str: String = match env.get_string(&name) {
        Ok(s) => s.into(),
        Err(e) => {
            eprintln!("[JNI-Callback] 获取文件名失败: {}", e);
            return;
        }
    };
    let size_val: i64 = size;

    println!(
        "[JNI-Callback] 接收到 Kotlin 空投: uri={}, name={}, size={}",
        uri_str, name_str, size_val
    );

    if let Some(app) = crate::CURRENT_APP_HANDLE
        .get()
        .and_then(|handle| handle.read().ok())
        .and_then(|handle| handle.clone())
    {
        let payload = serde_json::json!({
            "uri": uri_str,
            "name": name_str,
            "size": size_val,
        });
        let _ = app.emit("saf-file-selected", payload);
        println!("[JNI-Callback] ✓ 已通过 AppHandle 广播 saf-file-selected 事件");
    } else {
        eprintln!("[JNI-Callback] 错误：当前 UI 句柄尚未初始化！");
    }
}

#[cfg(not(target_os = "android"))]
pub struct AndroidFile;

#[cfg(not(target_os = "android"))]
impl AndroidFile {
    pub fn from_fd(_fd: i32) -> Result<Self, String> {
        Err("此功能仅在 Android 上可用".to_string())
    }

    pub fn into_file(self) -> std::fs::File {
        panic!("此功能仅在 Android 上可用")
    }

    pub fn from_content_uri(_uri: &str) -> Result<Self, String> {
        Err("此功能仅在 Android 上可用".to_string())
    }

    pub fn query_content_uri_info(_uri: &str) -> Result<(String, u64), String> {
        Err("此功能仅在 Android 上可用".to_string())
    }

    pub fn get_api_level() -> Result<i32, String> {
        Err("此功能仅在 Android 上可用".to_string())
    }

    pub fn take_persistable_uri_permission(_uri: &str) -> Result<(), String> {
        Err("此功能仅在 Android 上可用".to_string())
    }

    pub fn release_persistable_uri_permission(_uri: &str) -> Result<(), String> {
        Err("此功能仅在 Android 上可用".to_string())
    }

    pub fn into_raw_fd(self) -> i32 {
        panic!("此功能仅在 Android 上可用")
    }

    pub fn get_persisted_uri_count() -> Result<i32, String> {
        Err("此功能仅在 Android 上可用".to_string())
    }
}
