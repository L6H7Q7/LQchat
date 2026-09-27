// 图标 SVG 常量
const ICON_SELECT_LIST =
  `<svg viewBox="0 0 24 24" width="20" height="20" stroke="currentColor" stroke-width="2" fill="none" stroke-linecap="round" stroke-linejoin="round"><line x1="8" y1="6" x2="21" y2="6"></line><line x1="8" y1="12" x2="21" y2="12"></line><line x1="8" y1="18" x2="21" y2="18"></line><line x1="3" y1="6" x2="3.01" y2="6"></line><line x1="3" y1="12" x2="3.01" y2="12"></line><line x1="3" y1="18" x2="3.01" y2="18"></line></svg>`;
const ICON_CANCEL_X =
  `<svg viewBox="0 0 24 24" width="20" height="20" stroke="currentColor" stroke-width="2" fill="none" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"></line><line x1="6" y1="6" x2="18" y2="18"></line></svg>`;

// 初始化改名功能（点击用户名直接改名）
function initNameEditor() {
  const nameDisplay = document.getElementById("my-name");
  const editPanel = document.getElementById("edit-name-panel");
  const nameInput = document.getElementById("new-name-input");
  const saveBtn = document.getElementById("save-name-btn");
  const cancelBtn = document.getElementById("cancel-name-btn");
  const errorMsg = document.getElementById("error-msg");

  // 点击用户名切换改名面板
  nameDisplay.addEventListener("keydown", event => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      nameDisplay.click();
    }
  });
  nameDisplay.addEventListener("click", () => {
    if (editPanel.style.display === "block") {
      editPanel.style.display = "none";
      errorMsg.textContent = "";
    } else {
      editPanel.style.display = "block";
      nameInput.value = "";
      nameInput.focus();
      errorMsg.textContent = "";
    }
  });

  // 点击取消按钮
  cancelBtn.addEventListener("click", () => {
    editPanel.style.display = "none";
    errorMsg.textContent = "";
  });

  // 点击保存按钮
  saveBtn.addEventListener("click", async () => {
    const newName = nameInput.value.trim();

    if (!newName) {
      errorMsg.textContent = t("name_empty");
      return;
    }

    if (newName.length > 50) {
      errorMsg.textContent = t("name_too_long");
      return;
    }

    try {
      saveBtn.disabled = true;
      saveBtn.textContent = t("saving");
      errorMsg.textContent = "";

      const updatedName = await apiUpdateMyName(newName);

      // 更新显示
      nameDisplay.textContent = updatedName;
      const androidName = document.getElementById("android-device-name");
      if (androidName) androidName.textContent = updatedName;

      // 显示成功提示并等待 1.5 秒
      errorMsg.style.color = "var(--text)";
      errorMsg.textContent = t("name_saved");
      await new Promise(r => setTimeout(r, 1500));

      editPanel.style.display = "none";
      errorMsg.style.color = "";
      errorMsg.textContent = "";

      console.log("[UI] 用户名更新成功:", updatedName);
    } catch (e) {
      errorMsg.textContent = e.message || t("name_update_fail");
      console.error("[UI] 更新用户名失败:", e);
    } finally {
      saveBtn.disabled = false;
      saveBtn.textContent = t("save");
    }
  });

  // 支持回车键保存
  nameInput.addEventListener("keypress", (e) => {
    if (e.key === "Enter") {
      saveBtn.click();
    }
  });

  // 支持 ESC 键取消
  nameInput.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      cancelBtn.click();
    }
  });
}

// 添加新用户到列表
async function addUserToList(id, name, addr, isOffline = false, deferSort = false) {
  const list = document.getElementById("user-list");
  if (!list) return;

  // 检查是否已存在
  const existingItems = list.querySelectorAll("li");
  for (let item of existingItems) {
    if (item.dataset.id === id) {
      // 已存在,更新状态
      const sortChanged = updateUserStatus(item, name, addr, isOffline);
      if (sortChanged && !deferSort) sortUserList();
      return;
    }
  }

  // 不存在,创建新的
  const li = document.createElement("li");
  li.dataset.id = id;
  li.dataset.name = name;
  li.dataset.addr = addr;
  if (document.body.classList.contains("windows-app") || document.body.classList.contains("android-app")) {
    li.tabIndex = 0;
    li.setAttribute("role", "button");
    li.title = name;
    li.addEventListener("keydown", event => {
      if (event.key === "Enter" || event.key === " ") { event.preventDefault(); li.click(); }
    });
  }
  li.innerHTML = `
        <span class="android-peer-avatar" aria-hidden="true">
          <svg viewBox="0 0 24 24"><rect x="3" y="5" width="18" height="12" rx="2"></rect><path d="M8 21h8M12 17v4"></path></svg>
        </span>
        <span class="android-peer-copy">
          <span class="user-name">${name}</span>
          <span class="user-addr">${addr} · ${isOffline ? "离线" : "在线"}</span>
        </span>
        <span class="user-status">${isOffline ? "offline" : ""}</span>
        <span class="android-peer-chevron" aria-hidden="true">›</span>
    `;

  if (isOffline) {
    li.classList.add("offline");
  }

  // 添加点击事件,从 dataset 动态读取最新数据,而不是使用闭包里的旧变量
  li.addEventListener("click", function (e) {
    const targetLi = e.currentTarget;
    openChat({
      id: targetLi.dataset.id,
      name: targetLi.dataset.name,
      addr: targetLi.dataset.addr,
    });
  });

  // 桌面端右键
  li.addEventListener("contextmenu", function (e) {
    e.preventDefault();
    const targetLi = e.currentTarget;
    showUserActionDialog(targetLi.dataset.id, targetLi.dataset.name);
  });

  // 移动端长按逻辑
  let touchTimer;
  li.addEventListener("touchstart", function (e) {
    const targetLi = e.currentTarget;
    touchTimer = setTimeout(() => {
      showUserActionDialog(targetLi.dataset.id, targetLi.dataset.name);
    }, 600); // 600ms 定义为长按
  });
  li.addEventListener("touchend", () => clearTimeout(touchTimer));
  li.addEventListener("touchmove", () => clearTimeout(touchTimer));

  list.appendChild(li);
  if (!deferSort) sortUserList();

  // 初始化新用户的时间戳,避免误报未读消息
  if (!window.userLastMessageTimestamps) {
    window.userLastMessageTimestamps = {};
  }

  if (!window.userLastMessageTimestamps[id]) {
    try {
      const messages = await apiGetChatHistory(id, 1, 0);
      if (messages && messages.length > 0) {
        window.userLastMessageTimestamps[id] = messages[0].timestamp;
        console.log(
          "[UI] 初始化新用户",
          name,
          "的时间戳:",
          messages[0].timestamp,
        );
      } else {
        // 没有历史消息,设置为当前时间
        window.userLastMessageTimestamps[id] = Date.now() / 1000;
        console.log("[UI] 新用户", name, "没有历史消息,设置时间戳为当前时间");
      }
    } catch (e) {
      console.warn("[UI] 初始化用户时间戳失败:", e);
      // 失败时也设置为当前时间,避免误报
      window.userLastMessageTimestamps[id] = Date.now() / 1000;
    }
  }

  console.log(
    "[UI] 添加用户到列表:",
    name,
    id,
    isOffline ? "(离线)" : "(在线)",
  );
}

// 更新用户状态
function updateUserStatus(item, name, addr, isOffline) {
  const statusSpan = item.querySelector(".user-status");
  const nameSpan = item.querySelector(".user-name");
  const addrSpan = item.querySelector(".user-addr");

  if (nameSpan && nameSpan.textContent !== name) nameSpan.textContent = name;
  const displayAddr = document.body.classList.contains("android-app")
    ? `${addr} · ${isOffline ? "离线" : "在线"}`
    : addr;
  if (addrSpan && addrSpan.textContent !== displayAddr) addrSpan.textContent = displayAddr;
  const nextStatus = isOffline ? "OFF" : "";
  if (statusSpan && statusSpan.textContent !== nextStatus) {
    statusSpan.textContent = nextStatus;
  }

  // 实时更新 DOM 的隐式数据属性
  if (item.dataset.name !== name) item.dataset.name = name;
  if (item.dataset.addr !== addr) item.dataset.addr = addr;

  const wasOffline = item.classList.contains("offline");

  if (isOffline) {
    item.classList.add("offline");
  } else {
    item.classList.remove("offline");
  }

  // 只要状态发生了变化(上线或下线),就重排一次列表
  if (wasOffline !== isOffline) {
    console.log(`[UI] 用户 ${name} 状态变更为: ${isOffline ? "离线" : "在线"}`);
  }
  return wasOffline !== isOffline;
}

// 通用排序函数:未读 > 在线 > 字母顺序
function sortUserList(force = false) {
  const list = document.getElementById("user-list");
  if (!list) return;

  if (!force && document.body.classList.contains("android-app") &&
      document.querySelector("#user-mgmt-panel, .confirm-dialog-overlay")) {
    window.userListSortPending = true;
    return;
  }
  window.userListSortPending = false;

  const items = Array.from(list.querySelectorAll("li"));
  const sortedItems = [...items].sort((a, b) => {
    // 1. 检查未读状态 (最高优先级)
    const aUnread = a.classList.contains("has-unread") ? 1 : 0;
    const bUnread = b.classList.contains("has-unread") ? 1 : 0;
    if (aUnread !== bUnread) return bUnread - aUnread;

    // 2. 检查离线状态 (在线 > 离线)
    const aOffline = a.classList.contains("offline") ? 1 : 0;
    const bOffline = b.classList.contains("offline") ? 1 : 0;
    if (aOffline !== bOffline) return aOffline - bOffline;

    // 3. 按名称排序 (稳定性排序)
    const aName = a.querySelector(".user-name").textContent.toLowerCase();
    const bName = b.querySelector(".user-name").textContent.toLowerCase();
    return aName.localeCompare(bName);
  });

  const orderChanged = sortedItems.some((item, index) => item !== items[index]);
  if (!orderChanged) return;

  const fragment = document.createDocumentFragment();
  sortedItems.forEach((item) => fragment.appendChild(item));
  list.appendChild(fragment);
}

// 当前聊天对象 - 全局变量
window.currentChatPeer = null;

const CHAT_BOTTOM_THRESHOLD = 24;
const chatRenderController = {
  sessionId: 0,
  peerId: null,
  followLatest: true,
  userScrollIntentUntil: 0,
  correctionFrame: 0,
  messageResizeObserver: null,

  begin(peerId) {
    this.sessionId += 1;
    this.peerId = peerId;
    this.followLatest = true;
    this.userScrollIntentUntil = 0;
    window.currentChatMessages = null;
    return { peerId, sessionId: this.sessionId };
  },

  close() {
    this.sessionId += 1;
    this.peerId = null;
    this.followLatest = true;
    window.currentChatMessages = null;
    if (this.correctionFrame) cancelAnimationFrame(this.correctionFrame);
    this.messageResizeObserver?.disconnect();
  },

  snapshot(peerId = this.peerId) {
    return { peerId, sessionId: this.sessionId };
  },

  isCurrent(snapshot) {
    return Boolean(
      snapshot &&
      snapshot.sessionId === this.sessionId &&
      snapshot.peerId === this.peerId &&
      window.currentChatPeer?.id === snapshot.peerId,
    );
  },

  getContainer() {
    return document.getElementById("chat-messages");
  },

  ensureBottomAnchor(container = this.getContainer()) {
    if (!container) return null;
    let anchor = container.querySelector("#chat-bottom-anchor");
    if (!anchor) {
      anchor = document.createElement("div");
      anchor.id = "chat-bottom-anchor";
      anchor.className = "chat-bottom-anchor";
      anchor.setAttribute("aria-hidden", "true");
      container.appendChild(anchor);
    }
    return anchor;
  },

  isAtBottom(container = this.getContainer()) {
    if (!container) return true;
    return container.scrollHeight - container.scrollTop - container.clientHeight <=
      CHAT_BOTTOM_THRESHOLD;
  },

  noteUserScrollIntent() {
    this.userScrollIntentUntil = performance.now() + 700;
  },

  handleScroll() {
    const atBottom = this.isAtBottom();
    if (atBottom) {
      this.followLatest = true;
    } else if (performance.now() <= this.userScrollIntentUntil) {
      this.followLatest = false;
    }
    this.updateBottomButton(atBottom);
  },

  updateBottomButton(atBottom = this.isAtBottom()) {
    const button = document.getElementById("scroll-to-bottom-btn");
    if (!button) return;
    button.classList.toggle("show", !atBottom);
    if (atBottom) document.getElementById("unread-dot")?.classList.remove("show");
  },

  scrollToLatest({ force = false, behavior = "auto", snapshot = null } = {}) {
    if (snapshot && !this.isCurrent(snapshot)) return;
    if (!force && !this.followLatest) return;
    const container = this.getContainer();
    if (!container) return;
    this.ensureBottomAnchor(container);
    if (behavior === "smooth") {
      container.scrollTo({ top: container.scrollHeight, behavior });
    } else {
      container.scrollTop = container.scrollHeight;
    }
    this.updateBottomButton(true);
  },

  scheduleFollowCorrection(snapshot = this.snapshot()) {
    if (!this.followLatest || !this.isCurrent(snapshot)) return;
    if (this.correctionFrame) cancelAnimationFrame(this.correctionFrame);
    this.correctionFrame = requestAnimationFrame(() => {
      this.correctionFrame = requestAnimationFrame(() => {
        this.correctionFrame = 0;
        this.scrollToLatest({ snapshot });
      });
    });
  },

  observeMessages(container = this.getContainer()) {
    if (!container || typeof ResizeObserver === "undefined") return;
    if (!this.messageResizeObserver) {
      this.messageResizeObserver = new ResizeObserver(() => {
        if (this.followLatest) this.scheduleFollowCorrection();
      });
    }
    this.messageResizeObserver.disconnect();
    container.querySelectorAll(":scope > .message").forEach((message) => {
      this.messageResizeObserver.observe(message);
    });
  },

  insertBeforeAnchor(node, container = this.getContainer()) {
    if (!container) return;
    const anchor = this.ensureBottomAnchor(container);
    container.insertBefore(node, anchor);
    if (node.nodeType === Node.ELEMENT_NODE && node.classList.contains("message")) {
      this.messageResizeObserver?.observe(node);
    }
  },

  captureVisibleAnchor(container = this.getContainer()) {
    if (!container) return null;
    const containerTop = container.getBoundingClientRect().top;
    const message = [...container.querySelectorAll(":scope > .message")]
      .find((item) => item.getBoundingClientRect().bottom > containerTop);
    if (!message) return null;
    return {
      id: message.dataset.msgId,
      offset: message.getBoundingClientRect().top - containerTop,
    };
  },

  restoreVisibleAnchor(anchor, container = this.getContainer()) {
    if (!anchor || !container) return;
    const message = container.querySelector(`[data-msg-id="${anchor.id}"]`);
    if (!message) return;
    const containerTop = container.getBoundingClientRect().top;
    container.scrollTop += message.getBoundingClientRect().top - containerTop - anchor.offset;
  },
};
window.chatRenderController = chatRenderController;

// 初始化聊天功能
function initChat() {
  const closeChatBtn = document.getElementById("close-chat-btn");
  const sendBtn = document.getElementById("send-btn");
  const chatInput = document.getElementById("chat-input");
  const attachFileBtn = document.getElementById("attach-file-btn");
  const fileInput = document.getElementById("file-input");
  const chatContainer = document.getElementById("chat-container");

  ["touchstart", "pointerdown", "wheel"].forEach((eventName) => {
    chatMessagesIntentTarget()?.addEventListener(
      eventName,
      () => chatRenderController.noteUserScrollIntent(),
      { passive: true },
    );
  });

  // 关闭聊天窗口
  closeChatBtn.addEventListener("click", () => {
    if (document.getElementById("android-attachment-panel")?.classList.contains("open")) {
      if (location.hash === "#chat-attachment") history.back();
      else setAndroidAttachmentPanel(false);
      return;
    }
    // 如果在多选模式,先退出
    if (window.selectMode && window.selectMode.active) {
      exitSelectMode();
    }
    closeChat();
  });

  // 发送消息 - 统一处理发送和删除
  sendBtn.addEventListener("click", () => {
    if (window.selectMode && window.selectMode.active) {
      deleteSelectedMessages();
    } else if (isAndroidApp() && androidAttachmentState.selected[androidAttachmentState.kind].size > 0) {
      sendAndroidSelectedAttachments();
    } else {
      sendMessage();
    }
  });

  // 自动调整 textarea 高度
  function adjustTextareaHeight() {
    chatInput.style.height = "auto";
    const newHeight = Math.min(chatInput.scrollHeight, 200);
    chatInput.style.height = newHeight + "px";
  }

  // 输入时调整高度
  chatInput.addEventListener("input", () => {
    adjustTextareaHeight();
    updateAndroidComposerState();
  });

  // 回车发送(Shift+Enter 换行)
  chatInput.addEventListener("keypress", (e) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      sendMessage();
    }
  });

  // 选择文件
  attachFileBtn.addEventListener("click", () => {
    const tauri = window.__TAURI__;
    if (tauri) {
      // Android: 使用自定义 SAF 选择器（持久化权限）
      const isAndroid = navigator.userAgent.includes("Android");
      if (isAndroid) {
        window.androidSafChatTarget = captureChatTarget();
        tauri.core.invoke("open_saf_picker").catch(e => {
          console.error("[UI] SAF 选择器调用失败:", e);
          window.androidSafChatTarget = null;
          // 降级为桌面端对话框
          sendFile(null);
        });
      } else {
        // 桌面端 - 直接调用 sendFile,它会弹出对话框
        sendFile(null);
      }
    } else {
      // Web 端 - 触发文件选择
      fileInput.click();
    }
  });

  // 文件选择后发送(仅 Web 端)
  fileInput.addEventListener("change", async (e) => {
    const files = Array.from(e.target.files || []);
    for (const file of files) {
      await sendFile(file);
    }
    fileInput.value = ""; // 清空选择
  });

  // Android SAF 文件选择器回调（持久化权限 URI）
  (async () => {
    const tauri = window.__TAURI__;
    const isAndroid = navigator.userAgent.includes("Android");
    if (tauri && isAndroid && tauri.event) {
      try {
        await tauri.event.listen("saf-file-selected", async (event) => {
          const fileInfo = event.payload;
          const target = window.androidSafChatTarget;
          window.androidSafChatTarget = null;
          console.log("[UI] SAF 持久化文件已选择:", fileInfo);
          if (!target || !chatRenderController.isCurrent(target.session)) {
            showMessageActionToast("聊天对象已切换，请重新选择文件");
            return;
          }
          try {
            await apiSendFile(
              target.peer.id,
              target.peer.addr,
              null,
              fileInfo.uri
            );
            await refreshCapturedChat(target, { forceLatest: true });
          } catch (e) {
            console.error("[UI] SAF 文件发送失败:", e);
            showFileSendFailure(e);
          }
        });
        console.log("[UI] SAF 文件选择器监听已注册");
      } catch (e) {
        console.error("[UI] 注册 SAF 监听器失败:", e);
      }
    }
  })();

  initAndroidAttachmentPicker();
  updateAndroidComposerState();

  // 拖拽文件功能
  initDragAndDrop(chatContainer);

  // 粘贴文件功能
  initPasteFile();

  // 初始化多选模式
  initSelectMode();

  // 初始化回到底部按钮
  initScrollToBottomBtn();
}

const androidAttachmentState = {
  kind: "image",
  images: [],
  apps: [],
  album: "全部图片",
  selected: { image: new Map(), file: new Map(), app: new Map() },
  targetPeer: null,
  targetSession: null,
};

function copyChatPeer(peer = window.currentChatPeer) {
  return peer ? { id: peer.id, name: peer.name, addr: peer.addr } : null;
}

function captureChatTarget(peer = window.currentChatPeer) {
  const targetPeer = copyChatPeer(peer);
  return targetPeer
    ? { peer: targetPeer, session: chatRenderController.snapshot(targetPeer.id) }
    : null;
}

async function refreshCapturedChat(target, { forceLatest = false } = {}) {
  if (!target || !chatRenderController.isCurrent(target.session)) return false;
  if (forceLatest) chatRenderController.followLatest = true;
  await loadChatHistory(target.peer.id, true, target.session);
  if (forceLatest) {
    chatRenderController.scrollToLatest({ force: true, snapshot: target.session });
    chatRenderController.scheduleFollowCorrection(target.session);
  }
  return true;
}

function clearAndroidAttachmentSelections() {
  Object.values(androidAttachmentState.selected).forEach((selection) => selection.clear());
}

function isAndroidApp() {
  return document.body.classList.contains("android-app");
}

function normalizeAndroidAttachment(item, kind) {
  const uri = item.uri || item.path || item.sourceDir || "";
  return {
    ...item,
    uri,
    name: item.name || item.fileName || item.label || uri.split("/").pop() || "未命名附件",
    size: Number(item.size ?? item.fileSize ?? 0),
    kind,
    status: item.status || "ready",
  };
}

