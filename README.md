
# LQChat

> Private LAN messaging, file transfer, and Android notification relay for Windows and Android. No account or cloud service required.
>
> 📖 [中文说明](README_CN.md) · [Download release v19](https://github.com/acclt/LQchat/releases/latest)

<p align="center">
  <img src="artifacts/0.2/device/windows-build1022-wide.jpg" width="68%" alt="LQChat Windows interface" />
  <img src="artifacts/0.2/device/home-build1022.png" width="27%" alt="LQChat Android interface" />
</p>

## Current Release: v18

This fork continues [cap153/LANChat](https://github.com/cap153/LANChat) and is now centered on reliable Windows–Android use:

- Send text, images, files, and Android app packages between trusted devices on the same LAN.
- Forward selected Android app notifications to one or more LQChat devices. Windows receives and stores the forwarded notifications; Windows notification capture is not included.
- Show persistent forwarding/receiving state on Android and notify when a named device joins or leaves the LAN route.
- Keep Android receiving in the background with foreground-service, boot-start, recent-task, notification-permission, and battery-policy controls.
- Play one built-in notification sound for incoming messages and files on Windows and Android, controlled by a single switch.
- Run Windows as a portable app with close-to-tray, Windows sign-in autostart, and a **Start minimized to tray** option that also controls double-click launches.
- Automatically rediscover LAN peers, retry queued offline messages/files after reconnect, and support manual IP/hostname discovery across VLAN or WireGuard links.

The ready-to-use Windows x64 portable archive and Android ARM64 APK are available on the [Releases page](https://github.com/acclt/LQchat/releases/latest).

## Features

- 🚀 **No registration** — Local identity is generated on first launch and can be renamed.
- 🔍 **LAN discovery** — UDP broadcast/multicast discovery plus manual IP, hostname, domain, and custom-port peers.
- 💬 **Messaging and transfer** — Text, images, files, large chunked transfers, manual receive, previews, and SQLite history.
- 🔁 **Reconnect handling** — Offline queue, automatic re-send, peer join/leave detection, and connection notifications.
- 🔔 **Android notification relay** — Select source apps and target devices; receive forwarded notifications on Windows or Android.
- 🛡️ **Android background operation** — Persistent foreground notification, background keep-alive, boot startup, and vendor battery-policy guidance.
- 🔊 **Notification sound** — One bundled sound and one switch for received messages/files on both supported primary platforms.
- 💡 **Windows tray integration** — Original-icon flashing for unread events, click-to-open, close-to-tray, autostart, and start-minimized control.
- 📱 **Android file integration** — SAF persistent permissions, native picker, share intents, app selection, and custom download folders.
- 🌐 **Additional targets** — Linux desktop and a lightweight Web service remain available from the upstream architecture.
- 🌍 **Chinese and English UI** — Automatic language detection and manual switching.

## Quick Start

### AUR

```bash
paru -S lanchat-bin
```

### Releases

[https://github.com/acclt/LQchat/releases](https://github.com/acclt/LQchat/releases)

### Build from Source

Prerequisites:

[https://v2.tauri.app/start/prerequisites/](https://v2.tauri.app/start/prerequisites/)

```bash
# Desktop Linux
cargo tauri build --bundles deb
cargo tauri build --bundles rpm

# Android APK
cargo tauri android build --target aarch64
./sign-apk.sh

# Windows x64 portable package (run in PowerShell on Windows)
.\Build-Windows-Portable.ps1

# Web (lightweight, no GUI dependencies)
cd src-tauri
cargo build --release --bin lanchat-web --features web --no-default-features
cargo build --no-default-features --features web --release --target x86_64-pc-windows-gnu # Windows web
```

Or use the Makefile for one-click builds:

```bash
make all          # Build all tested platforms
make deb          # Linux .deb
make rpm          # Linux .rpm
make apk          # Android APK (auto-signed)
make windows-desktop  # Windows desktop (requires cargo-xwin)
make web          # Web (Linux)
make web-windows  # Web (Windows)
make help         # Show help
```

## Running

1. Start the service (default port `8888`, configurable in settings):

```bash
# Desktop
LQChat

# Web
lanchat-web
```

CLI arguments `--port` and `--db-path` override config file settings with highest priority.

```bash
# Web / Desktop
LQChat --port 8889 --db-path /custom/path/LQChat.db
```

> [!TIP]
> Run multiple instances by specifying different ports and database paths.  
> Use the **Add** panel with `<IP>:<port>` to discover across ports.  
> Once one side receives a heartbeat, the reply mechanism enables mutual discovery.

2. Firewall configuration (ufw example):

> [!IMPORTANT]
> Ensure both TCP and UDP are allowed for the configured port.

```bash
# TCP: Web pages and WebSocket communication
sudo ufw allow 8888/tcp
# UDP: Device discovery (broadcast/multicast/heartbeat)
sudo ufw allow 8888/udp
```

## Custom Themes

Place custom `.css` files in the config directory (any filename):

- **Linux**: `~/.config/lanchat/`
- **Windows portable**: `<directory containing LQChat.exe>\config`

See the built-in themes in the [upstream theme directory](https://github.com/acclt/LANChat/tree/main/src/css).

## Config File

Port, language, and other settings are stored in `config.json`. CLI arguments `--port` and `--db-path` take highest priority.

The Windows portable archive has the following layout. Actual data files are generated in these directories on first launch:

```text
LQChat-Portable\
├─ LQChat.exe
├─ LQChat.ico
├─ data\LQChat.db
├─ config\config.json
├─ downloads\
└─ cache\EBWebView\
```

- **Linux**: `~/.config/lanchat/config.json`
- **Windows portable**: `<directory containing LQChat.exe>\config\config.json`
- **macOS**: `~/Library/Application Support/lanchat/config.json`

Example content:

```json
{
  "db_path": null,
  "port": 8888,
  "lang": "en",
  "close_to_tray": true,
  "start_minimized": true
}
```

| Field | Description |
|---|---|
| `db_path` | [Database path](#database); `null` = default |
| `port` | Listening port, default 8888 |
| `lang` | Interface language: `zh` (Chinese), `en` (English) |
| `close_to_tray` | Hide the Windows window when its close button is clicked |
| `start_minimized` | Start directly in the Windows tray for both sign-in autostart and double-click launches |

## Database

Desktop and Web share the same database:

- **Linux**: `~/.local/share/com.lanchat.app/lanchat.db`
- **Windows portable**: `<directory containing LQChat.exe>\data\LQChat.db`

The database path can be changed in settings (requires restart). On Windows portable builds, the path setting is stored in the adjacent `config\config.json` file.

## Feature Status

### ✅ Done

- [x] Auto-generated random usernames
- [x] Click username to rename
- [x] LAN device discovery (UDP broadcast/multicast)
- [x] Real-time online user list
- [x] Standalone Web deployment
- [x] Shared database between desktop and Web
- [x] Settings panel (port, db path, download path)
- [x] Chat history query
- [x] Theme switching
- [x] Android support
- [x] Text message transfer
- [x] File transfer
- [x] Windows support
- [x] Single instance lock
- [x] Streaming file transfer
- [x] Dynamic chunk size based on system memory
- [x] Broadcast and multicast support
- [x] Android hotspot random subnet brute-force
- [x] Web: click file message to download directly
- [x] Desktop: click file message to open containing folder
- [x] Android: receive shared files from other apps and send
- [x] Android: share file messages to other apps
- [x] Desktop & Web: drag-and-drop file sending
- [x] Desktop: paste file sending (zero-copy, Wayland-first)
- [x] Web: paste file sending
- [x] Auto image preview
- [x] Red dot for unread messages
- [x] Lazy-load history (scroll to trigger)
- [x] Delete chat history
- [x] Android: open file messages
- [x] Smart file deduplication (reuse identical files, auto-rename on conflict)
- [x] Offline message re-send on reconnect
- [x] Clipboard image paste sending
- [x] Delete offline users
- [x] Clear chat history
- [x] Android: status bar / navigation bar adaptation
- [x] [LANClaw](https://github.com/cap153/LANClaw) streaming AI replies
- [x] Model switching command (`/model`)
- [x] New session command (`/new`)
- [x] Manual discovery (IP / domain / hostname, cross-VLAN / WireGuard)
- [x] UDP heartbeat auto-reply (cross-port / cross-subnet auto discovery)
- [x] Native system notifications on Windows and Android, plus Web/Linux notification support
- [x] Android notification relay with selectable source apps and target devices
- [x] Forwarding/receiving route status and named peer join/leave notifications
- [x] Notification receive history with source and delivery status
- [x] Android foreground-service background mode, boot startup, recent-task policy, and post-update recovery
- [x] Shared Windows/Android notification sound setting with bundled audio
- [x] Tray original-icon flash (unread notification, click to jump to latest)
- [x] Notification toggle (tray right-click menu, desktop only)
- [x] Windows close-to-tray, sign-in autostart, and start-minimized setting
- [x] Windows portable data/config/download/WebView layout
- [x] Manual file receive
- [x] Android SAF persistable permissions + zero-copy FD cache dual-track
- [x] Android SAF native file picker (`ACTION_OPEN_DOCUMENT` + `takePersistableUriPermission`)
- [x] Real-time file transfer speed display
- [x] Platform-specific config paths (Linux `~/.config/`, Windows portable directory)
- [x] i18n (auto-detect + manual switch + tray hot-reload)

### 🚧 In Progress

- [ ] Group chat

## Project Structure

```
LQChat/
├── src/                      # Frontend
│   ├── css/
│   │   ├── style.css        # Main stylesheet
│   │   └── vscode.css       # VSCode theme
│   ├── js/
│   │   ├── api.js           # API wrapper (Tauri + HTTP dual backend)
│   │   ├── app.js           # App logic
│   │   └── ui.js            # UI interactions
│   └── index.html           # Main page
├── src-tauri/               # Backend (desktop + Web)
│   ├── src/
│   │   ├── main.rs          # Desktop Tauri entry
│   │   ├── server_main.rs   # Web standalone entry
│   │   ├── lib.rs           # Android entry
│   │   ├── commands.rs      # Tauri commands
│   │   ├── web_server.rs    # HTTP/WebSocket server
│   │   ├── db.rs            # SQLite database logic
│   │   ├── config_file.rs   # Config file read/write
│   │   ├── peers.rs         # Online user manager
│   │   ├── models.rs        # Data models
│   │   ├── utils.rs         # Utilities
│   │   └── network/         # Network module
│   │       ├── discovery.rs # UDP device discovery
│   │       └── messaging.rs # WebSocket messaging
│   ├── capabilities/        # Tauri capability config
│   ├── permissions/         # Custom command permissions
│   └── Cargo.toml
├── Makefile                 # One-click build
└── README.md                # This file
```

## File Status Reference

| Role         | Status Value                 | Display Text |
|--------------|------------------------------|--------------|
| **Sender**   | `status: "pending"`          | pending      |
| **Sender**   | `file_status: "offering"`    | offering     |
| **Sender**   | `file_status: "uploading"`   | xx MB/s      |
| **Sender**   | `file_status: "sent"`        | (empty)      |
| **Receiver** | `file_status: "offered"`     | offered      |
| **Receiver** | `file_status: "invalid"`     | invalid      |
| **Receiver** | `file_status: "downloading"` | xx MB/s      |
| **Receiver** | `file_status: "accepted"`    | (empty)      |

## Troubleshooting

**Windows: missing `VCRUNTIME140.dll` / `VCRUNTIME140_1.dll`:**  
[https://aka.ms/vs/17/release/vc_redist.x64.exe](https://aka.ms/vs/17/release/vc_redist.x64.exe)  
[https://aka.ms/vs/17/release/vc_redist.x86.exe](https://aka.ms/vs/17/release/vc_redist.x86.exe)

**Windows: WebView2 not installed:**  
[https://developer.microsoft.com/en-us/microsoft-edge/webview2](https://developer.microsoft.com/en-us/microsoft-edge/webview2)

**Online user sends messages but they show as `Pending` on my side:**  
Disable mobile data / VPN and reconnect to WiFi.


**NVIDIA graphics card not rendering correctly on Linux desktop:**  
Add the environment variable `Exec=env __NV_DISABLE_EXPLICIT_SYNC=1 lanchat` to the desktop file.

**Cross-VLAN / WireGuard:**  
When devices are on different VLANs or connected via WireGuard, UDP broadcast won't cross subnets. Solution: use the **Add** panel to enter the peer's IP/domain/hostname with port. The system sends unicast heartbeats periodically (DNS resolution supported, 60s cache).

> [!TIP]
> Only one side needs to add the other. Upon receiving a heartbeat, the peer **auto-replies** with its own heartbeat, enabling mutual discovery without manual setup on both sides.
> Reply heartbeats include a `|1` marker to prevent infinite loops.

## Android Background Receiving

Android uses a foreground service and a persistent notification to keep the LAN core alive when the UI is in the background. The notification reports whether this device is forwarding notifications, receiving them, waiting for a peer, or has lost its LAN route.

The Settings panel independently controls background keep-alive, boot startup, and removal from Recents. It also reports notification permission and battery-optimization state, links to the vendor battery page, and provides an explicit **Stop background receiving and exit** action. Package updates can restore an enabled background session; Android force-stop always remains authoritative.

Deep Doze and vendor policies from iQOO/vivo, RedMagic, Xiaomi, Huawei, and similar devices can still delay or kill LAN traffic. Allow autostart, set battery use to unrestricted, and permit background activity when reliable screen-off receiving is required. Multicast discovery and direct communication with a known IP are separate paths and should be tested independently.

## Tech Stack

- **Backend**: Rust + Tauri 2.0
- **Frontend**: Vanilla HTML + CSS + JavaScript
- **Database**: SQLite (sqlx)
- **Network**: UDP broadcast/multicast + TCP/WebSocket + HTTP chunked upload
- **Web Server**: Axum
- **AI Bot**: [LANClaw](https://github.com/cap153/LANClaw) (separate process, driven by Pi RPC)

## License

MIT License

## Acknowledgements

- [Tauri](https://tauri.app/) - Cross-platform app framework
- [Axum](https://github.com/tokio-rs/axum) - Web framework
- [SQLx](https://github.com/launchbadge/sqlx) - Async SQL toolkit

## Sponsor

If you find this project helpful, feel free to buy me a coffee ☕️

<details>
  <summary><b>Click to reveal QR code (WeChat Pay)</b></summary>
  <br />
  <p align="center">
    <img src=".github/wechat_sponsor.png" width="250" />
    <br />
  </p>
  <p align="center">Your support is greatly appreciated!</p>
</details>
