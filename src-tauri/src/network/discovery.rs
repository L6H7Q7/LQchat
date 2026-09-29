use socket2::{Domain, Protocol, Socket, Type};
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

use crate::core_events::{CoreEvent, CoreEventBus};
use crate::peers::PeerManager;

const MULTICAST_IP: &str = "224.0.0.167";
const PRESENCE_TARGET_LIMIT: usize = 32;
const PRESENCE_BYTES_LIMIT: usize = 700;

fn valid_device_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
}

fn encode_notification_presence(
    enabled: bool,
    receive_enabled: bool,
    target_device_ids: &[String],
) -> String {
    let mut encoded = String::new();
    for id in target_device_ids
        .iter()
        .filter(|id| valid_device_id(id))
        .take(PRESENCE_TARGET_LIMIT)
    {
        let next_len = encoded.len() + usize::from(!encoded.is_empty()) + id.len();
        if next_len > PRESENCE_BYTES_LIMIT {
            break;
        }
        if !encoded.is_empty() {
            encoded.push(',');
        }
        encoded.push_str(id);
    }
    format!(
        "0|{}|{encoded}|{}",
        if enabled { "1" } else { "0" },
        if receive_enabled { "1" } else { "0" }
    )
}

fn parse_notification_presence(parts: &[&str]) -> Option<(bool, bool, Vec<String>)> {
    if parts.len() < 9 {
        return None;
    }
    let enabled = parts[7] == "1";
    let target_device_ids = parts[8]
        .split(',')
        .filter(|id| valid_device_id(id))
        .take(PRESENCE_TARGET_LIMIT)
        .map(str::to_owned)
        .collect();
    Some((enabled, parts.get(9) == Some(&"1"), target_device_ids))
}

fn create_discovery_socket(bind_addr: &str, is_listener: bool) -> std::io::Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    #[cfg(not(target_os = "windows"))]
    socket.set_reuse_port(true)?;

    let addr: SocketAddr = bind_addr
        .parse()
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
    socket.bind(&addr.into())?;

    let socket: UdpSocket = socket.into();
    socket.set_nonblocking(true)?;
    if is_listener {
        socket.join_multicast_v4(
            &MULTICAST_IP
                .parse::<Ipv4Addr>()
                .expect("valid multicast address"),
            &Ipv4Addr::UNSPECIFIED,
        )?;
    } else {
        socket.set_broadcast(true)?;
        socket.set_multicast_ttl_v4(1)?;
    }
    Ok(socket)
}

pub async fn bind_listener(port: u16) -> std::io::Result<tokio::net::UdpSocket> {
    let socket = create_discovery_socket(&format!("0.0.0.0:{port}"), true)?;
    tokio::net::UdpSocket::from_std(socket)
}

pub async fn bind_announcer() -> std::io::Result<tokio::net::UdpSocket> {
    let socket = create_discovery_socket("0.0.0.0:0", false)?;
    tokio::net::UdpSocket::from_std(socket)
}

fn get_smart_broadcast_addresses(port: u16) -> Vec<SocketAddr> {
    let mut addrs = Vec::with_capacity(260);
    for address in [
        format!("255.255.255.255:{port}"),
        format!("{MULTICAST_IP}:{port}"),
        format!("172.20.10.15:{port}"),
        format!("10.0.0.255:{port}"),
    ] {
        if let Ok(address) = address.parse() {
            addrs.push(address);
        }
    }
    for subnet in 0..=255 {
        addrs.push(SocketAddr::from(([192, 168, subnet, 255], port)));
    }
    addrs
}

