// 在线用户管理模块
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio_util::sync::CancellationToken;

const PRESENCE_GRACE: Duration = Duration::from_secs(12);
const PRESENCE_PROBE_TIMEOUT: Duration = Duration::from_millis(1500);

async fn same_peer_reachable(client: &reqwest::Client, expected_id: &str, addr: &str) -> bool {
    if addr.is_empty() {
        return false;
    }
    let reported_id = match client.get(format!("http://{addr}/api/get_my_id")).send().await {
        Ok(response) if response.status().is_success() => response
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|body| body.get("id")?.as_str().map(str::to_owned)),
        _ => None,
    };
    // 地址和端口可以被另一设备或重新安装的实例复用，不能只凭 TCP 通就判定旧 ID 在线。
    // 旧版客户端若没有身份接口，仍保守地使用 TCP 探测。
    match reported_id {
        Some(id) => id == expected_id,
        None => matches!(
            tokio::time::timeout(PRESENCE_PROBE_TIMEOUT, tokio::net::TcpStream::connect(addr)).await,
            Ok(Ok(_))
        ),
    }
}

#[derive(Clone)]
struct StaleCandidate {
    id: String,
    name: String,
    addr: String,
    last_seen: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerConnectionTransition {
    Unchanged,
    New,
    Reconnected,
}

impl PeerConnectionTransition {
    pub fn is_connected(self) -> bool {
        matches!(self, Self::New | Self::Reconnected)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unchanged => "unchanged",
            Self::New => "new",
            Self::Reconnected => "reconnected",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Peer {
    pub id: String, // UUID
    pub name: String,
    pub addr: String,
    pub last_seen: u64,           // Unix 时间戳
    pub is_offline: bool,         // 是否离线
    pub available_memory_mb: u64, // 可用内存（MB）
    #[serde(default)]
    pub notification_push_enabled: bool,
    #[serde(default)]
    pub notification_push_target_device_ids: Vec<String>,
}

// 全局在线用户列表
pub struct PeerManager {
    peers: Arc<RwLock<HashMap<String, Peer>>>, // key 是 UUID
    #[cfg(windows)]
    persistence: RwLock<Option<Arc<crate::peer_persistence::PeerPersistence>>>,
}

impl PeerManager {
    pub fn new() -> Self {
        Self {
            peers: Arc::new(RwLock::new(HashMap::new())),
            #[cfg(windows)]
            persistence: RwLock::new(None),
        }
    }

    pub fn remove_peer(&self, id: &str) {
        let mut peers = self.peers.write().unwrap();
        if peers.remove(id).is_some() {
            println!("[PeerManager] 已从内存中彻底移除用户: {}", id);
        }
    }

    fn remove_peer_if_offline(&self, id: &str) -> bool {
        let mut peers = self.peers.write().unwrap();
        if !peers.get(id).is_some_and(|peer| peer.is_offline) {
            return false;
        }
        peers.remove(id);
        true
    }

    // 从数据库加载历史用户
    pub async fn load_from_db(&self, pool: &sqlx::Pool<sqlx::Sqlite>) -> Result<(), String> {
        println!("[PeerManager] 从数据库加载历史用户...");

        let users = crate::db::get_all_users(pool).await?;

        let mut peers = self.peers.write().unwrap();
        for (id, name, addr, _last_seen, is_offline, available_memory_mb) in users {
            let peer = Peer {
                id: id.clone(),
                name,
                addr,
                last_seen: _last_seen as u64,
                is_offline,
                available_memory_mb,
                notification_push_enabled: false,
                notification_push_target_device_ids: Vec::new(),
            };
            peers.insert(id, peer);
        }

        println!("[PeerManager] 已加载 {} 个历史用户", peers.len());
        Ok(())
    }

    // 添加或更新用户
    pub fn add_or_update(&self, id: String, name: String, addr: String) -> bool {
        self.add_or_update_with_memory(id, name, addr, 0)
    }

    // 添加或更新用户（包含内存信息）
    // 返回 true 表示是新用户或重新上线，false 表示只是更新
    pub fn add_or_update_with_memory(
        &self,
        id: String,
        name: String,
        addr: String,
        available_memory_mb: u64,
    ) -> bool {
        self.observe_discovery(id, name, addr, available_memory_mb)
            .is_some_and(PeerConnectionTransition::is_connected)
    }

    pub fn observe_discovery(
        &self,
        id: String,
        name: String,
        addr: String,
        available_memory_mb: u64,
    ) -> Option<PeerConnectionTransition> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let mut peers = self.peers.write().unwrap();

        #[cfg(windows)]
        if let Some(store) = self.persistence() {
            if !store.observe(&id, &name, &addr) {
                return None;
            }
        }

        if let Some(peer) = peers.get_mut(&id) {
            // 已存在,更新信息
            let was_offline = peer.is_offline;
            peer.name = name;
            peer.addr = addr;
            peer.last_seen = now;
            peer.is_offline = false;
            peer.available_memory_mb = available_memory_mb;

            // 只在用户重新上线时打印日志
            if was_offline {
                println!(
                    "[PeerManager] 用户重新上线: {} ({}) - 可用内存: {} MB",
                    peer.name, peer.id, available_memory_mb
                );
                return Some(PeerConnectionTransition::Reconnected);
            }
            return Some(PeerConnectionTransition::Unchanged);
        } else {
            // 新用户
            let peer = Peer {
                id: id.clone(),
                name: name.clone(),
                addr,
                last_seen: now,
                is_offline: false,
                available_memory_mb,
                notification_push_enabled: false,
                notification_push_target_device_ids: Vec::new(),
            };
            println!(
                "[PeerManager] 添加新用户: {} ({}) - 可用内存: {} MB",
                name, id, available_memory_mb
            );
            peers.insert(id, peer);
            return Some(PeerConnectionTransition::New);
        }
    }

    pub fn update_notification_presence(
        &self,
        id: &str,
        enabled: bool,
        target_device_ids: Vec<String>,
    ) {
        if let Some(peer) = self.peers.write().unwrap().get_mut(id) {
            peer.notification_push_enabled = enabled;
            peer.notification_push_target_device_ids = target_device_ids;
        }
    }

    pub fn force_mark_offline(&self, id: &str) {
        let mut peers = self.peers.write().unwrap();
        if let Some(peer) = peers.get_mut(id) {
            if !peer.is_offline {
                println!(
                    "[PeerManager] 探测发现用户确已离线: {} ({})",
                    peer.name, peer.id
                );
                peer.is_offline = true;
            }
        }
    }

    fn stale_candidates(&self) -> Vec<StaleCandidate> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.peers
            .read()
            .unwrap()
            .values()
            .filter(|peer| {
                !peer.is_offline
                    && !peer.addr.is_empty()
                    && now.saturating_sub(peer.last_seen) >= PRESENCE_GRACE.as_secs()
            })
            .map(|peer| StaleCandidate {
                id: peer.id.clone(),
                name: peer.name.clone(),
                addr: peer.addr.clone(),
                last_seen: peer.last_seen,
            })
            .collect()
    }

