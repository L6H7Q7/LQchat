// src/js/app.js
async function renderPage() {
  console.log("[JS-App] 页面初始化开始...");

  const androidPreview = new URLSearchParams(location.search).get("android-preview") === "1";
  const desktopPreview = new URLSearchParams(location.search).get("desktop-preview") === "1";
  const previewMode = androidPreview || desktopPreview;
  const androidApp = (navigator.userAgent.includes("Android") && !!window.__TAURI__) || androidPreview;
  document.body.classList.toggle("android-app", androidApp);
  document.body.classList.toggle("windows-app", !androidApp &&
    (desktopPreview || (!!window.__TAURI__ && navigator.userAgent.includes("Windows"))));
  if (androidApp) initAndroidSaveBars();

  const myName = await apiGetMyName();
  const nameElement = document.getElementById("my-name");
  if (nameElement) {
    nameElement.innerText = myName;
  }
  const androidName = document.getElementById("android-device-name");
  if (androidName) androidName.innerText = myName;

  // 初始化改名功能
  initNameEditor();

  // 初始化设置功能
  initSettings();

  // 初始化手动添加设备功能
  initAddPeer();

  document.getElementById("android-settings-btn")?.addEventListener("click", () => document.getElementById("settings-btn")?.click());
  document.getElementById("desktop-refresh-btn")?.addEventListener("click", () => document.getElementById("android-refresh-peers-btn")?.click());
  document.getElementById("android-add-peer-btn")?.addEventListener("click", () => document.getElementById("add-peer-btn")?.click());
  document.getElementById("android-receive-sources-btn")?.addEventListener("click", () => window.NotificationUI?.openPushSources?.());
  document.getElementById("android-refresh-peers-btn")?.addEventListener("click", async (event) => {
    const button = event.currentTarget;
    button.disabled = true;
    try { await refreshPeerListNow(); }
    finally { button.disabled = false; }
  });
  document.getElementById("android-refresh-receive-btn")?.addEventListener("click", async (event) => {
    const button = event.currentTarget;
    button.disabled = true;
    try {
      await refreshPeerListNow();
      await window.NotificationUI?.refresh?.();
    } finally {
      button.disabled = false;
    }
  });
  document.getElementById("android-edit-name-btn")?.addEventListener("click", () => document.getElementById("my-name")?.click());
  try {
    if (!window.__TAURI__) throw new Error("preview");
    const info = await window.__TAURI__.core.invoke("get_local_device_info");
    const ip = document.getElementById("android-device-ip");
    if (ip) ip.textContent = `${info.ip}:${info.port}`;
    const desktopAddress = document.getElementById("desktop-local-address");
    if (desktopAddress) desktopAddress.textContent = `${info.ip}:${info.port}`;
  } catch (e) {
    console.warn("[JS-App] 获取本机 IP 失败:", e);
    const ip = document.getElementById("android-device-ip");
    if (ip) ip.textContent = "暂未连接局域网";
  }

  // 初始化语言功能（放在主题前面，确保翻译尽早应用）
  await initLanguage();

  // 初始化主题功能
  initTheme();

  // 初始化聊天功能
  initChat();
  await window.NotificationUI?.init({androidApp, desktopPreview});

  if (previewMode) {
    const summary = document.getElementById("android-peer-summary");
    if (summary) summary.textContent = "当前 1 台设备";
    const emptyState = document.getElementById("android-chat-empty");
    if (emptyState) emptyState.hidden = true;
    void addUserToList("preview-peer", "林然的电脑", "192.168.5.8:8888", false);
  }

  // 请求 Android 通知权限（Android 13+ 需要运行时权限）
  if (androidApp) {
    requestAndroidNotificationPermission().catch(error => {
      console.warn("[App] 通知权限申请未完成:", error);
    });
  }

  // 使用我们封装好的 apiListen
  await apiListen("new-peer", (event) => {
    addUserToList(
      event.payload.id,
      event.payload.name,
      event.payload.addr,
      false,
    );
  });

  // 监听新消息事件(桌面端)
  await apiListen("new-message", (event) => {
    console.debug("[JS-App] 收到 new-message 事件");
    onReceiveMessage(event.payload);
  });

  // 监听补发完成事件，自动剥除“待上线”黄点
  await apiListen("messages-resent", async (event) => {
    const peerId = event.payload;
    console.log("[JS-App] 收到补发完成事件，准备刷新用户:", peerId);

    // 如果当前正好在这个用户的聊天界面里，静默刷新一下历史记录即可
    if (window.currentChatPeer && window.currentChatPeer.id === peerId) {
      const renderSession = window.chatRenderController?.snapshot(peerId);
      await loadChatHistory(peerId, true, renderSession);
    }
  });

  // 启动用户列表轮询（桌面端和 Web 端都需要）
  if (!previewMode) {
    console.log("[JS-App] 启动用户列表轮询");
    startPeerPolling();
  }

  // 请求通知权限不再在初始化时调用（WebKit 在非用户手势下调用会抛警告），
  // 改由 showNotification 按需请求

  // 监听通知点击事件（Tauri 端：点击通知跳转到对应聊天）
  if (window.__TAURI__) {
    apiListen("actionPerformed", async (event) => {
      const notification = event?.payload?.notification;
      if (!notification) return;
      const from_id = notification.extra?.from_id || (() => {
        try { return localStorage.getItem("pendingNotificationFromId"); }
        catch (_) { return null; }
      })();
      if (!from_id) return;
      const userLi = document.querySelector(`#user-list li[data-id="${from_id}"]`);
      if (userLi) {
        userLi.click();
      }
    });
  }

  // Android 通知点击（通过 MainActivity 直接注入）
  window.addEventListener("notification-tapped", (e) => {
    const from_id = e.detail?.fromId;
    if (!from_id) return;
    const userLi = document.querySelector(`#user-list li[data-id="${from_id}"]`);
    if (userLi) userLi.click();
  });

  // 托盘「显示窗口」时打开最新未读聊天
  if (window.__TAURI__) {
    apiListen("open-latest-unread", () => {
      // 消费一次性的 pendingNotificationFromId（由最近一条通知设置）
      const pendingFromId = (() => {
        try {
          const id = localStorage.getItem("pendingNotificationFromId");
          localStorage.removeItem("pendingNotificationFromId");
          return id;
        } catch (_) { return null; }
      })();

      // 优先跳转到 pendingFromId 对应的用户
      if (pendingFromId) {
        const userLi = document.querySelector(`#user-list li[data-id="${pendingFromId}"]`);
        if (userLi) { userLi.click(); return; }
      }

      // 没有 pending 时，跳转到列表里第一个有未读的用户
      const firstUnread = document.querySelector("#user-list li.has-unread");
      if (firstUnread) firstUnread.click();
    });
  }

  // 监听托盘菜单的通知开关变更（刷新 JS 缓存）
  if (window.__TAURI__) {
    apiListen("notifications-changed", (event) => {
      window._notificationsEnabled = event.payload;
    });
  }

  // 启动消息轮询（桌面端和 Web 端都需要，用于检测状态变化）
  const tauri = window.__TAURI__;
  if (!previewMode) {
    console.log("[JS-App] 启动消息轮询");
    startMessagePolling();
  }

  // Web 端需要额外启动未读消息检查（桌面端通过事件处理）
  if (!tauri && !previewMode) {
    console.log("[JS-App] 启动未读消息检查（Web 端）");
    startUnreadMessageCheck();
    // Web 端：连接 WebSocket 接收流式事件
    startStreamingWebSocket();
  }

}