function setAndroidAttachmentPanel(open) {
  const panel = document.getElementById("android-attachment-panel");
  const wasOpen = panel?.classList.contains("open");
  panel?.classList.toggle("open", open);
  if (open && !wasOpen && window.innerWidth <= 768 && location.hash !== "#chat-attachment") {
    history.pushState({ attachmentOpen: true }, "", "#chat-attachment");
  }
}

function updateAndroidAttachmentMeta() {
  const kind = androidAttachmentState.kind;
  const labels = { image: "图片", file: "文件", app: "App" };
  const count = androidAttachmentState.selected[kind].size;
  document.getElementById("android-attachment-title").textContent = labels[kind];
  document.getElementById("android-attachment-count").textContent = `已选 ${count} 项`;
  const send = document.getElementById("android-send-attachments");
  send.textContent = `发送 (${count})`;
  send.disabled = count === 0;
  updateAndroidComposerState();
}

function updateAndroidComposerState() {
  if (!isAndroidApp()) return;
  const hasText = !!document.getElementById("chat-input")?.value.trim();
  const hasAttachments = ["image", "file", "app"].some((kind) => androidAttachmentState.selected[kind].size > 0);
  const send = document.getElementById("send-btn");
  if (send) send.disabled = !hasText && !hasAttachments;
}

function renderAndroidAttachmentPanel() {
  const body = document.getElementById("android-attachment-body");
  if (!body) return;
  const kind = androidAttachmentState.kind;
  const selected = androidAttachmentState.selected[kind];
  updateAndroidAttachmentMeta();

  const source = kind === "image"
    ? androidAttachmentState.images.filter((item) => androidAttachmentState.album === "全部图片" || item.album === androidAttachmentState.album)
    : androidAttachmentState.apps;
  if (!source.length) {
    body.innerHTML = `<div class="android-empty-state">正在读取${kind === "image" ? "相册" : "已安装 App"}…</div>`;
    return;
  }
  const grid = document.createElement("div");
  grid.className = "android-gallery-grid";
  if (kind === "app") grid.classList.add("android-app-gallery-grid");
  if (kind === "image") {
    const albums = ["全部图片", ...new Set(androidAttachmentState.images.map((item) => item.album || "其他"))];
    const selector = document.createElement("select");
    selector.className = "android-album-select";
    albums.forEach((album) => {
      const option = document.createElement("option");
      option.value = album;
      option.textContent = album;
      option.selected = album === androidAttachmentState.album;
      selector.appendChild(option);
    });
    selector.addEventListener("change", () => {
      androidAttachmentState.album = selector.value;
      renderAndroidAttachmentPanel();
    });
    body.replaceChildren(selector, grid);
  }
  source.forEach((raw) => {
    const item = normalizeAndroidAttachment(raw, kind);
    const button = document.createElement("button");
    button.type = "button";
    button.className = `android-gallery-item${selected.has(item.uri) ? " selected" : ""}`;
    const thumb = item.thumbnail || item.icon;
    if (kind === "app") {
      button.classList.add("android-app-gallery-item");
      const icon = document.createElement("span");
      icon.className = "android-app-icon";
      icon.innerHTML = thumb
        ? `<img src="${thumb}" alt=""><i></i>`
        : `<span class="android-gallery-placeholder">A</span><i></i>`;
      const name = document.createElement("span");
      name.className = "android-app-name";
      name.textContent = item.label || item.name.replace(/\.apk$/i, "");
      button.append(icon, name);
    } else {
      button.innerHTML = thumb ? `<img src="${thumb}" alt=""><i></i>` : `<span class="android-gallery-placeholder">▧</span><i></i>`;
    }
    const badge = button.querySelector("i");
    const refreshBadge = () => {
      const index = Array.from(selected.keys()).indexOf(item.uri);
      badge.textContent = index >= 0 ? String(index + 1) : "";
      button.classList.toggle("selected", index >= 0);
    };
    refreshBadge();
    button.title = kind === "app" ? (item.label || item.name.replace(/\.apk$/i, "")) : item.name;
    const toggleSelection = () => {
      if (selected.has(item.uri)) selected.delete(item.uri);
      else selected.set(item.uri, item);
      renderAndroidAttachmentPanel();
    };
    if (kind === "image") {
      badge.addEventListener("click", (event) => {
        event.stopPropagation();
        toggleSelection();
      });
      button.addEventListener("click", () => openAndroidImagePreview(item));
    } else {
      button.addEventListener("click", toggleSelection);
    }
    grid.appendChild(button);
  });
  if (kind !== "image") body.replaceChildren(grid);
}

function openAndroidImagePreview(item) {
  if (!item.thumbnail) return;
  const overlay = document.createElement("div");
  overlay.className = "android-image-preview";
  overlay.innerHTML = `<button type="button" aria-label="关闭预览">×</button><img alt="图片预览">`;
  overlay.querySelector("img").src = item.thumbnail;
  overlay.addEventListener("click", () => overlay.remove());
  document.body.appendChild(overlay);
}

async function openAndroidAttachment(kind, continueAdding = false) {
  if (!window.currentChatPeer) return;
  const currentTarget = captureChatTarget();
  const targetChanged = androidAttachmentState.targetPeer?.id !== currentTarget.peer.id;
  if (targetChanged) clearAndroidAttachmentSelections();
  if (!continueAdding || targetChanged || !androidAttachmentState.targetPeer) {
    androidAttachmentState.targetPeer = currentTarget.peer;
    androidAttachmentState.targetSession = currentTarget.session;
  }
  androidAttachmentState.kind = kind;
  const tauri = window.__TAURI__;
  if (kind === "file") {
    if (!tauri) return;
    await tauri.core.invoke("open_saf_multi_picker").catch((e) => console.error("[UI] 多文件选择器调用失败:", e));
    return;
  }
  setAndroidAttachmentPanel(true);
  renderAndroidAttachmentPanel();
  if (!tauri) return;
  if (kind === "image" && (!androidAttachmentState.images.length || !continueAdding)) {
    await tauri.core.invoke("load_android_media_images").catch((e) => console.error("[UI] 相册读取失败:", e));
  } else if (kind === "app" && (!androidAttachmentState.apps.length || !continueAdding)) {
    await tauri.core.invoke("load_android_apps").catch((e) => console.error("[UI] App 列表读取失败:", e));
  }
}

async function sendAndroidSelectedAttachments() {
  const kind = androidAttachmentState.kind;
  const selected = androidAttachmentState.selected[kind];
  if (!selected.size || !androidAttachmentState.targetPeer) return;
  const target = {
    peer: copyChatPeer(androidAttachmentState.targetPeer),
    session: androidAttachmentState.targetSession,
  };
  if (!chatRenderController.isCurrent(target.session)) {
    showMessageActionToast("聊天对象已切换，请重新选择附件");
    return;
  }
  const items = Array.from(selected.values());
  selected.clear();
  updateAndroidComposerState();
  if (document.getElementById("android-attachment-panel")?.classList.contains("open")) {
    if (location.hash === "#chat-attachment") history.back();
    else setAndroidAttachmentPanel(false);
  }
  await sendAndroidAttachments(items, target);
}

async function sendAndroidAttachments(items, target) {
  if (!items.length) return;
  if (!target || !chatRenderController.isCurrent(target.session)) {
    showMessageActionToast("聊天对象已切换，请重新选择附件");
    return;
  }
  if (document.getElementById("chat-input")?.value.trim()) {
    await sendMessage(target.peer);
  }
  for (const item of items) {
    try {
      await apiSendFile(target.peer.id, target.peer.addr, null, item.uri);
    } catch (e) {
      console.error("[UI] 安卓附件发送失败:", item.name, e);
      showMessageActionToast(`${item.name} 发送失败`, 3000);
    }
    try {
      await refreshCapturedChat(target, { forceLatest: true });
    } catch (e) {
      console.warn("[UI] 刷新安卓附件消息失败:", e);
    }
  }
}

function showIncomingSystemNotification(message) {
  if (!message.from_id || message.from_id === window.myId) return;
  if (message.msg_type === "file" && ["downloading", "uploading", "invalid"].includes(message.file_status)) return;
  const fromName = message.from_name || "未知用户";
  let body = message.content || "";
  if (message.msg_type === "file") body = `[文件] ${message.file_name || message.content || "未知文件"}`;
  else if (message.msg_type === "image") body = "[图片]";
  if (body.length > 100) body = body.substring(0, 100) + "...";
  if (body) showNotification(fromName, body, { from_id: message.from_id });
}

function initAndroidAttachmentPicker() {
  if (!isAndroidApp()) return;
  document.getElementById("android-image-btn")?.addEventListener("click", () => openAndroidAttachment("image"));
  document.getElementById("android-file-btn")?.addEventListener("click", () => openAndroidAttachment("file"));
  document.getElementById("android-app-btn")?.addEventListener("click", () => openAndroidAttachment("app"));
  document.getElementById("android-attachment-close")?.addEventListener("click", () => {
    if (location.hash === "#chat-attachment") history.back();
    else setAndroidAttachmentPanel(false);
  });
  document.getElementById("android-send-attachments")?.addEventListener("click", sendAndroidSelectedAttachments);
  document.getElementById("android-continue-add")?.addEventListener("click", () => openAndroidAttachment(androidAttachmentState.kind, true));

  window.addEventListener("android-files-selected", (event) => {
    const target = {
      peer: copyChatPeer(androidAttachmentState.targetPeer),
      session: androidAttachmentState.targetSession,
    };
    const items = (event.detail || []).map((item) => normalizeAndroidAttachment(item, "file"));
    sendAndroidAttachments(items, target);
  });
  window.addEventListener("android-media-images", (event) => {
    androidAttachmentState.images = event.detail || [];
    if (androidAttachmentState.kind === "image") renderAndroidAttachmentPanel();
  });
  window.addEventListener("android-apps-loaded", (event) => {
    androidAttachmentState.apps = event.detail || [];
    if (androidAttachmentState.kind === "app") renderAndroidAttachmentPanel();
  });
}

// 打开聊天
function openChat(peer) {
  window.NotificationUI?.leave();
  const chatContainer = document.getElementById("chat-container");
  if (!chatContainer) return;

  // 1. 检查是否已经是当前聊天的用户且窗口已开
  if (
    window.currentChatPeer && window.currentChatPeer.id === peer.id &&
    chatContainer.style.display === "flex"
  ) {
    return;
  }

  // 2. 手机端 Hash 处理
  if (window.innerWidth <= 768) {
    if (window.location.hash !== "#chat") {
      window.history.pushState({ chatOpen: true }, "", "#chat");
    }
  }

  window.currentChatPeer = peer;
  const chatSession = chatRenderController.begin(peer.id);

  // 3. 立即显示界面(提升响应感)
  document.body.classList.add("chat-open");
  chatContainer.style.display = "flex";

  const chatWithName = document.getElementById("chat-with-name");
  const chatMessages = document.getElementById("chat-messages");

  if (chatWithName) chatWithName.textContent = `${peer.name}`;
  if (chatMessages) {
    const loading = document.createElement("div");
    loading.className = "chat-loading-state";
    loading.setAttribute("role", "status");
    loading.textContent = "正在加载聊天记录…";
    chatMessages.replaceChildren(loading);
  }

  // 4. 消除红点和高亮
  const userLi = document.querySelector(`#user-list li[data-id="${peer.id}"]`);
  if (userLi) {
    userLi.classList.remove("has-unread");
    updateTrayFlash();
  }
  updateListHighlight(peer.id);

  // 5. 异步加载历史
  window.lastMessageTimestamp = 0;
  loadChatHistory(peer.id, false, chatSession).catch((e) => {
    console.error("[UI] 加载历史失败:", e);
    if (!chatRenderController.isCurrent(chatSession)) return;
    const errorState = document.createElement("div");
    errorState.className = "chat-loading-state chat-load-error";
    const message = document.createElement("span");
    message.textContent = "聊天记录加载失败";
    const retry = document.createElement("button");
    retry.type = "button";
    retry.textContent = "重试";
    retry.addEventListener("click", () => {
      errorState.replaceChildren("正在重新加载…");
      loadChatHistory(peer.id, false, chatSession).catch((retryError) => {
        console.error("[UI] 重新加载历史失败:", retryError);
        if (chatRenderController.isCurrent(chatSession)) {
          errorState.replaceChildren(message, retry);
        }
      });
    });
    errorState.append(message, retry);
    chatMessages?.replaceChildren(errorState);
  });

  // 6. 清除系统通知栏中该用户的未读通知（按 from_id 按组清除）
  if (window.__TAURI__) {
    window.__TAURI__.core.invoke("clear_notification", { fromId: peer.id })
      .catch(e => console.error("[UI] 清除通知失败:", e));
  }

  // 7. 切换到新聊天时隐藏回到底部按钮和红点
  const scrollBtn = document.getElementById("scroll-to-bottom-btn");
  if (scrollBtn) scrollBtn.classList.remove("show");
  const unreadDot = document.getElementById("unread-dot");
  if (unreadDot) unreadDot.classList.remove("show");

  console.log("[UI] 成功进入聊天:", peer.name);
}

// 2. 关闭聊天(由 X 按钮或物理返回键调用)
function closeChat() {
  // 如果是手机端且有 #chat,点击 X 按钮时触发 back() 即可,剩下的交给 popstate
  if (window.innerWidth <= 768 && window.location.hash === "#chat") {
    window.history.back();
    return;
  }
  performCloseChatUI();
}

// 3. 真正的 UI 隐藏逻辑(只管藏,不管历史记录)
function performCloseChatUI() {
  // 如果在多选模式,先退出
  if (window.selectMode && window.selectMode.active) {
    exitSelectMode();
  }

  const chatContainer = document.getElementById("chat-container");
  if (chatContainer) chatContainer.style.display = "none";
  document.body.classList.remove("chat-open");
  chatRenderController.close();
  window.currentChatPeer = null;
  updateListHighlight(null); // 清除高亮
}

// 4. 辅助函数:更新高亮
function updateListHighlight(activeId) {
  const items = document.querySelectorAll("#user-list li");
  items.forEach((item) => {
    if (activeId && item.dataset.id === activeId) {
      item.classList.add("active");
    } else {
      item.classList.remove("active");
    }
  });
}

// 5. 全局监听器:处理物理返回键和手动后退
window.addEventListener("popstate", function (event) {
  if (document.body.classList.contains("android-app") &&
      ["#settings", "#permissions", "#push-apps", "#lq-push"].includes(location.hash)) return;
  const chatContainer = document.getElementById("chat-container");

  const attachmentPanel = document.getElementById("android-attachment-panel");
  if (attachmentPanel?.classList.contains("open")) {
    attachmentPanel.classList.remove("open");
    if (window.innerWidth <= 768 && window.location.hash !== "#chat") {
      window.history.replaceState({ chatOpen: true }, "", "#chat");
    }
    return;
  }

  // 【场景 A】如果当前处于多选模式
  if (window.selectMode && window.selectMode.active) {
    console.log("[UI] 拦截返回键:退出多选模式");

    // 手动执行退出多选的 UI 恢复逻辑
    window.selectMode.active = false;
    window.selectMode.selectedMessages.clear();

    const selectModeBtn = document.getElementById("select-mode-btn");
    const sendBtn = document.getElementById("send-btn");
    const chatInput = document.getElementById("chat-input");
    const attachFileBtn = document.getElementById("attach-file-btn");

    // 恢复图标
    if (selectModeBtn) {
      selectModeBtn.innerHTML =
        '<svg viewBox="0 0 24 24" width="20" height="20" stroke="currentColor" stroke-width="2" fill="none" stroke-linecap="round" stroke-linejoin="round"><line x1="8" y1="6" x2="21" y2="6"></line><line x1="8" y1="12" x2="21" y2="12"></line><line x1="8" y1="18" x2="21" y2="18"></line><line x1="3" y1="6" x2="3.01" y2="6"></line><line x1="3" y1="12" x2="3.01" y2="12"></line><line x1="3" y1="18" x2="3.01" y2="18"></line></svg>';
      selectModeBtn.classList.remove("active");
    }

    if (sendBtn) {
      sendBtn.textContent = t("send");
      sendBtn.style.backgroundColor = "";
      sendBtn.style.borderColor = "";
      sendBtn.style.color = "";
    }

    if (chatInput) chatInput.disabled = false;
    if (attachFileBtn) attachFileBtn.disabled = false;

    const messages = document.querySelectorAll(".message");
    messages.forEach((msg) => {
      // 简单清理
      const checkbox = msg.querySelector(".select-checkbox");
      if (checkbox) checkbox.remove();
      msg.classList.remove("selectable", "selected");
    });

    const fileContainers = document.querySelectorAll(".message-file");
    fileContainers.forEach((container) =>
      container.style.pointerEvents = "auto"
    );

    // 【核心修复 1】平板/手机 URL 状态修正
    // 如果是手机端,且当前不是 #chat,补回 #chat 以保持聊天窗口打开
    if (window.innerWidth <= 768 && window.location.hash !== "#chat") {
      window.history.replaceState({ chatOpen: true }, "", "#chat");
    }

    return;
  }

  // 【场景 B】正常关闭聊天窗口逻辑
  // 如果 URL 已经不是 #chat 了
  if (window.location.hash !== "#chat") {
    if (window.innerWidth <= 768) {
      performCloseChatUI();
    }
  }
});

// 发送消息
async function sendMessage(targetPeer = window.currentChatPeer) {
  if (!targetPeer) return false;

  const chatInput = document.getElementById("chat-input");
  const draft = chatInput.value;
  const content = draft.trim();
  if (!content) return false;
  const chatSession = chatRenderController.snapshot(targetPeer.id);

  chatInput.value = "";
  chatInput.style.height = "auto";
  updateAndroidComposerState();

  try {
    // 1. 发送 API
    await apiSendMessage(
      targetPeer.id,
      targetPeer.addr,
      content,
    );

    if (chatRenderController.isCurrent(chatSession)) {
      chatRenderController.followLatest = true;
      await loadChatHistory(targetPeer.id, true, chatSession);
      chatRenderController.scrollToLatest({ force: true, snapshot: chatSession });
    }
    return true;
  } catch (e) {
    console.error("[UI] 发送异常:", e);
    if (chatRenderController.isCurrent(chatSession) && !chatInput.value) {
      chatInput.value = draft;
      chatInput.dispatchEvent(new Event("input", { bubbles: true }));
    }
    showMessageActionToast("发送失败：" + getActionErrorMessage(e), 4000);
    return false;
  }
}

function upsertMessageInContainer(container, message, isSent, prepend = false) {
  if (!container || !message.id || message.id === "null") return;
  const existing = container.querySelector(`[data-msg-id="${message.id}"]`);
  const next = createMessageElement(message, isSent);
  if (existing) {
    existing.replaceWith(next);
  } else if (prepend) {
    container.insertBefore(next, container.firstChild);
  } else {
    chatRenderController.insertBeforeAnchor(next, container);
  }
  chatRenderController.messageResizeObserver?.observe(next);
  chatRenderController.scheduleFollowCorrection();
}

function chatMessagesIntentTarget() {
  return document.getElementById("chat-messages");
}

function addMessageToChat(message, isSent) {
  // 如果 ID 依然是无效的，坚决不渲染到 DOM，防止产生无法选中的"僵尸"气泡
  if (!message.id || message.id === "null") return;
  const chatMessages = document.getElementById("chat-messages");
  upsertMessageInContainer(chatMessages, message, isSent);

  if (message.timestamp && !String(message.id).startsWith("temp_")) {
    if (message.timestamp > (window.lastMessageTimestamp || 0)) {
      window.lastMessageTimestamp = message.timestamp;
    }
  }
}

// 在聊天顶部插入消息（带防重检查）
function prependMessageToChat(message, isSent) {
  if (!message.id || message.id === "null") return;
  const chatMessages = document.getElementById("chat-messages");
  upsertMessageInContainer(chatMessages, message, isSent, true);
}

// 更新流式消息气泡内容
function updateStreamMessage(message) {
  if (!message.stream_id) return;
  const chatMessages = document.getElementById("chat-messages");
  let container = chatMessages.querySelector(`[data-stream-id="${message.stream_id}"]`);

  // 创建流式容器（首次）
  if (!container) {
    container = document.createElement("div");
    container.className = "message received";
    container.dataset.streamId = message.stream_id;
    const contentDiv = document.createElement("div");
    contentDiv.className = "message-content stream-content";
    container.appendChild(contentDiv);
    chatRenderController.insertBeforeAnchor(container, chatMessages);
    chatRenderController.messageResizeObserver?.observe(container);
  }

  const contentDiv = container.querySelector(".message-content");

  switch (message.msg_type) {
    case "text":
      if (message.is_thinking) {
        // thinking 块：如果最后一个子元素也是 thinking，更新之；否则追加新的
        let block = contentDiv.querySelector(":scope > .thinking-block:last-of-type");
        if (block && block === contentDiv.lastElementChild) {
          block.textContent = message.content;
        } else {
          block = document.createElement("div");
          block.className = "thinking-block";
          block.textContent = message.content;
          contentDiv.appendChild(block);
        }
      } else {
        // text 块：如果最后一个子元素也是 .message-text，更新之；否则追加新的
        let block = contentDiv.querySelector(":scope > .message-text:last-of-type");
        if (block && block === contentDiv.lastElementChild) {
          block.textContent = message.content;
        } else {
          block = document.createElement("div");
          block.className = "message-text";
          block.textContent = message.content;
          contentDiv.appendChild(block);
        }
      }
      break;

    case "tool_call":
      // 工具调用：一次性追加
      const name = message.tool_name || "";
      const args = message.tool_args || "";
      const tBlock = createToolCallBlock(name, args);
      contentDiv.appendChild(tBlock);
      break;

    case "tool_result":
      // 工具结果：一次性追加，默认折叠
      const rBlock = createToolResultBlock(message.tool_output || "", message.is_error);
      contentDiv.appendChild(rBlock);
      break;
  }
}