    fn complete_presence_probe(&self, candidate: &StaleCandidate, reachable: bool) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut peers = self.peers.write().unwrap();
        let Some(peer) = peers.get_mut(&candidate.id) else {
            return;
        };
        // A discovery packet arrived while the probe was running. Its newer
        // observation wins over this result.
        if peer.is_offline || peer.last_seen != candidate.last_seen {
            return;
        }
        if reachable {
            // 身份匹配（或旧版客户端的 TCP 回退）证明该设备仍在线。
            peer.last_seen = now;
            return;
        }
        if now.saturating_sub(peer.last_seen) >= PRESENCE_GRACE.as_secs() {
            println!(
                "[PeerManager] 用户离线: {} ({}) - {}秒未见且主动探测失败",
                candidate.name,
                candidate.id,
                now.saturating_sub(peer.last_seen)
            );
            peer.is_offline = true;
        }
    }

    // 获取所有用户（包括离线的）
    pub fn get_all_peers(&self) -> Vec<Peer> {
        let peers = self.peers.read().unwrap();
        peers.values().cloned().collect()
    }

    // 获取所有在线用户（过滤掉离线的）
    pub fn get_active_peers(&self) -> Vec<Peer> {
        let peers = self.peers.read().unwrap();
        peers.values().filter(|p| !p.is_offline).cloned().collect()
    }