let peerRefreshInFlight = false;
let lastPeerSnapshot = "";

async function refreshPeerListNow() {
  if (peerRefreshInFlight) return;
  peerRefreshInFlight = true;
  try {
    const peers = await apiGetPeers();
    if (!peers) return;
    const peerSnapshot = JSON.stringify(peers.map((peer) => [
      peer.id,
      peer.name,
      peer.addr,
      Boolean(peer.is_offline),
    ]));
    if (peerSnapshot === lastPeerSnapshot) return peers;
    lastPeerSnapshot = peerSnapshot;
    window.NotificationUI?.onPeers(peers);

    const apiPeerIds = new Set(peers.map((p) => p.id));
    const existingItems = Array.from(document.querySelectorAll("#user-list li"));
    const existingById = new Map(existingItems.map((item) => [item.dataset.id, item]));
    let needsSort = false;

    for (const peer of peers) {
      // 更新左侧列表 UI
      const existing = existingById.get(peer.id);
      if (!existing || existing.classList.contains("offline") !== Boolean(peer.is_offline)) {
        needsSort = true;
      }
      void addUserToList(peer.id, peer.name, peer.addr, peer.is_offline, true);

      // 如果该用户正处于聊天窗口中，实时同步他的最新 IP 和名字
      if (window.currentChatPeer && window.currentChatPeer.id === peer.id) {
        if (window.currentChatPeer.addr !== peer.addr) {
          console.log(
            `[JS-App] ⚡ 当前聊天对象 IP 已更新: ${window.currentChatPeer.addr} -> ${peer.addr}`,
          );
          window.currentChatPeer.addr = peer.addr;
        }
        if (window.currentChatPeer.name !== peer.name) {
          window.currentChatPeer.name = peer.name;
          // 顶部标题也改了
          const chatWithName = document.getElementById("chat-with-name");
          if (chatWithName) chatWithName.textContent = peer.name;
        }
      }
    }

    const summary = document.getElementById("android-peer-summary");
    if (summary) {
      const nextSummary = `当前 ${peers.length} 台设备`;
      if (summary.textContent !== nextSummary) summary.textContent = nextSummary;
    }
    const emptyState = document.getElementById("android-chat-empty");
    if (emptyState) emptyState.hidden = peers.length > 0;

    // 处理“自动移除”：如果 DOM 中的用户 ID 不在 API 列表中，说明该用户被删除了
    existingItems.forEach((li) => {
      const domId = li.dataset.id;
      if (!apiPeerIds.has(domId)) {
        console.log(`[JS-App] 用户 ${domId} 不在列表，执行移除`);
        li.remove();
        needsSort = true;
      }
    });

    if (needsSort) sortUserList();
    return peers;
  } finally {
    peerRefreshInFlight = false;
  }
}