// 创建文件图标元素
function createFileIcon(message) {
  const fileInfo = document.createElement("div");
  fileInfo.className = "file-info-wrapper";

  // 1. 图标
  const fileIcon = document.createElement("span");
  fileIcon.className = "file-icon";
  fileIcon.textContent = "📄";

  // 2. 文件信息
  const fileInfoText = document.createElement("div");
  fileInfoText.className = "file-info";

  // 文件名
  const fileName = document.createElement("div");
  fileName.className = "file-name";
  fileName.textContent = message.file_name || message.content;
  fileName.title = fileName.textContent;

  // 文件大小
  const fileSize = document.createElement("div");
  fileSize.className = "file-size";
  fileSize.textContent = message.file_size
    ? formatFileSize(message.file_size)
    : "未知大小";

  fileInfoText.appendChild(fileName);
  fileInfoText.appendChild(fileSize);

  fileInfo.appendChild(fileIcon);
  fileInfo.appendChild(fileInfoText);

  return fileInfo;
}

// 检查是否是图片文件
function isImageFile(fileName) {
  if (!fileName) return false;

  const imageExtensions = [
    ".jpg",
    ".jpeg",
    ".png",
    ".gif",
    ".bmp",
    ".webp",
    ".svg",
    ".ico",
  ];
  const lowerFileName = fileName.toLowerCase();

  return imageExtensions.some((ext) => lowerFileName.endsWith(ext));
}

// 将 Android SAF 的 content URI 转成用户可理解的保存位置。
function formatAndroidStoredPath(filePath, fileName) {
  if (!filePath) return fileName || "未知位置";

  if (filePath.startsWith("fd:")) {
    return `临时文件/${fileName || "未命名文件"}`;
  }

  if (filePath.startsWith("content://")) {
    try {
      const decoded = decodeURIComponent(filePath);
      const documentMarker = "/document/";
      const documentIndex = decoded.lastIndexOf(documentMarker);
      if (documentIndex >= 0) {
        const documentId = decoded.slice(documentIndex + documentMarker.length);
        if (documentId.startsWith("primary:")) {
          return `内部存储/${documentId.slice("primary:".length)}`;
        }
        if (documentId.includes(":")) {
          const separator = documentId.indexOf(":");
          const volume = documentId.slice(0, separator);
          const path = documentId.slice(separator + 1);
          return `${volume}/${path}`;
        }
      }
      return `系统选定目录/${fileName || "未命名文件"}`;
    } catch (_) {
      return `系统选定目录/${fileName || "未命名文件"}`;
    }
  }

  return filePath.replaceAll("\\", "/");
}

function showMessageActionToast(text, duration = 1400) {
  document.querySelector(".message-action-toast")?.remove();
  const toast = document.createElement("div");
  toast.className = "message-action-toast";
  toast.setAttribute("role", "status");
  toast.textContent = text;
  document.body.appendChild(toast);
  requestAnimationFrame(() => toast.classList.add("show"));
  setTimeout(() => {
    toast.classList.remove("show");
    setTimeout(() => toast.remove(), 180);
  }, duration);
}

function getActionErrorMessage(error) {
  if (typeof error === "string") return error;
  if (error?.message) return error.message;
  try {
    return JSON.stringify(error);
  } catch (_) {
    return String(error || "未知错误");
  }
}

async function copyMessageText(text) {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
    } else {
      const textarea = document.createElement("textarea");
      textarea.value = text;
      textarea.setAttribute("readonly", "");
      textarea.style.position = "fixed";
      textarea.style.opacity = "0";
      document.body.appendChild(textarea);
      textarea.select();
      if (!document.execCommand("copy")) throw new Error("copy failed");
      textarea.remove();
    }
    showMessageActionToast("已复制");
  } catch (error) {
    console.error("[UI] 复制消息失败:", error);
    showMessageActionToast("复制失败");
  }
}

// 更新托盘闪烁状态（桌面端：有未读红点时闪烁）
function updateTrayFlash() {
  if (!window.__TAURI__) {
    console.log("[UI] updateTrayFlash: 非 Tauri 环境");
    return;
  }
  const hasUnreadUser = document.querySelector("#user-list li.has-unread") !== null;
  const hasUnreadScroll = document.querySelector("#unread-dot.show") !== null;
  const hasUnread = hasUnreadUser || hasUnreadScroll;
  const tauri = window.__TAURI__;
  if (!tauri?.core?.invoke) {
    console.log("[UI] updateTrayFlash: invoke 不可用");
    return;
  }
  console.log("[UI] updateTrayFlash: hasUnread =", hasUnread);
  if (hasUnread) {
    tauri.core.invoke("start_tray_flash").then(
      () => console.log("[UI] updateTrayFlash: 开始闪烁"),
      (e) => console.error("[UI] updateTrayFlash: start_tray_flash 失败", e),
    );
  } else {
    tauri.core.invoke("stop_tray_flash").then(
      () => console.log("[UI] updateTrayFlash: 停止闪烁"),
      (e) => console.error("[UI] updateTrayFlash: stop_tray_flash 失败", e),
    );
  }
}

// 展开/收起后检查滚动按钮状态
function checkScrollButton() {
  chatRenderController.updateBottomButton();
}

// 滚动到聊天窗口底部(等待图片加载)
async function scrollToBottom() {
  chatRenderController.followLatest = true;
  chatRenderController.scrollToLatest({ force: true });
  chatRenderController.scheduleFollowCorrection();
}

// 加载聊天历史(支持懒加载)
function suspendMessagePolling() {
  const count = window.messagePollingSuspendCount || 0;
  if (count === 0) window.messagePollingResumeState = window.messagePollingEnabled;
  window.messagePollingSuspendCount = count + 1;
  window.messagePollingEnabled = false;
}

function resumeMessagePolling() {
  window.messagePollingSuspendCount = Math.max(
    0,
    (window.messagePollingSuspendCount || 1) - 1,
  );
  if (window.messagePollingSuspendCount === 0) {
    window.messagePollingEnabled = window.messagePollingResumeState !== false;
  }
}

async function loadChatHistory(peerId, preserveScroll = false, sessionSnapshot = null) {
  const snapshot = sessionSnapshot || chatRenderController.snapshot(peerId);
  const isStaleRequest = () => !chatRenderController.isCurrent(snapshot);
  suspendMessagePolling();
  try {
    // 一次读取足够构成首屏的消息，再批量提交 DOM，避免边加载边闪烁。
    const pageSize = 20;
    const messages = await apiGetChatHistory(peerId, pageSize, 0);

    if (isStaleRequest()) return false;

    const chatMessages = document.getElementById("chat-messages");
    if (!chatMessages) throw new Error("找不到聊天消息容器");
    const wasFollowing = chatRenderController.followLatest ||
      chatRenderController.isAtBottom(chatMessages);
    const visibleAnchor = preserveScroll && !wasFollowing
      ? chatRenderController.captureVisibleAnchor(chatMessages)
      : null;

    // 存储当前对话的消息总数和已加载数量
    const messageState = window.currentChatMessages?.peerId === peerId
      ? window.currentChatMessages
      : {
      peerId: peerId,
      isLoading: false,
    };

    if (preserveScroll && !wasFollowing && chatMessages.querySelector(".message")) {
      for (const msg of messages) {
        upsertMessageInContainer(chatMessages, msg, msg.from_id === "me");
      }
    } else {
      const fragment = document.createDocumentFragment();
      for (const msg of messages) {
        fragment.appendChild(createMessageElement(msg, msg.from_id === "me"));
      }
      const bottomAnchor = document.createElement("div");
      bottomAnchor.id = "chat-bottom-anchor";
      bottomAnchor.className = "chat-bottom-anchor";
      bottomAnchor.setAttribute("aria-hidden", "true");
      fragment.appendChild(bottomAnchor);
      chatMessages.replaceChildren(fragment);
    }

    for (const msg of messages) {
      // 更新最后消息时间戳
      if (msg.timestamp > (window.lastMessageTimestamp || 0)) {
        window.lastMessageTimestamp = msg.timestamp;
      }
    }

    if (isStaleRequest()) return false;
    messageState.loadedCount = chatMessages.querySelectorAll(":scope > .message").length;
    messageState.totalCount = messageState.loadedCount;
    if (!(preserveScroll && !wasFollowing && typeof messageState.hasMore === "boolean")) {
      messageState.hasMore = messages.length === pageSize;
    }
    messageState.sessionId = snapshot.sessionId;
    window.currentChatMessages = messageState;
    chatRenderController.observeMessages(chatMessages);

    if (!preserveScroll || wasFollowing) {
      chatRenderController.followLatest = true;
      chatRenderController.scrollToLatest({ force: true, snapshot });
      chatRenderController.scheduleFollowCorrection(snapshot);
    } else {
      requestAnimationFrame(() => {
        if (!isStaleRequest()) {
          chatRenderController.restoreVisibleAnchor(visibleAnchor, chatMessages);
          chatRenderController.updateBottomButton();
        }
      });
    }

    // 只在首次加载时初始化滚动监听器
    if (!preserveScroll && !window.scrollListenerAttached) {
      initScrollListener();
    }

    return true;
  } finally {
    resumeMessagePolling();
  }
}

// 初始化滚动监听器(懒加载)
function initScrollListener() {
  const chatMessages = document.getElementById("chat-messages");
  if (!chatMessages) return;
  if (window.scrollListenerAttached) {
    chatMessages.removeEventListener("scroll", window.handleChatScroll);
  }

  window.handleChatScroll = async function () {
    const state = window.currentChatMessages;
    if (!state || state.isLoading || !state.hasMore) return;
    if (chatMessages.scrollHeight <= chatMessages.clientHeight || chatMessages.scrollTop >= 80) {
      return;
    }

    const snapshot = { peerId: state.peerId, sessionId: state.sessionId };
    if (!chatRenderController.isCurrent(snapshot)) return;
    state.isLoading = true;
    suspendMessagePolling();
    const visibleAnchor = chatRenderController.captureVisibleAnchor(chatMessages);

    try {
      const pageSize = 10;
      const historyOffset = chatMessages.querySelectorAll(
        ':scope > .message[data-msg-id]',
      ).length;
      const moreMessages = await apiGetChatHistory(
        state.peerId,
        pageSize,
        historyOffset,
      );
      if (!chatRenderController.isCurrent(snapshot) || window.currentChatMessages !== state) {
        return;
      }

      if (moreMessages.length === 0) {
        state.hasMore = false;
        return;
      }

      const fragment = document.createDocumentFragment();
      for (const message of moreMessages) {
        if (!chatMessages.querySelector(`[data-msg-id="${message.id}"]`)) {
          fragment.appendChild(createMessageElement(message, message.from_id === "me"));
        }
      }
      chatMessages.insertBefore(fragment, chatMessages.firstChild);
      state.loadedCount = chatMessages.querySelectorAll(
        ':scope > .message[data-msg-id]',
      ).length;
      state.totalCount = state.loadedCount;
      state.hasMore = moreMessages.length === pageSize;
      chatRenderController.observeMessages(chatMessages);

      requestAnimationFrame(() => {
        if (chatRenderController.isCurrent(snapshot)) {
          chatRenderController.restoreVisibleAnchor(visibleAnchor, chatMessages);
          chatRenderController.updateBottomButton();
        }
      });
    } catch (error) {
      console.error("[UI] 加载更多消息失败:", error);
      showMessageActionToast("旧记录加载失败，上滑可重试");
    } finally {
      if (window.currentChatMessages === state) state.isLoading = false;
      resumeMessagePolling();
    }
  };

  chatMessages.addEventListener("scroll", window.handleChatScroll);
  window.scrollListenerAttached = true;
}

// 创建消息元素
function fileTransferLabel(status, isSent) {
  const labels = {
    pending: "等待设备", offering: "等待接收", offered: "等待接收",
    uploading: "正在发送", downloading: "正在接收", saving: "正在保存",
    sent: "已发送", accepted: isSent ? "已发送" : "已完成",
    received: "已完成", completed: "已完成", invalid: "接收失败",
    save_failed: "保存失败", failed: isSent ? "发送失败" : "接收失败",
    chunk_failed: "分块错误",
    retrying: "重新发送",
  };
  return labels[status] || (isSent ? "已发送" : "已完成");
}

function showFileSendFailure(error, fileName = "") {
  const reason = getActionErrorMessage(error);
  // 分块失败已写入对应文件消息；不再叠加全局提示。
  if (/上传分块失败|分块错误/.test(reason)) return;
  showMessageActionToast(`${fileName ? fileName + " " : ""}文件发送失败：${reason}`, 4000);
}

