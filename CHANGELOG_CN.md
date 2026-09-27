# LQ Chat 升级日志

本日志只收录能够通过代码提交、正式安装包或构建记录核实的版本。时间均为北京时间（Asia/Shanghai）。

## 2026-09-28 — 17.0（Android versionCode 1700）

- Android 设置页按系统通知、通知访问、后台运行和外部文件夹访问分组，开关自动保存。
- 应用选择增加读取提示，并将应用列表限制在页面框内滚动。
- 优化 Android 主页在横屏和模拟器中的布局。
- 改进局域网设备在线身份识别与通知转发配置。

## 2026-09-25 — 11.3（Android versionCode 1103）

- Windows 与 Android 新增统一的“通知音效”开关，默认开启。
- 收到消息、图片、文件邀请或文件接收完成时播放内置“叮咚”提示音。
- Windows 前台和后台均可播放；Android 后台使用专用通知渠道，前台直接播放同一音效。
- 连续事件在 800 毫秒内合并提示，避免多文件或密集消息连续响铃。
- 音效内置于 Windows EXE 与 Android APK，便携版无需附带外部音频文件。

## 2026-09-12 19:10:47 — 7.2（Android versionCode 1072）

### 优化内容

- 手机电量提醒支持自定义提醒间隔、提醒次数和多个电量节点，默认保持 5 秒、3 次、50%/100%。
- 电量节点支持添加、删除、自动排序和去重；配置关闭后保留，重新开启继续使用。
- 跨越多个电量节点时只触发最接近当前电量的节点，保存新规则后立即终止旧提醒序列。

### 构建验证

- Android 正式 APK：`LQChat-v7.2-android-arm64.apk`
- Windows x64 安装程序：`LQChat-v7.2-windows-x64.exe`（产品版本 7.2.0）。
- Android 包名：`com.lanchat.app`；版本：`7.2`（versionCode 1072）；架构：`arm64-v8a`。
- 前端设置页回归测试通过；Android ARM64 主代码和测试代码编译通过；Rust 单元测试 34 项通过、1 项按设计忽略，通知传输集成测试通过。
- APK ZIP 对齐验证通过，v2/v3 签名验证通过，签名证书与既有正式版一致。
- MuMu 模拟器覆盖安装成功，包版本读取与冷启动验证通过。
- Windows 安装程序静默安装成功，安装后的主程序产品版本与文件版本均为 7.2.0，桌面快捷方式启动验证通过。
- APK SHA-256：`75BCF57A9AE6D7776F2947B6DC435B29046685613F125C60C41647D1F78DCA7E`
- Windows 安装程序 SHA-256：`E4E64131793AB0248D02C1AECB36B212AF015EFF1ABD7352D197A88CD282817C`

## 2026-09-10 07:39:26 — 7.1（Android versionCode 1071）

### 修复与优化

- 修复接收端通知标题缺少应用名称的问题：Android 和 Windows 的普通远程通知统一显示为“设备名称 · 应用名称 · 原通知标题”。
- 原通知标题为空或已经包含应用名称时不会重复拼接；电量通知继续保持“设备名称 · 电量”。

### 构建验证

- Android 正式 APK：`LQChat-v7.1-android-arm64.apk`
- Windows x64 安装程序：`LQChat-v7.1-windows-x64.exe`（产品版本 7.1.0）。
- Android 包名：`com.lanchat.app`；版本：`7.1`（versionCode 1071）；架构：`arm64-v8a`。
- Android 主代码和测试代码编译通过；Rust 单元测试 34 项通过、1 项按设计忽略。
- APK ZIP 对齐验证通过，v2/v3 签名验证通过，签名证书与既有正式版一致，可以覆盖升级。
- APK SHA-256：`24D53CAA9F7C2F899E62A0D1BC208F145522D152B047D5AFD9BFAD860344F79D`
- Windows 安装程序 SHA-256：`90DBA6912ED9E1B8FC5481452A4C009FC717FA460CC58BDC275AB63486B87A78`

## 2026-09-09 20:31:10 — Android 7.0（versionCode 1070）

### 优化内容

- 其他设备推送到 Android 接收端的普通通知，标题增加发送设备名称，格式为“设备名称 · 原通知标题”。
- 电量通知继续显示为“设备名称 · 电量”，避免重复设备名称；设备名称为空时保留原标题。

### 构建验证

- Android 正式 APK：`LQChat-v7.0-android-arm64.apk`
- Android 包名：`com.lanchat.app`；版本：`7.0`（versionCode 1070）；架构：`arm64-v8a`。
- Android 主代码和测试代码编译通过；Rust 单元测试 34 项通过、1 项按设计忽略。
- APK ZIP 对齐验证通过，v2/v3 签名验证通过，签名证书与既有正式版一致，可以覆盖升级。
- APK SHA-256：`7DB7F655A86302ED89C90135A19DF7469D4F07298A539D7350355416B98F962F`