async function startPeerPolling() {
  await refreshPeerListNow();
  const pollInterval = 1000;
  setInterval(() => void refreshPeerListNow(), pollInterval);
}

function initAndroidSaveBars() {
  // Keep actions above the IME, including WebViews which overlay instead of resize.
  const updateViewport = () => {
    const viewport = window.visualViewport;
    const inset = viewport && viewport.scale === 1
      ? Math.max(0, innerHeight - viewport.height - viewport.offsetTop)
      : 0;
    document.documentElement.style.setProperty("--android-keyboard-inset", `${inset}px`);
  };
  updateViewport();
  window.visualViewport?.addEventListener("resize", updateViewport);
  window.visualViewport?.addEventListener("scroll", updateViewport);
  window.addEventListener("resize", updateViewport);

  // Reserve the actual footer height, including wrapped save/error messages.
  const observer = new ResizeObserver((entries) => {
    for (const { target } of entries) {
      const height = target.getBoundingClientRect().height;
      const panel = target.closest(".settings-panel, .android-permissions-panel, .android-overlay-panel");
      if (panel && height > 0) panel.style.setProperty("--android-save-bar-height", `${height}px`);
    }
  });
  document.querySelectorAll(".android-save-bar").forEach((bar) => observer.observe(bar));
}

document.addEventListener("DOMContentLoaded", () => {
  void consumeAndroidShare();
  void renderPage();
});

// 监听 Android 分享事件
window.addEventListener("android-share-received", () => {
  console.log("[JS-App] ========== 收到 Android 分享事件 ==========");
  void consumeAndroidShare();
});

async function consumeAndroidShare() {
  try {
    if (document.querySelector(".share-dialog")) await apiClearAndroidSharedFiles();
    const sharedFiles = await apiGetAndroidSharedFiles();
    console.log("[JS-App] 分享的文件:", sharedFiles);

    if (sharedFiles.length === 0) {
      console.log("[JS-App] 没有待处理的分享文件");
      return;
    }

    console.log("[JS-App] 准备显示分享对话框");
    showShareDialog(sharedFiles);
  } catch (e) {
    console.error("[JS-App] 处理 Android 分享失败:", e);
  }
}

console.log("[JS-App] Android 分享事件监听器已注册");

window.shareDialogPeerObserver = null;
window.shareDialogPeerSignature = "";

function stopShareDialogPeerObserver() {
  window.shareDialogPeerObserver?.disconnect();
  window.shareDialogPeerObserver = null;
  window.shareDialogPeerSignature = "";
}

