//! Windows desktop only: persist peer identity, never heartbeat telemetry.
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use sqlx::{Pool, Sqlite};
use tokio::sync::{Mutex as AsyncMutex, Notify};
use tokio_util::sync::CancellationToken;

use crate::peers::{Peer, PeerManager};

const COALESCE: Duration = Duration::from_secs(1);
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);
const PRESENCE_GRACE: Duration = Duration::from_secs(12);

#[derive(Clone, Debug, PartialEq, Eq)]
struct Profile {
    id: String,
    name: String,
    addr: String,
}

#[derive(Default)]
struct State {
    confirmed: HashMap<String, Profile>,
    desired: HashMap<String, Profile>,
    dirty: HashSet<String>,
    deleting: HashSet<String>,
    seen: HashMap<String, Instant>,
    accepting: bool,
    transactions: u64,
    last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PersistenceStatus {
    pub pending_profiles: usize,
    pub successful_transactions: u64,
    pub last_error: Option<String>,
}

pub struct PeerPersistence {
    pool: Pool<Sqlite>,
    state: Mutex<State>,
    wake: Notify,
    // All profile writes and explicit deletes share this gate. No peer lock
    // is held while awaiting SQLite.
    pub(crate) write_gate: AsyncMutex<()>,
}

impl PeerPersistence {
    pub fn new(pool: Pool<Sqlite>, peers: &[Peer]) -> Self {
        let confirmed: HashMap<_, _> = peers
            .iter()
            .map(|p| {
                (
                    p.id.clone(),
                    Profile {
                        id: p.id.clone(),
                        name: p.name.clone(),
                        addr: p.addr.clone(),
                    },
                )
            })
            .collect();
        Self {
            pool,
            state: Mutex::new(State {
                desired: confirmed.clone(),
                confirmed,
                accepting: true,
                ..State::default()
            }),
            wake: Notify::new(),
            write_gate: AsyncMutex::new(()),
        }
    }

    pub fn start_session(&self) {
        let mut state = self.state.lock().unwrap();
        state.accepting = true;
        state.seen.clear();
        state.deleting.clear();
        if !state.dirty.is_empty() {
            self.wake.notify_one();
        }
    }

    /// Called while PeerManager holds its write lock, including during delete
    /// barriers. Equal profiles only refresh the in-memory presence clock.
    pub fn observe(&self, id: &str, name: &str, addr: &str) -> bool {
        let mut state = self.state.lock().unwrap();
        if !state.accepting || state.deleting.contains(id) {
            return false;
        }
        state.seen.insert(id.to_owned(), Instant::now());
        if state
            .desired
            .get(id)
            .is_some_and(|p| p.name == name && p.addr == addr)
        {
            return true;
        }
        let profile = Profile {
            id: id.to_owned(),
            name: name.to_owned(),
            addr: addr.to_owned(),
        };
        state.desired.insert(id.to_owned(), profile.clone());
        if state.confirmed.get(id) == Some(&profile) {
            state.dirty.remove(id);
        } else {
            state.dirty.insert(id.to_owned());
            self.wake.notify_one();
        }
        true
    }

    pub fn is_stale(&self, id: &str) -> bool {
        self.state
            .lock()
            .unwrap()
            .seen
            .get(id)
            .is_none_or(|seen| seen.elapsed() >= PRESENCE_GRACE)
    }

    pub fn status(&self) -> PersistenceStatus {
        let state = self.state.lock().unwrap();
        PersistenceStatus {
            pending_profiles: state.dirty.len(),
            successful_transactions: state.transactions,
            last_error: state.last_error.clone(),
        }
    }

    pub(crate) fn suppress(&self, id: &str) {
        self.state.lock().unwrap().deleting.insert(id.to_owned());
    }

    pub(crate) fn finish_delete(&self, id: &str, success: bool) {
        let mut state = self.state.lock().unwrap();
        if success {
            state.desired.remove(id);
            state.confirmed.remove(id);
            state.dirty.remove(id);
            state.seen.remove(id);
        }
        state.deleting.remove(id);
        if !state.dirty.is_empty() {
            self.wake.notify_one();
        }
    }