pub async fn run_announcer(
    socket: tokio::net::UdpSocket,
    port: u16,
    user_id: String,
    pool: sqlx::Pool<sqlx::Sqlite>,
    cancellation: CancellationToken,
) -> Result<(), String> {
    use sysinfo::System;

    let mut system = System::new();
    let target_addrs = get_smart_broadcast_addresses(port);
    let mut dns_cache: HashMap<String, (Vec<SocketAddr>, Instant)> = HashMap::new();
    const DNS_TTL: Duration = Duration::from_secs(60);

    loop {
        let username = crate::db::get_username(&pool)
            .await
            .unwrap_or_else(|_| "Unknown".to_string());
        system.refresh_memory();
        let available_memory_mb = system.available_memory() / (1024 * 1024);
        let (push_enabled, receive_enabled, target_device_ids) =
            crate::notification_sync::discovery_presence(&pool).await;
        let presence = encode_notification_presence(push_enabled, receive_enabled, &target_device_ids);
        let message =
            format!("LANChat|ONLINE|{user_id}|{username}|{port}|{available_memory_mb}|{presence}");

        for address in &target_addrs {
            let _ = socket.send_to(message.as_bytes(), address).await;
        }

        let custom_peers = crate::db::get_custom_peers(&pool).await;
        let now = Instant::now();
        for peer in &custom_peers {
            let with_port = if peer.contains(':') {
                peer.clone()
            } else {
                format!("{peer}:{port}")
            };
            let cached = dns_cache
                .get(&with_port)
                .filter(|(_, expiry)| *expiry > now)
                .map(|(addresses, _)| addresses.clone());
            let addresses = match cached {
                Some(addresses) => addresses,
                None => match tokio::net::lookup_host(&with_port).await {
                    Ok(addresses) => {
                        let addresses: Vec<_> = addresses.collect();
                        if !addresses.is_empty() {
                            dns_cache.insert(with_port.clone(), (addresses.clone(), now + DNS_TTL));
                        }
                        addresses
                    }
                    Err(error) => {
                        eprintln!("[UDP] DNS 解析失败 ({peer}): {error}");
                        Vec::new()
                    }
                },
            };
            for address in addresses {
                let _ = socket.send_to(message.as_bytes(), address).await;
            }
        }

        tokio::select! {
            _ = cancellation.cancelled() => return Ok(()),
            _ = tokio::time::sleep(Duration::from_secs(2)) => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn run_listener(
    socket: tokio::net::UdpSocket,
    port: u16,
    my_id: String,
    my_name: String,
    peer_manager: Arc<PeerManager>,
    pool: sqlx::Pool<sqlx::Sqlite>,
    event_bus: CoreEventBus,
    cancellation: CancellationToken,
) -> Result<(), String> {
    let mut buffer = [0_u8; 1024];
    let identity_client = crate::network::lan_http_client(Some(crate::peers::PRESENCE_PROBE_TIMEOUT))?;
    println!("[UDP] 正在端口 {port} 监听邻居...");

    loop {
        let (size, address) = tokio::select! {
            _ = cancellation.cancelled() => return Ok(()),
            result = socket.recv_from(&mut buffer) => result.map_err(|error| error.to_string())?,
        };
        let message = String::from_utf8_lossy(&buffer[..size]);
        let parts: Vec<&str> = message.split('|').collect();
        if parts.len() < 6 || parts[0] != "LANChat" {
            continue;
        }

        let peer_id = parts[2].to_string();
        if peer_id == my_id {
            continue;
        }
        let name = parts[3].to_string();
        let peer_port = parts[4];
        let available_memory_mb = parts[5].parse().unwrap_or(0);
        let peer_addr = format!("{}:{peer_port}", address.ip());
        // UDP 来源 IP 在模拟器/NAT 下可能是宿主机。只有地址返回同一设备 ID，
        // 才能把该地址标为在线或用它补发消息。
        let matches_identity = tokio::select! {
            _ = cancellation.cancelled() => return Ok(()),
            matches = crate::peers::same_peer_reachable(&identity_client, &peer_id, &peer_addr) => matches,
        };
        if !matches_identity {
            continue;
        }
        let notification_presence = parse_notification_presence(&parts);
        let Some(connection_transition) = peer_manager.observe_discovery(
            peer_id.clone(),
            name.clone(),
            peer_addr.clone(),
            available_memory_mb,
        ) else {
            continue;
        };
        let (push_enabled, receive_enabled, target_device_ids) = notification_presence
            .as_ref()
            .map(|(push, receive, targets)| (*push, *receive, targets.clone()))
            .unwrap_or((false, false, Vec::new()));
        peer_manager.update_notification_presence(
            &peer_id,
            push_enabled,
            receive_enabled,
            target_device_ids,
        );

        #[cfg(not(windows))]
        let persist_heartbeat = true;
        #[cfg(windows)]
        let persist_heartbeat = peer_manager.persistence().is_none();
        if persist_heartbeat {
            crate::db::save_or_update_user(
                &pool,
                peer_id.clone(),
                name.clone(),
                peer_addr.clone(),
                false,
                available_memory_mb,
            )
            .await
            .map_err(|error| format!("保存发现设备失败: {error}"))?;
        }

        let mut event = serde_json::json!({
            "id": peer_id,
            "name": name,
            "addr": peer_addr,
            "available_memory_mb": available_memory_mb,
            "connection_transition": connection_transition.as_str(),
            "pushes_to_local": notification_presence.as_ref().is_some_and(|(enabled, _, targets)| {
                *enabled && targets.iter().any(|target| target == &my_id)
            }),
        });
        if let Some((enabled, receive_enabled, targets)) = notification_presence {
            event["notification_push_enabled"] = serde_json::json!(enabled);
            event["notification_receive_enabled"] = serde_json::json!(receive_enabled);
            event["notification_push_target_device_ids"] = serde_json::json!(targets);
        }
        event_bus.publish(CoreEvent::PeerDiscovered(event));

        if parts.get(6).is_none_or(|marker| *marker == "0") {
            let reply_name = crate::db::get_username(&pool)
                .await
                .unwrap_or_else(|_| my_name.clone());
            let (enabled, receive_enabled, targets) = crate::notification_sync::discovery_presence(&pool).await;
            let presence = encode_notification_presence(enabled, receive_enabled, &targets);
            let reply = format!(
                "LANChat|ONLINE|{my_id}|{reply_name}|{port}|0|1{}",
                &presence[1..]
            );
            let target = format!("{}:{peer_port}", address.ip());
            let _ = socket.send_to(reply.as_bytes(), &target).await;
        }
    }
}

pub async fn start_announcing(
    port: u16,
    user_id: String,
    pool: sqlx::Pool<sqlx::Sqlite>,
    cancellation: CancellationToken,
) -> Result<(), String> {
    let socket = bind_announcer().await.map_err(|error| error.to_string())?;
    run_announcer(socket, port, user_id, pool, cancellation).await
}

#[allow(clippy::too_many_arguments)]
pub async fn start_listening(
    port: u16,
    my_id: String,
    my_name: String,
    peer_manager: Arc<PeerManager>,
    pool: sqlx::Pool<sqlx::Sqlite>,
    event_bus: CoreEventBus,
    cancellation: CancellationToken,
) -> Result<(), String> {
    let socket = bind_listener(port)
        .await
        .map_err(|error| error.to_string())?;
    run_listener(
        socket,
        port,
        my_id,
        my_name,
        peer_manager,
        pool,
        event_bus,
        cancellation,
    )
    .await
}

pub async fn send_single_broadcast(
    port: u16,
    user_id: String,
    username: String,
) -> Result<(), String> {
    let socket = bind_announcer().await.map_err(|error| error.to_string())?;
    let message = format!("LANChat|ONLINE|{user_id}|{username}|{port}|0");
    for address in get_smart_broadcast_addresses(port) {
        let _ = socket.send_to(message.as_bytes(), address).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_presence_round_trips_and_rejects_invalid_targets() {
        let encoded = encode_notification_presence(
            true,
            true,
            &["target-a".into(), "bad:target".into(), "target_b".into()],
        );
        let message = format!("LANChat|ONLINE|source|Phone|8888|64|{encoded}");
        let parts: Vec<_> = message.split('|').collect();
        assert_eq!(
            parse_notification_presence(&parts),
            Some((true, true, vec!["target-a".into(), "target_b".into()]))
        );
    }

    #[test]
    fn legacy_discovery_has_no_notification_presence() {
        let parts: Vec<_> = "LANChat|ONLINE|source|Phone|8888|64".split('|').collect();
        assert_eq!(parse_notification_presence(&parts), None);
    }

    #[test]
    fn previous_notification_presence_is_not_assumed_to_accept_pushes() {
        let parts: Vec<_> = "LANChat|ONLINE|source|Phone|8888|64|0|1|target-a".split('|').collect();
        assert_eq!(parse_notification_presence(&parts), Some((true, false, vec!["target-a".into()])));
    }
}
