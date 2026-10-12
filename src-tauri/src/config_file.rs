use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[cfg(windows)]
pub const WINDOWS_DATABASE_FILE_NAME: &str = "LQChat.db";

#[cfg(windows)]
pub fn windows_portable_root() -> Result<PathBuf, String> {
    let executable =
        std::env::current_exe().map_err(|error| format!("无法读取 LQChat 程序路径: {error}"))?;
    executable
        .parent()
        .map(PathBuf::from)
        .ok_or_else(|| "LQChat 程序路径缺少父目录".to_string())
}

#[cfg(windows)]
pub fn windows_portable_data_dir() -> Result<PathBuf, String> {
    Ok(windows_portable_root()?.join("data"))
}

#[cfg(windows)]
pub fn windows_portable_download_dir() -> Result<PathBuf, String> {
    Ok(windows_portable_root()?.join("downloads"))
}

#[cfg(windows)]
pub fn windows_portable_webview_dir() -> Result<PathBuf, String> {
    Ok(windows_portable_root()?.join("cache").join("EBWebView"))
}

pub fn custom_theme_dir() -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        Ok(windows_portable_root()?.join("config"))
    }

    #[cfg(not(windows))]
    {
        dirs::home_dir()
            .map(|home| home.join(".config").join("lanchat"))
            .ok_or_else(|| "无法获取用户主目录".to_string())
    }
}

#[cfg(windows)]
pub fn prepare_windows_portable_layout() -> Result<(), String> {
    for directory in [
        windows_portable_root()?.join("config"),
        windows_portable_data_dir()?,
        windows_portable_download_dir()?,
        windows_portable_webview_dir()?,
    ] {
        std::fs::create_dir_all(&directory)
            .map_err(|error| format!("无法创建便携目录 {}: {error}", directory.display()))?;
    }

    if !config_path().is_file() {
        write_config(&Config::default())?;
    }
    Ok(())
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub db_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub close_to_tray: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_minimized: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_grace_secs: Option<u64>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            db_path: None,
            port: None,
            lang: None,
            close_to_tray: None,
            start_minimized: None,
            presence_grace_secs: None,
        }
    }
}

/// 配置文件路径。
/// Windows 便携版: <LQChat.exe>\config\config.json
/// Linux:   ~/.config/lanchat/config.json
/// macOS:   ~/Library/Application Support/lanchat/config.json
/// Android: /data/data/com.lanchat.app/.config/lanchat/config.json
fn config_path() -> PathBuf {
    #[cfg(windows)]
    {
        windows_portable_root()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("config")
            .join("config.json")
    }

    #[cfg(not(windows))]
    {
        dirs::config_dir()
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".config")
            })
            .join("lanchat")
            .join("config.json")
    }
}

/// 读取配置文件，不存在则返回默认值
pub fn read_config() -> Config {
    let path = config_path();
    if !path.exists() {
        return Config::default();
    }
    match std::fs::File::open(&path) {
        Ok(file) => match serde_json::from_reader(file) {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!("[Config] 解析 config.json 失败: {e}，使用默认值");
                Config::default()
            }
        },
        Err(e) => {
            eprintln!("[Config] 读取 config.json 失败: {e}，使用默认值");
            Config::default()
        }
    }
}

/// 写入配置文件
pub fn write_config(config: &Config) -> Result<(), String> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create config dir failed: {e}"))?;
    }
    let tmp_path = path.with_extension("json.tmp");
    let file =
        std::fs::File::create(&tmp_path).map_err(|e| format!("create temp file failed: {e}"))?;
    serde_json::to_writer_pretty(&file, config)
        .map_err(|e| format!("serialize config failed: {e}"))?;
    std::fs::rename(&tmp_path, &path).map_err(|e| format!("rename config file failed: {e}"))?;
    println!("[Config] config written: {:?}", path);
    Ok(())
}

/// 从存储的路径解析数据库目录
/// 如果路径以 .db 结尾（文件路径），取其父目录
/// 否则直接作为目录处理
pub fn resolve_db_dir(stored: &str) -> PathBuf {
    let p = PathBuf::from(stored);
    if p.extension().map(|e| e == "db").unwrap_or(false) {
        p.parent().unwrap_or(&p).to_path_buf()
    } else {
        p
    }
}

/// 获取平台默认的数据库目录（Web 端）
pub fn get_default_db_dir() -> PathBuf {
    #[cfg(windows)]
    {
        windows_portable_data_dir().unwrap_or_else(|_| PathBuf::from(".").join("data"))
    }

    #[cfg(not(windows))]
    {
        dirs::data_dir()
            .map(|p| p.join("com.lanchat.app"))
            .unwrap_or_else(|| PathBuf::from(".").join("data"))
    }
}