    async fn flush(&self) -> Result<(), String> {
        let _gate = self.write_gate.lock().await;
        let profiles: Vec<Profile> = {
            let state = self.state.lock().unwrap();
            state
                .dirty
                .iter()
                .filter(|id| !state.deleting.contains(*id))
                .filter_map(|id| state.desired.get(id).cloned())
                .collect()
        };
        if profiles.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await.map_err(|e| e.to_string())?;
        let mut changed = 0;
        for profile in &profiles {
            changed += sqlx::query(
                "INSERT INTO users (id, name, addr, last_seen, is_offline, available_memory_mb) \
                 VALUES (?, ?, ?, 0, 1, 0) ON CONFLICT(id) DO UPDATE SET name=excluded.name, addr=excluded.addr \
                 WHERE users.name IS NOT excluded.name OR users.addr IS NOT excluded.addr"
            ).bind(&profile.id).bind(&profile.name).bind(&profile.addr)
                .execute(&mut *tx).await.map_err(|e| e.to_string())?.rows_affected();
        }
        tx.commit().await.map_err(|e| e.to_string())?;
        let mut state = self.state.lock().unwrap();
        if changed > 0 {
            state.transactions += 1;
        }
        state.last_error = None;
        for profile in profiles {
            let id = profile.id.clone();
            state.confirmed.insert(id.clone(), profile.clone());
            if state.desired.get(&id) == Some(&profile) {
                state.dirty.remove(&id);
            } else {
                // A -> B (being committed) -> A must schedule A again.
                state.dirty.insert(id);
            }
        }
        Ok(())
    }

    async fn bounded_flush(&self) -> Result<(), String> {
        let result = tokio::time::timeout(WRITE_TIMEOUT, self.flush())
            .await
            .unwrap_or_else(|_| Err("设备资料保存超时".into()));
        if let Err(error) = &result {
            self.state.lock().unwrap().last_error = Some(error.clone());
            eprintln!("[PeerPersistence] 设备资料尚未保存: {error}");
        } else if self.status().pending_profiles == 0 {
            self.state.lock().unwrap().last_error = None;
        }
        result
    }

    pub async fn run(self: Arc<Self>, cancellation: CancellationToken) -> Result<(), String> {
        let mut failures = 0_usize;
        loop {
            if cancellation.is_cancelled() {
                break;
            }
            if self.status().pending_profiles == 0 {
                tokio::select! {
                    _ = cancellation.cancelled() => break,
                    _ = self.wake.notified() => {}
                }
                continue;
            }
            let delay = if failures == 0 {
                COALESCE
            } else {
                Duration::from_secs([5, 15, 30, 60][(failures - 1).min(3)])
            };
            tokio::select! {
                _ = cancellation.cancelled() => break,
                _ = tokio::time::sleep(delay) => {}
            }
            // Notifications from repeated heartbeats cannot shorten retries.
            failures = if self.bounded_flush().await.is_ok() {
                0
            } else {
                failures.saturating_add(1)
            };
        }
        self.state.lock().unwrap().accepting = false;
        self.bounded_flush().await
    }

    /// This task owns deletion until completion even if the UI/HTTP caller
    /// disconnects, so cancellation cannot strand the deletion barrier.
    pub async fn delete(
        self: &Arc<Self>,
        manager: Arc<PeerManager>,
        my_id: String,
        peer_id: String,
    ) -> Result<(), String> {
        let store = self.clone();
        tokio::spawn(async move {
            let _gate = store.write_gate.lock().await;
            manager.begin_profile_delete(&store, &peer_id);
            let result = async {
                let mut tx = store.pool.begin().await.map_err(|e| e.to_string())?;
                sqlx::query("DELETE FROM messages WHERE (sender_id=? AND receiver_id=?) OR (sender_id=? AND receiver_id=?)")
                    .bind(&my_id).bind(&peer_id).bind(&peer_id).bind(&my_id)
                    .execute(&mut *tx).await.map_err(|e| e.to_string())?;
                sqlx::query("DELETE FROM users WHERE id=?").bind(&peer_id)
                    .execute(&mut *tx).await.map_err(|e| e.to_string())?;
                tx.commit().await.map_err(|e| e.to_string())
            }.await;
            manager.end_profile_delete(&store, &peer_id, result.is_ok());
            result
        }).await.map_err(|e| format!("删除设备任务失败: {e}"))?
    }