// 显示分享对话框
function showShareDialog(sharedFiles) {
  console.log("[JS-App] showShareDialog 被调用，文件数:", sharedFiles.length);

  // 仅清理旧 DOM，绝不能在这里调用 apiClearAndroidSharedFiles 误杀 FD！
  stopShareDialogPeerObserver();
  const oldDialog = document.querySelector(".share-dialog");
  if (oldDialog) oldDialog.remove();

  // 创建新弹窗 DOM
  const dialog = document.createElement("div");
  dialog.className = "share-dialog";
  dialog.innerHTML = `
        <div class="share-dialog-content">
            <h3>选择接收者</h3>
            <p>共 ${sharedFiles.length} 个文件</p>
            <ul class="share-user-list" id="share-user-list"></ul>
            <button class="cancel-btn" id="share-cancel-btn">取消</button>
        </div>
    `;

  document.body.appendChild(dialog);

  // 绑定专门的取消事件
  const cancelBtn = document.getElementById("share-cancel-btn");
  if (cancelBtn) {
    cancelBtn.addEventListener("click", cancelShareDialog);
  }

  syncShareUserList(sharedFiles);
  const peerList = document.getElementById("user-list");
  if (peerList) {
    window.shareDialogPeerObserver = new MutationObserver(() => syncShareUserList(sharedFiles));
    window.shareDialogPeerObserver.observe(peerList, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["class", "data-name", "data-addr"],
    });
  }
}

// ================= 弹窗内用户列表的增量同步魔法 =================
function syncShareUserList(sharedFiles) {
  const userList = document.getElementById("share-user-list");
  if (!userList) return;

  // 获取主界面上最新的用户列表
  const allUsers = document.querySelectorAll("#user-list li");

  // 收集所有用户（含离线），排序后构建渲染数组
  const allEntries = [];
  allUsers.forEach((userItem) => {
    const isOffline = userItem.classList.contains("offline");
    allEntries.push({
      id: userItem.dataset.id,
      name: userItem.dataset.name,
      addr: userItem.dataset.addr,
      isOffline,
    });
  });

  // 排序：在线 > 离线，同名按名称字母
  allEntries.sort((a, b) => {
    if (a.isOffline !== b.isOffline) return a.isOffline ? 1 : -1;
    return a.name.localeCompare(b.name);
  });

  const signature = JSON.stringify(allEntries);
  if (signature === window.shareDialogPeerSignature) return;
  window.shareDialogPeerSignature = signature;

  userList.innerHTML = "";

  if (allEntries.length === 0) {
    userList.innerHTML = '<li class="no-users">暂无用户</li>';
    return;
  }

  allEntries.forEach(({ id, name, addr, isOffline }) => {
    const li = document.createElement("li");
    li.dataset.id = id;
    if (isOffline) li.classList.add("offline");
    li.innerHTML = isOffline
      ? `${name} <span class="offline-tag">(离线)</span>`
      : name;
    li.onclick = () => handleShareToUser(id, name, addr, sharedFiles);
    userList.appendChild(li);
  });
}

// 用户主动取消分享
function cancelShareDialog() {
  stopShareDialogPeerObserver();
  const dialog = document.querySelector(".share-dialog");
  if (dialog) dialog.remove();

  // 只有用户主动取消，才安全释放所有底层 FD
  apiClearAndroidSharedFiles();
}

// 处理分享到指定用户
async function handleShareToUser(userId, userName, userAddr, sharedFiles) {
  console.log("[JS-App] 分享文件到:", userName);

  // 用户确认发送，前端立刻交出 FD 的管理权！
  // 这样无论后续弹窗怎么销毁，前端都不会再去误杀这些 FD，后端的 Rust 拿到 FD 传完会自动释放
  window.__ANDROID_SHARED_FILES__ = null;

  // 清理弹窗 DOM
  stopShareDialogPeerObserver();
  const dialog = document.querySelector(".share-dialog");
  if (dialog) dialog.remove();

  // 打开聊天窗口并发送
  openChat({ id: userId, name: userName, addr: userAddr });

  for (const fileInfo of sharedFiles) {
    try {
      console.log("[JS-App] 发送文件:", fileInfo.fileName);
      await apiSendFileFromAndroidUri(userId, userAddr, fileInfo);
      console.log("[JS-App] 文件发送成功:", fileInfo.fileName);
    } catch (e) {
      console.error("[JS-App] 文件发送失败:", fileInfo.fileName, e);
      showFileSendFailure(e, fileInfo.fileName);
    }
  }

  console.log("[JS-App] 重新加载聊天历史");
  await loadChatHistory(userId);
}