function formatTransferBytes(bytes) {
  const value = Math.max(0, Number(bytes) || 0);
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  if (value < 1024 * 1024 * 1024) return `${(value / (1024 * 1024)).toFixed(1)} MB`;
  return `${(value / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function renderFileTransfer(msgEl, status, transferred, total, speedMbS = 0, force = false) {
  if (!msgEl?.querySelector(".message-file")) return;
  const statusEl = msgEl.querySelector(".file-transfer-status");
  const progress = msgEl.querySelector(".file-transfer-progress");
  if (!statusEl || !progress) return;
  const isSent = msgEl.classList.contains("sent");
  const size = Math.max(0, Number(total ?? progress.dataset.total) || 0);
  const incomingBytes = Math.min(size || Number.MAX_SAFE_INTEGER, Math.max(0, Number(transferred) || 0));
  const previousBytes = Number(progress.dataset.transferred || 0);
  const bytes = status === progress.dataset.status && ["uploading", "downloading"].includes(status)
    ? Math.max(previousBytes, incomingBytes) : incomingBytes;
  const percent = size ? Math.min(100, Math.floor(bytes * 100 / size)) : 0;
  const now = performance.now();
  if (!force && status === progress.dataset.status && now - Number(progress.dataset.updated || 0) < 150 && percent === Number(progress.dataset.percent || 0)) return;
  progress.dataset.updated = String(now);
  progress.dataset.percent = String(percent);
  progress.dataset.transferred = String(bytes);
  progress.dataset.status = status;
  progress.dataset.total = String(size);
  statusEl.textContent = fileTransferLabel(status, isSent);
  statusEl.className = "file-transfer-status";
  if (["failed", "chunk_failed", "save_failed", "invalid"].includes(status)) {
    statusEl.classList.add("file-transfer-error");
  }
  const active = ["uploading", "downloading", "saving", "retrying"].includes(status);
  progress.hidden = !active;
  progress.querySelector(".file-transfer-fill").style.width = `${percent}%`;
  progress.querySelector(".file-transfer-bar").setAttribute("aria-valuenow", String(percent));
  const speed = Math.max(0, Number(speedMbS) || 0);
  const detail = progress.querySelector(".file-transfer-detail");
  detail.textContent = `${percent}% · ${formatTransferBytes(bytes)} / ${formatTransferBytes(size)}` +
    (speed > 0 && status !== "saving" ? ` · ${formatTransferBytes(speed * 1024 * 1024)}/s` : "");
}

function updateFileTransferById(senderMsgId, status, transferred, total, speedMbS = 0, force = false, direction = "sent") {
  if (senderMsgId === undefined || senderMsgId === null) return;
  const chat = document.getElementById("chat-messages");
  const id = String(senderMsgId);
  if (direction === "sent") {
    if (!window.__pendingFileProgress) window.__pendingFileProgress = new Map();
    const previous = window.__pendingFileProgress.get(id);
    if (status === "uploading" && previous?.status === "uploading") {
      transferred = Math.max(Number(transferred) || 0, Number(previous.transferred) || 0);
    }
    if (["uploading", "retrying"].includes(status)) {
      window.__pendingFileProgress.set(id, { status, transferred, total, speedMbS });
    } else {
      window.__pendingFileProgress.delete(id);
    }
  }
  const msgEl = [...(chat?.querySelectorAll(".message[data-sender-msg-id], .message[data-msg-id]") || [])]
    .find(el => el.classList.contains(direction) &&
      (el.dataset.senderMsgId === id || (direction === "sent" && el.dataset.msgId === id)));
  if (!msgEl) return;
  const currentStatus = msgEl.querySelector(".file-transfer-progress")?.dataset.status;
  if (status === "uploading" && ["sent", "accepted", "failed", "chunk_failed"].includes(currentStatus)) {
    window.__pendingFileProgress?.delete(id);
    return;
  }
  renderFileTransfer(msgEl, status, transferred, total, speedMbS, force);
}

function createMessageElement(message, isSent) {
  const messageDiv = document.createElement("div");
  messageDiv.className = `message ${isSent ? "sent" : "received"}`;
  // 严谨地检查 ID 是否存在,防止绑定 "undefined"
  if (message.id !== undefined && message.id !== null) {
    messageDiv.dataset.msgId = message.id;
  }

  // 存储 sender_msg_id（用于手动下载时的 file_status_update 回查）
  if (message.sender_msg_id !== undefined && message.sender_msg_id !== null) {
    messageDiv.dataset.senderMsgId = message.sender_msg_id;
  }

  // 把当前状态存入数据集,方便轮询检测
  messageDiv.dataset.status = message.status || "sent";

  const contentDiv = document.createElement("div");
  contentDiv.className = "message-content";
  let externalFileActionBtn = null;
  let refreshAndroidFileState = null;
  let renderAndroidFileState = null;

  // ---- 构建消息主体 ----
  if (message.msg_type === "file") {
    const fileContainer = document.createElement("div");
    fileContainer.className = "message-file";

    const isImage = isImageFile(message.file_name || message.content);
    const hasLocalFile = [
      "sent",
      "accepted",
      "offering",
      "uploading",
      "retrying",
      "pending",
      "save_failed",
    ].includes(message.file_status);
    if (
      isImage && message.file_path &&
      hasLocalFile
    ) {
      const imgPreview = document.createElement("div");
      imgPreview.className = "image-preview";
      const img = document.createElement("img");

      const tauri = window.__TAURI__;
      if (tauri) {
        const isAndroid = navigator.userAgent.includes("Android");
        if (
          isAndroid && message.file_path &&
          (message.file_path.startsWith("content://") || message.file_path.startsWith("fd:"))
        ) {
          apiGetMediaToken().then((token) => {
            img.src = `http://127.0.0.1:8888/api/media?uri=${
              encodeURIComponent(message.file_path)
            }&token=${token}`;
          });
        } else {
          img.src = tauri.core.convertFileSrc(message.file_path);
        }
      } else if (message.file_id) {
        img.src = `/api/download/${message.file_id}`;
      }

      img.alt = message.file_name || message.content;
      img.loading = "lazy";
      img.onerror = () => {
        console.error(
          "[Image] 加载失败:",
          img.src,
          "file_path:",
          message.file_path,
          "file_status:",
          message.file_status,
        );
        imgPreview.innerHTML = "";
        imgPreview.appendChild(createFileIcon(message));
      };
      imgPreview.appendChild(img);
      fileContainer.appendChild(imgPreview);
    } else {
      fileContainer.appendChild(createFileIcon(message));
    }
    contentDiv.appendChild(fileContainer);

    const isAndroidFile = Boolean(
      window.__TAURI__ &&
      navigator.userAgent.includes("Android") &&
      message.file_path
    );
    let androidFileState = "UNKNOWN";
    if (isAndroidFile) {
      const saveStateDiv = document.createElement("div");
      saveStateDiv.className = "file-save-state";
      saveStateDiv.title = message.file_path;
      contentDiv.appendChild(saveStateDiv);

      renderAndroidFileState = (state) => {
        androidFileState = state;
        saveStateDiv.className = "file-save-state";
        const storedPath = formatAndroidStoredPath(
          message.file_path,
          message.file_name || message.content,
        );

        const separator = typeof state === "string" ? state.indexOf(":") : -1;
        const stateType = separator >= 0 ? state.slice(0, separator) : state;
        const stateDetail = separator >= 0 ? state.slice(separator + 1) : "";

        if (stateType === "DELETED" || message.file_status === "deleted") {
          saveStateDiv.classList.add("deleted");
          saveStateDiv.textContent = "文件已删除";
        } else if (stateType === "INACCESSIBLE") {
          saveStateDiv.classList.add("inaccessible");
          saveStateDiv.textContent = "文件不可访问 · 保存权限已失效";
        } else if (stateType === "PROVIDER_ERROR" || stateType === "ERROR") {
          saveStateDiv.classList.add("action-error");
          saveStateDiv.textContent = `文件检查失败：${stateDetail || "无法读取文件状态"}`;
        } else if (stateType === "OPEN_ERROR") {
          saveStateDiv.classList.add("action-error");
          saveStateDiv.textContent = `打开失败：${stateDetail || "未知错误"}`;
        } else if (stateType === "SHARE_ERROR") {
          saveStateDiv.classList.add("action-error");
          saveStateDiv.textContent = `分享失败：${stateDetail || "未知错误"}`;
        } else if (message.file_status === "save_failed") {
          saveStateDiv.classList.add("save-failed");
          saveStateDiv.textContent = `保存失败 · 已保留临时文件：${storedPath}`;
        } else {
          saveStateDiv.classList.add("saved");
          saveStateDiv.textContent = `${isSent ? "文件位置" : "保存到"}：${storedPath}`;
        }
      };

      refreshAndroidFileState = async () => {
        const state = await apiGetAndroidFileState(message.file_path);
        renderAndroidFileState(state);
        return state;
      };

      renderAndroidFileState(message.file_status === "deleted" ? "DELETED" : "UNKNOWN");
      refreshAndroidFileState().catch((error) => {
        console.warn("[UI] 检查 Android 文件状态失败:", error);
      });
    }

    const ensureAndroidFileAvailable = async () => {
      if (!isAndroidFile || !refreshAndroidFileState) return true;
      // 每次操作前重新检查，确保文件被用户从下载目录删除后立即更新为“已删除”。
      const state = await refreshAndroidFileState();
      if (state === "AVAILABLE" || state === "NOT_APPLICABLE") return true;
      if (state === "DELETED") showMessageActionToast("文件已删除");
      else if (state === "INACCESSIBLE") showMessageActionToast("文件访问权限已失效");
      else showMessageActionToast(
        state.includes(":") ? state.slice(state.indexOf(":") + 1) : "无法读取文件",
      );
      return false;
    };

    // 文件点击事件
    if (message.file_status === "offered" || message.file_status === "invalid") {
      fileContainer.style.cursor = "pointer";
      fileContainer.title = "点击请求对方发送文件";
      fileContainer.addEventListener("click", async () => {
        if (!window.currentChatPeer) return;
        const senderAddr = window.currentChatPeer.addr;
        const senderMsgId = message.sender_msg_id;
        const fromId = message.from_id;
        if (!senderMsgId) {
          console.error("[UI] 无法请求文件: 缺少 sender_msg_id");
          return;
        }
        renderFileTransfer(messageDiv, "downloading", 0, message.file_size, 0, true);
        console.log("[手动下载] 请求文件: msg_id=", senderMsgId, "from=", fromId, "addr=", senderAddr);
        try {
          const tauri = window.__TAURI__;
          if (tauri) {
            await tauri.core.invoke("request_file", {
              senderAddr,
              senderMsgId,
            });
          } else {
            const resp = await fetch("/api/request_file", {
              method: "POST",
              headers: { "Content-Type": "application/json" },
              body: JSON.stringify({
                sender_addr: senderAddr,
                sender_msg_id: senderMsgId,
              }),
            });
            if (!resp.ok) {
              const data = await resp.json().catch(() => ({}));
              console.error("[UI] 请求文件失败:", data.error || resp.status);
              renderFileTransfer(messageDiv, "failed", 0, message.file_size, 0, true);
            }
          }
        } catch (e) {
          console.error("[UI] 请求文件失败:", e.message);
          renderFileTransfer(messageDiv, "failed", 0, message.file_size, 0, true);
        }
      });
    } else if (hasLocalFile) {
      fileContainer.style.cursor = "pointer";
      const tauri = window.__TAURI__;
      if (tauri) {
        if (message.file_path) {
          if (navigator.userAgent.includes("Android")) {
            const openAndroidFile = async () => {
              try {
                if (!await ensureAndroidFileAvailable()) return;
                await apiOpenFileInAndroid(message.file_path);
              } catch (e) {
                const reason = getActionErrorMessage(e);
                renderAndroidFileState?.(`OPEN_ERROR:${reason}`);
              }
            };
            fileContainer.setAttribute("role", "button");
            fileContainer.tabIndex = 0;
            fileContainer.title = isImage ? "点击查看图片" : "点击选择应用打开";
            fileContainer.addEventListener("click", openAndroidFile);
            fileContainer.addEventListener("keydown", (event) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                openAndroidFile();
              }
            });
            const shareBtn = document.createElement("button");
            shareBtn.className = "file-share-btn";
            shareBtn.type = "button";
            shareBtn.title = "分享到其他应用";
            shareBtn.setAttribute("aria-label", "分享到其他应用");
            shareBtn.innerHTML =
              `<svg viewBox="0 0 24 24" width="22" height="22" stroke="currentColor" stroke-width="2.2" fill="none" stroke-linecap="round" stroke-linejoin="round"><circle cx="18" cy="5" r="3"></circle><circle cx="6" cy="12" r="3"></circle><circle cx="18" cy="19" r="3"></circle><line x1="8.59" y1="13.51" x2="15.42" y2="17.49"></line><line x1="15.41" y1="6.51" x2="8.59" y2="10.49"></line></svg>`;
            shareBtn.addEventListener("click", async (e) => {
              e.stopPropagation();
              try {
                if (!await ensureAndroidFileAvailable()) return;
                await apiShareFileToOtherApp(message.file_path);
              } catch (e) {
                const reason = getActionErrorMessage(e);
                renderAndroidFileState?.(`SHARE_ERROR:${reason}`);
              }
            });
            externalFileActionBtn = shareBtn;
          } else {
            fileContainer.addEventListener(
              "click",
              () => openFileLocation(message.file_path),
            );
          }
        }
      } else if (message.file_id) {
        fileContainer.addEventListener(
          "click",
          () =>
            downloadFile(message.file_id, message.file_name || message.content),
        );
      }
    }
  } else if (
    message.msg_type === "text" &&
    message.content &&
    message.content.startsWith("[MODEL_LIST]")
  ) {
    console.log("[UI] ✓ 渲染模型选择按钮");
    // ── 模型选择按钮 ──
    const modelContainer = document.createElement("div");
    modelContainer.className = "model-select-container";

    // 内容格式: [MODEL_LIST]\n🟢 当前模型: ...\n[{"id":...}]
    // 用换行分割，跳过 [MODEL_LIST] 行，提取当前模型行和 JSON
    let currentModelLine = "";
    let jsonStr = "";
    const nlIdx = message.content.indexOf("\n");
    const rest = nlIdx > 0 ? message.content.slice(nlIdx + 1) : "";
    // rest 格式: "🟢 当前模型: ...\n[{"id":...}]"
    // 找 rest 中第一个 "[" 出现的位置（即 JSON 数组起始）
    const bracketIdx = rest.indexOf("\n[");
    if (bracketIdx > 0) {
      currentModelLine = rest.slice(0, bracketIdx).trim();
      jsonStr = rest.slice(bracketIdx + 1).trim();
    } else {
      jsonStr = rest.trim();
    }

    if (currentModelLine) {
      const currentEl = document.createElement("div");
      currentEl.className = "model-select-current";
      currentEl.textContent = currentModelLine;
      modelContainer.appendChild(currentEl);
    }

    const titleEl = document.createElement("div");
    titleEl.className = "model-select-title";
    titleEl.textContent = "📋 Select the model to switch";
    modelContainer.appendChild(titleEl);

    // 解析 JSON
    let models = [];
    try {
      models = JSON.parse(jsonStr);
    } catch (e) {
      console.warn("[UI] 解析模型列表失败:", e);
    }

    if (models.length === 0) {
      const emptyEl = document.createElement("div");
      emptyEl.className = "model-select-empty";
      emptyEl.textContent = "⚠️ 没有可用模型";
      modelContainer.appendChild(emptyEl);
    } else {
      models.forEach((model) => {
        const btn = document.createElement("button");
        btn.className = "model-select-btn";
        btn.dataset.provider = model.provider || "";
        btn.dataset.modelId = model.id || "";

        // 显示名称
        const nameSpan = document.createElement("span");
        nameSpan.className = "model-select-btn-name";
        nameSpan.textContent = model.name || model.id || "Unknown";
        btn.appendChild(nameSpan);

        // 显示 provider
        const provSpan = document.createElement("span");
        provSpan.className = "model-select-btn-provider";
        provSpan.textContent = model.provider || "";
        btn.appendChild(provSpan);

        btn.addEventListener("click", async () => {
          if (window._switchingModel || btn.disabled) return;
          window._switchingModel = true;

          // 禁用所有模型按钮
          document.querySelectorAll(".model-select-btn").forEach((b) => {
            b.disabled = true;
          });

          const provider = btn.dataset.provider;
          const modelId = btn.dataset.modelId;
          const peerId = window.currentChatPeer ? window.currentChatPeer.id : "";
          const peerAddr = window.currentChatPeer ? window.currentChatPeer.addr : "";

          if (!peerId || !peerAddr) {
            console.warn("[UI] 无法发送模型选择: 当前聊天对象为空");
            window._switchingModel = false;
            return;
          }

          try {
            await apiSendMessage(
              peerId,
              peerAddr,
              `/model select ${provider} ${modelId}`,
            );
          } catch (e) {
            console.error("[UI] 发送模型选择失败:", e);
            window._switchingModel = false;
            document.querySelectorAll(".model-select-btn").forEach((b) => {
              b.disabled = false;
            });
          }
        });

        modelContainer.appendChild(btn);
      });
    }

    contentDiv.appendChild(modelContainer);
  } else if (message.msg_type === "text" && typeof message.content === "string") {
    // 尝试解析为多段落 JSON
    let segments = null;
    try {
      const parsed = JSON.parse(message.content);
      if (parsed && parsed.segments && Array.isArray(parsed.segments)) {
        segments = parsed.segments;
      }
    } catch (_) {}
    if (segments) {
      console.log("[UI] 多段落消息, 段落数:", segments.length);
      // 多段落消息：渲染各段落
      for (const seg of segments) {
        switch (seg.type) {
          case "thinking": {
            const block = document.createElement("div");
            block.className = "thinking-block";
            block.textContent = seg.content || "";
            contentDiv.appendChild(block);
            break;
          }
          case "text": {
            const block = document.createElement("div");
            block.className = "message-text";
            block.textContent = seg.content || "";
            contentDiv.appendChild(block);
            break;
          }
          case "tool_call": {
            const block = createToolCallBlock(seg.name || "", seg.args || "");
            contentDiv.appendChild(block);
            break;
          }
          case "tool_result": {
            const block = createToolResultBlock(seg.output || "", seg.is_error);
            contentDiv.appendChild(block);
            break;
          }
        }
      }
    } else {
      console.log("[UI] 普通文本消息:", message.content.substring(0, 80), "...");
      // 普通文本
      const textSpan = document.createElement("span");
      textSpan.className = "message-text";
      textSpan.textContent = message.content;
      contentDiv.appendChild(textSpan);
    }
  } else {
    const textSpan = document.createElement("span");
    textSpan.className = "message-text";
    textSpan.textContent = message.content;
    contentDiv.appendChild(textSpan);
  }

  // 普通文本气泡点击即复制；模型选择等交互型消息保持原有行为。
  if (
    message.msg_type === "text" &&
    typeof message.content === "string" &&
    !message.content.startsWith("[MODEL_LIST]")
  ) {
    contentDiv.classList.add("copyable-text-message");
    contentDiv.title = "点击复制文本";
    contentDiv.addEventListener("click", (event) => {
      if (window.selectMode?.active) return;
      if (event.target.closest("button, a, input, textarea")) return;
      const visibleText = contentDiv.innerText?.trim() || message.content;
      copyMessageText(visibleText);
    });
  }

  // 每个文件消息持有独立进度；历史记录只持久化终态，传输字节由实时事件补齐。
  const statusDiv = document.createElement("div");
  statusDiv.className = "file-transfer-status";
  if (message.msg_type === "file") {
    const progress = document.createElement("div");
    progress.className = "file-transfer-progress";
    progress.innerHTML = '<div class="file-transfer-bar" role="progressbar" aria-valuemin="0" aria-valuemax="100"><div class="file-transfer-fill"></div></div><div class="file-transfer-detail"></div>';
    contentDiv.append(statusDiv, progress);
  }

  // 优先级 1: 只要数据库中 status 是 pending,一律展示待上线
  if (message.status === "pending") {
    statusDiv.classList.add("file-pending");
    statusDiv.textContent = t("file_pending");
    statusDiv.dataset.fileStatus = "pending";
  } // 优先级 2: 如果不是 pending 且是文件,展示上传/下载进度
  else if (message.msg_type === "file") {
    if (message.file_status === "downloading") {
      statusDiv.classList.add("file-downloading");
      statusDiv.textContent = "0 MB/s";
    } else if (message.file_status === "uploading") {
      statusDiv.classList.add("file-uploading");
      statusDiv.textContent = "0 MB/s";
    } else if (message.file_status === "offered") {
      statusDiv.classList.add("file-pending");
      statusDiv.textContent = t("file_offered");
      statusDiv.dataset.fileStatus = "offered";
    } else if (message.file_status === "offering") {
      statusDiv.classList.add("file-pending");
      statusDiv.textContent = isSent ? t("file_offering") : t("file_offered");
      statusDiv.dataset.fileStatus = "offering";
    } else if (message.file_status === "invalid") {
      statusDiv.classList.add("file-pending");
      statusDiv.textContent = t("file_invalid");
      statusDiv.dataset.fileStatus = "invalid";
    }
    // 成功状态(sent/accepted/accepted)不再塞入任何多余的文本,保持极简
  }

  if (message.msg_type !== "file" && statusDiv.classList.length > 1) {
    contentDiv.appendChild(statusDiv);
  }

  const timeDiv = document.createElement("div");
  timeDiv.className = "message-time";
  const date = new Date(message.timestamp * 1000);
  messageDiv.dataset.timestamp = message.timestamp;
  const now = new Date();
  const sameDay = date.toDateString() === now.toDateString();
  const sameYear = date.getFullYear() === now.getFullYear();
  const locale = currentLang === "zh" ? "zh-CN" : "en-US";
  if (sameDay) {
    timeDiv.textContent = date.toLocaleTimeString([], {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    });
  } else if (sameYear) {
    timeDiv.textContent = date.toLocaleDateString(locale, {
      month: "short",
      day: "numeric",
    }) + " " + date.toLocaleTimeString([], {
      hour: "2-digit",
      minute: "2-digit",
    });
  } else {
    timeDiv.textContent = date.toLocaleDateString(locale, {
      year: "numeric",
      month: "short",
      day: "numeric",
    }) + " " + date.toLocaleTimeString([], {
      hour: "2-digit",
      minute: "2-digit",
    });
  }

  if (externalFileActionBtn) {
    const actionLayout = document.createElement("div");
    actionLayout.className = "file-action-layout";
    if (isSent) {
      actionLayout.append(externalFileActionBtn, contentDiv);
    } else {
      actionLayout.append(contentDiv, externalFileActionBtn);
    }
    messageDiv.appendChild(actionLayout);
  } else {
    messageDiv.appendChild(contentDiv);
  }
  messageDiv.appendChild(timeDiv);

  // 多选模式拦截
  if (window.selectMode && window.selectMode.active) {
    messageDiv.classList.add("selectable");
    addSelectCheckbox(messageDiv);
    if (
      message.id && window.selectMode.selectedMessages.has(parseInt(message.id))
    ) {
      messageDiv.classList.add("selected");
      const checkbox = messageDiv.querySelector(".select-checkbox");
      if (checkbox) checkbox.checked = true;
    }
  }

  if (message.msg_type === "file") {
    const initialStatus = ["failed", "chunk_failed", "retrying"].includes(message.file_status) ? message.file_status :
      (message.status === "pending" ? "pending" : (message.file_status || "accepted"));
    renderFileTransfer(messageDiv, initialStatus, 0, message.file_size, 0, true);
    const pending = isSent && window.__pendingFileProgress?.get(String(message.id));
    if (pending && ["uploading", "retrying"].includes(initialStatus)) {
      renderFileTransfer(messageDiv, pending.status, pending.transferred, pending.total,
        pending.speedMbS, true);
    }
    if (isSent && ["sent", "accepted", "failed", "chunk_failed"].includes(initialStatus)) {
      window.__pendingFileProgress?.delete(String(message.id));
    }
  }
  return messageDiv;
}

// 接收到新消息
function onReceiveMessage(message) {
  console.debug("[UI] ========== onReceiveMessage 被调用 ==========");
  console.debug("[UI] 消息内容:", JSON.stringify(message, null, 2));
  console.debug("[UI] 当前聊天对象:", window.currentChatPeer);

  // ── file_status_update：更新已存在消息的文件状态（不渲染新消息） ──
  if (message.msg_type === "file_status_update") {
    const senderMsgId = message.sender_msg_id;
    const newStatus = message.file_status;
    updateFileTransferById(senderMsgId, newStatus, 0, message.total, 0, true,
      message.direction === "received" || ["invalid", "downloading"].includes(newStatus) ? "received" : "sent");
    return;
  }

  // ── file_download_progress：直接展示发送端传来的速度 ──
  if (message.msg_type === "file_download_progress") {
    const senderMsgId = message.sender_msg_id;
    updateFileTransferById(senderMsgId, message.received >= message.total ? "saving" : "downloading",
      message.received, message.total, message.speed_mb_s, false, "received");
    return;
  }

  // 接收端回传的块内已写入字节数；原有分块响应仍负责最终确认。
  if (message.msg_type === "file_upload_progress") {
    updateFileTransferById(message.sender_msg_id, "uploading",
      message.transferred, message.total, message.speed_mb_s);
    return;
  }

  // ── start_upload：桌面端接收到对方请求发送文件，直接上传（由 Rust handler 处理，此处仅 UI 反馈） ──
  if (message.msg_type === "start_upload") {
    updateFileTransferById(message.sender_msg_id, "retrying", 0, message.file_size, 0, true);
    return;
  }

  // 仅在当前聊天窗口时处理
  if (window.currentChatPeer && window.currentChatPeer.id === message.from_id) {
    const renderSession = chatRenderController.snapshot(message.from_id);
    // —— 模型切换完成/失败时，重置按钮状态 ——
    if (window._switchingModel) {
      const text = message.content || "";
      if (text.startsWith("✅") || text.startsWith("❌")) {
        window._switchingModel = false;
        document.querySelectorAll(".model-select-btn").forEach((b) => {
          b.disabled = false;
        });
      }
    }

    // 流式消息处理
    if (message.is_streaming === true) {
      const shouldFollow = chatRenderController.followLatest;
      updateStreamMessage(message);
      if (shouldFollow) chatRenderController.scheduleFollowCorrection(renderSession);
      return;
    }
    if (message.is_streaming === false) {
      // 流式结束：给容器标记完成，不替换（容器已有所有段落块）
      const chatMessages = document.getElementById("chat-messages");
      const container = chatMessages.querySelector(`[data-stream-id="${message.stream_id}"]`);
      if (container) {
        if (message.id) {
          container.dataset.msgId = message.id;
        }
        container.dataset.status = "sent";
        delete container.dataset.streamId;
      }
      if (message.timestamp > (window.lastMessageTimestamp || 0)) {
        window.lastMessageTimestamp = message.timestamp;
      }
      if (chatRenderController.followLatest) {
        chatRenderController.scheduleFollowCorrection(renderSession);
      } else {
        // 用户不在底部时，显示红点并触发通知
        const scrollBtn = document.getElementById("scroll-to-bottom-btn");
        const unreadDot = document.getElementById("unread-dot");
        if (scrollBtn) scrollBtn.classList.add("show");
        if (unreadDot) {
          unreadDot.classList.add("show");
          updateTrayFlash();
        }
        // 触发桌面通知（流式结束帧触发一次）
        if (message.from_id && message.from_id !== window.myId) {
          // 跳过文件消息的过渡状态
          if (message.msg_type === "file" && ["downloading", "uploading", "invalid"].includes(message.file_status)) {
            // skip
          } else {
            const fromName = message.from_name || "未知用户";
            let body = message.content || "";
            try {
              const parsed = JSON.parse(body);
              if (parsed?.segments) {
                const texts = parsed.segments.filter(s => s.type === "text").map(s => s.content).join(" ");
                body = texts || "[流式回复完成]";
              }
            } catch (_) {}
            if (body.length > 100) body = body.substring(0, 100) + "...";
            showNotification(fromName, body, { from_id: message.from_id });
          }
        }
      }
      return;
    }

    if (message.id === undefined || message.id === null) {
      console.log(
        "[UI] 收到一条暂时没有 ID 的实时通知,等待轮询系统自动同步...",
      );
      return;
    }



    const scrollBtn = document.getElementById("scroll-to-bottom-btn");
    const shouldFollow = chatRenderController.followLatest;

    addMessageToChat(message, false);

    if (shouldFollow) {
        chatRenderController.scheduleFollowCorrection(renderSession);
        if (document.hidden || !document.hasFocus()) {
          showIncomingSystemNotification(message);
        }
      } else {
        const unreadDot = document.getElementById("unread-dot");
        if (scrollBtn) scrollBtn.classList.add("show");
        if (unreadDot) {
          unreadDot.classList.add("show");
          updateTrayFlash();
        }
        // 当前聊天但不在底部，也触发通知
        if (message.from_id && message.from_id !== window.myId && message.is_streaming !== false) {
          const fromName = message.from_name || "未知用户";
          let body = message.content || "";
          if (message.msg_type === "file") {
            // 跳过过渡状态（downloading/uploading），只对终端状态弹通知
            if (["downloading", "uploading", "invalid"].includes(message.file_status)) {
              // skip transitional file status notifications
            } else {
              body = `[文件] ${message.file_name || "未知文件"}`;
            }
          } else if (message.msg_type === "image") {
            body = "[图片]";
          } else if (body.length > 100) {
            body = body.substring(0, 100) + "...";
          }
          if (body) {
            showNotification(fromName, body, { from_id: message.from_id });
          }
        }
      }
  } else {
    console.log("[UI] ✗ 不匹配当前聊天对象");
    // 处理未读红点和排序
    const userLi = document.querySelector(
      `#user-list li[data-id="${message.from_id}"]`,
    );
    if (userLi) {
      userLi.classList.add("has-unread");
      sortUserList();
      updateTrayFlash();
    }
    // 触发桌面通知（非流式消息或流结束帧才通知，中间块不通知）
    if (message.from_id && message.from_id !== window.myId && !message.is_streaming) {
      const fromName = message.from_name || "未知用户";
      let body = message.content || "";
      // 流式最终帧的 content 是 segments JSON，从中提取文本
      if (message.is_streaming === false) {
        try {
          const parsed = JSON.parse(body);
          if (parsed?.segments) {
            const texts = parsed.segments.filter(s => s.type === "text").map(s => s.content).join(" ");
            body = texts || "[流式回复完成]";
          }
        } catch (_) {}
      } else if (message.msg_type === "file") {
        // 跳过过渡状态，只对终端状态弹通知
        if (["downloading", "uploading", "invalid"].includes(message.file_status)) {
          body = "";
        } else {
          body = `[文件] ${message.file_name || "未知文件"}`;
        }
      } else if (message.msg_type === "image") {
        body = "[图片]";
      }
      if (body) {
        if (body.length > 100) body = body.substring(0, 100) + "...";
        showNotification(fromName, body, { from_id: message.from_id });
      }
    }
    console.log("[UI]   - message.from_id:", message.from_id);
    console.log(
      "[UI]   - currentChatPeer.id:",
      window.currentChatPeer ? window.currentChatPeer.id : "null",
    );
  }
  console.log("[UI] ==========================================");
}