    /// 与档案写入共用屏障，防止旧的待写任务把刚清除的离线设备写回来。
    pub async fn delete_offline_profile(
        self: &Arc<Self>,
        manager: Arc<PeerManager>,
        peer_id: String,
    ) -> Result<bool, String> {
        let store = self.clone();
        tokio::spawn(async move {
            let _gate = store.write_gate.lock().await;
            if !manager.begin_offline_profile_delete(&store, &peer_id) {
                return Ok(false);
            }
            let result = crate::db::delete_peer_profile(&store.pool, &peer_id).await;
            manager.end_profile_delete(&store, &peer_id, result.is_ok());
            result.map(|_| true)
        }).await.map_err(|e| format!("清除设备任务失败: {e}"))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::io::Read;
    use std::path::PathBuf;

    struct Fixture {
        path: PathBuf,
        pool: Pool<Sqlite>,
        manager: Arc<PeerManager>,
        store: Arc<PeerPersistence>,
    }

    impl Fixture {
        async fn new(existing: bool) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("lq-peer-persistence-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("test.db");
            let pool = SqlitePoolOptions::new()
                .max_connections(3)
                .connect_with(
                    SqliteConnectOptions::new()
                        .filename(&path)
                        .create_if_missing(true)
                        .busy_timeout(Duration::from_secs(1)),
                )
                .await
                .unwrap();
            sqlx::raw_sql(
                "CREATE TABLE users(id TEXT PRIMARY KEY,name TEXT,addr TEXT,last_seen INTEGER,is_offline INTEGER,available_memory_mb INTEGER); \
                 CREATE TABLE messages(id INTEGER PRIMARY KEY,sender_id TEXT,receiver_id TEXT,content TEXT,msg_type TEXT,timestamp INTEGER,file_path TEXT,file_size INTEGER,status TEXT); \
                 CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT); \
                 INSERT INTO settings VALUES('user_id','me'),('username','local');"
            ).execute(&pool).await.unwrap();
            if existing {
                crate::db::save_or_update_user(
                    &pool,
                    "peer".into(),
                    "name".into(),
                    "127.0.0.1:12345".into(),
                    false,
                    4096,
                )
                .await
                .unwrap();
            }
            let manager = Arc::new(PeerManager::new());
            manager.load_from_db(&pool).await.unwrap();
            manager.enable_windows_persistence(pool.clone());
            let store = manager.persistence().unwrap();
            Self {
                path,
                pool,
                manager,
                store,
            }
        }

        fn observe(&self, name: &str, memory: u64) {
            assert!(self
                .manager
                .observe_discovery("peer".into(), name.into(), "127.0.0.1:12345".into(), memory)
                .is_some());
        }

        fn counter(&self) -> u32 {
            let mut header = [0; 28];
            std::fs::File::open(&self.path)
                .unwrap()
                .read_exact(&mut header)
                .unwrap();
            u32::from_be_bytes(header[24..28].try_into().unwrap())
        }

        async fn count(&self, table: &str) -> i64 {
            sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
                .fetch_one(&self.pool)
                .await
                .unwrap()
        }
    }

    #[tokio::test]
    async fn unchanged_heartbeats_memory_and_reconnects_make_zero_disk_commits() {
        let f = Fixture::new(true).await;
        assert!(f.manager.get_all_peers()[0].is_offline);
        assert_eq!(f.manager.get_all_peers()[0].available_memory_mb, 0);
        let before = f.counter();
        let historical: (i64, i64, i64) =
            sqlx::query_as("SELECT last_seen,is_offline,available_memory_mb FROM users")
                .fetch_one(&f.pool)
                .await
                .unwrap();
        for n in 0..1500 {
            if n % 50 == 0 {
                f.manager.force_mark_offline("peer");
            }
            f.observe("name", n);
        }
        f.store.flush().await.unwrap();
        assert_eq!(f.counter(), before);
        assert_eq!(f.store.status().successful_transactions, 0);
        assert_eq!(f.store.status().pending_profiles, 0);
        assert_eq!(f.manager.get_all_peers()[0].available_memory_mb, 1499);
        let after: (i64, i64, i64) =
            sqlx::query_as("SELECT last_seen,is_offline,available_memory_mb FROM users")
                .fetch_one(&f.pool)
                .await
                .unwrap();
        assert_eq!(after, historical);
        // A short heartbeat gap is below the Windows grace period and reads do
        // not mutate presence; the shared monitor performs active confirmation.
        f.store
            .state
            .lock()
            .unwrap()
            .seen
            .insert("peer".into(), Instant::now() - Duration::from_secs(6));
        assert!(!f.store.is_stale("peer"));
        assert_eq!(f.manager.get_active_peers().len(), 1);
        f.store
            .state
            .lock()
            .unwrap()
            .seen
            .insert("peer".into(), Instant::now() - Duration::from_secs(13));
        assert!(f.store.is_stale("peer"));
    }

    #[tokio::test]
    async fn new_peers_are_batched_and_only_identity_fields_are_saved() {
        let f = Fixture::new(false).await;
        f.observe("name", 1234);
        f.manager.observe_discovery(
            "second".into(),
            "second name".into(),
            "127.0.0.1:23456".into(),
            9999,
        );
        f.store.flush().await.unwrap();
        assert_eq!(f.count("users").await, 2);
        assert_eq!(f.store.status().successful_transactions, 1);
        let transient: (i64, i64, i64) = sqlx::query_as(
            "SELECT last_seen,is_offline,available_memory_mb FROM users WHERE id='peer'",
        )
        .fetch_one(&f.pool)
        .await
        .unwrap();
        assert_eq!(transient, (0, 1, 0));
        f.observe("renamed", 5432);
        f.store.flush().await.unwrap();
        let name: String = sqlx::query_scalar("SELECT name FROM users WHERE id='peer'")
            .fetch_one(&f.pool)
            .await
            .unwrap();
        assert_eq!(name, "renamed");
        assert_eq!(f.store.status().successful_transactions, 2);
    }

    #[tokio::test]
    async fn reverted_metadata_change_does_not_open_a_write_transaction() {
        let f = Fixture::new(true).await;
        let before = f.counter();
        f.observe("temporary", 10);
        f.observe("name", 20);
        f.store.flush().await.unwrap();
        assert_eq!(f.counter(), before);
        assert_eq!(f.store.status().pending_profiles, 0);
    }

    async fn wait_for_writer(store: &PeerPersistence) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if store.write_gate.try_lock().is_err() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn metadata_reverted_during_commit_is_not_lost() {
        let f = Fixture::new(true).await;
        let mut blocker = f.pool.acquire().await.unwrap();
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *blocker)
            .await
            .unwrap();
        f.observe("temporary", 1);
        let store = f.store.clone();
        let write = tokio::spawn(async move { store.flush().await });
        wait_for_writer(&f.store).await;
        f.observe("name", 2);
        sqlx::query("COMMIT").execute(&mut *blocker).await.unwrap();
        write.await.unwrap().unwrap();
        assert_eq!(f.store.status().pending_profiles, 1);
        f.store.flush().await.unwrap();
        let name: String = sqlx::query_scalar("SELECT name FROM users")
            .fetch_one(&f.pool)
            .await
            .unwrap();
        assert_eq!(name, "name");
        assert_eq!(f.store.status().pending_profiles, 0);
    }