    #[cfg(windows)]
    pub fn enable_windows_persistence(&self, pool: sqlx::Pool<sqlx::Sqlite>) {
        let mut peers = self.peers.write().unwrap();
        let store = Arc::new(crate::peer_persistence::PeerPersistence::new(
            pool,
            &peers.values().cloned().collect::<Vec<_>>(),
        ));
        for peer in peers.values_mut() {
            peer.is_offline = true;
            peer.last_seen = 0;
            peer.available_memory_mb = 0;
        }
        *self.persistence.write().unwrap() = Some(store);
    }

    #[cfg(windows)]
    pub fn persistence(&self) -> Option<Arc<crate::peer_persistence::PeerPersistence>> {
        self.persistence.read().unwrap().clone()
    }

    #[cfg(windows)]
    pub fn begin_presence_session(&self) {
        let mut peers = self.peers.write().unwrap();
        if let Some(store) = self.persistence() {
            store.start_session();
            for peer in peers.values_mut() {
                peer.is_offline = true;
                peer.last_seen = 0;
                peer.available_memory_mb = 0;
            }
        }
    }

    #[cfg(windows)]
    pub(crate) fn begin_profile_delete(
        &self,
        store: &crate::peer_persistence::PeerPersistence,
        id: &str,
    ) {
        let _peers = self.peers.write().unwrap();
        store.suppress(id);
    }

    #[cfg(windows)]
    pub(crate) fn begin_offline_profile_delete(
        &self,
        store: &crate::peer_persistence::PeerPersistence,
        id: &str,
    ) -> bool {
        let peers = self.peers.write().unwrap();
        if !peers.get(id).is_some_and(|peer| peer.is_offline) {
            return false;
        }
        store.suppress(id);
        true
    }

    #[cfg(windows)]
    pub(crate) fn end_profile_delete(
        &self,
        store: &crate::peer_persistence::PeerPersistence,
        id: &str,
        success: bool,
    ) {
        let mut peers = self.peers.write().unwrap();
        if success {
            peers.remove(id);
        }
        store.finish_delete(id, success);
    }

    pub async fn delete_user_and_history(
        self: &Arc<Self>,
        pool: &sqlx::Pool<sqlx::Sqlite>,
        my_id: &str,
        peer_id: &str,
    ) -> Result<(), String> {
        #[cfg(windows)]
        if let Some(store) = self.persistence() {
            return store
                .delete(self.clone(), my_id.to_owned(), peer_id.to_owned())
                .await;
        }
        crate::db::delete_user_and_history(pool, my_id, peer_id).await?;
        self.remove_peer(peer_id);
        Ok(())
    }

    /// 清除当前不在局域网的设备档案，不删除双方的聊天记录。
    pub async fn clear_offline_peer_profiles(
        self: &Arc<Self>,
        pool: &sqlx::Pool<sqlx::Sqlite>,
    ) -> Result<Vec<String>, String> {
        use futures_util::{stream::FuturesUnordered, StreamExt};

        let candidates: Vec<Peer> = self.get_all_peers().into_iter()
            .filter(|peer| peer.is_offline)
            .collect();
        let client = crate::network::lan_http_client(Some(PRESENCE_PROBE_TIMEOUT))?;
        let mut probes = FuturesUnordered::new();
        for peer in candidates {
            let client = client.clone();
            probes.push(async move {
                let same_device_reachable = same_peer_reachable(&client, &peer.id, &peer.addr).await;
                (peer.id, same_device_reachable)
            });
        }
        let mut removed = Vec::new();
        while let Some((peer_id, reachable)) = probes.next().await {
            if reachable {
                continue;
            }
            #[cfg(windows)]
            if let Some(store) = self.persistence() {
                if store.delete_offline_profile(self.clone(), peer_id.clone()).await? {
                    removed.push(peer_id);
                }
                continue;
            }

            if !self.get_all_peers().iter().any(|peer| peer.id == peer_id && peer.is_offline) {
                continue;
            }
            crate::db::delete_peer_profile(pool, &peer_id).await?;
            if self.remove_peer_if_offline(&peer_id) {
                removed.push(peer_id);
            } else if let Some(peer) = self.get_all_peers().into_iter().find(|peer| peer.id == peer_id) {
                // 写库期间重新发现的在线设备不能从列表消失，顺带恢复其档案。
                crate::db::save_or_update_user(
                    pool, peer.id, peer.name, peer.addr, false, peer.available_memory_mb,
                ).await?;
            }
        }
        Ok(removed)
    }
}

