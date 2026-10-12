use clap::Parser;
use std::sync::Arc;
use std::time::Duration;
use tokio;

use lanchat::peers::PeerManager;

#[derive(Parser, Debug)]
struct Args {
    #[arg(short, long)]
    port: Option<u16>, // 改为 Option，优先用 DB 中的值

    #[arg(long)]
    db_path: Option<String>, // 可选的数据库路径
}

#[tokio::main]
async fn main() {
    #[cfg(windows)]
    lanchat::config_file::prepare_windows_portable_layout().expect("无法初始化 LQChat 便携目录");

    let args = Args::parse();

    // Step 2: 若 --db-path 没传，读 config.json
    let db_dir = if let Some(ref p) = args.db_path {
        // CLI 参数优先级最高
        Some(std::path::PathBuf::from(p))
    } else {
        // 读配置文件的 db_path
        let cfg = lanchat::config_file::read_config();
        cfg.db_path
            .map(|p| lanchat::config_file::resolve_db_dir(&p))
    };

    // Step 3: 打开数据库
    println!("[Server Main] 正在初始化数据库...");
    let pool = lanchat::db::init_db_standalone(db_dir)
        .await
        .expect("数据库初始化失败");

    // 从数据库读取用户名和 ID
    let my_name = lanchat::db::get_username(&pool)
        .await
        .unwrap_or_else(|_| "Web-User".to_string());

    let my_id = lanchat::db::get_user_id(&pool)
        .await
        .expect("无法获取或生成用户 ID");

    println!("[Server Main] 我的用户名: {}", my_name);
    println!("[Server Main] 我的 ID: {}", my_id);

    // Step 4: 若 --port 没传，读配置文件取 port
    let port: u16 = args
        .port
        .unwrap_or_else(|| lanchat::config_file::get_port_from_config().unwrap_or(8888));

    // 局域网断开判定时长取自 config.json（未配置时为出厂默认值）
    lanchat::config_file::apply_presence_grace_secs_from_config();

    // 创建全局用户管理器
    let peer_manager = Arc::new(PeerManager::new());

    // 从数据库加载历史用户
    if let Err(e) = peer_manager.load_from_db(&pool).await {
        eprintln!("[Server Main] 加载历史用户失败: {}", e);
    }

    // Step 5: 以 port 启动服务
    let event_bus = lanchat::core_events::CoreEventBus::default();
    let cancellation = tokio_util::sync::CancellationToken::new();
    // 1. 启动 Web 服务 (TCP)
    let pool_clone = pool.clone();
    let peer_manager_clone = peer_manager.clone();
    let server_bus = event_bus.clone();
    let server_cancellation = cancellation.child_token();
    tokio::spawn(async move {
        let _ = lanchat::web_server::start_server(
            port,
            port,
            pool_clone,
            peer_manager_clone,
            server_bus,
            server_cancellation,
        )
        .await;
    });

    // 2. 启动 UDP 监听
    let listen_id = my_id.clone();
    let listen_name = my_name.clone();
    let peer_manager_clone = peer_manager.clone();
    let pool_for_discovery = pool.clone();
    let listener_bus = event_bus.clone();
    let listener_cancellation = cancellation.child_token();
    tokio::spawn(async move {
        let _ = lanchat::network::discovery::start_listening(
            port,
            listen_id,
            listen_name,
            peer_manager_clone,
            pool_for_discovery,
            listener_bus,
            listener_cancellation,
        )
        .await;
    });

    // 3. 启动 UDP 广播
    let announce_id = my_id.clone();
    let announce_pool = pool.clone();
    let announcer_cancellation = cancellation.child_token();
    tokio::spawn(async move {
        let _ = lanchat::network::discovery::start_announcing(
            port,
            announce_id,
            announce_pool,
            announcer_cancellation,
        )
        .await;
    });

    println!("[Server Main] ========================================");
    println!("[Server Main] Web 服务器: http://localhost:{}", port);
    println!("[Server Main] WebSocket: ws://localhost:{}/ws", port);
    println!("[Server Main] UDP 广播端口: {}", port);
    println!("[Server Main] ========================================");

    // 防止主线程退出
    loop {
        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
}
