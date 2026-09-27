use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, OnceLock, Weak};
use std::time::Duration;

use serde::Serialize;
use sqlx::{Pool, Sqlite};
use tokio::runtime::Runtime;
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;

use crate::core_events::{CoreEvent, CoreEventBus, CoreState, CoreStatus};
use crate::peers::PeerManager;

const STOP_TIMEOUT: Duration = Duration::from_secs(8);
const STATE_WAIT_TIMEOUT: Duration = Duration::from_secs(12);
const DATABASE_CLOSE_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PoolOwnership {
    Shared,
    Owned,
}

struct PreparedCore {
    pool: Pool<Sqlite>,
    peer_manager: Arc<PeerManager>,
    pool_ownership: PoolOwnership,
}

struct RunningCore {
    pool: Pool<Sqlite>,
    peer_manager: Arc<PeerManager>,
    cancellation: CancellationToken,
    supervisor: JoinHandle<()>,
    pool_ownership: PoolOwnership,
}

struct Inner {
    status: CoreStatus,
    runtime: Option<Runtime>,
    prepared: Option<PreparedCore>,
    running: Option<RunningCore>,
    startup_cancellation: Option<CancellationToken>,
    peer_manager_hint: Option<Weak<PeerManager>>,
    last_stop_report: Option<CoreStopReport>,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            status: CoreStatus::default(),
            runtime: None,
            prepared: None,
            running: None,
            startup_cancellation: None,
            peer_manager_hint: None,
            last_stop_report: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CoreStopReport {
    pub ok: bool,
    pub status: CoreStatus,
    pub tasks_stopped: bool,
    pub ports_released: bool,
    pub resources_released: bool,
    pub forced: bool,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

impl CoreStopReport {
    fn failure(status: CoreStatus, code: &str, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            status,
            tasks_stopped: false,
            ports_released: false,
            resources_released: false,
            forced: false,
            error_code: Some(code.to_string()),
            error_message: Some(message.into()),
        }
    }
}

pub struct CoreRuntime {
    inner: Mutex<Inner>,
    state_changed: Condvar,
    event_bus: CoreEventBus,
}

static CORE_RUNTIME: OnceLock<CoreRuntime> = OnceLock::new();

impl CoreRuntime {
    fn new() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            state_changed: Condvar::new(),
            event_bus: CoreEventBus::default(),
        }
    }

    pub fn global() -> &'static Self {
        CORE_RUNTIME.get_or_init(Self::new)
    }

    pub fn event_bus(&self) -> CoreEventBus {
        self.event_bus.clone()
    }

    pub fn status(&self) -> CoreStatus {
        self.inner
            .lock()
            .expect("core mutex poisoned")
            .status
            .clone()
    }

    pub fn prepare(&self, pool: Pool<Sqlite>, peer_manager: Arc<PeerManager>) {
        let mut inner = self.inner.lock().expect("core mutex poisoned");
        inner.peer_manager_hint = Some(Arc::downgrade(&peer_manager));
        if inner.prepared.is_none() && inner.running.is_none() {
            inner.prepared = Some(PreparedCore {
                pool,
                peer_manager,
                pool_ownership: PoolOwnership::Shared,
            });
        }
    }

    /// Read-only access to the current session; does not start or extend its lifetime.
    pub fn notification_session(&self) -> Option<(tokio::runtime::Handle, CancellationToken, u64)> {
        let inner = self.inner.lock().ok()?;
        if inner.status.state != CoreState::Running {
            return None;
        }
        Some((
            inner.runtime.as_ref()?.handle().clone(),
            inner.running.as_ref()?.cancellation.child_token(),
            inner.status.generation,
        ))
    }

    pub fn shared_resources(&self) -> Option<(Pool<Sqlite>, Arc<PeerManager>)> {
        let inner = self.inner.lock().expect("core mutex poisoned");
        inner
            .running
            .as_ref()
            .map(|running| (running.pool.clone(), running.peer_manager.clone()))
            .or_else(|| {
                inner
                    .prepared
                    .as_ref()
                    .map(|prepared| (prepared.pool.clone(), prepared.peer_manager.clone()))
            })
    }

    pub fn start(&'static self, app_data_dir: PathBuf) -> Result<CoreStatus, String> {
        self.start_with_port(app_data_dir, None)
    }

    pub fn start_with_port(
        &'static self,
        app_data_dir: PathBuf,
        port_override: Option<u16>,
    ) -> Result<CoreStatus, String> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "CORE_LOCK_POISONED".to_string())?;
        match inner.status.state {
            CoreState::Running | CoreState::Starting => return Ok(inner.status.clone()),
            CoreState::Stopping => return Err("CORE_STOPPING".to_string()),
            CoreState::Stopped | CoreState::Error => {}
        }

        inner.status.generation = inner.status.generation.saturating_add(1);
        inner.status.state = CoreState::Starting;
        inner.status.port = None;
        inner.status.started_at = None;
        inner.status.last_error_code = None;
        inner.status.last_error_message = None;
        inner.last_stop_report = None;
        let generation = inner.status.generation;
        let startup_cancellation = CancellationToken::new();
        inner.startup_cancellation = Some(startup_cancellation.clone());
        let peer_manager_hint = inner.peer_manager_hint.as_ref().and_then(Weak::upgrade);
        self.event_bus
            .publish(CoreEvent::CoreStateChanged(inner.status.clone()));

        if inner.runtime.is_none() {
            match Runtime::new() {
                Ok(runtime) => inner.runtime = Some(runtime),
                Err(error) => {
                    inner.startup_cancellation = None;
                    self.fail_locked(&mut inner, "RUNTIME_CREATE_FAILED", error.to_string());
                    self.state_changed.notify_all();
                    return Err(error.to_string());
                }
            }
        }

        let runtime = inner.runtime.take().expect("runtime initialized");
        let prepared = inner.prepared.take();
        drop(inner);
        let result = runtime.block_on(self.start_async(
            app_data_dir,
            port_override,
            generation,
            prepared,
            peer_manager_hint,
            startup_cancellation,
        ));
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "CORE_LOCK_POISONED".to_string())?;
        inner.runtime = Some(runtime);
        inner.startup_cancellation = None;
        match result {
            Ok((port, running)) => {
                inner.status.state = CoreState::Running;
                inner.status.port = Some(port);
                inner.status.started_at = Some(chrono::Utc::now().timestamp());
                inner.running = Some(running);
                self.event_bus
                    .publish(CoreEvent::CoreStateChanged(inner.status.clone()));
                self.state_changed.notify_all();
                Ok(inner.status.clone())
            }
            Err((code, message)) => {
                self.fail_locked(&mut inner, &code, message.clone());
                self.state_changed.notify_all();
                Err(message)
            }
        }
    }

    async fn start_async(
        &'static self,
        app_data_dir: PathBuf,
        port_override: Option<u16>,
        generation: u64,
        prepared: Option<PreparedCore>,
        peer_manager_hint: Option<Arc<PeerManager>>,
        startup_cancellation: CancellationToken,
    ) -> Result<(u16, RunningCore), (String, String)> {
        let prepared = match prepared {
            Some(prepared) => prepared,
            None => {
                let pool = crate::db::init_db_with_path(app_data_dir)
                    .await
                    .map_err(|error| ("DATABASE_OPEN_FAILED".into(), error.to_string()))?;
                let peer_manager =
                    peer_manager_hint.unwrap_or_else(|| Arc::new(PeerManager::new()));
                if let Err(error) = peer_manager.load_from_db(&pool).await {
                    pool.close().await;
                    return Err(("PEER_LOAD_FAILED".into(), error));
                }
                PreparedCore {
                    pool,
                    peer_manager,
                    pool_ownership: PoolOwnership::Owned,
                }
            }
        };

        if startup_cancellation.is_cancelled() {
            Self::release_prepared(prepared).await;
            return Err(("START_CANCELLED".into(), "核心启动已取消".into()));
        }

        let port = match port_override {
            Some(port) => port,
            None => crate::db::get_port(&prepared.pool).await.unwrap_or(8888),
        };
        let my_id = match crate::db::get_user_id(&prepared.pool).await {
            Ok(value) => value,
            Err(error) => {
                Self::release_prepared(prepared).await;
                return Err(("IDENTITY_LOAD_FAILED".into(), error));
            }
        };
        let my_name = crate::db::get_username(&prepared.pool)
            .await
            .unwrap_or_else(|_| "Unknown".to_string());

        // All sockets are bound before any long-running task is spawned. Dropping
        // these values rolls the startup transaction back without a half-running core.
        let udp_listener = match crate::network::discovery::bind_listener(port).await {
            Ok(socket) => socket,
            Err(error) => {
                Self::release_prepared(prepared).await;
                return Err(("UDP_BIND_FAILED".into(), error.to_string()));
            }
        };
        let udp_announcer = match crate::network::discovery::bind_announcer().await {
            Ok(socket) => socket,
            Err(error) => {
                drop(udp_listener);
                Self::release_prepared(prepared).await;
                return Err(("UDP_ANNOUNCER_BIND_FAILED".into(), error.to_string()));
            }
        };
        let http_listener = match crate::web_server::bind_server(port).await {
            Ok(listener) => listener,
            Err(error) => {
                drop(udp_announcer);
                drop(udp_listener);
                Self::release_prepared(prepared).await;
                return Err(("HTTP_BIND_FAILED".into(), error.to_string()));
            }
        };

        if startup_cancellation.is_cancelled() {
            drop(http_listener);
            drop(udp_announcer);
            drop(udp_listener);
            Self::release_prepared(prepared).await;
            return Err(("START_CANCELLED".into(), "核心启动已取消".into()));
        }

        let cancellation = CancellationToken::new();
        let supervisor_cancellation = cancellation.clone();
        let pool = prepared.pool;
        let peer_manager = prepared.peer_manager;
        peer_manager.begin_presence_session();
        let pool_ownership = prepared.pool_ownership;
        let supervisor_pool = pool.clone();
        let supervisor_peer_manager = peer_manager.clone();
        let event_bus = self.event_bus.clone();
        let supervisor = tokio::spawn(async move {
            let mut tasks = JoinSet::<Result<(), String>>::new();
            #[cfg(windows)]
            if let Some(store) = supervisor_peer_manager.persistence() {
                tasks.spawn(store.run(supervisor_cancellation.child_token()));
            }
            #[cfg(target_os = "android")]
            tasks.spawn(crate::db::run_chat_retention(
                supervisor_pool.clone(),
                supervisor_cancellation.child_token(),
            ));
            #[cfg(target_os = "android")]
            tasks.spawn(crate::notification_sync::run_route_status_monitor(
                supervisor_pool.clone(),
                supervisor_peer_manager.clone(),
                event_bus.clone(),
                supervisor_cancellation.child_token(),
            ));
            tasks.spawn(crate::peers::run_presence_monitor(
                supervisor_peer_manager.clone(),
                supervisor_cancellation.child_token(),
            ));
            tasks.spawn(crate::network::discovery::run_listener(
                udp_listener,
                port,
                my_id.clone(),
                my_name,
                supervisor_peer_manager.clone(),
                supervisor_pool.clone(),
                event_bus.clone(),
                supervisor_cancellation.child_token(),
            ));
            tasks.spawn(crate::network::discovery::run_announcer(
                udp_announcer,
                port,
                my_id,
                supervisor_pool.clone(),
                supervisor_cancellation.child_token(),
            ));
            tasks.spawn(crate::web_server::run_server(
                http_listener,
                supervisor_pool.clone(),
                supervisor_peer_manager,
                event_bus,
                supervisor_cancellation.child_token(),
            ));

            let first = tasks.join_next().await;
            if !supervisor_cancellation.is_cancelled() {
                let message = match first {
                    Some(Ok(Ok(()))) => "核心网络任务意外退出".to_string(),
                    Some(Ok(Err(error))) => error,
                    Some(Err(error)) => format!("核心网络任务 panic: {error}"),
                    None => "核心网络任务集合意外为空".to_string(),
                };
                supervisor_cancellation.cancel();
                tasks.abort_all();
                while tasks.join_next().await.is_some() {}
                self.complete_unexpected_exit(generation, message);
            } else {
                while tasks.join_next().await.is_some() {}
            }
        });

        Ok((
            port,
            RunningCore {
                pool,
                peer_manager,
                cancellation,
                supervisor,
                pool_ownership,
            },
        ))
    }

    pub fn stop(&'static self) -> Result<CoreStatus, String> {
        let report = self.stop_report();
        if report.ok {
            Ok(report.status)
        } else {
            Err(report
                .error_message
                .unwrap_or_else(|| "核心停止后置条件未满足".to_string()))
        }
    }

    pub fn stop_report(&'static self) -> CoreStopReport {
        self.stop_report_internal(false)
    }

    pub fn force_stop_report(&'static self) -> CoreStopReport {
        if let Ok(inner) = self.inner.lock() {
            if let Some(cancellation) = &inner.startup_cancellation {
                cancellation.cancel();
            }
            if let Some(running) = &inner.running {
                running.cancellation.cancel();
                running.supervisor.abort();
            }
        }
        self.stop_report_internal(true)
    }

    fn stop_report_internal(&'static self, force_requested: bool) -> CoreStopReport {
        let (runtime, running, prepared, port) = {
            let mut inner = match self.inner.lock() {
                Ok(inner) => inner,
                Err(_) => {
                    return CoreStopReport::failure(
                        CoreStatus::default(),
                        "CORE_LOCK_POISONED",
                        "核心状态锁已损坏",
                    )
                }
            };

            if inner.status.state == CoreState::Starting {
                if let Some(cancellation) = &inner.startup_cancellation {
                    cancellation.cancel();
                }
                let waited =
                    self.state_changed
                        .wait_timeout_while(inner, STATE_WAIT_TIMEOUT, |inner| {
                            inner.status.state == CoreState::Starting
                        });
                let Ok((waited_inner, timeout)) = waited else {
                    return CoreStopReport::failure(
                        CoreStatus::default(),
                        "CORE_LOCK_POISONED",
                        "等待核心启动取消时状态锁已损坏",
                    );
                };
                inner = waited_inner;
                if timeout.timed_out() && inner.status.state == CoreState::Starting {
                    return CoreStopReport::failure(
                        inner.status.clone(),
                        "START_CANCEL_TIMEOUT",
                        "等待核心启动取消超时",
                    );
                }
            }

            if inner.status.state == CoreState::Stopping {
                let waited =
                    self.state_changed
                        .wait_timeout_while(inner, STATE_WAIT_TIMEOUT, |inner| {
                            inner.status.state == CoreState::Stopping
                        });
                let Ok((waited_inner, timeout)) = waited else {
                    return CoreStopReport::failure(
                        CoreStatus::default(),
                        "CORE_LOCK_POISONED",
                        "等待重复停止时状态锁已损坏",
                    );
                };
                inner = waited_inner;
                if timeout.timed_out() && inner.status.state == CoreState::Stopping {
                    return CoreStopReport::failure(
                        inner.status.clone(),
                        "STOP_WAIT_TIMEOUT",
                        "等待正在进行的停止操作超时",
                    );
                }
                if let Some(report) = &inner.last_stop_report {
                    return report.clone();
                }
            }

            if inner.status.state == CoreState::Stopped
                && inner.runtime.is_none()
                && inner.running.is_none()
                && inner.prepared.is_none()
            {
                return inner
                    .last_stop_report
                    .clone()
                    .unwrap_or_else(|| CoreStopReport {
                        ok: true,
                        status: inner.status.clone(),
                        tasks_stopped: true,
                        ports_released: true,
                        resources_released: true,
                        forced: force_requested,
                        error_code: None,
                        error_message: None,
                    });
            }

            inner.status.state = CoreState::Stopping;
            self.event_bus
                .publish(CoreEvent::CoreStateChanged(inner.status.clone()));
            let running = inner.running.take();
            let prepared = inner.prepared.take();
            let port = inner.status.port;
            let runtime = inner.runtime.take();
            inner.startup_cancellation = None;
            (runtime, running, prepared, port)
        };

        let mut forced = force_requested;
        let mut tasks_stopped = running.is_none();
        let mut resources_released = true;
        let mut profile_save_error: Option<String> = None;

        if let Some(runtime) = runtime {
            let cleanup = runtime.block_on(async move {
                let mut forced_task_abort = false;
                let mut stopped = true;
                let profile_error: Option<String> = None;
                #[cfg(windows)]
                let mut profile_error = profile_error;
                if let Some(mut running) = running {
                    running.cancellation.cancel();
                    if tokio::time::timeout(STOP_TIMEOUT, &mut running.supervisor)
                        .await
                        .is_err()
                    {
                        forced_task_abort = true;
                        running.supervisor.abort();
                        stopped =
                            tokio::time::timeout(Duration::from_secs(1), &mut running.supervisor)
                                .await
                                .is_ok();
                    }
                    #[cfg(windows)]
                    if let Some(store) = running.peer_manager.persistence() {
                        let status = store.status();
                        if status.pending_profiles > 0 {
                            profile_error = Some(
                                status
                                    .last_error
                                    .unwrap_or_else(|| "停止时仍有设备资料尚未保存".into()),
                            );
                        }
                    }
                    if running.pool_ownership == PoolOwnership::Owned
                        && tokio::time::timeout(DATABASE_CLOSE_TIMEOUT, running.pool.close())
                            .await
                            .is_err()
                    {
                        forced_task_abort = true;
                    }
                    drop(running.peer_manager);
                    drop(running.pool);
                }
                if let Some(prepared) = prepared {
                    if !Self::release_prepared(prepared).await {
                        forced_task_abort = true;
                    }
                }
                (stopped, forced_task_abort, profile_error)
            });
            forced |= !cleanup.0 || cleanup.1;
            profile_save_error = cleanup.2;

            // Runtime destruction is the final boundary for active Axum connection
            // and file-transfer tasks that are children of the Core session.
            runtime.shutdown_timeout(Duration::from_secs(2));
            tasks_stopped = true;
        } else {
            if running.is_some() {
                resources_released = false;
            }
            drop(running);
            drop(prepared);
        }

        let port_result = port.map(Self::verify_port_released).unwrap_or(Ok(()));
        let ports_released = port_result.is_ok();
        let mut inner = match self.inner.lock() {
            Ok(inner) => inner,
            Err(_) => {
                return CoreStopReport::failure(
                    CoreStatus::default(),
                    "CORE_LOCK_POISONED",
                    "写入停止结果时核心状态锁已损坏",
                )
            }
        };
        resources_released &= inner.running.is_none()
            && inner.prepared.is_none()
            && inner.runtime.is_none()
            && inner.startup_cancellation.is_none();

        let (error_code, error_message) = if !tasks_stopped {
            (
                Some("TASK_STOP_INCOMPLETE".to_string()),
                Some("仍有核心任务未确认退出".to_string()),
            )
        } else if !resources_released {
            (
                Some("RESOURCE_RELEASE_INCOMPLETE".to_string()),
                Some("仍有核心资源未释放".to_string()),
            )
        } else if let Err(error) = port_result {
            (Some("PORT_RELEASE_INCOMPLETE".to_string()), Some(error))
        } else if let Some(error) = profile_save_error {
            (
                Some("PEER_PROFILE_SAVE_INCOMPLETE".to_string()),
                Some(error),
            )
        } else {
            (None, None)
        };

        let ok = error_code.is_none();
        if ok {
            inner.status.state = CoreState::Stopped;
            inner.status.port = None;
            inner.status.started_at = None;
            inner.status.last_error_code = None;
            inner.status.last_error_message = None;
            self.event_bus
                .publish(CoreEvent::CoreStateChanged(inner.status.clone()));
        } else {
            self.fail_locked(
                &mut inner,
                error_code.as_deref().unwrap_or("STOP_INCOMPLETE"),
                error_message
                    .clone()
                    .unwrap_or_else(|| "核心停止不完整".to_string()),
            );
        }
        let report = CoreStopReport {
            ok,
            status: inner.status.clone(),
            tasks_stopped,
            ports_released,
            resources_released,
            forced,
            error_code,
            error_message,
        };
        inner.last_stop_report = Some(report.clone());
        self.state_changed.notify_all();
        report
    }

    async fn release_prepared(prepared: PreparedCore) -> bool {
        let closed = if prepared.pool_ownership == PoolOwnership::Owned {
            tokio::time::timeout(DATABASE_CLOSE_TIMEOUT, prepared.pool.close())
                .await
                .is_ok()
        } else {
            true
        };
        drop(prepared.peer_manager);
        drop(prepared.pool);
        closed
    }

    fn verify_port_released(port: u16) -> Result<(), String> {
        let udp = std::net::UdpSocket::bind(("0.0.0.0", port))
            .map_err(|error| format!("UDP 端口仍被占用: {error}"))?;
        let tcp = std::net::TcpListener::bind(("0.0.0.0", port))
            .map_err(|error| format!("HTTP 端口仍被占用: {error}"))?;
        drop(tcp);
        drop(udp);
        Ok(())
    }

    fn fail_locked(&self, inner: &mut Inner, code: &str, message: String) {
        inner.status.state = CoreState::Error;
        inner.status.port = None;
        inner.status.started_at = None;
        inner.status.last_error_code = Some(code.to_string());
        inner.status.last_error_message = Some(message);
        self.event_bus
            .publish(CoreEvent::CoreError(inner.status.clone()));
    }

    fn complete_unexpected_exit(&self, generation: u64, message: String) {
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        if inner.status.generation != generation || inner.status.state != CoreState::Running {
            return;
        }
        let running = inner.running.take();
        self.fail_locked(&mut inner, "NETWORK_TASK_EXITED", message);
        self.state_changed.notify_all();
        drop(inner);
        if let Some(running) = running {
            tokio::spawn(async move {
                if running.pool_ownership == PoolOwnership::Owned {
                    let _ =
                        tokio::time::timeout(DATABASE_CLOSE_TIMEOUT, running.pool.close()).await;
                }
                drop(running.peer_manager);
                drop(running.pool);
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{TcpListener, UdpSocket};

    #[test]
    fn status_starts_stopped_and_generation_zero() {
        let status = CoreStatus::default();
        assert_eq!(status.state, CoreState::Stopped);
        assert_eq!(status.generation, 0);
    }

    #[test]
    fn startup_rolls_back_and_start_stop_are_idempotent() {
        let test_dir =
            std::env::temp_dir().join(format!("lanchat-core-test-{}", uuid::Uuid::new_v4()));
        let setup_runtime = Runtime::new().unwrap();
        let pool = setup_runtime
            .block_on(crate::db::init_db_with_path(test_dir.clone()))
            .unwrap();
        let blocker = TcpListener::bind("0.0.0.0:0").unwrap();
        let port = blocker.local_addr().unwrap().port();
        setup_runtime
            .block_on(crate::db::set_port(&pool, port))
            .unwrap();

        let core = Box::leak(Box::new(CoreRuntime::new()));
        core.prepare(pool, Arc::new(PeerManager::new()));
        assert!(core.start(test_dir.clone()).is_err());
        let failed = core.status();
        assert_eq!(failed.state, CoreState::Error);
        assert_eq!(failed.generation, 1);

        drop(blocker);
        let running = core.start(test_dir.clone()).unwrap();
        assert_eq!(running.state, CoreState::Running);
        assert_eq!(running.generation, 2);
        let duplicate = core.start(PathBuf::new()).unwrap();
        assert_eq!(duplicate.generation, running.generation);

        let stopped = core.stop().unwrap();
        assert_eq!(stopped.state, CoreState::Stopped);
        assert!(core.shared_resources().is_none());
        let duplicate_stop = core.stop().unwrap();
        assert_eq!(duplicate_stop.generation, stopped.generation);
        {
            let _tcp = TcpListener::bind(("0.0.0.0", port)).unwrap();
            let _udp = UdpSocket::bind(("0.0.0.0", port)).unwrap();
        }

        let restarted = core.start(test_dir).unwrap();
        assert_eq!(restarted.state, CoreState::Running);
        assert_eq!(restarted.generation, 3);
        assert!(core.stop_report().ok);
        assert!(core.shared_resources().is_none());
    }

    #[test]
    fn stopping_prepared_shared_resources_does_not_close_tauri_pool() {
        let test_dir =
            std::env::temp_dir().join(format!("lanchat-shared-pool-test-{}", uuid::Uuid::new_v4()));
        let setup_runtime = Runtime::new().unwrap();
        let pool = setup_runtime
            .block_on(crate::db::init_db_with_path(test_dir))
            .unwrap();
        let core = Box::leak(Box::new(CoreRuntime::new()));
        core.prepare(pool.clone(), Arc::new(PeerManager::new()));

        let report = core.stop_report();
        assert!(report.ok);
        assert!(report.resources_released);
        assert!(core.shared_resources().is_none());
        setup_runtime
            .block_on(crate::db::get_username(&pool))
            .expect("Tauri DbState pool must remain usable");
    }

    #[test]
    fn failed_port_postcondition_never_reports_stopped() {
        let blocker = TcpListener::bind("0.0.0.0:0").unwrap();
        let port = blocker.local_addr().unwrap().port();
        let core = Box::leak(Box::new(CoreRuntime::new()));
        {
            let mut inner = core.inner.lock().unwrap();
            inner.status.state = CoreState::Running;
            inner.status.port = Some(port);
            inner.runtime = Some(Runtime::new().unwrap());
        }

        let report = core.stop_report();
        assert!(!report.ok);
        assert!(!report.ports_released);
        assert_eq!(report.status.state, CoreState::Error);
        assert_eq!(
            report.error_code.as_deref(),
            Some("PORT_RELEASE_INCOMPLETE")
        );

        drop(blocker);
        let forced = core.force_stop_report();
        assert!(forced.ok);
        assert!(forced.forced);
        assert_eq!(forced.status.state, CoreState::Stopped);
    }

    #[cfg(windows)]
    #[test]
    fn windows_stop_reports_profile_flush_success_or_failure() {
        for fail_write in [false, true] {
            let test_dir = std::env::temp_dir().join(format!(
                "lanchat-windows-stop-test-{}",
                uuid::Uuid::new_v4()
            ));
            let setup_runtime = Runtime::new().unwrap();
            let pool = setup_runtime
                .block_on(crate::db::init_db_with_path(test_dir))
                .unwrap();
            if fail_write {
                setup_runtime.block_on(sqlx::query(
                    "CREATE TRIGGER reject_profile BEFORE INSERT ON users BEGIN SELECT RAISE(ABORT, 'test write failure'); END"
                ).execute(&pool)).unwrap();
            }
            let manager = Arc::new(PeerManager::new());
            manager.enable_windows_persistence(pool.clone());
            manager.observe_discovery(
                "peer".into(),
                "latest name".into(),
                "127.0.0.1:12345".into(),
                4096,
            );
            let store = manager.persistence().unwrap();
            let runtime = Runtime::new().unwrap();
            let cancellation = CancellationToken::new();
            let worker_cancellation = cancellation.child_token();
            let supervisor = runtime.spawn(async move {
                let _ = store.run(worker_cancellation).await;
            });
            let core = Box::leak(Box::new(CoreRuntime::new()));
            {
                let mut inner = core.inner.lock().unwrap();
                inner.status.state = CoreState::Running;
                inner.runtime = Some(runtime);
                inner.running = Some(RunningCore {
                    pool: pool.clone(),
                    peer_manager: manager.clone(),
                    cancellation,
                    supervisor,
                    pool_ownership: PoolOwnership::Shared,
                });
            }
            let report = core.stop_report();
            assert_eq!(report.ok, !fail_write);
            assert!(report.tasks_stopped && report.ports_released && report.resources_released);
            assert!(core.shared_resources().is_none());
            if fail_write {
                assert_eq!(
                    report.error_code.as_deref(),
                    Some("PEER_PROFILE_SAVE_INCOMPLETE")
                );
                assert_eq!(manager.persistence().unwrap().status().pending_profiles, 1);
            } else {
                assert_eq!(manager.persistence().unwrap().status().pending_profiles, 0);
            }
            let count: i64 = setup_runtime
                .block_on(
                    sqlx::query_scalar("SELECT count(*) FROM users WHERE id='peer'")
                        .fetch_one(&pool),
                )
                .unwrap();
            assert_eq!(count, if fail_write { 0 } else { 1 });
            assert!(manager
                .observe_discovery(
                    "later".into(),
                    "ignored".into(),
                    "127.0.0.1:12345".into(),
                    0
                )
                .is_none());
            setup_runtime.block_on(pool.close());
        }
    }
}