// 通过文件路径发送文件(桌面端零拷贝,直接从硬盘读取)
async function sendFileByPath(filePath, targetPeer = window.currentChatPeer) {
  const target = captureChatTarget(targetPeer);
  if (!target) return;
  const tauri = window.__TAURI__;
  if (!tauri) return;

  try {
    let actualPath = filePath;
    if (filePath.startsWith("file://")) {
      actualPath = decodeURIComponent(filePath.substring(7));
      console.log("[UI] 转换 URI 为路径:", actualPath);
    }

    await apiSendFile(
      target.peer.id,
      target.peer.addr,
      null,
      actualPath,
    );

    // 纯洁地刷新
    await refreshCapturedChat(target, { forceLatest: true });
  } catch (e) {
    showFileSendFailure(e);
    await refreshCapturedChat(target);
  }
}

// 发送文件
async function sendFile(file, targetPeer = window.currentChatPeer) {
  const target = captureChatTarget(targetPeer);
  if (!target) return;

  const tauri = window.__TAURI__;

  if (tauri) {
    if (file) {
      try {
        const arrayBuffer = await file.arrayBuffer();
        const uint8Array = new Uint8Array(arrayBuffer);
        const tempDir = await tauri.path.tempDir();
        const tempFilePath = await tauri.path.join(tempDir, file.name);
        await tauri.fs.writeFile(tempFilePath, uint8Array);

        // 调用后端命令
        await apiSendFile(
          target.peer.id,
          target.peer.addr,
          null,
          tempFilePath,
        );

        // 发送完刷新数据库渲染
        await refreshCapturedChat(target, { forceLatest: true });

        try {
          await tauri.fs.remove(tempFilePath);
        } catch (e) {}
      } catch (e) {
        showFileSendFailure(e);
      }
    } else {
      try {
        await apiSendFile(
          target.peer.id,
          target.peer.addr,
          null,
        );
        await refreshCapturedChat(target, { forceLatest: true });
      } catch (e) {
        console.error("[UI] 文件发送失败:", e);
        showFileSendFailure(e);
      }
    }
  } else {
    // Web 端
    try {
      await apiSendFile(
        target.peer.id,
        target.peer.addr,
        file,
      );

      // 完成后刷新 UI
      await refreshCapturedChat(target, { forceLatest: true });
    } catch (e) {
      console.error("[UI] ✗ 文件发送失败:", e);
      showFileSendFailure(e);
      await refreshCapturedChat(target);
    }
  }
}

// 格式化文件大小
function formatFileSize(bytes) {
  if (bytes < 1024) return bytes + " B";
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + " KB";
  if (bytes < 1024 * 1024 * 1024)
    return (bytes / (1024 * 1024)).toFixed(1) + " MB";
  return (bytes / (1024 * 1024 * 1024)).toFixed(1) + " GB";
}

// ── 工具结果折叠块 ─────────────────────────────────────────────
// 预览最多 3 行且不超过 200 字符
function createToolCallBlock(name, args) {
  const block = document.createElement("div");
  block.className = "toolcall-block";

  // 格式化参数文本
  let formatted = `🔧 ${name}`;
  if (args) {
    try {
      const parsed = JSON.parse(args);
      const entries = Object.entries(parsed);
      if (entries.length === 1) {
        const [k, v] = entries[0];
        formatted += `\n(${typeof v === "string" ? '"' + v + '"' : JSON.stringify(v)})`;
      } else if (entries.length > 1) {
        const parts = entries.map(([k, v]) => {
          const val = typeof v === "string" ? '"' + v + '"' : JSON.stringify(v);
          return `${k}: ${val}`;
        });
        formatted += `\n(${parts.join(", ")})`;
      }
    } catch {
      formatted += `\n${args}`;
    }
  }

  // 预览行（取前 3 行 / 200 字符）
  const preview = document.createElement("div");
  preview.className = "toolcall-preview";
  const lines = formatted.split("\n");
  let previewText = "";
  let lineCount = 0;
  for (const line of lines) {
    if (lineCount >= 3) break;
    const wouldBe = previewText ? previewText + "\n" + line : line;
    if (wouldBe.length > 200) {
      previewText = (previewText ? previewText + "\n" : "") + line.substring(0, 200 - previewText.length - (previewText ? 1 : 0));
      lineCount++;
      break;
    }
    previewText = wouldBe;
    lineCount++;
  }
  const hasMore = lines.length > lineCount || formatted.length > 200;
  if (hasMore) previewText += "\n...";
  preview.textContent = previewText;
  block.appendChild(preview);

  // 完整内容（默认隐藏）
  const full = document.createElement("div");
  full.className = "toolcall-full";
  full.textContent = formatted;
  block.appendChild(full);

  if (hasMore) {
    const toggle = document.createElement("span");
    toggle.className = "toolcall-toggle";
    toggle.textContent = "展开 ▼";
    toggle.addEventListener("click", () => {
      const expanded = block.classList.toggle("expanded");
      toggle.textContent = expanded ? "收起 ▲" : "展开 ▼";
      checkScrollButton();
    });
    block.appendChild(toggle);
  }
  return block;
}

function createToolResultBlock(output, isError) {
  const block = document.createElement("div");
  block.className = "toolresult-block" + (isError ? " toolresult-error" : "");
  const preview = document.createElement("div");
  preview.className = "toolresult-preview";
  const lines = output.split("\n");
  let previewText = "";
  let lineCount = 0;
  for (const line of lines) {
    if (lineCount >= 3) break;
    const wouldBe = previewText ? previewText + "\n" + line : line;
    if (wouldBe.length > 200) {
      previewText = (previewText ? previewText + "\n" : "") + line.substring(0, 200 - previewText.length - (previewText ? 1 : 0));
      lineCount++;
      break;
    }
    previewText = wouldBe;
    lineCount++;
  }
  const hasMore = lines.length > lineCount || output.length > 200;
  if (hasMore) previewText += "\n...";
  preview.textContent = previewText;
  block.appendChild(preview);
  const full = document.createElement("div");
  full.className = "toolresult-full";
  full.textContent = output;
  block.appendChild(full);
  if (hasMore) {
    const toggle = document.createElement("span");
    toggle.className = "toolresult-toggle";
    toggle.textContent = "展开 ▼";
    toggle.addEventListener("click", () => {
      const expanded = block.classList.toggle("expanded");
      toggle.textContent = expanded ? "收起 ▲" : "展开 ▼";
      checkScrollButton();
    });
    block.appendChild(toggle);
  }
  return block;
}

// ── 工具调用块创建（可折叠） ─────────────────────────────────────

// 下载文件
async function downloadFile(fileId, fileName) {
  try {
    const url = `/api/download/${fileId}`;

    // 使用 fetch 来获取下载进度
    const response = await fetch(url);
    if (!response.ok) {
      throw new Error(`HTTP ${response.status}`);
    }

    const contentLength = response.headers.get("content-length");
    const totalSize = parseInt(contentLength, 10);

    const reader = response.body.getReader();
    const chunks = [];
    let receivedLength = 0;
    const startTime = Date.now();
    let lastLogTime = startTime;

    // 更新下载速度显示
    const updateDownloadSpeed = () => {
      const elapsed = (Date.now() - startTime) / 1000;
      if (elapsed > 0) {
        const speed = receivedLength / (1024 * 1024) / elapsed;
        const chatMessages = document.getElementById("chat-messages");
        const msgEl = chatMessages?.querySelector(`[data-msg-id="${fileId}"], [data-sender-msg-id="${fileId}"]`);
        const statusDiv = msgEl?.querySelector(".file-downloading");
        if (statusDiv) {
          statusDiv.textContent = Math.round(speed) + " MB/s";
        }
      }
    };

    while (true) {
      const { done, value } = await reader.read();

      if (done) break;

      chunks.push(value);
      receivedLength += value.length;

      // 每秒更新一次速度显示
      const now = Date.now();
      if (now - lastLogTime > 1000) {
        updateDownloadSpeed();
        lastLogTime = now;
      }
    }

    // 合并所有分块
    const chunksAll = new Uint8Array(receivedLength);
    let position = 0;
    for (const chunk of chunks) {
      chunksAll.set(chunk, position);
      position += chunk.length;
    }

    // 创建 Blob 并下载
    const blob = new Blob([chunksAll]);
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = fileName;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(a.href);

    const totalTime = (Date.now() - startTime) / 1000;
    const avgSpeed = (receivedLength / (1024 * 1024)) / totalTime;
    console.log(
      "[UI] ✓ 文件下载完成:",
      fileName,
      "耗时:",
      totalTime.toFixed(2),
      "秒,平均速度:",
      avgSpeed.toFixed(2),
      "MB/s",
    );
  } catch (e) {
    console.error("[UI] 下载文件失败:", e);
    showMessageActionToast("下载失败：" + getActionErrorMessage(e), 4000);
  }
}

// 打开文件所在位置(仅桌面端)
async function openFileLocation(filePath) {
  const tauri = window.__TAURI__;

  if (!tauri) {
    showMessageActionToast("此功能仅在桌面端支持", 3000);
    return;
  }

  try {
    await tauri.core.invoke("open_file_location", { filePath: filePath });
    console.log("[UI] ✓ 打开文件位置:", filePath);
  } catch (e) {
    console.error("[UI] 打开文件位置失败:", e);
    showMessageActionToast("打开文件位置失败：" + getActionErrorMessage(e), 4000);
  }
}

