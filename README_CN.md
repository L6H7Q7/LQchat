
# LQChat

> 面向 Windows 与 Android 的局域网消息、文件传输和 Android 通知转发工具。无需账号，不依赖云端。
>
> 🌐 [English Documentation](README.md) · [下载发布版 v16](https://github.com/acclt/LQchat/releases/latest)

<p align="center">
  <img src="artifacts/0.2/device/windows-build1022-wide.jpg" width="68%" alt="LQChat Windows 界面" />
  <img src="artifacts/0.2/device/home-build1022.png" width="27%" alt="LQChat Android 界面" />
</p>

## 当前版本：v11.3

本仓库基于 [cap153/LANChat](https://github.com/cap153/LANChat) 继续开发，目前重点是 Windows 与 Android 之间可靠、常驻的局域网互联：

- 在可信局域网设备之间发送文字、图片、文件和 Android 安装包。
- Android 可把选定应用的通知转发给一台或多台 LQChat 设备；Windows 可接收并保存转发通知，但当前不采集 Windows 自身通知。
- Android 常驻通知显示“正在转发/正在接收”，设备接入或断开局域网转发链路时按设备名称提醒。
- Android 提供前台服务、后台保活、开机自启、最近任务行为、通知权限和电池策略设置。
- Windows 与 Android 收到消息或文件时使用同一个内置提示音，由一个“通知音效”开关控制。
- Windows 为便携版，支持关闭到托盘、Windows 登录自启，以及同时控制自启和双击启动的“启动时自动缩小到托盘”开关。
- 支持局域网自动重连、离线消息/文件补发，以及跨 VLAN、WireGuard 的手动 IP/主机名发现。

可直接使用的 Windows x64 便携包和 Android ARM64 正式 APK 请前往 [Releases](https://github.com/acclt/LQchat/releases/latest) 下载。

## 主要功能

- 🚀 **无需注册** — 首次运行自动创建本地身份，可随时修改设备名称。
- 🔍 **设备发现** — UDP 广播/组播自动发现，并支持 IP、域名、主机名和自定义端口。
- 💬 **聊天与传输** — 文字、图片、文件、大文件分块、手动接收、图片预览和 SQLite 历史记录。
- 🔁 **断线恢复** — 离线队列、自动补发、设备上线/离线判定及连接状态提醒。
- 🔔 **Android 通知转发** — 可选择来源应用和目标设备，在 Windows 或另一台 Android 设备接收。
- 🛡️ **Android 后台运行** — 常驻前台通知、后台保活、开机启动及厂商电池策略指引。
- 🔊 **通知音效** — Windows 与 Android 共用内置提示音和统一开关。
- 💡 **Windows 托盘** — 原图标闪烁、点击打开、关闭到托盘、开机自启和启动自动缩小。
- 📱 **Android 文件能力** — SAF 持久权限、系统文件选择、分享入口、应用选择及自定义下载目录。
- 🌐 **其他运行方式** — 保留上游架构中的 Linux 桌面端和轻量 Web 服务。
- 🌍 **中英文界面** — 自动检测系统语言，也可手动切换。

## 快速开始

### aur

```bash
paru -S lanchat-bin
```

### Releases

[https://github.com/acclt/LQchat/releases](https://github.com/acclt/LQchat/releases)

### 编译

前置要求：

[https://v2.tauri.app/start/prerequisites/](https://v2.tauri.app/start/prerequisites/)   

```bash
# 桌面端
cargo tauri build --bundles deb
cargo tauri build --bundles rpm

# apk
cargo tauri android build --target aarch64
./sign-apk.sh

# Windows x64 便携包（在 Windows PowerShell 中运行）
.\Build-Windows-Portable.ps1

# Web 端（精简版，无 GUI 依赖）
cd src-tauri
cargo build --release --bin lanchat-web --features web --no-default-features
cargo build --no-default-features --features web --release --target x86_64-pc-windows-gnu # windows网页端
```

也可以使用 Makefile 一键构建：

```bash
make all          # 构建测试过的所有平台
make deb          # Linux .deb
make rpm          # Linux .rpm
make apk          # Android APK（自动签名）
make windows-desktop  # Windows 桌面端（需要 cargo-xwin）
make web          # Web 端 Linux
make web-windows  # Web 端 Windows
make help         # 查看帮助
```

## 运行

1. 启动服务（默认 `8888` 端口，可在设置中修改）：

```bash
# 桌面端
LQChat

# Web 端
lanchat-web
```

CLI 参数 `--port` 和 `--db-path` 会覆盖配置文件中的对应设置，优先级最高。

```bash
# Web 端/桌面端
LQChat --port 8889 --db-path /custom/path/LQChat.db
```

> [!TIP]
> 通过指定不同端口和数据库路径可实现多开。  
> 在**添加**功能中输入 `<IP>:<端口>`，即可跨端口发现。  
> 任一端收到心跳后，回复机制会让双方自动互相发现。  

2. 配置防火墙示例 (ufw):

> [!IMPORTANT]
> 确保对应端口的 TCP 和 UDP 均已放行。

```bash
# TCP：Web 页面和 WebSocket 通信
sudo ufw allow 8888/tcp
# UDP：设备发现（广播/组播/心跳）
sudo ufw allow 8888/udp
```

## 自定义主题

支持自定义`css`，文件名称随意，存储路径：

- **Linux**: `~/.config/lanchat/`
- **Windows 便携版**: `<LQChat.exe 所在目录>\config`

可以参考[上游主题目录](https://github.com/acclt/LANChat/tree/main/src/css)中的内置主题。

## 配置文件

端口和语言等设置存储在 `config.json` 中，CLI 参数 `--port` 和 `--db-path` 拥有最高优先级。

Windows 便携包解压后的目录结构如下，首次启动会在这些目录中生成实际数据：

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
- **Windows 便携版**: `<LQChat.exe 所在目录>\config\config.json`
- **macOS**: `~/Library/Application Support/lanchat/config.json`

配置文件内容：

```json
{
  "db_path": null,
  "port": 8888,
  "lang": "zh",
  "close_to_tray": true,
  "start_minimized": true
}
```

| 字段 | 说明 |
|---|---|
| `db_path` | [数据库路径](#数据库路径)，`null` 表示默认路径 |
| `port` | 监听端口，默认 8888 |
| `lang` | 界面语言：`zh`（中文）、`en`（英文） |
| `close_to_tray` | 点击 Windows 主窗口关闭按钮时隐藏到托盘 |
| `start_minimized` | Windows 登录自启和双击启动时均直接进入托盘 |

## 数据库路径

桌面端和 Web 端共享同一个数据库：

- **Linux**: `~/.local/share/com.lanchat.app/lanchat.db`
- **Windows 便携版**: `<LQChat.exe 所在目录>\data\LQChat.db`

可在设置面板中修改数据库路径，修改后需重启生效。Windows 便携版的路径配置存储在同级 `config\config.json` 中。

## 功能状态

### ✅ 已完成

- [x] 自动生成随机用户名
- [x] 点击用户名直接改名
- [x] 局域网设备发现（UDP 广播/组播）
- [x] 实时显示在线用户
- [x] Web 端独立部署
- [x] 桌面端和 Web 端共享数据库
- [x] 设置页面（端口、数据库路径、下载路径）
- [x] 消息历史记录查询
- [x] 主题切换功能
- [x] Android 端适配
- [x] 文本消息传输
- [x] 文件传输功能
- [x] Windows 端适配
- [x] 单实例锁定功能
- [x] 文件流式传输
- [x] 根据系统内存动态调整文件分块大小
- [x] 支持广播和组播
- [x] Android 热点随机网段暴力覆盖
- [x] Web 端文件消息点击直接下载
- [x] 桌面端文件消息点击打开所在路径
- [x] Android 端接收其他应用分享的文件并发送
- [x] Android 端文件消息点击分享到其他应用
- [x] 桌面端、Web 端支持拖拽文件发送
- [x] 桌面端支持粘贴文件发送（零拷贝，Wayland 优先）
- [x] Web 端支持粘贴文件发送
- [x] 图片消息自动预览
- [x] 存在未读消息时红点标注
- [x] 历史消息懒加载（滚动时触发加载历史消息）
- [x] 删除指定聊天记录
- [x] Android 端文件消息点击打开
- [x] 重复文件智能去重（接收端存在文件相同且完整直接引用，文件不同自动重命名）
- [x] 离线用户重新上线消息补发
- [x] 剪切板图片粘贴发送
- [x] 删除离线用户
- [x] 清空聊天记录
- [x] Android 端适配状态栏/三大金刚键
- [x] [LANClaw](https://github.com/cap153/LANClaw) 流式 AI 回复
- [x] 模型切换命令（`/model`）
- [x] 新建会话命令（`/new`）
- [x] 手动发现 IP / 域名 / 主机名（跨 VLAN / WireGuard）
- [x] UDP 心跳自动回复（跨端口/跨网段自动发现）
- [x] Windows 与 Android 原生系统通知，并保留 Web/Linux 通知能力
- [x] Android 通知转发，可选择来源应用和一台或多台目标设备
- [x] 常驻通知显示转发/接收链路状态，并按设备名称提醒接入与断开
- [x] 通知接收历史、来源设备及投递状态记录
- [x] Android 前台服务后台运行、开机自启、最近任务策略及覆盖更新恢复
- [x] Windows/Android 统一通知音效开关与内置提示音
- [x] 托盘原图标闪烁（未读消息时闪烁提示，点击后跳转最新未读）
- [x] 通知开关（托盘右键菜单，仅桌面端）
- [x] Windows 关闭到托盘、登录自启及“启动时自动缩小”设置
- [x] Windows 便携数据、配置、下载和 WebView 缓存目录
- [x] 手动接收文件
- [x] Android SAF 持久化权限 + 零拷贝 FD 缓存双轨机制
- [x] Android SAF 原生文件选择器（`ACTION_OPEN_DOCUMENT` + `takePersistableUriPermission`）
- [x] 文件传输速度实时显示
- [x] 配置文件路径按平台规则存储（Linux `~/.config/`、Windows 便携目录）
- [x] 中英文界面（自动检测系统语言 + 手动切换 + 托盘热更新）

### 🚧 进行中

- [ ] 聊天室功能

## 项目结构

```
LQChat/
├── src/                      # 前端代码
│   ├── css/
│   │   ├── style.css        # 样式文件
│   │   └── vscode.css       # VSCode 主题
│   ├── js/
│   │   ├── api.js           # API 封装（Tauri + HTTP 双端）
│   │   ├── app.js           # 应用逻辑
│   │   └── ui.js            # UI 交互
│   └── index.html           # 主页面
├── src-tauri/               # 后端代码（桌面端 + Web 端）
│   ├── src/
│   │   ├── main.rs          # 桌面端 Tauri 入口
│   │   ├── server_main.rs   # Web 端独立入口
│   │   ├── lib.rs           # Android 入口
│   │   ├── commands.rs      # Tauri 桌面命令
│   │   ├── web_server.rs    # HTTP/WebSocket 服务器
│   │   ├── db.rs            # SQLite 数据库逻辑
│   │   ├── config_file.rs   # 配置文件读写（config.json）
│   │   ├── peers.rs         # 在线用户管理器
│   │   ├── models.rs        # 数据模型
│   │   ├── utils.rs         # 工具函数
│   │   └── network/         # 网络模块
│   │       ├── discovery.rs # UDP 设备发现（广播/组播/单播回复）
│   │       └── messaging.rs # WebSocket 消息收发
│   ├── capabilities/        # Tauri 权限配置
│   ├── permissions/         # 自定义命令权限
│   └── Cargo.toml
├── Makefile                 # 一键构建脚本
└── README_CN.md               # 中文说明
```

## 文件状态表整理

| 角色       | 状态值                       | 显示文字 |
|------------|------------------------------|----------|
| **发送端** | `status: "pending"`          | 待上线   |
| **发送端** | `file_status: "offering"`    | 待接收   |
| **发送端** | `file_status: "uploading"`   | xx MB/s  |
| **发送端** | `file_status: "sent"`        | 无文字   |
| **接收端** | `file_status: "offered"`     | 未下载   |
| **接收端** | `file_status: "invalid"`     | 已失效   |
| **接收端** | `file_status: "downloading"` | xx MB/s  |
| **接收端** | `file_status: "accepted"`    | 无文字   |

## 疑难解答

**Windows运行软件时提示找不到`VCRUNTIME140.dll`、`VCRUNTIME140_1.dll`：**（安装下面的软件）  
[https://aka.ms/vs/17/release/vc_redist.x64.exe](https://aka.ms/vs/17/release/vc_redist.x64.exe)  
[https://aka.ms/vs/17/release/vc_redist.x86.exe](https://aka.ms/vs/17/release/vc_redist.x86.exe)

**Windows运行软件时提示未安装WebView2：**（安装WebView2）  
[https://developer.microsoft.com/zh-cn/microsoft-edge/webview2](https://developer.microsoft.com/zh-cn/microsoft-edge/webview2)

**在线用户发送消息失败，发送出去的消息显示`待上线`：**  
关闭流量，重新连接wifi等网络

**NVIDIA显卡linux桌面端未正常渲染：**  
desktop文件加上`Exec=env __NV_DISABLE_EXPLICIT_SYNC=1 lanchat`环境变量

**跨 VLAN / WireGuard 场景：**  
当设备处于不同 VLAN 或通过 WireGuard 连接时，UDP 广播无法跨网段。解决方案：在底部「添加」面板中填写对方的 IP 地址、域名或主机名加端口，系统会定期发送单播心跳完成发现（支持 DNS 解析，60 秒缓存）。

> [!TIP]
> 只要一方手动添加了对方地址，收到心跳后会**自动回复**一条心跳，双方都能互相发现。无需两边都设置。
> 心跳回复包含 `|1` 标记，不会产生无限循环。

## Android 后台接收

Android 版通过前台服务和常驻通知让局域网核心在界面退到后台后继续运行。常驻通知会显示本机正在转发、正在接收、等待目标设备或局域网链路断开等状态。

设置页可分别控制“后台运行”“开机自启”和“从最近任务隐藏”，并显示通知权限、电池优化状态和厂商电源策略入口；“停止后台接收并退出”会明确停止服务。覆盖安装后可按已保存设置恢复后台服务，但 Android 的“强制停止”始终具有最高优先级，应用不会绕过系统限制自行启动。

iQOO/vivo、红魔、小米、华为等厂商系统的深度休眠策略仍可能延迟或终止局域网通信。需要可靠息屏接收时，请允许应用自启动和后台活动，并把电池策略设为“不受限制”。组播发现和已知 IP 直连是两条不同链路，应分别验证。

## 技术栈

- **后端**: Rust + Tauri 2.0
- **前端**: 原生 HTML + CSS + JavaScript
- **数据库**: SQLite (sqlx)
- **网络**: UDP 广播/组播 + TCP/WebSocket 传输 + HTTP 分块上传
- **Web 服务器**: Axum
- **AI 机器人**: [LANClaw](https://github.com/cap153/LANClaw)（独立进程，通过 Pi RPC 驱动）


## 许可证

MIT License

## 致谢

- [Tauri](https://tauri.app/) - 跨平台应用框架
- [Axum](https://github.com/tokio-rs/axum) - Web 框架
- [SQLx](https://github.com/launchbadge/sqlx) - 异步 SQL 工具包

## 赞助

如果你觉得这个项目对你有帮助，可以请作者喝杯咖啡 ☕️

<details>
  <summary><b>点击展开赞赏码 (WeChat Pay)</b></summary>
  <br />
  <p align="center">
    <img src=".github/wechat_sponsor.png" width="250" />
    <br />
  </p>
  <p align="center">感谢您的支持！您的名字将被记录在 <a href="https://github.com/cap153/LANChat/blob/main/.github/SPONSOR.md#-%E6%84%9F%E8%B0%A2%E5%90%8D%E5%8D%95-backers">赞助者名单</a> 中。</p>
</details>