// 格式化文件大小
function formatFileSize(bytes) {
  if (bytes < 1024) return bytes + " B";
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(2) + " KB";
  if (bytes < 1024 * 1024 * 1024) {
    return (bytes / (1024 * 1024)).toFixed(2) + " MB";
  }
  return (bytes / (1024 * 1024 * 1024)).toFixed(2) + " GB";
}

// Web 端轮询新消息
async function startMessagePolling() {
  const pollInterval = 1000;
  window.messagePollingEnabled = true;
  let pollInFlight = false;

  const checkNewMessages = async () => {
    if (!window.messagePollingEnabled || !window.currentChatPeer || pollInFlight) return;
    pollInFlight = true;

    try {
      const chatMessages = document.getElementById("chat-messages");
      if (!chatMessages) return;
      const peerId = window.currentChatPeer.id;
      const renderSession = window.chatRenderController?.snapshot(peerId);

      const shouldFollow = window.chatRenderController?.followLatest ?? true;

      // 2. 获取最新消息
      const latestMessages = await apiGetChatHistory(
        peerId,
        20,
        0,
      );
      if (window.chatRenderController
        ? !window.chatRenderController.isCurrent(renderSession)
        : window.currentChatPeer?.id !== peerId) return;
      if (!latestMessages || latestMessages.length === 0) return;

      const newMessages = latestMessages.filter((msg) =>
        msg.timestamp > (window.lastMessageTimestamp || 0)
      );

      const statusChangedMessages = latestMessages.filter((msg) => {
        if (msg.timestamp > (window.lastMessageTimestamp || 0)) return false;
        const domEl = document.querySelector(`[data-msg-id="${msg.id}"]`);
        return domEl && domEl.dataset.status !== msg.status;
      });

      // 3. 如果有新消息或状态变更
      if (newMessages.length > 0 || statusChangedMessages.length > 0) {
        // 处理状态变更（补发成功）
        for (const msg of statusChangedMessages) {
          addMessageToChat(msg, msg.from_id === "me");
        }
        if (statusChangedMessages.length > 0 && shouldFollow) {
          window.chatRenderController?.scheduleFollowCorrection(renderSession);
        }

        // 处理新消息
        if (newMessages.length > 0) {
          for (const msg of newMessages) {
            addMessageToChat(msg, msg.from_id === "me");
          }

          // 核心逻辑：根据位置决定是自动滚动还是提醒
          if (shouldFollow) {
            window.chatRenderController?.scheduleFollowCorrection(renderSession);
          } else {
            // 不在底部时，强行点亮悬浮按钮和红点
            const scrollBtn = document.getElementById("scroll-to-bottom-btn");
            const unreadDot = document.getElementById("unread-dot");
            if (scrollBtn) scrollBtn.classList.add("show");
            if (unreadDot) unreadDot.classList.add("show");
            console.log("[JS-App] 用户在上方，点亮新消息红点");
          }
        }
      }
    } catch (e) {
      console.error("[JS-App] 轮询失败:", e);
    } finally {
      pollInFlight = false;
    }
  };

  setInterval(() => void checkNewMessages(), pollInterval);
}