// 初始化设置功能
function initSettings() {
  const settingsBtn = document.getElementById("settings-btn");
  const settingsPanel = document.getElementById("settings-panel");
  const saveSettingsBtn = document.getElementById("save-settings-btn");
  const cancelSettingsBtn = document.getElementById("cancel-settings-btn");
  const choosePathBtn = document.getElementById("choose-path-btn");
  const downloadPathInput = document.getElementById("download-path-input");
  const androidDownloadButton = document.getElementById("android-download-location-btn");
  const androidDownloadValue = document.getElementById("android-download-value");
  const portInput = document.getElementById("port-input");
  const dbPathInput = document.getElementById("db-path-input");
  const chooseDbPathBtn = document.getElementById("choose-db-path-btn");
  const dbPathSetting = document.getElementById("db-path-setting");
  const settingsErrorMsg = document.getElementById("settings-error-msg");
  const settingsSuccessMsg = document.getElementById("settings-success-msg");
  const settingsNameInput = document.getElementById("settings-device-name-input");
  const permissionsPanel = document.getElementById("permissions-panel");
  const permissionsBtn = document.getElementById("android-permissions-btn");
  const permissionsBackBtn = document.getElementById("android-permissions-back-btn");
  const savePermissionsBtn = document.getElementById("save-permissions-btn");
  const settingsBackBtn = document.getElementById("android-settings-back-btn");
  const permissionFileBtn = document.getElementById("android-permission-file-btn");
  let initialPort = "8888";
  let initialName = "";
  let initialDbPath = "";
  let initialDlPath = "";
  let initialAutoDl = true;
  let initialNotifications = true;
  let initialNotificationSound = true;
  let initialCloseToTray = true;
  let initialAutostart = false;
  let initialStartMinimized = true;
  let androidDownloadTarget = "";
  const autoDownloadToggle = document.getElementById("auto-download-toggle");
  const notificationToggle = document.getElementById("notification-toggle");
  const notificationSoundToggle = document.getElementById("notification-sound-toggle");
  const notificationHint = document.getElementById("notification-permission-hint");
  const closeToTraySetting = document.getElementById("close-to-tray-setting");
  const closeToTrayToggle = document.getElementById("close-to-tray-toggle");
  const autostartSetting = document.getElementById("autostart-setting");
  const autostartToggle = document.getElementById("autostart-toggle");
  const startMinimizedSetting = document.getElementById("start-minimized-setting");
  const startMinimizedToggle = document.getElementById("start-minimized-toggle");
  const backgroundReceiveSetting = document.getElementById("background-receive-setting");
  const backgroundReceiveStatus = document.getElementById("background-receive-status");
  const backgroundReceiveError = document.getElementById("background-receive-error");
  const backgroundStopConfirm = document.getElementById("background-stop-confirm");
  const backgroundStopOk = document.getElementById("background-stop-ok");
  const backgroundStopCancel = document.getElementById("background-stop-cancel");
  const retryBackgroundServiceBtn = document.getElementById("retry-background-service-btn");
  const batteryOptimizationStatus = document.getElementById("battery-optimization-status");
  const openBatterySettingsBtn = document.getElementById("open-battery-settings-btn");
  let stoppingBackground = false;
  const backgroundKeepRunningToggle = document.getElementById("background-keep-running-toggle");
  const backgroundStartOnBootToggle = document.getElementById("background-start-on-boot-toggle");
  const backgroundExcludeRecentsToggle = document.getElementById("background-exclude-recents-toggle");
  const batteryAlertToggle = document.getElementById("battery-alert-toggle");
  const batteryAlertControls = document.getElementById("battery-alert-controls");
  const batteryAlertIntervalInput = document.getElementById("battery-alert-interval-input");
  const batteryAlertRepeatInput = document.getElementById("battery-alert-repeat-input");
  const batteryAlertLevelList = document.getElementById("battery-alert-level-list");
  const batteryAlertAddLevelBtn = document.getElementById("battery-alert-add-level-btn");
  const batteryAlertNewLevel = document.getElementById("battery-alert-new-level");
  const batteryAlertValidation = document.getElementById("battery-alert-validation");
  let batteryAlertLevels = [50, 100];

  const renderBatteryAlertLevels = () => {
    if (!batteryAlertLevelList) return;
    batteryAlertLevelList.replaceChildren(...batteryAlertLevels.map((level) => {
      const chip = document.createElement("button");
      chip.type = "button";
      chip.className = "battery-alert-level-chip";
      chip.dataset.level = String(level);
      chip.setAttribute("aria-label", `删除 ${level}% 提醒`);
      chip.append(document.createTextNode(`${level}% `));
      const remove = document.createElement("span");
      remove.setAttribute("aria-hidden", "true");
      remove.textContent = "×";
      chip.append(remove);
      return chip;
    }));
  };
  const setBatteryAlertControlsEnabled = () => {
    if (batteryAlertControls) batteryAlertControls.disabled = !batteryAlertToggle?.checked;
  };
  const showBatteryAlertValidation = (message = "") => {
    if (batteryAlertValidation) batteryAlertValidation.textContent = message;
  };

  batteryAlertToggle?.addEventListener("change", () => {
    setBatteryAlertControlsEnabled();
    showBatteryAlertValidation();
  });
  batteryAlertLevelList?.addEventListener("click", (event) => {
    const chip = event.target.closest(".battery-alert-level-chip");
    if (!chip || !batteryAlertLevelList.contains(chip)) return;
    if (batteryAlertLevels.length <= 1) {
      showBatteryAlertValidation("至少保留一个提醒电量");
      return;
    }
    batteryAlertLevels = batteryAlertLevels.filter((level) => level !== Number(chip.dataset.level));
    showBatteryAlertValidation();
    renderBatteryAlertLevels();
  });
  batteryAlertAddLevelBtn?.addEventListener("click", () => {
    if (batteryAlertLevels.length >= 10) {
      showBatteryAlertValidation("最多添加 10 个提醒电量");
      return;
    }
    const trimmed = batteryAlertNewLevel?.value.trim() || "";
    const level = Number(trimmed);
    if (!/^\d{1,3}$/.test(trimmed) || !Number.isInteger(level) || level < 1 || level > 100) {
      showBatteryAlertValidation("提醒电量必须是 1～100 的整数");
      batteryAlertNewLevel?.focus();
      return;
    }
    if (batteryAlertLevels.includes(level)) {
      showBatteryAlertValidation(`${level}% 已存在`);
      return;
    }
    batteryAlertLevels = [...batteryAlertLevels, level].sort((a, b) => a - b);
    batteryAlertNewLevel.value = "";
    showBatteryAlertValidation();
    renderBatteryAlertLevels();
  });
  batteryAlertNewLevel?.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      batteryAlertAddLevelBtn?.click();
    }
  });

  // Android 端隐藏数据库路径配置
  const isAndroid = !!window.__TAURI__ &&
    (navigator.userAgent.includes("Android") || document.body.classList.contains("android-app"));
  const isWindowsDesktop = !!window.__TAURI__ && !isAndroid && navigator.userAgent.includes("Windows");
  let settingsSession = 0;
  let saving = false;
  let loading = false;
  const saveButtonLabels = new Map();
  const updateSaveButtons = () => {
    saveSettingsBtn.disabled = saving || loading;
    if (savePermissionsBtn) savePermissionsBtn.disabled = saving || loading;
    if (isAndroid) {
      for (const button of [saveSettingsBtn, savePermissionsBtn].filter(Boolean)) {
        button.setAttribute("aria-busy", String(saving));
        if (saving) {
          if (!saveButtonLabels.has(button)) saveButtonLabels.set(button, button.textContent);
          button.textContent = "正在保存…";
        } else if (saveButtonLabels.has(button)) {
          button.textContent = saveButtonLabels.get(button);
          saveButtonLabels.delete(button);
        }
      }
    }
  };
  const clearSettingsFeedback = () => {
    settingsErrorMsg.textContent = "";
    settingsSuccessMsg.textContent = "";
    settingsSuccessMsg.classList.remove("show");
  };
  const closeSettings = () => {
    if (isAndroid && location.hash === "#settings") history.back();
    else {
      permissionsPanel?.classList.remove("is-open");
      settingsPanel.style.display = "none";
      clearSettingsFeedback();
    }
  };
  const closePermissions = () => {
    if (isAndroid && location.hash === "#permissions") history.back();
    else permissionsPanel?.classList.remove("is-open");
  };
  if (isAndroid) {
    window.addEventListener("popstate", () => {
      const inSettings = ["#settings", "#permissions", "#push-apps", "#lq-push"].includes(location.hash);
      settingsPanel.style.display = inSettings ? "block" : "none";
      permissionsPanel?.classList.toggle("is-open", location.hash === "#permissions");
      if (!inSettings) clearSettingsFeedback();
    });
  }
  if (isAndroid && dbPathSetting) {
    dbPathSetting.style.display = "none";
  }
  if (closeToTraySetting && !isWindowsDesktop) {
    closeToTraySetting.style.display = "none";
  }
  if (autostartSetting && !isWindowsDesktop) {
    autostartSetting.style.display = "none";
  }
  if (startMinimizedSetting && !isWindowsDesktop) {
    startMinimizedSetting.style.display = "none";
  }
  if (backgroundReceiveSetting && isAndroid) {
    backgroundReceiveSetting.style.display = "block";
  }
  if (isAndroid) {
    downloadPathInput.readOnly = true;
    downloadPathInput.placeholder = "应用专属存储（默认）";
  }

  function showAndroidDownloadTarget(target, label = "") {
    androidDownloadTarget = target || "";
    downloadPathInput.title = androidDownloadTarget;
    downloadPathInput.value = androidDownloadTarget.startsWith("content://")
      ? `系统文件夹：${label || "已授权目录"}`
      : "应用专属存储（默认）";
    if (androidDownloadValue) {
      androidDownloadValue.textContent = downloadPathInput.value;
      androidDownloadValue.title = androidDownloadTarget || downloadPathInput.value;
      androidDownloadButton?.setAttribute("aria-label", `下载位置：${downloadPathInput.value}，选择下载目录`);
    }
    if (permissionFileBtn) {
      const authorized = androidDownloadTarget.startsWith("content://");
      permissionFileBtn.textContent = authorized
        ? "已授权"
        : "设置";
      permissionFileBtn.classList.toggle("is-authorized", authorized);
    }
  }

  if (isAndroid) {
    window.addEventListener("android-download-directory-selected", (event) => {
      const target = event.detail?.uri || "";
      if (!target) return;
      showAndroidDownloadTarget(target, event.detail?.label || "");
      settingsErrorMsg.textContent = "";
      settingsSuccessMsg.textContent = "已选择系统文件夹，请点击保存。";
      settingsSuccessMsg.classList.add("show");
    });
    window.addEventListener("android-download-directory-error", (event) => {
      settingsErrorMsg.textContent = "选择文件夹失败: " +
        (event.detail?.message || "系统未授予写入权限");
    });
  }

  async function refreshBackgroundReceiveState() {
    if (!isAndroid || !window.__TAURI__) return;
    try {
      const state = await window.__TAURI__.core.invoke("get_background_receive_state");
      const labels = {
        RUNNING: "● 正在运行",
        STARTING: "● 正在启动",
        STOPPING: "○ 正在停止",
        STOPPED: "○ 已停止",
        ERROR: "△ 启动失败",
      };
      backgroundReceiveStatus.textContent = labels[state.state] || state.state || "未知";
      backgroundReceiveStatus.dataset.state = state.state || "UNKNOWN";
      const canStop = ["RUNNING", "STARTING"].includes(state.state);
      backgroundReceiveStatus.disabled = stoppingBackground || !canStop;
      backgroundReceiveStatus.title = canStop ? "点击停止后台接收并退出" : "";
      backgroundReceiveStatus.setAttribute("aria-label", canStop
        ? `${backgroundReceiveStatus.textContent}，点击停止后台接收并退出`
        : backgroundReceiveStatus.textContent);
      backgroundReceiveError.textContent = state.last_error_message || "";
      retryBackgroundServiceBtn.style.display = state.state === "ERROR" ? "inline-block" : "none";
      retryBackgroundServiceBtn.parentElement.hidden = state.state !== "ERROR";
      const battery = await window.__TAURI__.core.invoke("get_battery_optimization_state");
      batteryOptimizationStatus.textContent = battery === "unrestricted" ? "不受限制" : "受系统优化限制";
      const notification = await window.__TAURI__.core.invoke("get_notification_permission_state");
      notificationHint.textContent = notification === "granted"
        ? ""
        : "系统通知权限已关闭；后台仍可运行，但新消息提醒可能不可见。";
    } catch (error) {
      backgroundReceiveError.textContent = "读取后台状态失败: " + error;
    }
  }

  if (isAndroid) {
    retryBackgroundServiceBtn?.addEventListener("click", async () => {
      await window.__TAURI__.core.invoke("retry_background_service");
      setTimeout(refreshBackgroundReceiveState, 500);
    });
    openBatterySettingsBtn?.addEventListener("click", () =>
      window.__TAURI__.core.invoke("open_battery_optimization_settings"));
    backgroundReceiveStatus?.addEventListener("click", () => {
      if (stoppingBackground || backgroundReceiveStatus.disabled) return;
      backgroundStopConfirm.hidden = false;
    });
    backgroundStopCancel?.addEventListener("click", () => {
      backgroundStopConfirm.hidden = true;
    });
    backgroundStopOk?.addEventListener("click", async () => {
      if (stoppingBackground) return;
      backgroundStopConfirm.hidden = true;
      stoppingBackground = true;
      backgroundReceiveStatus.disabled = true;
      try {
        await window.__TAURI__.core.invoke("stop_background_receive_and_exit");
      } catch (error) {
        const message = "停止失败：" + getActionErrorMessage(error);
        backgroundReceiveError.textContent = message;
        showMessageActionToast(message, 4000);
      } finally {
        stoppingBackground = false;
        backgroundReceiveStatus.disabled = !["RUNNING", "STARTING"].includes(backgroundReceiveStatus.dataset.state);
      }
    });
    window.__TAURI__.event?.listen("core-state-changed", refreshBackgroundReceiveState);
    window.addEventListener("focus", refreshBackgroundReceiveState);
  }

  // 获取默认下载路径
  async function getDefaultDownloadPath() {
    const tauri = window.__TAURI__;
    if (tauri) {
      try {
        return await tauri.core.invoke("get_default_download_path");
      } catch (_) {
        // fall through
      }
    }
    return "/tmp/LQChat";
  }

  // 打开/关闭设置面板 - 切换显示/隐藏
  settingsBtn.addEventListener("click", async () => {
    if (settingsPanel.style.display === "block") {
      closeSettings();
    } else {
      if (loading || saving) return;
      settingsSession += 1;
      loading = true;
      updateSaveButtons();
      if (isAndroid && location.hash !== "#settings") {
        history.pushState({ settingsOpen: true }, "", "#settings");
      }
      settingsPanel.style.display = "block";
      permissionsPanel?.classList.remove("is-open");
      settingsErrorMsg.textContent = "";
      settingsSuccessMsg.textContent = "";
      settingsSuccessMsg.classList.remove("show");

      try {
        if (settingsNameInput) {
          initialName = await apiGetMyName();
          settingsNameInput.value = initialName;
        }
        const settings = await apiGetSettings();
        const defaultDlPath = await getDefaultDownloadPath();

        const configuredDownloadPath = settings.download_path || defaultDlPath;
        if (isAndroid) {
          showAndroidDownloadTarget(configuredDownloadPath);
        } else {
          downloadPathInput.value = configuredDownloadPath;
        }
        portInput.value = settings.port || "8888";
        initialPort = portInput.value;
        dbPathInput.value = settings.db_path || "";
        initialDbPath = dbPathInput.value;
        initialDlPath = configuredDownloadPath;
        autoDownloadToggle.checked = settings.auto_download !== false;
        initialAutoDl = autoDownloadToggle.checked;
        if (isWindowsDesktop) {
          closeToTrayToggle.checked = settings.close_to_tray !== false;
          initialCloseToTray = closeToTrayToggle.checked;
          initialAutostart = await window.__TAURI__.core.invoke("get_autostart_enabled").catch(() => false);
          autostartToggle.checked = initialAutostart;
          startMinimizedToggle.checked = settings.start_minimized !== false;
          initialStartMinimized = startMinimizedToggle.checked;
        }
        if (window.__TAURI__) {
          if (isAndroid) {
            const background = await window.__TAURI__.core.invoke("get_background_runtime_settings");
            backgroundKeepRunningToggle.checked = !!background.keep_running;
            backgroundStartOnBootToggle.checked = !!background.start_on_boot;
            backgroundExcludeRecentsToggle.checked = !!background.exclude_from_recents;
            batteryAlertToggle.checked = !!background.battery_alert_enabled;
            batteryAlertIntervalInput.value = String(background.battery_alert_interval_seconds ?? 5);
            batteryAlertRepeatInput.value = String(background.battery_alert_repeat_count ?? 3);
            batteryAlertLevels = Array.isArray(background.battery_alert_levels) && background.battery_alert_levels.length
              ? [...new Set(background.battery_alert_levels.map(Number))]
                  .filter((level) => Number.isInteger(level) && level >= 1 && level <= 100)
                  .sort((a, b) => a - b)
              : [50, 100];
            if (!batteryAlertLevels.length) batteryAlertLevels = [50, 100];
            renderBatteryAlertLevels();
            setBatteryAlertControlsEnabled();
            showBatteryAlertValidation();
          }
          initialNotifications = await window.__TAURI__.core.invoke("get_notifications_enabled").catch(() => true);
          notificationToggle.checked = initialNotifications;
          initialNotificationSound = await window.__TAURI__.core.invoke("get_notification_sound_enabled").catch(() => true);
          notificationSoundToggle.checked = initialNotificationSound;
          if (isAndroid && typeof Notification !== "undefined" && Notification.permission === "denied") {
            notificationHint.textContent = "系统通知权限已关闭，请在系统设置中允许 LQChat 通知。";
          } else {
            notificationHint.textContent = "";
          }
        }
        await refreshBackgroundReceiveState();
        await window.NotificationUI?.refreshSettings?.();
      } catch (e) {
        settingsErrorMsg.textContent = "加载设置失败: " + e.message;
      } finally {
        loading = false;
        updateSaveButtons();
      }

    }
  });

  settingsBackBtn?.addEventListener("click", () => cancelSettingsBtn.click());
  permissionsBtn?.addEventListener("click", async () => {
    if (isAndroid && location.hash !== "#permissions") {
      history.pushState({ permissionsOpen: true }, "", "#permissions");
    }
    permissionsPanel?.classList.add("is-open");
    await refreshBackgroundReceiveState();
    window.NotificationUI?.refreshSettings?.();
  });
  permissionsBackBtn?.addEventListener("click", closePermissions);
  permissionFileBtn?.addEventListener("click", () => choosePathBtn.click());
  androidDownloadButton?.addEventListener("click", () => {
    if (isAndroid && !saving && !loading) choosePathBtn.click();
  });
  savePermissionsBtn?.addEventListener("click", () => {
    saveSettingsBtn.dataset.saveDestination = "settings";
    saveSettingsBtn.click();
  });

  // 选择下载路径
  choosePathBtn.addEventListener("click", async () => {
    const tauri = window.__TAURI__;

    if (isAndroid) {
      try {
        settingsErrorMsg.textContent = "";
        await tauri.core.invoke("request_storage_permission");
      } catch (e) {
        settingsErrorMsg.textContent = "打开系统文件夹选择器失败: " + e;
      }
    } else if (tauri) {
      try {
        const defaultPath = await getDefaultDownloadPath();
        const selected = await tauri.dialog.open({
          directory: true,
          multiple: false,
          title: "选择下载文件夹",
          defaultPath: downloadPathInput.value || defaultPath,
        });

        if (selected) {
          const path = Array.isArray(selected) ? selected[0] : selected;
          downloadPathInput.value = path;
          settingsErrorMsg.textContent = "";
        }
      } catch (e) {
        console.error("[UI] 文件选择器错误:", e);
        settingsErrorMsg.textContent = "选择路径失败: " + e.message;
      }
    } else {
      settingsErrorMsg.textContent = "请在下载位置输入框中填写路径";
      downloadPathInput.focus();
      downloadPathInput.select();
    }
  });

  // 选择数据库路径
  chooseDbPathBtn.addEventListener("click", async () => {
    const tauri = window.__TAURI__;

    if (tauri && !isAndroid) {
      try {
        const selected = await tauri.dialog.open({
          directory: true,
          multiple: false,
          title: "选择数据库文件夹",
          defaultPath: dbPathInput.value || undefined,
        });

        if (selected) {
          const path = Array.isArray(selected) ? selected[0] : selected;
          dbPathInput.value = path + "/LQChat.db";
          settingsErrorMsg.textContent = "";
        }
      } catch (e) {
        console.error("[UI] 数据库路径选择器错误:", e);
        settingsErrorMsg.textContent = "选择路径失败: " + e.message;
      }
    } else {
      settingsErrorMsg.textContent = "请在数据库路径输入框中填写路径";
      dbPathInput.focus();
      dbPathInput.select();
    }
  });

  // 保存设置
  saveSettingsBtn.addEventListener("click", async () => {
    if (saving || loading) return;
    const saveDestination = saveSettingsBtn.dataset.saveDestination || "home";
    delete saveSettingsBtn.dataset.saveDestination;
    const saveSession = settingsSession;
    const saveHash = location.hash;
    saving = true;
    updateSaveButtons();
    document.querySelector(".message-action-toast")?.remove();
    try {
      settingsErrorMsg.textContent = "";
      settingsSuccessMsg.textContent = "";
      settingsSuccessMsg.classList.remove("show");

      // 校验端口
      const portVal = portInput.value.trim();
      if (!portVal) {
        // 空值恢复默认
        portInput.value = "8888";
      } else {
        const portNum = parseInt(portVal, 10);
        if (isNaN(portNum) || portNum < 1 || portNum > 65535) {
          throw new Error(t("port_invalid"));
        }
        portInput.value = String(portNum);
      }

      const deviceName = settingsNameInput?.value.trim() || initialName;
      if (isAndroid && !deviceName) {
        settingsNameInput?.focus();
        throw new Error(t("name_empty"));
      }
      if (isAndroid && deviceName.length > 50) {
        settingsNameInput?.focus();
        throw new Error(t("name_too_long"));
      }

      // 空值恢复默认
      const dlPath = isAndroid
        ? (androidDownloadTarget || (await getDefaultDownloadPath()))
        : (downloadPathInput.value.trim() || (await getDefaultDownloadPath()));
      const myPort = portInput.value || "8888";
      const myDbPath = dbPathInput.value.trim() || "";
      const autoDl = autoDownloadToggle.checked;
      const notificationsEnabled = notificationToggle.checked;
      const notificationSoundEnabled = notificationSoundToggle.checked;
      const batteryAlertEnabled = isAndroid && batteryAlertToggle.checked;
      const batteryAlertIntervalSeconds = isAndroid ? Number(batteryAlertIntervalInput.value) : 5;
      const batteryAlertRepeatCount = isAndroid ? Number(batteryAlertRepeatInput.value) : 3;
      if (isAndroid && (!Number.isInteger(batteryAlertIntervalSeconds) || batteryAlertIntervalSeconds < 1 || batteryAlertIntervalSeconds > 60)) {
        batteryAlertIntervalInput.focus();
        throw new Error("提醒间隔必须是 1～60 秒的整数");
      }
      if (isAndroid && (!Number.isInteger(batteryAlertRepeatCount) || batteryAlertRepeatCount < 1 || batteryAlertRepeatCount > 10)) {
        batteryAlertRepeatInput.focus();
        throw new Error("提醒次数必须是 1～10 次的整数");
      }
      if (isAndroid && (!batteryAlertLevels.length || batteryAlertLevels.length > 10)) {
        throw new Error("请设置 1～10 个提醒电量");
      }
      const closeToTray = isWindowsDesktop ? closeToTrayToggle.checked : undefined;
      const autostartEnabled = isWindowsDesktop ? autostartToggle.checked : false;
      const startMinimized = isWindowsDesktop ? startMinimizedToggle.checked : undefined;
      const nameChanged = isAndroid && deviceName !== initialName;
      const backgroundSettings = isAndroid ? {
        keep_running: backgroundKeepRunningToggle.checked,
        start_on_boot: backgroundStartOnBootToggle.checked,
        exclude_from_recents: backgroundExcludeRecentsToggle.checked,
        battery_alert_enabled: batteryAlertEnabled,
        battery_alert_interval_seconds: batteryAlertIntervalSeconds,
        battery_alert_repeat_count: batteryAlertRepeatCount,
        battery_alert_levels: batteryAlertLevels,
      } : null;

      // Permission timeouts must not leave a partially written configuration.
      if ((notificationsEnabled || batteryAlertEnabled) && isAndroid) {
        const permission = await requestAndroidNotificationPermission();
        if (permission !== "granted") {
          throw new Error("系统通知权限未获允许，请授权后重试；或关闭通知和手机电量提示后保存");
        }
      }

      await apiUpdateSettings(dlPath, myPort, myDbPath, autoDl, closeToTray, startMinimized);
      if (nameChanged) {
        const updatedName = await apiUpdateMyName(deviceName);
        initialName = updatedName;
        document.getElementById("my-name").textContent = updatedName;
        document.getElementById("android-device-name").textContent = updatedName;
      }
      if (window.__TAURI__) {
        await window.__TAURI__.core.invoke("set_notifications_enabled", { enabled: notificationsEnabled });
        await window.__TAURI__.core.invoke("set_notification_sound_enabled", { enabled: notificationSoundEnabled });
        window._notificationsEnabled = notificationsEnabled;
        if (isWindowsDesktop && autostartEnabled !== initialAutostart) {
          await window.__TAURI__.core.invoke("set_autostart_enabled", { enabled: autostartEnabled });
        }
        if (isAndroid) {
          await window.__TAURI__.core.invoke("set_background_runtime_settings", {
            settings: backgroundSettings,
          });
        }
      }

      // 检测是否有实际改动
      const portChanged = myPort !== initialPort;
      const dbPathChanged = myDbPath !== initialDbPath;
      const startMinimizedChanged = isWindowsDesktop && startMinimized !== initialStartMinimized;

      const finishSave = () => {
        if (settingsSession !== saveSession || settingsPanel.style.display !== "block" ||
            (isAndroid && location.hash !== saveHash)) return;
        if (saveDestination === "settings") {
          closePermissions();
        } else {
          closeSettings();
        }
      };

      if (isAndroid) {
        initialPort = myPort;
        initialDbPath = myDbPath;
        initialDlPath = dlPath;
        initialAutoDl = autoDl;
        initialNotifications = notificationsEnabled;
        initialNotificationSound = notificationSoundEnabled;
        showMessageActionToast(portChanged || dbPathChanged
          ? "保存成功，部分设置需重启后生效" : "保存成功", 2400);
        finishSave();
        return;
      }

      if (portChanged || dbPathChanged || startMinimizedChanged) {
        settingsSuccessMsg.textContent = t("settings_save_restart");
      } else {
        settingsSuccessMsg.textContent = t("settings_saved");
      }
      settingsSuccessMsg.classList.add("show");
      setTimeout(() => {
        finishSave();
        settingsSuccessMsg.classList.remove("show");
      }, saveDestination === "settings" ? 650 : 1200);

      console.log("[UI] 设置保存成功");
    } catch (e) {
      const message = t("settings_save_fail") + ": " + getActionErrorMessage(e);
      settingsErrorMsg.textContent = message;
      if (isAndroid) showMessageActionToast(message, 4000);
    } finally {
      saving = false;
      updateSaveButtons();
    }
  });

  // 取消
  cancelSettingsBtn.addEventListener("click", closeSettings);
}