## 2026-09-09 19:28:28 — Android 6.9（versionCode 1069）

### 修复与优化

- 增加应用覆盖更新完成后的后台恢复：收到 `MY_PACKAGE_REPLACED` 后，仅在“后台运行”已开启且用户未主动停止时，自动恢复前台服务和 Rust 核心。
- 更新恢复不会打开应用界面，也不会仅因启用“开机自启”而绕过用户主动停止状态。

### 构建验证

- Android 正式 APK：`LQChat-v6.9-android-arm64.apk`
- Windows x64 安装程序：`LQChat-v6.9-windows-x64.exe`（产品版本 6.9.0）。
- Android 包名：`com.lanchat.app`；版本：`6.9`（versionCode 1069）；架构：`arm64-v8a`。
- Android 主代码和测试代码编译通过；APK 清单确认包含 `MY_PACKAGE_REPLACED`；Rust 单元测试 33 项通过、1 项按设计忽略。
- APK ZIP 对齐验证通过，v2/v3 签名验证通过，签名证书与桌面 6.8 一致，可以覆盖升级。
- APK SHA-256：`F172FC09963704D06F021F2F1B339B9AE5FEDF2117EF249DF5DD0AEEF2BCB2C9`
- Windows 安装程序 SHA-256：`78D277CC5464993512A68379F279E9FCF8CF71DB9E52B509946762ABE45A14EB`

## 2026-09-09 15:22:26 — Android 6.8（versionCode 1068）

### 修复与优化

- 修复主页设置按钮点击后无法进入设置页的问题：移除重复的页面初始化，避免一次点击先打开设置又立即关闭。
- “选择推送应用”开放全部已启用应用，并增加“全部显示 / 用户软件 / 系统软件”分类下拉框；搜索同时匹配应用名称和包名，系统应用显示包名与分类标记。
- `com.android.systemui` 现在可以在“系统软件”分类中选择并保存，用于转发符合现有通知过滤条件的 System UI 通知。

### 构建验证

- Android 正式 APK：`LQChat-v6.8-android-arm64.apk`
- Android 包名：`com.lanchat.app`；版本：`6.8`（versionCode 1068）；架构：`arm64-v8a`。
- APK ZIP 对齐验证通过，v2/v3 签名验证通过，签名证书与设备现装正式版一致，可以覆盖升级。
- APK SHA-256：`25795311DB655E5C400B64AB500F8DAB9CD286ACC2C434C04F0EB896FEB9B938`

## 2026-09-09 10:03:29 — Android 6.7（versionCode 1067）

### 优化内容

- 优化系统文件分享入口：由主页面直接接收分享文件，移除中转 Activity 及固定延迟重试，减少双画面切换；文件描述符由前端就绪后主动领取并在取消或销毁时正确释放。
- 分享接收者弹窗改为与 Android 主界面一致的浅色卡片和紫色强调风格，设备列表仅在状态实际变化时更新。

### 构建验证

- Android 正式 APK：`LQChat-v6.7-android-arm64.apk`
- Android 包名：`com.lanchat.app`；版本：`6.7`（versionCode 1067）；架构：`arm64-v8a`。
- APK ZIP 对齐验证通过，v2/v3 签名验证通过，签名证书与 6.6 一致，可以覆盖升级。
- APK SHA-256：`F04A25533FE0ECA80AE36B284BD037E0CC7C969043C44C223F489A3BB84D31EE`

## 2026-09-09 08:43:08 — Android 6.6（versionCode 1066）

### 优化内容

- Android 本机电量通知与远程电量通知统一格式：标题显示为“`设备名称 · 电量`”，内容保持为 `50%` 或 `100%`。
- 将 Rust 数据库中的设备名称同步保存到 Android 本地设置，确保应用退到后台或前台服务恢复后仍能读取设备名称；名称不可用时使用“本机”作为兜底。
- 保持发往其他设备的电量提醒协议字段不变，兼容 6.5 已实现的远程电量通知识别和显示逻辑。

### 构建验证

- Android 正式 APK：`LQChat-v6.6-android-arm64.apk`
- Android 包名：`com.lanchat.app`；版本：`6.6`（versionCode 1066）；架构：`arm64-v8a`。
- APK ZIP 对齐验证通过，v2/v3 签名验证通过，签名证书与 6.5 一致，可以覆盖升级。
- APK SHA-256：`586E83490A5EE04A1D6F564B388A6A4ABE0FA2D4C59FFDBDF234658C64314DFC`

## 2026-09-08 15:56:29 — 6.5（Android versionCode 1065）

### 修复内容