/// 获取平台默认的数据库路径（桌面端，使用 Tauri 的 app_data_dir）
/// 仅在桌面端调用，Web 端用 get_default_db_dir()
pub fn get_default_db_path() -> String {
    #[cfg(windows)]
    let path = get_default_db_dir().join(WINDOWS_DATABASE_FILE_NAME);
    #[cfg(not(windows))]
    let path = get_default_db_dir().join("lanchat.db");
    path.to_string_lossy().to_string()
}

/// 从配置读取端口，不存在返回 None
pub fn get_port_from_config() -> Option<u16> {
    read_config().port
}

/// 保存端口到配置
pub fn save_port_to_config(port: u16) -> Result<(), String> {
    if port == 0 {
        return Err("Port cannot be 0".to_string());
    }
    let mut cfg = read_config();
    cfg.port = Some(port);
    write_config(&cfg)?;
    println!("[Config] 端口配置已保存: {}", port);
    Ok(())
}

/// 从配置读取语言，不存在返回 None
pub fn get_lang_from_config() -> Option<String> {
    read_config().lang
}

/// 保存语言到配置
pub fn save_lang_to_config(lang: &str) -> Result<(), String> {
    if lang != "zh" && lang != "en" {
        return Err(format!("unsupported language: {lang}"));
    }
    let mut cfg = read_config();
    cfg.lang = Some(lang.to_string());
    write_config(&cfg)
}

/// 点击主窗口关闭按钮时是否隐藏到托盘。未配置时默认开启，保持历史行为。
pub fn get_close_to_tray_from_config() -> bool {
    read_config().close_to_tray.unwrap_or(true)
}

/// 保存主窗口关闭按钮行为。
pub fn save_close_to_tray_to_config(enabled: bool) -> Result<(), String> {
    let mut cfg = read_config();
    cfg.close_to_tray = Some(enabled);
    write_config(&cfg)
}

/// 启动时是否直接隐藏到系统托盘。未配置时默认开启，保持原有开机自启行为。
pub fn get_start_minimized_from_config() -> bool {
    read_config().start_minimized.unwrap_or(true)
}

/// 保存启动时隐藏到系统托盘的行为。
pub fn save_start_minimized_to_config(enabled: bool) -> Result<(), String> {
    let mut cfg = read_config();
    cfg.start_minimized = Some(enabled);
    write_config(&cfg)
}

/// 局域网断开判定时长（秒）。未配置时返回出厂默认值，越界的配置收敛到合法区间。
pub fn get_presence_grace_secs_from_config() -> u64 {
    crate::peers::sanitize_presence_grace_secs(
        read_config()
            .presence_grace_secs
            .unwrap_or(crate::peers::DEFAULT_PRESENCE_GRACE_SECS),
    )
}

/// 保存局域网断开判定时长。
pub fn save_presence_grace_secs_to_config(secs: u64) -> Result<(), String> {
    crate::peers::validate_presence_grace_secs(secs)?;
    let mut cfg = read_config();
    cfg.presence_grace_secs = Some(secs);
    write_config(&cfg)
}

/// Android 与端口设置一样存入应用设置数据库；其他平台保留 config.json。
pub async fn update_presence_grace_secs(
    _pool: &sqlx::Pool<sqlx::Sqlite>,
    secs: u64,
) -> Result<(), String> {
    crate::peers::persist_presence_grace_secs(secs, async {
        #[cfg(target_os = "android")]
        {
            crate::db::set_presence_grace_secs(_pool, secs).await
        }
        #[cfg(not(target_os = "android"))]
        {
            save_presence_grace_secs_to_config(secs)
        }
    }).await
}

/// UI 冷启动和后台核心启动从相同存储读取；Android 不再访问通用配置目录。
pub async fn apply_presence_grace_secs_from_storage(_pool: &sqlx::Pool<sqlx::Sqlite>) {
    #[cfg(target_os = "android")]
    {
        let secs = crate::db::get_presence_grace_secs(_pool).await;
        if let Err(error) = crate::peers::set_presence_grace_secs(secs) {
            eprintln!("[Config] 应用断开判定时长失败: {error}");
        }
    }
    #[cfg(not(target_os = "android"))]
    apply_presence_grace_secs_from_config();
}

/// 启动时把配置中的断开判定时长应用到运行期。
pub fn apply_presence_grace_secs_from_config() {
    let secs = get_presence_grace_secs_from_config();
    if let Err(error) = crate::peers::set_presence_grace_secs(secs) {
        eprintln!("[Config] 应用断开判定时长失败: {error}");
    }
}
