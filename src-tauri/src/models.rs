use serde::{Deserialize, Serialize};

// 消息结构体 - 对应 messages 表
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Message {
    pub id: i64,
    pub sender_id: String,
    pub receiver_id: Option<String>,
    pub content: String,
    pub msg_type: String,
    pub timestamp: i64,
    pub file_path: Option<String>,
    pub file_status: Option<String>,
    pub file_size: Option<i64>,
    pub sender_msg_id: Option<String>,
    #[sqlx(default)]
    pub status: Option<String>,
    /// 与图片/文件合并发送的文字说明。
    #[sqlx(default)]
    pub caption: Option<String>,
    /// 发送成功后由应用清理本机副本的标记。
    #[sqlx(default)]
    pub local_copy_cleaned: Option<i64>,
}

// API 响应用的消息结构体（字段名适配前端）
#[derive(Debug, Serialize, Deserialize)]
pub struct MessageResponse {
    pub id: i64,
    pub from_id: String,
    pub content: String,
    pub timestamp: i64,
    pub msg_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender_msg_id: Option<i64>,
    /// 图片/文件消息气泡内的文字说明（图片在上、空一行、文字在下）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    /// 仅当传输成功且应用确实清理了本机副本时为 true。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_copy_cleaned: Option<bool>,
}

impl From<Message> for MessageResponse {
    fn from(msg: Message) -> Self {
        let mut response = MessageResponse {
            id: msg.id,
            from_id: msg.sender_id,
            content: msg.content.clone(),
            timestamp: msg.timestamp,
            msg_type: msg.msg_type.clone(),
            file_id: None,
            file_name: None,
            file_path: None,
            file_status: None,
            file_size: None,
            sender_msg_id: None,
            status: msg.status.clone(),
            caption: msg
                .caption
                .as_ref()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string()),
            local_copy_cleaned: msg.local_copy_cleaned.map(|value| value != 0),
        };

        // 如果是文件消息，添加文件信息
        if msg.msg_type == "file" {
            response.file_name = Some(msg.content.clone()); // content 存储的是文件名
            response.file_status = msg.file_status.clone();
            response.sender_msg_id = msg
                .sender_msg_id
                .as_ref()
                .and_then(|s| s.parse::<i64>().ok());

            // 文件大小：优先使用 DB 值
            if let Some(sz) = msg.file_size.filter(|&s| s > 0) {
                response.file_size = Some(sz as u64);
            }

            // file_path 和 file_id 仅在路径有效时设置
            if let Some(ref path) = msg.file_path {
                response.file_path = Some(path.clone());
                let filename = std::path::Path::new(path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                response.file_id = Some(filename.to_string());

                // 如果 DB 中没有文件大小，尝试从文件系统获取
                if response.file_size.is_none() {
                    if let Ok(metadata) = std::fs::metadata(path) {
                        response.file_size = Some(metadata.len());
                    }
                }
            }
        }

        response
    }
}

// 设置结构体 - 对应 settings 表
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Setting {
    pub key: String,
    pub value: String,
}