    #[tokio::test]
    async fn deleting_invalidates_pending_profiles_but_allows_later_rediscovery() {
        let f = Fixture::new(true).await;
        sqlx::query(
            "INSERT INTO messages(id,sender_id,receiver_id) VALUES(1,'me','peer'),(2,'me','other')",
        )
        .execute(&f.pool)
        .await
        .unwrap();
        f.observe("queued change", 1);
        f.manager
            .delete_user_and_history(&f.pool, "me", "peer")
            .await
            .unwrap();
        f.store.flush().await.unwrap();
        assert_eq!(f.count("users").await, 0);
        assert_eq!(f.count("messages").await, 1);
        assert!(f.manager.get_all_peers().is_empty());
        f.observe("name", 2);
        f.store.flush().await.unwrap();
        assert_eq!(f.count("users").await, 1);
        assert_eq!(f.count("messages").await, 1);
    }

    #[tokio::test]
    async fn clearing_offline_profiles_keeps_online_peers_and_chat_history() {
        let f = Fixture::new(true).await;
        sqlx::query("INSERT INTO messages(id,sender_id,receiver_id) VALUES(1,'me','peer')")
            .execute(&f.pool).await.unwrap();
        f.manager.observe_discovery("online".into(), "在线设备".into(), "127.0.0.1:8889".into(), 512);
        let removed = f.manager.clear_offline_peer_profiles(&f.pool).await.unwrap();
        assert_eq!(removed, vec!["peer"]);
        assert_eq!(f.count("messages").await, 1);
        assert!(f.manager.get_all_peers().iter().all(|peer| peer.id != "peer"));
        assert!(f.manager.get_all_peers().iter().any(|peer| peer.id == "online" && !peer.is_offline));
        let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE id='peer'")
            .fetch_one(&f.pool).await.unwrap();
        assert_eq!(remaining, 0);
    }