// Web 端检查所有用户的未读消息（用于显示左侧列表红点）
async function startUnreadMessageCheck() {
  const pollInterval = 2000; // 2秒检查一次，不需要太频繁

  // 记录每个用户的最后消息时间戳
  if (!window.userLastMessageTimestamps) {
    window.userLastMessageTimestamps = {};
  }

  // 初始化：获取所有用户的当前最新消息时间戳，避免误报
  const initializeTimestamps = async () => {
    try {
      const userList = document.getElementById("user-list");
      if (!userList) return;

      const userItems = userList.querySelectorAll("li");

      for (const userItem of userItems) {
        const userId = userItem.dataset.id;
        if (!userId) continue;

        // 如果还没有记录时间戳，初始化为当前最新消息的时间戳
        if (!window.userLastMessageTimestamps[userId]) {
          const messages = await apiGetChatHistory(userId, 1, 0);
          if (messages && messages.length > 0) {
            window.userLastMessageTimestamps[userId] = messages[0].timestamp;
            console.log(
              "[JS-App] 初始化用户",
              userId,
              "的时间戳:",
              messages[0].timestamp,
            );
          } else {
            // 没有历史消息，设置为当前时间
            window.userLastMessageTimestamps[userId] = Date.now() / 1000;
          }
        }
      }
    } catch (e) {
      console.error("[JS-App] 初始化时间戳失败:", e);
    }
  };

  const checkUnreadMessages = async () => {
    try {
      // 获取所有在线用户
      const userList = document.getElementById("user-list");
      if (!userList) return;

      const userItems = userList.querySelectorAll("li");

      for (const userItem of userItems) {
        const userId = userItem.dataset.id;
        if (!userId) continue;

        // 跳过当前正在聊天的用户（已经在 startMessagePolling 中处理）
        if (window.currentChatPeer && window.currentChatPeer.id === userId) {
          continue;
        }

        // 获取该用户的最新1条消息
        const messages = await apiGetChatHistory(userId, 1, 0);

        if (messages && messages.length > 0) {
          const latestMsg = messages[0];
          const lastTimestamp = window.userLastMessageTimestamps[userId] || 0;

          // 如果有新消息且不是自己发的
          if (
            latestMsg.timestamp > lastTimestamp && latestMsg.from_id !== "me"
          ) {
            console.log("[JS-App] 检测到用户", userId, "有新消息，显示红点");
            userItem.classList.add("has-unread");
            sortUserList();
            updateTrayFlash();
            // 更新时间戳
            window.userLastMessageTimestamps[userId] = latestMsg.timestamp;
          }
        }
      }
      // 全部检查后同步托盘闪烁（可能已有红点或被其他标签页消除）
      updateTrayFlash?.();
    } catch (e) {
      console.error("[JS-App] 检查未读消息失败:", e);
    }
  };

  // 先初始化时间戳
  await initializeTimestamps();

  // 然后开始轮询
  setInterval(checkUnreadMessages, pollInterval);
}