// 初始化手动添加设备功能
function initAddPeer() {
  const addBtn = document.getElementById("add-peer-btn");
  const addPanel = document.getElementById("add-peer-panel");
  const saveBtn = document.getElementById("save-peer-btn");
  const cancelBtn = document.getElementById("cancel-peer-btn");
  const errorMsg = document.getElementById("peer-error-msg");
  const successMsg = document.getElementById("peer-success-msg");
  const customPeerInput = document.getElementById("custom-peer-input");
  const addCustomPeerBtn = document.getElementById("add-custom-peer-btn");
  const customPeerList = document.getElementById("custom-peer-list");
  const androidBackBtn = document.getElementById("android-add-peer-back-btn");

  // 打开/关闭面板
  addBtn.addEventListener("click", () => {
    if (addPanel.style.display === "block") {
      addPanel.style.display = "none";
      errorMsg.textContent = "";
    } else {
      addPanel.style.display = "block";
      errorMsg.textContent = "";
      renderCustomPeers();
    }
  });

  // 保存按钮：有内容时先添加并显示提示，1.5s 后关闭；无内容直接关闭
  saveBtn.addEventListener("click", async () => {
    const val = customPeerInput.value.trim();
    if (val) {
      const validation = validateCustomPeer(val);
      if (!validation.error) {
        try {
          await apiAddCustomPeer(validation.address);
          customPeerInput.value = "";
          renderCustomPeers();
          successMsg.textContent = isIp(val) ? t("peer_added_ip") : t("peer_added_domain");
          successMsg.classList.add("show");
        } catch (_) {}
      }
      await new Promise(r => setTimeout(r, 1500));
    }
    addPanel.style.display = "none";
    errorMsg.textContent = "";
    successMsg.classList.remove("show");
    successMsg.textContent = "";
  });

  // 取消按钮
  cancelBtn.addEventListener("click", () => {
    addPanel.style.display = "none";
    errorMsg.textContent = "";
  });
  androidBackBtn?.addEventListener("click", () => cancelBtn.click());

  // 设备地址校验：返回 { address, error }，address 含端口
  function validateCustomPeer(val) {
    const ipv4Seg = "(?:25[0-5]|2[0-4]\\d|1\\d\\d|[1-9]?\\d)";
    const ipv4Pattern = new RegExp(`^${ipv4Seg}\\.${ipv4Seg}\\.${ipv4Seg}\\.${ipv4Seg}$`);
    const ipv4PortPattern = new RegExp(`^${ipv4Seg}\\.${ipv4Seg}\\.${ipv4Seg}\\.${ipv4Seg}:(?:[1-9]\\d{0,3}|[1-5]\\d{4}|6[0-4]\\d{3}|65[0-4]\\d{2}|655[0-2]\\d|6553[0-5])$`);

    const hostnamePart = "[a-zA-Z0-9]([a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?";
    const hostnamePattern = new RegExp(`^${hostnamePart}(\\.${hostnamePart})*$`);
    const portPattern = "(?:[1-9]\\d{0,3}|[1-5]\\d{4}|6[0-4]\\d{3}|65[0-4]\\d{2}|655[0-2]\\d|6553[0-5])";
    const hostnamePortPattern = new RegExp(`^${hostnamePart}(\\.${hostnamePart})*:${portPattern}$`);

    if (val.includes(":")) {
      if (ipv4PortPattern.test(val)) {
        return { address: val };
      }
      if (hostnamePortPattern.test(val)) {
        return { address: val };
      }
      return { error: t("peer_format_error") };
    } else {
      if (ipv4Pattern.test(val)) {
        const myPort = window.__TAURI__ ? "8888" : (window.location.port || "8888");
        return { address: `${val}:${myPort}` };
      }
      if (hostnamePattern.test(val)) {
        const myPort = window.__TAURI__ ? "8888" : (window.location.port || "8888");
        return { address: `${val}:${myPort}` };
      }
      return { error: t("peer_format_error2") };
    }
  }

  // 渲染自定义设备列表
  async function renderCustomPeers() {
    const peers = await apiGetCustomPeers();
    customPeerList.innerHTML = "";
    if (peers.length === 0) {
      const empty = document.createElement("div");
      empty.className = "custom-peer-item";
      empty.style.color = "var(--text-muted)";
      empty.style.fontStyle = "italic";
      empty.textContent = t("no_custom_devices");
      customPeerList.appendChild(empty);
      return;
    }
    for (const peer of peers) {
      const item = document.createElement("div");
      item.className = "custom-peer-item";
      item.innerHTML = `<span class="peer-addr">${peer}</span><button class="peer-remove-btn" data-peer="${peer}">✕</button>`;
      item.querySelector(".peer-remove-btn").addEventListener("click", async (e) => {
        const p = e.target.dataset.peer;
        try {
          await apiRemoveCustomPeer(p);
          renderCustomPeers();
          errorMsg.textContent = "";
        } catch (err) {
          errorMsg.textContent = "删除失败: " + err.message;
        }
      });
      customPeerList.appendChild(item);
    }
  }

  // 判断是否为 IP 地址（含端口）
  function isIp(val) {
    const ipv4Seg = "(?:25[0-5]|2[0-4]\\d|1\\d\\d|[1-9]?\\d)";
    const ip = new RegExp(`^${ipv4Seg}\\.${ipv4Seg}\\.${ipv4Seg}\\.${ipv4Seg}$`);
    const ipPort = new RegExp(`^${ipv4Seg}\\.${ipv4Seg}\\.${ipv4Seg}\\.${ipv4Seg}:\\d+$`);
    return ip.test(val) || ipPort.test(val);
  }

  // 添加自定义设备
  addCustomPeerBtn.addEventListener("click", async () => {
    const val = customPeerInput.value.trim();
    if (!val) return;

    const validation = validateCustomPeer(val);
    if (validation.error) {
      errorMsg.textContent = validation.error;
      return;
    }

    try {
      await apiAddCustomPeer(validation.address);
      customPeerInput.value = "";
      errorMsg.textContent = "";
      renderCustomPeers();
      // 显示添加成功提示
      successMsg.textContent = isIp(val) ? t("peer_added_ip") : t("peer_added_domain");
      successMsg.classList.add("show");
      setTimeout(() => {
        successMsg.classList.remove("show");
        successMsg.textContent = "";
      }, 2000);
    } catch (err) {
      errorMsg.textContent = "添加失败: " + err.message;
    }
  });

  customPeerInput.addEventListener("keydown", (e) => {
    if (e.key === "Enter") addCustomPeerBtn.click();
  });
}

// ─── 国际化 (i18n) ──────────────────────────────────────────────

const i18n = {
  zh: {
    language: "语言",
    theme: "主题",
    theme_default: "默认主题",
    apply: "应用",
    cancel: "取消",
    send: "发送",
    delete: "删除",
    settings: "设置",
    add: "添加",
    theme_btn: "主题",
    chat_input_placeholder: "输入消息...",
    name_placeholder: "输入新名字",
    saving: "保存中...",
    save: "保存",
    name_empty: "用户名不能为空",
    name_too_long: "用户名过长（最多50个字符）",
    name_saved: "✓ 用户名已更新",
    name_update_fail: "更新失败",
    add_peer_title: "手动添加设备",
    peer_input_placeholder: "例: 192.168.1.100 或 myhost.local:8888",
    peer_input_label: "IP / 域名:端口",
    port_label: "端口:",
    download_path_label: "下载路径:",
    db_path_label: "数据库路径:",
    choose: "选择",
    auto_download_label: "自动下载:",
    close_to_tray_label: "点击 X 时最小化到托盘:",
    autostart_label: "开机自动启动 LQChat:",
    autostart_hint: "是否随 Windows 登录启动；启动后的显示方式由下方开关控制",
    start_minimized_label: "启动时自动缩小到托盘:",
    start_minimized_hint: "同时控制开机自启和双击启动，不显示启动窗口",
    settings_save_restart: "✓ 设置保存成功，需重启生效",
    settings_saved: "✓ 设置保存成功",
    settings_save_fail: "保存失败",
    port_invalid: "端口格式无效（1-65535）",
    peer_added_ip: "✓ 已添加 IP",
    peer_added_domain: "✓ 已添加域名",
    peer_format_error: "格式错误，支持 IP (192.168.1.100:8888) 或 域名 (myhost.local:8888)",
    peer_format_error2: "格式错误，支持 IP (192.168.1.100) 或 域名 (myhost.local)",
    apply_success: "✓ 应用成功",
    theme_apply_error: "应用主题失败",
    file_pending: "待上线",
    file_offering: "待接收",
    file_offered: "未下载",
    file_invalid: "已失效",
    no_custom_devices: "暂无自定义设备",
  },
  en: {
    language: "Language",
    theme: "Theme",
    theme_default: "Default",
    apply: "Apply",
    cancel: "Cancel",
    send: "Send",
    delete: "Delete",
    settings: "Settings",
    add: "Add",
    theme_btn: "Theme",
    chat_input_placeholder: "Type a message...",
    name_placeholder: "Enter new name",
    saving: "Saving...",
    save: "Save",
    name_empty: "Name cannot be empty",
    name_too_long: "Name too long (max 50 characters)",
    name_saved: "✓ Name updated",
    name_update_fail: "Update failed",
    add_peer_title: "Add Peer",
    peer_input_placeholder: "e.g. 192.168.1.100 or myhost.local:8888",
    peer_input_label: "IP / Domain:Port",
    port_label: "Port:",
    download_path_label: "Download Path:",
    db_path_label: "Database Path:",
    choose: "Choose",
    auto_download_label: "Auto Download:",
    close_to_tray_label: "Minimize to tray when clicking X:",
    autostart_label: "Start LQChat when Windows starts:",
    autostart_hint: "Controls whether LQChat starts when you sign in to Windows",
    start_minimized_label: "Start minimized to tray:",
    start_minimized_hint: "Applies to both Windows autostart and double-click launches",
    settings_save_restart: "✓ Saved, restart to apply",
    settings_saved: "✓ Saved",
    settings_save_fail: "Save failed",
    port_invalid: "Invalid port (1-65535)",
    peer_added_ip: "✓ IP added",
    peer_added_domain: "✓ Domain added",
    peer_format_error: "Format: IP (192.168.1.100:8888) or Domain (myhost.local:8888)",
    peer_format_error2: "Format: IP (192.168.1.100) or Domain (myhost.local)",
    apply_success: "✓ Applied",
    theme_apply_error: "Apply failed",
    file_pending: "pending",
    file_offering: "offering",
    file_offered: "offered",
    file_invalid: "expired",
    no_custom_devices: "No custom devices",
  },
};

let currentLang = "zh";

// 获取系统语言
function detectSystemLang() {
  const lang = (navigator.language || navigator.userLanguage || "en").toLowerCase();
  if (lang.startsWith("zh")) return "zh";
  return "en";
}

// 加载语言设置
async function loadLanguage() {
  const tauri = getTauri();
  try {
    let saved;
    if (tauri) {
      saved = await tauri.core.invoke("get_language");
    } else {
      const resp = await fetch("/api/get_language");
      const data = await resp.json();
      saved = data.lang;
    }
    if (saved && saved !== "auto") {
      currentLang = saved;
    } else {
      currentLang = detectSystemLang();
    }
  } catch (e) {
    currentLang = detectSystemLang();
  }
}

// 保存语言设置
async function saveLanguage(lang) {
  const tauri = getTauri();
  try {
    if (tauri) {
      await tauri.core.invoke("set_language", { lang });
    } else {
      await fetch("/api/set_language", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ lang }),
      });
    }
  } catch (e) {
    console.error("[i18n] 保存语言设置失败:", e);
  }
}

// 翻译 key
function t(key) {
  const dict = i18n[currentLang] || i18n.zh;
  return dict[key] || key;
}

// 应用翻译
function applyTranslation() {
  const dict = i18n[currentLang] || i18n.zh;
  // 处理 data-i18n 属性元素
  document.querySelectorAll("[data-i18n]").forEach(el => {
    const key = el.getAttribute("data-i18n");
    if (dict[key] !== undefined) {
      if (el.tagName === "INPUT" || el.tagName === "TEXTAREA") {
        el.placeholder = dict[key];
      } else {
        el.textContent = dict[key];
      }
    }
  });
  // 处理没有 data-i18n 但需要翻译的元素
  const sendBtn = document.getElementById("send-btn");
  if (sendBtn) sendBtn.textContent = t("send");
  const settingsBtn = document.getElementById("settings-btn");
  if (settingsBtn) settingsBtn.textContent = t("settings");
  const addPeerBtn = document.getElementById("add-peer-btn");
  if (addPeerBtn) addPeerBtn.textContent = t("add");
  const themeBtn = document.getElementById("theme-btn");
  if (themeBtn) themeBtn.textContent = t("theme_btn");
  const chatInput = document.getElementById("chat-input");
  if (chatInput) chatInput.placeholder = t("chat_input_placeholder");
  const nameInput = document.getElementById("new-name-input");
  if (nameInput) nameInput.placeholder = t("name_placeholder");
  const addPeerTitle = document.querySelector("#add-peer-panel h2");
  if (addPeerTitle) addPeerTitle.textContent = t("add_peer_title");
  const peerInput = document.getElementById("custom-peer-input");
  if (peerInput) peerInput.placeholder = t("peer_input_placeholder");
}

// 同步语言 radio 选中状态
function syncLangRadio() {
  const radio = document.querySelector(`input[name="lang"][value="${currentLang}"]`);
  if (radio) {
    radio.checked = true;
    document.querySelectorAll(".lang-item").forEach(item => item.classList.remove("active"));
    document.querySelector(`.lang-item[data-lang="${currentLang}"]`)?.classList.add("active");
  }
}

// 初始化语言功能
async function initLanguage() {
  await loadLanguage();
  applyTranslation();
  syncLangRadio();

  // 语言 radio 变更只记录选中状态，不立刻生效
  document.querySelectorAll('input[name="lang"]').forEach(radio => {
    radio.addEventListener("change", (e) => {
      document.querySelectorAll(".lang-item").forEach(item => item.classList.remove("active"));
      const parent = e.target.closest(".lang-item");
      if (parent) parent.classList.add("active");
    });
  });

  // 点击整个 lang-item 也能选中 radio
  document.querySelectorAll(".lang-item").forEach(item => {
    item.addEventListener("click", (e) => {
      if (e.target.tagName !== "INPUT") {
        const radio = item.querySelector('input[type="radio"]');
        if (radio) radio.click();
      }
    });
  });
}

// 初始化主题功能
function initTheme() {
  const themeBtn = document.getElementById("theme-btn");
  const themePanel = document.getElementById("theme-panel");
  const applyThemeBtn = document.getElementById("apply-theme-btn");
  const cancelThemeBtn = document.getElementById("cancel-theme-btn");
  const themeList = document.getElementById("theme-list");
  const themeErrorMsg = document.getElementById("theme-error-msg");
  const themeSuccessMsg = document.getElementById("theme-success-msg");

  // 打开/关闭主题面板
  themeBtn.addEventListener("click", async () => {
    if (themePanel.style.display === "block") {
      themePanel.style.display = "none";
      themeErrorMsg.textContent = "";
      themeSuccessMsg.textContent = "";
      themeSuccessMsg.classList.remove("show");
    } else {
      try {
        await loadThemeList();
        themePanel.style.display = "block";
        themeErrorMsg.textContent = "";
        themeSuccessMsg.textContent = "";
        themeSuccessMsg.classList.remove("show");
      } catch (e) {
        themeErrorMsg.textContent = "加载主题列表失败: " + e.message;
        themePanel.style.display = "block";
      }
    }
  });

  // 应用主题 / 语言
  applyThemeBtn.addEventListener("click", async () => {
    const selectedTheme = document.querySelector('input[name="theme"]:checked');
    if (!selectedTheme) {
      themeErrorMsg.textContent = "请选择一个主题";
      return;
    }

    try {
      themeErrorMsg.textContent = "";
      themeSuccessMsg.textContent = "";
      themeSuccessMsg.classList.remove("show");

      const selectedLang = document.querySelector('input[name="lang"]:checked');
      const currentTheme = await apiGetCurrentTheme().catch(() => null);
      const themeChanged = selectedTheme.value !== currentTheme;
      const langChanged = selectedLang && selectedLang.value !== currentLang;

      if (!themeChanged && !langChanged) {
        // 没有任何改动，直接关闭
        themePanel.style.display = "none";
        return;
      }

      if (themeChanged) {
        await applyTheme(selectedTheme.value);
        await apiSaveCurrentTheme(selectedTheme.value);
      }

      if (langChanged && selectedLang) {
        currentLang = selectedLang.value;
        applyTranslation();
        await saveLanguage(currentLang);
        document.dispatchEvent(new CustomEvent("language-changed", { detail: { lang: currentLang } }));
      }

      themeSuccessMsg.textContent = t("apply_success");
      themeSuccessMsg.classList.add("show");

      setTimeout(() => {
        themePanel.style.display = "none";
        themeSuccessMsg.classList.remove("show");
      }, 1500);

      console.log("[UI] 应用成功:", selectedTheme.value, selectedLang?.value);
    } catch (e) {
      themeErrorMsg.textContent = t("theme_apply_error") + ": " + e.message;
      console.error("[UI] 应用失败:", e);
    }
  });

  // 取消
  cancelThemeBtn.addEventListener("click", () => {
    themePanel.style.display = "none";
    themeErrorMsg.textContent = "";
    themeSuccessMsg.textContent = "";
    themeSuccessMsg.classList.remove("show");
  });

  // 页面加载时应用保存的主题
  loadSavedTheme();
}

// 加载主题列表
async function loadThemeList() {
  const themeList = document.getElementById("theme-list");
  const themes = await apiGetThemeList();
  const currentTheme = await apiGetCurrentTheme();

  themeList.innerHTML = "";

  for (const theme of themes) {
    const themeItem = document.createElement("div");
    themeItem.className = "theme-item";

    const isSelected = theme.name === currentTheme;

    themeItem.innerHTML = `
            <input type="radio" id="theme-${theme.name}" name="theme" value="${theme.name}" ${
      isSelected ? "checked" : ""
    }>
            <label for="theme-${theme.name}">${theme.display_name}${theme.is_custom ? " (custom)" : ""}</label>
        `;

    if (isSelected) {
      themeItem.classList.add("active");
    }

    // 点击整个项目也能选中
    themeItem.addEventListener("click", (e) => {
      if (e.target.tagName !== "INPUT") {
        const radio = themeItem.querySelector('input[type="radio"]');
        radio.checked = true;

        // 更新active状态
        document.querySelectorAll(".theme-item").forEach((item) =>
          item.classList.remove("active")
        );
        themeItem.classList.add("active");
      }
    });

    // 监听radio变化
    const radio = themeItem.querySelector('input[type="radio"]');
    radio.addEventListener("change", () => {
      if (radio.checked) {
        document.querySelectorAll(".theme-item").forEach((item) =>
          item.classList.remove("active")
        );
        themeItem.classList.add("active");
      }
    });

    themeList.appendChild(themeItem);
  }

  console.log("[UI] 加载了", themes.length, "个主题,当前主题:", currentTheme);
}

// 应用主题
async function applyTheme(themeName) {
  // 移除现有的自定义主题样式
  const existingCustomStyle = document.getElementById("custom-theme-style");
  if (existingCustomStyle) {
    existingCustomStyle.remove();
  }

  // 获取默认样式表
  const defaultStylesheet = document.querySelector(
    'link[href="css/style.css"]',
  );

  if (themeName === "default") {
    const desktopStylesheet = document.getElementById("desktop-ui-stylesheet");
    if (desktopStylesheet) desktopStylesheet.disabled = false;
    // 恢复默认主题:启用默认CSS
    if (defaultStylesheet) {
      defaultStylesheet.disabled = false;
    }
    console.log("[UI] 应用默认主题");
    return;
  }

  // 获取自定义主题CSS
  const css = await apiGetThemeCss(themeName);
  const desktopStylesheet = document.getElementById("desktop-ui-stylesheet");
  if (desktopStylesheet) desktopStylesheet.disabled = true;

  // 禁用默认样式表
  if (defaultStylesheet) {
    defaultStylesheet.disabled = true;
  }

  // 创建新的style元素
  const styleElement = document.createElement("style");
  styleElement.id = "custom-theme-style";
  styleElement.textContent = css;

  // 添加到head中
  document.head.appendChild(styleElement);

  console.log("[UI] 应用自定义主题:", themeName, "(已禁用默认CSS)");
}

// 加载保存的主题
async function loadSavedTheme() {
  try {
    const currentTheme = await apiGetCurrentTheme();
    if (currentTheme && currentTheme !== "default") {
      await applyTheme(currentTheme);
      console.log("[UI] 自动加载保存的主题:", currentTheme);
    }
  } catch (e) {
    console.warn("[UI] 加载保存的主题失败:", e);
  }
}

// 初始化拖拽文件功能
function initDragAndDrop(chatContainer) {
  console.log("[UI] 初始化拖拽文件功能");

  const tauri = window.__TAURI__;

  if (tauri) {
    // 桌面端:使用 Tauri 的原生拖拽事件(可以获取文件路径)
    console.log("[UI] 使用 Tauri 原生拖拽事件");

    // 监听 Tauri 的文件拖放事件
    tauri.event.listen("tauri://drag-drop", async (event) => {
      console.log("[UI] Tauri 拖放事件:", event);

      if (!window.currentChatPeer) {
        console.log("[UI] 没有打开聊天窗口,忽略拖放");
        return;
      }

      const paths = event.payload.paths;
      if (paths && paths.length > 0) {
        console.log("[UI] 拖放的文件路径:", paths);

        // 依次发送所有文件(使用文件路径,零拷贝)
        for (const filePath of paths) {
          console.log("[UI] 发送文件:", filePath);
          await sendFileByPath(filePath);
        }
      }
    });

    // 监听拖拽悬停事件(显示视觉反馈)
    tauri.event.listen("tauri://drag-enter", () => {
      if (window.currentChatPeer) {
        chatContainer.classList.add("drag-over");
      }
    });

    tauri.event.listen("tauri://drag-leave", () => {
      chatContainer.classList.remove("drag-over");
    });

    tauri.event.listen("tauri://drag-drop", () => {
      chatContainer.classList.remove("drag-over");
    });

    // 持久监听上传进度（覆盖 file_request 手动下载场景）
    tauri.event.listen("upload_progress", (event) => {
      const p = event.payload;
      updateFileTransferById(p.sender_msg_id,
        p.transfer_status === "chunk_failed" ? "chunk_failed" : p.transfer_status === "failed" ? "failed" : "uploading",
        p.transferred, p.total, p.speed_mb_s);
    });
  } else {
    // Web 端:使用传统的 HTML5 拖拽 API(需要读取文件内容)
    console.log("[UI] 使用 HTML5 拖拽 API");

    // 防止默认的拖拽行为(打开文件)
    ["dragenter", "dragover", "dragleave", "drop"].forEach((eventName) => {
      chatContainer.addEventListener(eventName, preventDefaults, false);
      document.body.addEventListener(eventName, preventDefaults, false);
    });

    function preventDefaults(e) {
      e.preventDefault();
      e.stopPropagation();
    }

    // 拖拽进入时高亮
    ["dragenter", "dragover"].forEach((eventName) => {
      chatContainer.addEventListener(eventName, () => {
        if (window.currentChatPeer) {
          chatContainer.classList.add("drag-over");
        }
      }, false);
    });

    // 拖拽离开时取消高亮
    ["dragleave", "drop"].forEach((eventName) => {
      chatContainer.addEventListener(eventName, () => {
        chatContainer.classList.remove("drag-over");
      }, false);
    });

    // 处理文件拖放
    chatContainer.addEventListener("drop", async (e) => {
      if (!window.currentChatPeer) {
        console.log("[UI] 没有打开聊天窗口,忽略拖放");
        return;
      }

      const files = e.dataTransfer.files;

      if (files && files.length > 0) {
        console.log("[UI] 拖放了", files.length, "个文件");

        // 依次发送所有文件
        for (let i = 0; i < files.length; i++) {
          const file = files[i];
          console.log("[UI] 拖放的文件:", file.name, file.size);
          await sendFile(file);
        }
      } else {
        console.log("[UI] 没有检测到文件");
      }
    }, false);
  }
}