pub async fn run_presence_monitor(
    manager: Arc<PeerManager>,
    cancellation: CancellationToken,
) -> Result<(), String> {
    use futures_util::{stream::FuturesUnordered, StreamExt};

    let client = crate::network::lan_http_client(Some(PRESENCE_PROBE_TIMEOUT))?;
    let mut interval = tokio::time::interval(Duration::from_secs(2));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = cancellation.cancelled() => return Ok(()),
            _ = interval.tick() => {}
        }

        let mut probes = FuturesUnordered::new();
        for candidate in manager.stale_candidates() {
            let client = client.clone();
            probes.push(async move {
                let reachable = same_peer_reachable(&client, &candidate.id, &candidate.addr).await;
                (candidate, reachable)
            });
        }
        while let Some((candidate, reachable)) = probes.next().await {
            manager.complete_presence_probe(&candidate, reachable);
        }
    }
}

impl Default for PeerManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod presence_tests {
    use super::*;

    fn stale_manager() -> PeerManager {
        let manager = PeerManager::new();
        manager.observe_discovery("peer".into(), "对端".into(), "127.0.0.1:8888".into(), 128);
        manager
            .peers
            .write()
            .unwrap()
            .get_mut("peer")
            .unwrap()
            .last_seen = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 20;
        manager
    }

    #[test]
    fn reachable_stale_peer_stays_online() {
        let manager = stale_manager();
        let candidate = manager.stale_candidates().pop().unwrap();
        assert_eq!(candidate.addr, "127.0.0.1:8888");
        manager.complete_presence_probe(&candidate, true);
        let peer = manager.peers.read().unwrap().get("peer").unwrap().clone();
        assert!(!peer.is_offline);
        assert!(peer.last_seen > candidate.last_seen);
    }

    #[test]
    fn unreachable_stale_peer_is_confirmed_offline() {
        let manager = stale_manager();
        let candidate = manager.stale_candidates().pop().unwrap();
        manager.complete_presence_probe(&candidate, false);
        assert!(manager.peers.read().unwrap()["peer"].is_offline);
    }

    #[test]
    fn newer_discovery_wins_over_failed_probe() {
        let manager = stale_manager();
        let candidate = manager.stale_candidates().pop().unwrap();
        manager.observe_discovery("peer".into(), "对端".into(), "127.0.0.1:8888".into(), 256);
        manager.complete_presence_probe(&candidate, false);
        let peer = manager.peers.read().unwrap().get("peer").unwrap().clone();
        assert!(!peer.is_offline);
        assert_eq!(peer.available_memory_mb, 256);
    }

    #[test]
    fn discovery_distinguishes_new_reconnected_and_heartbeat() {
        let manager = PeerManager::new();
        assert_eq!(
            manager.observe_discovery("peer".into(), "对端".into(), "127.0.0.1:8888".into(), 128,),
            Some(PeerConnectionTransition::New),
        );
        assert_eq!(
            manager.observe_discovery("peer".into(), "对端".into(), "127.0.0.1:8888".into(), 128,),
            Some(PeerConnectionTransition::Unchanged),
        );
        manager.force_mark_offline("peer");
        assert_eq!(
            manager.observe_discovery("peer".into(), "对端".into(), "127.0.0.1:8888".into(), 128,),
            Some(PeerConnectionTransition::Reconnected),
        );
    }

    #[tokio::test]
    async fn reused_address_only_keeps_matching_device_online() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let router = axum::Router::new().route("/api/get_my_id", axum::routing::get(|| async {
            axum::Json(serde_json::json!({ "id": "current" }))
        }));
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let manager = PeerManager::new();
        manager.observe_discovery("old".into(), "4060".into(), addr.clone(), 128);
        manager.observe_discovery("current".into(), "4060".into(), addr, 128);
        for peer in manager.peers.write().unwrap().values_mut() {
            peer.last_seen -= 20;
        }

        let client = crate::network::lan_http_client(Some(PRESENCE_PROBE_TIMEOUT)).unwrap();
        for candidate in manager.stale_candidates() {
            let reachable = same_peer_reachable(&client, &candidate.id, &candidate.addr).await;
            manager.complete_presence_probe(&candidate, reachable);
        }
        assert!(manager.peers.read().unwrap()["old"].is_offline);
        assert!(!manager.peers.read().unwrap()["current"].is_offline);
        server.abort();
    }
}