- 调整 Android 接收端的远程电量通知显示：由设备 `IQOO` 发来的 50% 电量提醒显示为“`IQOO · 电量` / `50%`”，设备名称按实际发送端名称替换。
- 只转换来自 LQChat、且通知标识以 `lq-battery-` 开头的电量提醒；接收端自身的“电量提醒”以及其他远程应用通知保持原样。
- 去除远程电量通知中重复的“来自设备”文字，设备名称只保留在主标题中。

### 构建验证

- Android 正式 APK：`LQChat-v6.5-android-arm64.apk`
- Android 包名：`com.lanchat.app`；版本：`6.5`（versionCode 1065）；架构：`arm64-v8a`。
- APK v2/v3 签名验证通过，签名与 6.4 一致，可以覆盖升级。
- APK SHA-256：`D5B78328FEAE6D4BEFD5E69821D7E41213CF0D70EF006314B91279DAA83F2544`
- Windows 正式安装程序：`LQChat-v6.5-windows-x64.exe`；产品版本：`6.5.0`；内置主程序架构：x64。
- EXE SHA-256：`250B6E825CB3693EDA4BCDD81CD2AEA95EB2CBA51A3DFC021D8F9A365C4FC664`

## 2026-09-08 14:46:29 — Android 6.4（versionCode 1064）

### 新增内容

- 在权限设置中增加“电量提醒”总开关；开启后，电量达到 50% 或 100% 时以“电量提醒”为标题、以当前百分比为内容，每隔 5 秒提醒一次，共提醒 3 次。
- 在“选择推送应用”下方增加“LQ 推送管理”入口和独立页面，目前可控制是否把 LQChat 自身的电量提醒推送给已选择的其他设备。
- 电量监听由现有 Android 前台服务管理，支持应用界面退到后台后继续工作；接收设备只显示收到的提醒，不会把它再次转发。

### 构建验证

- 正式 APK：`LQChat-v6.4-android-arm64.apk`
- 包名：`com.lanchat.app`
- 架构：`arm64-v8a`
- APK v2/v3 签名验证通过，签名与 6.3 一致，可以覆盖升级。
- SHA-256：`4FDA3EE2A54AC28D450C5FDD67079AE8DF19F3D14FD74A7780490AE926337A7E`

## 2026-09-08 08:32:49 — Android 6.3（versionCode 1063）

### 修复内容

- 关闭 Android 主界面的硬件图形加速，规避红魔 Android 16 后台回收界面资源时发生的 `GPU completion` 原生崩溃。已确认原故障表现为 `pthread_mutex_lock called on a destroyed mutex`。
- 增加后台服务启动检查：服务创建 10 秒后检查是否完整启动；未完成时每 30 秒重试一次，最多重试 3 次。
- 增加通知监听恢复检查：后台运行已开启但消息接收核心未就绪时，每 60 秒请求恢复后台服务。
- 增加后台恢复状态记录，保存服务创建、前台状态、Android 上下文初始化、核心启动和异常阶段，便于应用不开界面时继续排查。

### 构建验证

- 正式 APK：`LQChat-v6.3-android-arm64-latest.apk`
- 包名：`com.lanchat.app`
- 架构：`arm64-v8a`
- APK v2/v3 签名验证通过，签名与 6.2 一致，可以覆盖升级。
- SHA-256：`AC808418F62236FEE3A9C794D4B6BC08D4E5BE31A1BF2C491761FCDA6CCA2293`

## 2026-09-08 00:18:57 — Android 6.2（versionCode 1062）

### 修复内容

- 将 Android 系统上下文初始化交给后台前台服务，不再依赖主界面处于打开状态。
- 修复应用退到后台后接收文件时出现 `android context was not initialized`，导致文件上传分块失败或应用崩溃的问题。
- 后台服务可以继续使用 Android 文件保存能力，使文本正常传输但后台文件传输失败的场景得到修复。

### 构建验证

- 正式 APK：`LQChat-v6.2-android-arm64-latest.apk`
- 包名：`com.lanchat.app`
- 架构：`arm64-v8a`
- SHA-256：`9496CA05A84569B16FFC0D90607D75CB721741296260F818278D170D76724804`

## 2026-09-05 13:02:42 — 6.0.0（Android versionCode 1060）

### 升级与修复内容

- 增加 Android 信息接收设置，并统一设备操作入口。
- 修复多条接收通知互相覆盖的问题，使每条接收到的通知能够单独保留。
- 调整 Android 设置页：补充“允许接收其他设备推送”和“自动下载”开关，简化权限项目说明和设备详情操作。
- 将项目版本更新为 6.0.0，Android 内部升级号更新为 1060。

### 可追溯记录

- `dc5de83`（2026-09-05 08:10:09）：增加信息接收设置并统一设备操作。
- `bbc9f86`（2026-09-05 08:31:34）：修复接收通知覆盖问题。
- `3cd3627`（2026-09-05 13:02:42）：发布版本 6.0.0。