// 语言切换时更新所有可见消息的时间格式和文件状态文字
document.addEventListener("language-changed", () => {
  // 更新文件状态文字
  document.querySelectorAll("[data-file-status]").forEach((el) => {
    const status = el.dataset.fileStatus;
    if (status === "pending") {
      el.textContent = t("file_pending");
    } else if (status === "offered") {
      el.textContent = t("file_offered");
    } else if (status === "offering") {
      el.textContent = t("file_offering");
    } else if (status === "invalid") {
      el.textContent = t("file_invalid");
    }
  });

  // 更新时间格式
  document.querySelectorAll(".message-time").forEach((el) => {
    const msgDiv = el.closest(".message");
    if (!msgDiv) return;
    const ts = msgDiv.dataset.timestamp;
    if (!ts) return;
    const date = new Date(parseInt(ts) * 1000);
    const now = new Date();
    const sameDay = date.toDateString() === now.toDateString();
    const sameYear = date.getFullYear() === now.getFullYear();
    const locale = currentLang === "zh" ? "zh-CN" : "en-US";
    if (sameDay) {
      el.textContent = date.toLocaleTimeString([], {
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
      });
    } else if (sameYear) {
      el.textContent = date.toLocaleDateString(locale, {
        month: "short",
        day: "numeric",
      }) + " " + date.toLocaleTimeString([], {
        hour: "2-digit",
        minute: "2-digit",
      });
    } else {
      el.textContent = date.toLocaleDateString(locale, {
        year: "numeric",
        month: "short",
        day: "numeric",
      }) + " " + date.toLocaleTimeString([], {
        hour: "2-digit",
        minute: "2-digit",
      });
    }
  });
});

// 初始化粘贴文件功能
function initPasteFile() {
  console.log("[UI] 初始化粘贴文件功能");

  const tauri = window.__TAURI__;

  // 监听全局粘贴事件
  document.addEventListener("paste", async (e) => {
    // 只在聊天窗口打开时处理
    if (!window.currentChatPeer) {
      console.log("[UI] 没有打开聊天窗口,忽略粘贴");
      return;
    }

    // 桌面端:优先尝试使用 clipboard-rs 读取文件路径(零拷贝)
    if (tauri) {
      try {
        console.log("[UI] 尝试从剪贴板读取文件路径");
        const filePaths = await tauri.core.invoke("read_clipboard_files");

        if (filePaths && filePaths.length > 0) {
          console.log("[UI] 剪贴板中的文件路径:", filePaths);
          e.preventDefault(); // 阻止默认粘贴行为

          // 使用零拷贝方式发送文件
          for (const filePath of filePaths) {
            await sendFileByPath(filePath);
          }
          return;
        } else {
          console.log("[UI] 剪贴板中没有文件");
        }
      } catch (err) {
        console.log("[UI] 读取剪贴板文件路径失败,尝试使用传统方式:", err);
        // 继续使用传统方式处理
      }
    }

    // 传统方式:从 ClipboardEvent 读取文件(需要读取内容)
    const clipboardData = e.clipboardData || window.clipboardData;
    if (!clipboardData) {
      console.log("[UI] 无法访问剪贴板");
      return;
    }

    // 检查是否有文件
    const items = clipboardData.items;
    if (!items || items.length === 0) {
      console.log("[UI] 剪贴板中没有内容");
      return;
    }

    let hasFile = false;

    for (let i = 0; i < items.length; i++) {
      const item = items[i];
      console.log("[UI] 剪贴板项目类型:", item.type, item.kind);

      if (item.kind === "file") {
        hasFile = true;
        e.preventDefault(); // 阻止默认粘贴行为

        const file = item.getAsFile();
        if (file) {
          console.log("[UI] 粘贴的文件:", file.name, file.size, file.type);
          await sendFile(file);
        }
      }
    }

    if (hasFile) {
      console.log("[UI] 已处理粘贴的文件");
    }
  });

  // 添加快捷键提示
  console.log("[UI] Ctrl+V 粘贴文件功能已启用(支持零拷贝)");
}

// 初始化"回到底部"悬浮按钮
function initScrollToBottomBtn() {
  const chatMessages = document.getElementById("chat-messages");
  const inputContainer = document.querySelector(".chat-input-container");

  if (!chatMessages || !inputContainer) {
    console.warn("[UI] 无法初始化回到底部按钮:找不到必要的元素");
    return;
  }

  // 检查是否已经创建过按钮,避免重复创建
  let btn = document.getElementById("scroll-to-bottom-btn");
  if (btn) {
    console.log("[UI] 回到底部按钮已存在,跳过创建");
    return;
  }

  // 1. 动态创建按钮 DOM
  btn = document.createElement("div");
  btn.id = "scroll-to-bottom-btn";
  btn.className = "scroll-bottom-btn";
  // 注入一个向下箭头的 SVG 图标 和 未读小红点
  btn.innerHTML = `
        <svg viewBox="0 0 24 24" width="22" height="22" stroke="currentColor" stroke-width="2.5" fill="none" stroke-linecap="round" stroke-linejoin="round">
            <line x1="12" y1="5" x2="12" y2="19"></line>
            <polyline points="19 12 12 19 5 12"></polyline>
        </svg>
        <div id="unread-dot" class="unread-dot"></div>
    `;

  // 将按钮插入到 chat-messages 的平级,输入框的上方
  inputContainer.appendChild(btn);
  console.log("[UI] 回到底部按钮已创建");

  // 2. 绑定点击事件:平滑滚动到底部
  btn.addEventListener("click", () => {
    chatRenderController.followLatest = true;
    chatRenderController.scrollToLatest({ force: true, behavior: "smooth" });
    // 隐藏未读红点
    const unreadDot = document.getElementById("unread-dot");
    if (unreadDot) {
      unreadDot.classList.remove("show");
    }
    // 清除当前聊天的未读状态
    if (window.currentChatPeer) {
      const userLi = document.querySelector(`#user-list li[data-id="${window.currentChatPeer.id}"]`);
      if (userLi) {
        userLi.classList.remove("has-unread");
        updateTrayFlash();
      }
    }
  });

  // 3. 监听滚动事件,控制显示/隐藏 和 自动跟随状态
  chatMessages.addEventListener("scroll", () => {
    const wasFollowing = chatRenderController.followLatest;
    chatRenderController.handleScroll();
    if (!wasFollowing && chatRenderController.followLatest) updateTrayFlash();
  });
}

// 多选模式相关
window.selectMode = {
  active: false,
  selectedMessages: new Set(),
};

// 初始化多选模式
function initSelectMode() {
  const selectModeBtn = document.getElementById("select-mode-btn");

  // 确保按钮存在
  if (!selectModeBtn) return;

  // 点击多选按钮
  selectModeBtn.addEventListener("click", () => {
    if (window.selectMode.active) {
      exitSelectMode();
    } else {
      enterSelectMode();
    }
  });
}

// 进入多选模式
function enterSelectMode(initialMessageId = null) {
  console.log("[UI] 进入多选模式");
  window.selectMode.active = true;
  window.selectMode.selectedMessages.clear();

  // 只要是移动端(包括安卓平板)或者屏幕够小,都写入历史记录,防止物理返回键直接退出 App
  const isMobile = navigator.userAgent.includes("Android") ||
    window.innerWidth <= 768;
  if (isMobile) {
    window.history.pushState({ selectMode: true }, "", "#chat-select");
  }

  // 更新UI
  const selectModeBtn = document.getElementById("select-mode-btn");
  const sendBtn = document.getElementById("send-btn");
  const chatInput = document.getElementById("chat-input");
  const attachFileBtn = document.getElementById("attach-file-btn");

  // 切换为取消图标,并添加激活样式
  selectModeBtn.innerHTML = ICON_CANCEL_X;
  selectModeBtn.classList.add("active");

  // 发送按钮变为删除,改为红色警告色
  sendBtn.textContent = t("delete");
  sendBtn.disabled = false;
  sendBtn.style.backgroundColor = "#ff5555"; // Dracula Red
  sendBtn.style.borderColor = "#ff5555";
  sendBtn.style.color = "#fff";

  chatInput.disabled = true;
  attachFileBtn.disabled = true;

  // 禁用模型选择按钮
  document.querySelectorAll(".model-select-btn").forEach((b) => {
    b.disabled = true;
  });
  // 给所有消息添加复选框和点击事件
  const messages = document.querySelectorAll(".message");
  messages.forEach((msg) => {
    addSelectCheckbox(msg);
    msg.classList.add("selectable");

    if (initialMessageId && msg.dataset.msgId === String(initialMessageId)) {
      msg.classList.add("selected");
      const checkbox = msg.querySelector(".select-checkbox");
      if (checkbox) checkbox.checked = true;
      // 立即将初始消息加入集合
      window.selectMode.selectedMessages.add(parseInt(initialMessageId));
    }
  });
  sendBtn.disabled = window.selectMode.selectedMessages.size === 0;
}

// 退出多选模式
function exitSelectMode() {
  console.log("[UI] 退出多选模式");
  window.selectMode.active = false;
  window.selectMode.selectedMessages.clear();

  // 如果当前 URL 是 #chat-select,说明是移动端通过按钮退出的,需要后退一步恢复到 #chat
  // 如果是按返回键触发的 popstate,URL 已经变了,就不需要 back()
  if (window.location.hash === "#chat-select") {
    window.history.back();
  }

  // 恢复UI
  const selectModeBtn = document.getElementById("select-mode-btn");
  const sendBtn = document.getElementById("send-btn");
  const chatInput = document.getElementById("chat-input");
  const attachFileBtn = document.getElementById("attach-file-btn");

  // 恢复列表图标
  selectModeBtn.innerHTML = ICON_SELECT_LIST;
  selectModeBtn.classList.remove("active");

  // 恢复发送按钮
  sendBtn.textContent = t("send");
  sendBtn.style.backgroundColor = ""; // 恢复 CSS 中的默认值
  sendBtn.style.borderColor = "";
  sendBtn.style.color = "";
  updateAndroidComposerState();

  chatInput.disabled = false;
  attachFileBtn.disabled = false;

  // 恢复模型选择按钮（非切换中才启用）
  if (!window._switchingModel) {
    document.querySelectorAll(".model-select-btn").forEach((b) => {
      b.disabled = false;
    });
  }

  // 移除复选框逻辑保持不变
  const messages = document.querySelectorAll(".message");
  messages.forEach((msg) => {
    removeSelectCheckbox(msg);
    msg.classList.remove("selectable", "selected");
  });
}

// 添加复选框到消息
function addSelectCheckbox(messageElement) {
  if (messageElement.querySelector(".select-checkbox")) return;

  const checkbox = document.createElement("input");
  checkbox.type = "checkbox";
  checkbox.className = "select-checkbox";

  // 点击复选框
  checkbox.addEventListener("change", (e) => {
    e.stopPropagation();
    toggleMessageSelection(messageElement);
  });

  // 点击消息本身
  messageElement.addEventListener("click", handleMessageClick);

  messageElement.insertBefore(checkbox, messageElement.firstChild);
}

// 移除复选框
function removeSelectCheckbox(messageElement) {
  const checkbox = messageElement.querySelector(".select-checkbox");
  if (checkbox) {
    checkbox.remove();
  }
  messageElement.removeEventListener("click", handleMessageClick);
}

// 处理消息点击
function handleMessageClick(e) {
  if (!window.selectMode.active) return;

  // 如果点击的是复选框,不处理(复选框自己会处理)
  if (e.target.classList.contains("select-checkbox")) return;

  const messageElement = e.currentTarget;
  toggleMessageSelection(messageElement);
}

// 切换消息选中状态
function toggleMessageSelection(messageElement) {
  const msgId = parseInt(messageElement.dataset.msgId);

  // 如果节点缺少合法 ID,直接刷新界面纠正数据,然后退出
  if (!msgId || isNaN(msgId)) {
    console.warn("[UI] 发现没有合法 ID 的幽灵消息,强制刷新界面...");
    if (window.currentChatPeer) {
      const target = captureChatTarget();
      refreshCapturedChat(target).catch((error) => {
        console.error("[UI] 纠正消息列表失败:", error);
      });
    }
    return;
  }

  const checkbox = messageElement.querySelector(".select-checkbox");

  if (window.selectMode.selectedMessages.has(msgId)) {
    window.selectMode.selectedMessages.delete(msgId);
    messageElement.classList.remove("selected");
    if (checkbox) checkbox.checked = false;
    console.log("[UI] 取消选中消息:", msgId);
  } else {
    window.selectMode.selectedMessages.add(msgId);
    messageElement.classList.add("selected");
    if (checkbox) checkbox.checked = true;
    console.log("[UI] 选中消息:", msgId);
  }

  console.log(
    "[UI] 已选中消息:",
    Array.from(window.selectMode.selectedMessages),
  );
  document.getElementById("send-btn").disabled = window.selectMode.selectedMessages.size === 0;
}

// 删除选中的消息
async function deleteSelectedMessages() {
  const selectedIds = Array.from(window.selectMode.selectedMessages);

  if (selectedIds.length === 0) {
    showMessageActionToast("请先选择要删除的消息");
    return;
  }

  try {
    console.log("[UI] 删除消息:", selectedIds);

    // 调用API删除
    await apiDeleteMessages(selectedIds);

    // 从DOM中移除
    selectedIds.forEach((msgId) => {
      const msgElement = document.querySelector(
        `.message[data-msg-id="${msgId}"]`,
      );
      if (msgElement) {
        msgElement.remove();
      }
    });

    // 手动派发 scroll 事件,强制更新"回到底部"按钮的状态
    const chatMessages = document.getElementById("chat-messages");
    if (chatMessages) {
      chatMessages.dispatchEvent(new Event("scroll"));
    }

    // 更新已加载数量
    if (window.currentChatMessages) {
      window.currentChatMessages.loadedCount -= selectedIds.length;
      window.currentChatMessages.totalCount -= selectedIds.length;
    }

    console.log("[UI] 消息删除成功");

    // 退出多选模式
    exitSelectMode();
  } catch (e) {
    console.error("[UI] 删除消息失败:", e);
    showMessageActionToast("删除消息失败：" + getActionErrorMessage(e), 4000);
  }
}

// 长按进入多选模式(移动端)暂未使用
function initLongPressSelectMode() {
  let longPressTimer = null;
  let longPressTarget = null;

  document.addEventListener("touchstart", (e) => {
    // 只在聊天消息上触发
    const messageElement = e.target.closest(".message");
    if (!messageElement || window.selectMode.active) return;

    longPressTarget = messageElement;
    longPressTimer = setTimeout(() => {
      const msgId = parseInt(messageElement.dataset.msgId);
      if (msgId) {
        enterSelectMode(msgId);
      }
    }, 500); // 500ms 长按
  });

  document.addEventListener("touchend", () => {
    if (longPressTimer) {
      clearTimeout(longPressTimer);
      longPressTimer = null;
      longPressTarget = null;
    }
  });

  document.addEventListener("touchmove", () => {
    if (longPressTimer) {
      clearTimeout(longPressTimer);
      longPressTimer = null;
      longPressTarget = null;
    }
  });
}

// ==========================================
// 1. 完全独立的二次确认弹窗工具函数
// ==========================================
function showConfirm(message, onOk) {
  const overlay = document.createElement("div");
  overlay.className = "confirm-dialog-overlay";
  overlay.setAttribute("role", "presentation");

  const dialog = document.createElement("div");
  dialog.className = "confirm-dialog-content";
  dialog.setAttribute("role", "alertdialog");
  dialog.setAttribute("aria-modal", "true");

  const messageElement = document.createElement("p");
  messageElement.textContent = message;
  const buttonGroup = document.createElement("div");
  buttonGroup.className = "confirm-button-group";
  const okButton = document.createElement("button");
  okButton.className = "confirm-btn-ok";
  okButton.type = "button";
  okButton.textContent = "确定";
  const cancelButton = document.createElement("button");
  cancelButton.className = "confirm-btn-cancel";
  cancelButton.type = "button";
  cancelButton.textContent = "取消";
  buttonGroup.append(okButton, cancelButton);
  dialog.append(messageElement, buttonGroup);
  overlay.appendChild(dialog);

  document.body.appendChild(overlay);
  const closeConfirm = () => {
    overlay.remove();
    if (!document.getElementById("user-mgmt-panel") && window.userListSortPending) {
      sortUserList(true);
    }
  };

  // 取消:直接移除 DOM
  cancelButton.onclick = closeConfirm;
  overlay.addEventListener("keydown", (event) => {
    if (event.key === "Escape") cancelButton.click();
  });
  cancelButton.focus();

  // 确定:拦截处理状态,并执行传入的 onOk 回调
  okButton.onclick = async () => {
    okButton.disabled = true;
    cancelButton.disabled = true;
    okButton.textContent = "处理中...";
    try {
      await onOk(); // 这里才真正执行删除动作
    } catch (e) {
      console.error("执行失败:", e);
      showMessageActionToast("操作失败：" + getActionErrorMessage(e), 4000);
    } finally {
      closeConfirm(); // 无论成功失败,都关闭确认弹窗
    }
  };
}

// ==========================================
// 2. 完整的用户管理弹窗函数
// ==========================================
async function showUserActionDialog(peerId, userName) {
  // 1. 判断是否离线(通过检查左侧列表中是否有 offline 灰显类名)
  const userLi = document.querySelector(`#user-list li[data-id="${peerId}"]`);
  const isOffline = userLi ? userLi.classList.contains("offline") : true;

  // 2. 检测是否有聊天记录
  let history;
  try {
    history = await apiGetChatHistory(peerId, 1, 0);
  } catch (error) {
    console.error("[UI] 读取用户聊天记录失败:", error);
    showMessageActionToast("聊天记录读取失败，请稍后重试");
    return;
  }
  const hasHistory = history && history.length > 0;

  // 如果是在线用户,且连聊天记录都没有,那完全没有任何可管理的操作,直接忽略长按/右键
  if (!isOffline && !hasHistory) {
    console.log(`[UI] 在线用户 "${userName}" 无聊天记录,无需弹出管理菜单`);
    return;
  }

  const old = document.getElementById("user-mgmt-panel");
  if (old) old.remove();

  // 绘制管理弹窗
  const panel = document.createElement("div");
  panel.id = "user-mgmt-panel";
  panel.className = "edit-panel";
  panel.style.display = "block";

  // 动态生成按钮的 HTML
  let buttonsHtml = "";

  // 只有"离线用户"才允许显示删除按钮
  if (isOffline) {
    buttonsHtml +=
      `<button id="mgmt-del-btn" class="mgmt-action-btn btn-grad-danger">删除用户</button>`;
  }

  // 只要有历史记录,就允许清空
  if (hasHistory) {
    buttonsHtml +=
      `<button id="mgmt-clear-btn" class="mgmt-action-btn btn-grad-warning">清空聊天记录</button>`;
  }

  buttonsHtml +=
    `<button id="mgmt-cancel-btn" class="btn-cancel-custom">取消</button>`;

  const title = document.createElement("h2");
  title.textContent = `管理 ${userName}`;
  const actions = document.createElement("div");
  actions.className = "user-mgmt-actions";
  actions.innerHTML = buttonsHtml;
  panel.append(title, actions);

  document.body.appendChild(panel);

  const closeMgmt = () => {
    panel.remove();
    if (window.userListSortPending) sortUserList(true);
  };
  document.getElementById("mgmt-cancel-btn").onclick = closeMgmt;

  // --- 绑定删除逻辑 (注意加判空,因为在线用户没有这个按钮) ---
  const delBtn = document.getElementById("mgmt-del-btn");
  if (delBtn) {
    delBtn.onclick = () => {
      showConfirm(
        `确定要彻底删除离线用户 "${userName}" 吗?此操作不可恢复。`,
        async () => {
          if (window.currentChatPeer && window.currentChatPeer.id === peerId) {
            performCloseChatUI();
          }

          await apiDeleteUserComplete(peerId);

          const el = document.querySelector(
            `#user-list li[data-id="${peerId}"]`,
          );
          if (el) el.remove();

          sortUserList();

          closeMgmt();
        },
      );
    };
  }

  // --- 绑定清空记录逻辑---
  const clearBtn = document.getElementById("mgmt-clear-btn");
  if (clearBtn) {
    clearBtn.onclick = () => {
      showConfirm(`确定要清空与 "${userName}" 的所有聊天记录吗?`, async () => {
        await apiClearChatHistory(peerId);

        if (window.currentChatPeer && window.currentChatPeer.id === peerId) {
          const box = document.getElementById("chat-messages");
          const session = chatRenderController.begin(peerId);
          if (box) {
            box.replaceChildren();
            chatRenderController.ensureBottomAnchor(box);
          }
          window.currentChatMessages = {
            peerId,
            sessionId: session.sessionId,
            loadedCount: 0,
            totalCount: 0,
            hasMore: false,
            isLoading: false,
          };
          window.lastMessageTimestamp = 0;
          // 清空后隐藏滚动按钮
          const btn = document.getElementById("scroll-to-bottom-btn");
          if (btn) btn.classList.remove("show");
        }
        closeMgmt();
      });
    };
  }
}