    #[tokio::test]
    async fn clearing_offline_profiles_skips_reachable_stale_device() {
        let f = Fixture::new(false).await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        f.manager.observe_discovery("reachable".into(), "暂未重发现".into(), addr, 256);
        f.manager.force_mark_offline("reachable");
        let removed = f.manager.clear_offline_peer_profiles(&f.pool).await.unwrap();
        assert!(removed.is_empty());
        assert!(f.manager.get_all_peers().iter().any(|peer| peer.id == "reachable"));
    }

    #[tokio::test]
    async fn deletion_waits_for_inflight_save_and_cannot_be_undone_by_it() {
        let f = Fixture::new(true).await;
        let mut blocker = f.pool.acquire().await.unwrap();
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *blocker)
            .await
            .unwrap();
        f.observe("in flight", 1);
        let store = f.store.clone();
        let write = tokio::spawn(async move { store.flush().await });
        wait_for_writer(&f.store).await;
        let manager = f.manager.clone();
        let pool = f.pool.clone();
        let delete =
            tokio::spawn(async move { manager.delete_user_and_history(&pool, "me", "peer").await });
        f.observe("newer pending", 2);
        sqlx::query("COMMIT").execute(&mut *blocker).await.unwrap();
        write.await.unwrap().unwrap();
        delete.await.unwrap().unwrap();
        f.store.flush().await.unwrap();
        assert_eq!(f.count("users").await, 0);
        assert_eq!(f.store.status().pending_profiles, 0);
    }

    #[tokio::test]
    async fn failed_delete_rolls_back_messages_and_releases_barrier() {
        let f = Fixture::new(true).await;
        sqlx::raw_sql("INSERT INTO messages(id,sender_id,receiver_id) VALUES(1,'me','peer'); CREATE TRIGGER refuse_delete BEFORE DELETE ON users BEGIN SELECT RAISE(ABORT,'test failure'); END;")
            .execute(&f.pool).await.unwrap();
        assert!(f
            .manager
            .delete_user_and_history(&f.pool, "me", "peer")
            .await
            .is_err());
        assert_eq!(f.count("messages").await, 1);
        assert_eq!(f.count("users").await, 1);
        f.observe("still reachable", 10);
        f.store.flush().await.unwrap();
    }

    #[tokio::test]
    async fn failed_save_keeps_latest_profile_and_shutdown_reports_failure() {
        let f = Fixture::new(false).await;
        sqlx::query("CREATE TRIGGER refuse_insert BEFORE INSERT ON users BEGIN SELECT RAISE(ABORT,'test failure'); END;")
            .execute(&f.pool).await.unwrap();
        f.observe("name", 1);
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(f.store.clone().run(cancel).await.is_err());
        assert_eq!(f.store.status().pending_profiles, 1);
        assert!(f.store.status().last_error.is_some());
        sqlx::query("DROP TRIGGER refuse_insert")
            .execute(&f.pool)
            .await
            .unwrap();
        f.store.start_session();
        f.observe("latest", 2);
        f.store.bounded_flush().await.unwrap();
        assert_eq!(f.store.status().pending_profiles, 0);
        assert!(f.store.status().last_error.is_none());
    }

    #[tokio::test]
    async fn shutdown_flushes_only_pending_metadata_and_preserves_chat_records() {
        let f = Fixture::new(false).await;
        sqlx::query("INSERT INTO messages(id,sender_id,receiver_id,content) VALUES(1,'me','peer','keep this')").execute(&f.pool).await.unwrap();
        f.observe("name", 1);
        let cancel = CancellationToken::new();
        cancel.cancel();
        f.store.clone().run(cancel).await.unwrap();
        assert_eq!(f.count("users").await, 1);
        assert_eq!(f.count("messages").await, 1);
        let before = f.counter();
        let cancel = CancellationToken::new();
        cancel.cancel();
        f.store.clone().run(cancel).await.unwrap();
        assert_eq!(before, f.counter());
    }

    #[tokio::test]
    async fn loopback_discovery_keeps_receiving_without_heartbeat_database_writes() {
        let f = Fixture::new(true).await;
        let before = f.counter();
        let listener = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let target = listener.local_addr().unwrap();
        let sender = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let bus = crate::core_events::CoreEventBus::default();
        let mut events = bus.subscribe();
        let cancel = CancellationToken::new();
        let worker = tokio::spawn(f.store.clone().run(cancel.child_token()));
        let discovery = tokio::spawn(crate::network::discovery::run_listener(
            listener,
            target.port(),
            "me".into(),
            "local".into(),
            f.manager.clone(),
            f.pool.clone(),
            bus,
            cancel.child_token(),
        ));
        for n in 0..100 {
            let packet = format!("LANChat|ONLINE|peer|name|12345|{n}|1");
            sender.send_to(packet.as_bytes(), target).await.unwrap();
            tokio::time::timeout(Duration::from_secs(2), events.recv())
                .await
                .unwrap()
                .unwrap();
        }
        cancel.cancel();
        discovery.await.unwrap().unwrap();
        worker.await.unwrap().unwrap();
        assert_eq!(f.counter(), before);
        assert_eq!(f.store.status().successful_transactions, 0);
        assert_eq!(f.manager.get_all_peers()[0].available_memory_mb, 99);
    }