// 处理 start_upload 事件（Web 端手动下载：浏览器上传文件到接收端）
async function handleStartUpload(data) {
  const upload = window.__pendingUploads?.[data.sender_msg_id];
  if (!upload) {
    console.error("[JS-App] start_upload: 找不到待上传文件，msg_id=", data.sender_msg_id);
    updateFileTransferById(data.sender_msg_id, "failed", 0, data.file_size, 0, true);
    fetch("/api/mark_upload_complete", {
      method: "POST", headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ msg_id: data.sender_msg_id, status: "failed" }),
    }).catch(console.warn);
    return;
  }
  console.log("[JS-App] 开始上传:", upload.fileName, "->", data.receiver_addr);
  // 手动请求意味着本次从零重传。
  updateFileTransferById(data.sender_msg_id, "retrying", 0, upload.fileSize, 0, true);
  // 先更新 UI 为上传中状态
  const chatMessages = document.getElementById("chat-messages");
  let msgEl = chatMessages?.querySelector(`[data-sender-msg-id="${data.sender_msg_id}"]`);
  if (!msgEl) {
    msgEl = chatMessages?.querySelector(`[data-msg-id="${data.sender_msg_id}"]`);
  }
  if (msgEl) {
    const statusDiv = msgEl.querySelector(".file-pending");
    if (statusDiv) {
      statusDiv.textContent = "0 MB/s";
      statusDiv.className = "file-uploading";
    }
  }
  try {
    // 重新调用上传逻辑（直接上传到接收端）
    const peerAddr = data.receiver_addr;
    const file = upload.file;
    const fileSize = upload.fileSize;
    const fileName = upload.fileName;
    const myId = await apiGetMyId();

    const chunkSize = calculateOptimalChunkSize(fileSize);
    const totalChunks = Math.ceil(fileSize / chunkSize);
    const uploadUrl = `http://${peerAddr}/api/upload`;

    let offset = 0;
    let chunkIndex = 0;
    const startTime = Date.now();
    let lastLogTime = startTime;

    while (offset < fileSize) {
      const size = Math.min(chunkSize, fileSize - offset);
      const chunk = file.slice(offset, offset + size);
      const formData = new FormData();
      formData.append("peer_id", myId);
      formData.append("file_name", fileName);
      formData.append("file_size", fileSize.toString());
      formData.append("chunk_index", chunkIndex.toString());
      formData.append("chunk_total", totalChunks.toString());
      formData.append("sender_msg_id", data.sender_msg_id.toString());
      const elapsed = (Date.now() - startTime) / 1000;
      const speed = chunkIndex > 0 && elapsed > 0 ? offset / (1024 * 1024) / elapsed : 0;
      formData.append("speed_mb_s", speed.toFixed(1));
      formData.append("chunk", chunk, "chunk");

      const resp = await fetch(uploadUrl, { method: "POST", body: formData, mode: "cors" });
      if (!resp.ok) throw new Error(`HTTP ${resp.status}`);

      // 秒传检查
      if (chunkIndex === 0) {
        const respData = await resp.json();
        if (respData.status === "already_exists") {
          console.log("[JS-App] ✓ 秒传命中");
          updateFileTransferById(data.sender_msg_id, "sent", fileSize, fileSize, 0, true);
          // 标记本地记录为 accepted
          fetch("/api/mark_upload_complete", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ msg_id: data.sender_msg_id, status: "accepted" }),
          }).catch(e => console.warn("[JS-App] 标记秒传完成失败:", e));
          // 清除 UI 上传中状态
          const chatMessages = document.getElementById("chat-messages");
          const msgEl = chatMessages?.querySelector(`[data-sender-msg-id="${data.sender_msg_id}"]`);
          if (msgEl) {
            const statusDiv = msgEl.querySelector(".file-uploading");
            if (statusDiv) {
              statusDiv.className = "";
              statusDiv.textContent = "";
            }
          }
          delete window.__pendingUploads[data.sender_msg_id];
          return;
        }
      }

      offset += size;
      chunkIndex++;
      updateFileTransferById(data.sender_msg_id, "uploading", offset, fileSize,
        offset / Math.max(0.001, (Date.now() - startTime) / 1000) / (1024 * 1024));

      // 每秒更新一次速度
      const now = Date.now();
      if (now - lastLogTime > 1000) {
        const elapsed = (now - startTime) / 1000;
        const speed = offset / (1024 * 1024) / elapsed;
        console.log("[JS-App] 手动上传: ", Math.round(offset / 1024 / 1024), "MB, 速度:", Math.round(speed), "MB/s");
        lastLogTime = now;
      }
    }

    console.log("[JS-App] ✓ 文件上传完成:", fileName);
    delete window.__pendingUploads[data.sender_msg_id];
    // 更新 DB 状态为 sent（使刷新后不再显示上传中）
    try {
      await fetch("/api/mark_upload_complete", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ msg_id: data.sender_msg_id, status: "sent" }),
      });
    } catch (_) {}
    updateFileTransferById(data.sender_msg_id, "sent", fileSize, fileSize, 0, true);
  } catch (e) {
    console.error("[JS-App] 上传失败:", e.message);
    fetch("/api/mark_upload_complete", {
      method: "POST", headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ msg_id: data.sender_msg_id, status: "failed" }),
    }).catch(console.warn);
    updateFileTransferById(data.sender_msg_id, "failed", 0, upload.fileSize, 0, true);
  }
}

// Web 端：连接 WebSocket 接收流式事件
function startStreamingWebSocket() {
  const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
  const wsUrl = `${protocol}//${window.location.host}/ws`;
  console.log("[JS-App] 连接流式 WebSocket:", wsUrl);

  function connect() {
    const ws = new WebSocket(wsUrl);

    ws.onopen = () => {
      console.log("[JS-App] ✓ 流式 WebSocket 已连接");
    };

    ws.onmessage = (event) => {
      console.debug("[JS-App] WebSocket 收到消息:", event.data.substring(0, 80));
      try {
        const data = JSON.parse(event.data);
        if (data.msg_type === "start_upload") {
          // ── 接收端请求开始上传（手动下载） ──
          handleStartUpload(data);
        } else if (data.msg_type === "file_status_update" || data.msg_type === "file_download_progress") {
          console.debug("[JS-App] 转发", data.msg_type, "到 onReceiveMessage");
          onReceiveMessage(data);
        } else if (data.stream_id || data.from_id) {
          console.debug("[JS-App] 转发到 onReceiveMessage, is_streaming:", data.is_streaming);
          onReceiveMessage(data);
        }
      } catch (e) {
        console.error("[JS-App] WebSocket 消息解析失败:", e);
      }
    };

    ws.onclose = () => {
      console.log("[JS-App] 流式 WebSocket 断开，3秒后重连");
      setTimeout(connect, 3000);
    };

    ws.onerror = (err) => {
      console.error("[JS-App] 流式 WebSocket 错误:", err);
      ws.close();
    };
  }

  connect();
}