    #[tokio::test]
    async fn default_manager_preserves_legacy_path_without_enabling_windows_policy() {
        let f = Fixture::new(true).await;
        let legacy = PeerManager::new();
        legacy.load_from_db(&f.pool).await.unwrap();
        assert!(legacy.persistence().is_none());
        assert_eq!(legacy.get_all_peers()[0].available_memory_mb, 4096);
    }

    #[tokio::test]
    async fn event_driven_worker_coalesces_and_retries_without_heartbeat_wakeups() {
        let f = Fixture::new(false).await;
        sqlx::query("CREATE TRIGGER refuse_insert BEFORE INSERT ON users BEGIN SELECT RAISE(ABORT,'test failure'); END;")
            .execute(&f.pool).await.unwrap();
        let cancel = CancellationToken::new();
        let worker = tokio::spawn(f.store.clone().run(cancel.child_token()));
        f.observe("name", 1);
        // No immediate per-packet commit; after the first failure, identical
        // heartbeats and corrected DB permissions must not bypass the backoff.
        tokio::time::timeout(Duration::from_secs(3), async {
            while f.store.status().last_error.is_none() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        sqlx::query("DROP TRIGGER refuse_insert")
            .execute(&f.pool)
            .await
            .unwrap();
        for n in 0..20 {
            f.observe("name", n);
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(f.store.status().successful_transactions, 0);
        tokio::time::timeout(Duration::from_secs(7), async {
            while f.store.status().pending_profiles > 0 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(f.store.status().successful_transactions, 1);
        cancel.cancel();
        worker.await.unwrap().unwrap();
    }

    #[tokio::test]
    #[ignore = "10-minute controlled disk test; uses only temporary databases and loopback UDP"]
    async fn ten_minute_idle_heartbeat_disk_comparison() {
        let legacy = Fixture::new(true).await;
        let optimized = Fixture::new(true).await;
        let legacy_before = legacy.counter();
        let optimized_before = optimized.counter();
        let listener = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let target = listener.local_addr().unwrap();
        let sender = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let bus = crate::core_events::CoreEventBus::default();
        let mut events = bus.subscribe();
        let cancel = CancellationToken::new();
        let worker = tokio::spawn(optimized.store.clone().run(cancel.child_token()));
        let discovery = tokio::spawn(crate::network::discovery::run_listener(
            listener,
            target.port(),
            "me".into(),
            "local".into(),
            optimized.manager.clone(),
            optimized.pool.clone(),
            bus,
            cancel.child_token(),
        ));
        let started = Instant::now();
        let mut interval = tokio::time::interval(Duration::from_millis(400));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut packets = 0_u64;
        while started.elapsed() < Duration::from_secs(600) {
            interval.tick().await;
            packets += 1;
            crate::db::save_or_update_user(
                &legacy.pool,
                "peer".into(),
                "name".into(),
                "127.0.0.1:12345".into(),
                false,
                packets,
            )
            .await
            .unwrap();
            let packet = format!("LANChat|ONLINE|peer|name|12345|{packets}|1");
            sender.send_to(packet.as_bytes(), target).await.unwrap();
            tokio::time::timeout(Duration::from_secs(2), events.recv())
                .await
                .unwrap()
                .unwrap();
        }
        cancel.cancel();
        discovery.await.unwrap().unwrap();
        worker.await.unwrap().unwrap();
        let old_delta = legacy.counter() - legacy_before;
        let new_delta = optimized.counter() - optimized_before;
        println!("IDLE_DISK_COMPARISON seconds={:.2} heartbeats={} legacy_db_changes={} optimized_db_changes={} optimized_profile_transactions={}",
            started.elapsed().as_secs_f64(), packets, old_delta, new_delta, optimized.store.status().successful_transactions);
        assert_eq!(old_delta as u64, packets);
        assert_eq!(new_delta, 0);
        assert_eq!(optimized.count("messages").await, 0);
    }
}
