/* Notification views intentionally never set currentChatPeer or read chat history. */
window.NotificationUI = (() => {
  let enabled = false,
    android = false,
    localId = "",
    peers = [],
    records = [],
    current = null;
  let config = {
    push_enabled: false,
    receive_enabled: false,
    lq_battery_push_enabled: false,
    lq_reminder_push_enabled: false,
    allowed_packages: [],
    target_device_ids: [],
  };
  let detailSignature = "";
  let info = {},
    dialog,
    pushDetailsDialog,
    appDialog,
    pushSourcesPanel,
    receiveDialog,
    panel,
    welcome,
    pushList,
    receiveList,
    refreshTimer,
    busy = false;
  let appOptions = [],
    appSelection = new Set(),
    appLoading = false,
    appCategory = "all";
  let accessRefresh = null;
  let pendingAccessAuthorization = false;

  function renderNotificationAccess() {
    const toggle = document.getElementById("android-notification-access-toggle");
    if (toggle) {
      toggle.checked = !!info.access;
      toggle.disabled = false;
    }
  }

  function refreshNotificationAccess() {
    if (!enabled || !android) return;
    if (accessRefresh) return accessRefresh;
    // Resume refresh must not replace settings controls or their unsaved values.
    accessRefresh = (async () => {
      try {
        const latest = await invoke("notification_settings");
        const wasGranted = !!info.access;
        info.access = latest.access;
        renderNotificationAccess();
        if (pendingAccessAuthorization || wasGranted !== !!info.access) {
          pendingAccessAuthorization = false;
          if (wasGranted !== !!info.access) {
            await save({ ...config, push_enabled: !!info.access, lq_battery_push_enabled: !!info.access }, pushDetailsDialog);
            renderSettings();
          }
        }
      } catch (error) {
        console.warn("[NotificationSync] 权限状态刷新失败", getErrorMessage(error));
      } finally {
        accessRefresh = null;
      }
    })();
    return accessRefresh;
  }
  const statusText = {
    sending: "处理中",
    success: "成功",
    failure: "失败",
    timeout: "超时",
    offline: "离线已丢弃",
    busy: "繁忙已丢弃",
  };
  const el = (tag, cls, text) => {
    const n = document.createElement(tag);
    if (cls) n.className = cls;
    if (text !== undefined) n.textContent = text;
    return n;
  };
  const button = (text, action, cls = "ns-button") => {
    const b = el("button", cls, text);
    b.type = "button";
    b.addEventListener("click", action);
    return b;
  };
  const invoke = (command, args = {}) =>
    window.__TAURI__.core.invoke(command, args);
  const peerFor = (id) => peers.find((p) => p.id === id);
  const nameFor = (id) =>
    peerFor(id)?.name || records.find((r) => r.peer_id === id)?.peer_name || id;
  const pushSourcePeers = () =>
    peers.filter(
      (peer) =>
        peer.notification_push_enabled === true &&
        Array.isArray(peer.notification_push_target_device_ids) &&
        peer.notification_push_target_device_ids.includes(localId),
    );
  const isNarrow = () => android || innerWidth < 960;
  function group(title, sending) {
    const box = el("section", "ns-group");
    const head = el("div", "ns-group-heading");
    const disclosure = button(
      title,
      () => {
        const expanded = disclosure.getAttribute("aria-expanded") === "true";
        disclosure.setAttribute("aria-expanded", String(!expanded));
        list.hidden = expanded;
      },
      "ns-disclosure",
    );
    disclosure.setAttribute("aria-expanded", "true");
    head.append(disclosure, button("设置", openSettings, "ns-text-button"));
    const list = el("div", "ns-device-list");
    list.dataset.kind = sending ? "notification_push" : "notification_receive";
    box.append(head, list);
    return { box, list };
  }
  function renderPushSourcesPage() {
    if (!android || !pushSourcesPanel) return;
    const list = pushSourcesPanel.querySelector("#android-push-sources-list");
    const empty = pushSourcesPanel.querySelector("#android-push-sources-empty");
    const sources = pushSourcePeers();
    list.replaceChildren();
    empty.hidden = sources.length > 0;
    for (const peer of sources) {
      const row = button(
        "",
        () => {
          closePushSources();
          open(peer.id, "notification_receive");
        },
        "ns-device",
      );
      row.dataset.peer = peer.id;
      const copy = el("span", "ns-device-copy");
      copy.append(
        el("strong", "ns-name", peer.name || peer.id),
        el(
          "small",
          "ns-address",
          `${peer.addr || "当前地址未知"} · ${peer.is_offline ? "离线" : "在线"}`,
        ),
      );
      row.append(copy, el("span", "ns-badge", "›"));
      row.classList.toggle("online", !peer.is_offline);
      list.append(row);
    }
  }
  function openPushSources() {
    if (!android || !pushSourcesPanel) return;
    renderPushSourcesPage();
    pushSourcesPanel.style.display = "block";
    if (location.hash !== "#push-sources")
      history.pushState({ pushSources: true }, "", "#push-sources");
  }
  function closePushSources() {
    if (!pushSourcesPanel) return;
    pushSourcesPanel.style.display = "none";
  }
  function renderDevices() {
    if (!enabled) return;
    const fill = (list, ids, kind) => {
      const wanted = new Set(ids);
      for (const row of [...list.children])
        if (!wanted.has(row.dataset.peer)) row.remove();
      if (android && kind === "notification_receive") {
        const count = document.getElementById("android-receive-count");
        if (count) count.textContent = `信息接收 ${ids.length} 台设备`;
      }
      if (!ids.length) {
        if (android) {
          list.replaceChildren();
          return;
        }
        if (!list.firstChild)
          list.append(
            el(
              "p",
              "ns-empty-small",
              kind === "notification_push"
                ? "在设置中选择推送设备"
                : config.receive_enabled
                  ? "等待其他设备推送通知"
                  : "尚未开启信息接收",
            ),
          );
        else
          list.firstChild.textContent = config.receive_enabled
            ? "等待其他设备推送通知"
            : "尚未开启信息接收";
        return;
      }
      for (const id of ids) {
        let row = [...list.children].find((n) => n.dataset.peer === id);
        if (!row) {
          row = button("", () => open(id, kind), "ns-device");
          row.dataset.peer = id;
          const copy = el("span", "ns-device-copy");
          copy.append(el("strong", "ns-name"), el("small", "ns-address"));
          row.append(copy, el("span", "ns-badge"));
          list.append(row);
        }
        const peer = peerFor(id),
          name = nameFor(id),
          online = peer && !peer.is_offline;
        const label = android
          ? name
          : `${name}（${kind === "notification_push" ? "信息推送" : "信息接收"}）`;
        row.querySelector(".ns-name").textContent = label;
        row.title = label;
        row.querySelector(".ns-address").textContent =
          `${peer?.addr || "当前地址未知"} · ${online ? "在线" : "离线"}`;
        const badge = row.querySelector(".ns-badge");
        badge.textContent = android
          ? "›"
          : kind === "notification_push"
            ? `${config.push_enabled ? "已开启" : "已暂停"} · ${online ? "在线" : "离线"}`
            : online
              ? "在线"
              : "离线";
        badge.classList.toggle("online", !!online);
        row.classList.toggle("online", !!online);
        row.classList.toggle(
          "selected",
          current?.id === id && current?.kind === kind,
        );
        row.setAttribute(
          "aria-pressed",
          String(current?.id === id && current?.kind === kind),
        );
      }
    };
    fill(
      receiveList,
      android
        ? pushSourcePeers().map((peer) => peer.id)
        : [...new Set(records.filter((r) => r.view_kind === "notification_receive").map((r) => r.peer_id))],
      "notification_receive",
    );
    renderPushSourcesPage();
    if (current) renderDetail();
    if (welcome) {
      welcome.querySelector(".ns-welcome-hint").textContent =
        config.receive_enabled
          ? "在手机的信息推送设置中勾选本机，即可在这里接收通知。"
          : "开启信息接收，让手机上的重要通知出现在电脑上。";
    }
  }
  function leave() {
    if (!enabled) return;
    current = null;
    panel.hidden = true;
    document.body.classList.remove("notification-open");
    renderDevices();
  }
  function open(id, kind) {
    if (!enabled) return;
    if (window.currentChatPeer) performCloseChatUI();
    current = { id, kind };
    panel.hidden = false;
    document.body.classList.add("notification-open");
    if (isNarrow() && location.hash !== "#notifications")
      history.pushState({ notifications: true }, "", "#notifications");
    detailSignature = "";
    panel.querySelector(".ns-cards").replaceChildren();
    panel.querySelector(".ns-cards").scrollTop = 0;
    renderDevices();
  }
  function close() {
    if (location.hash === "#notifications") history.back();
    else leave();
  }
  function renderDetail() {
    if (!current) return;
    const p = peerFor(current.id),
      push = current.kind === "notification_push";
    panel.querySelector(".ns-detail-title").textContent =
      `${nameFor(current.id)}（${push ? "信息推送" : "信息接收"}）`;
    panel.querySelector(".ns-detail-status").textContent =
      `${p && !p.is_offline ? "设备在线" : "设备离线，不会补收离线通知"} · ${push ? (config.push_enabled ? "推送已开启" : "推送已暂停") : config.receive_enabled ? "接收已开启" : "接收已关闭"}`;
    const list = panel.querySelector(".ns-cards"),
      oldTop = list.scrollTop;
    const entries = records.filter(
      (r) => r.peer_id === current.id && r.view_kind === current.kind,
    );
    const signature = JSON.stringify([current, entries]);
    if (signature === detailSignature) return;
    detailSignature = signature;
    const anchor = [...list.children].find(
      (n) =>
        n.getBoundingClientRect().bottom >= list.getBoundingClientRect().top,
    );
    const anchorTop = anchor?.getBoundingClientRect().top;
    const keys = new Set();
    let added = false;
    for (const entry of entries) {
      const n = entry.notification,
        key = entry.record_id || (push
          ? n.event_id
          : JSON.stringify([n.source_device_id, n.package, n.notification_key]));
      keys.add(key);
      let card = [...list.children].find((c) => c.dataset.key === key);
      if (!card) {
        card = el("article", "ns-card");
        card.dataset.key = key;
        if (entry.record_id) card.dataset.recordId = entry.record_id;
        const meta = el("div", "ns-app-rail");
        const appIcon = el("span", "ns-app-icon");
        const image = el("img");
        image.alt = "";
        image.hidden = true;
        image.addEventListener("error", () => {
          image.hidden = true;
          appIcon.querySelector(".ns-icon-fallback").hidden = false;
        });
        image.addEventListener("load", () => {
          image.hidden = false;
          appIcon.querySelector(".ns-icon-fallback").hidden = true;
        });
        appIcon.append(image, el("span", "ns-icon-fallback"));
        meta.append(appIcon, el("strong", "ns-app"), el("time", "ns-time"));
        const more = button(
          "展开正文",
          () => {
            const expanded = card.classList.toggle("expanded");
            more.textContent = expanded ? "收起正文" : "展开正文";
            more.setAttribute("aria-expanded", String(expanded));
          },
          "ns-text-button ns-more",
        );
        more.setAttribute("aria-expanded", "false");
        const copy = el("div", "ns-notification-copy");
        const box = el("div", "ns-content-box");
        box.append(el("p", "ns-body"), more, el("small", "ns-result"));
        copy.append(el("h3", "ns-title"), box);
        card.append(meta, copy);
        added = true;
      }
      card.querySelector(".ns-app").textContent = n.app_name || n.package;
      const appIcon = card.querySelector(".ns-app-icon");
      const image = appIcon.querySelector("img");
      const fallback = appIcon.querySelector(".ns-icon-fallback");
      fallback.textContent = Array.from(n.app_name || n.package || "应用")[0];
      const encoded = typeof n.app_icon === "string" && n.app_icon.length <= 10924 && /^iVBOR[A-Za-z0-9+/]*={0,2}$/.test(n.app_icon) ? n.app_icon : "";
      if (image.dataset.icon !== encoded) {
        image.dataset.icon = encoded;
        image.hidden = true;
        fallback.hidden = false;
        if (encoded) image.src = `data:image/png;base64,${encoded}`;
        else image.removeAttribute("src");
      }
      const date = new Date(n.post_time);
      card.querySelector(".ns-time").textContent = Number.isFinite(
        date.getTime(),
      )
        ? date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false })
        : "";
      card.querySelector(".ns-time").title = Number.isFinite(date.getTime()) ? date.toLocaleString() : "";
      card.querySelector(".ns-title").textContent = n.title;
      card.querySelector(".ns-body").textContent = n.text;
      // Check rendered overflow after insertion; short multi-line text can also be clipped.
      card.querySelector(".ns-result").textContent = push
        ? statusText[entry.status] || "失败"
        : "";
      list.append(card);
      const body = card.querySelector(".ns-body");
      card.querySelector(".ns-more").hidden = !card.classList.contains("expanded") && body.scrollHeight <= body.clientHeight + 1;
    }
    for (const child of [...list.children])
      if (!keys.has(child.dataset.key)) child.remove();
    if (!entries.length)
      list.append(
        el(
          "div",
          "ns-empty",
          push
            ? "暂无推送记录。仅推送已允许 App 的新通知。"
            : config.receive_enabled
              ? "尚未收到通知。请在手机推送设置中勾选本机。"
              : "接收已关闭。可在接收设置中开启。",
        ),
      );
    list.scrollTop = oldTop;
    if (anchor?.isConnected && oldTop > 30)
      list.scrollTop += anchor.getBoundingClientRect().top - anchorTop;
    const newer = panel.querySelector(".ns-new");
    if (added && oldTop > 30) newer.hidden = false;
  }
  async function refreshRecords() {
    if (!enabled) return;
    try {
      const page = await invoke("notification_records", { query: { page: 1, page_size: 100 } });
      records = Array.isArray(page) ? page : page.records || [];
      renderDevices();
    } catch (_) {
      /* Keep the current view when the bridge is temporarily unavailable. */
    }
  }
  async function handleSyncedActivation(detail) {
    const source = detail?.sourceDeviceId;
    const recordId = detail?.recordId;
    if (!source) return;
    try { await invoke("notification_pending_activation"); } catch (_) {}
    await refreshRecords();
    if (recordId && !records.some((item) => item.record_id === recordId)) {
      try {
        const exact = await invoke("notification_record", { recordId });
        if (exact) records.unshift(exact);
      } catch (_) {}
    }
    open(source, "notification_receive");
    requestAnimationFrame(() => {
      const card = recordId
        ? panel.querySelector(`[data-record-id="${CSS.escape(recordId)}"]`)
        : null;
      if (card) {
        card.tabIndex = -1;
        card.scrollIntoView({ block: "center" });
        card.focus({ preventScroll: true });
      }
    });
  }
  function refreshSoon() {
    clearTimeout(refreshTimer);
    refreshTimer = setTimeout(refreshRecords, 60);
  }
  function checkRow(text, checked, action) {
    const row = el("label", "ns-check-row"),
      label = el("span", "", text),
      input = el("input");
    input.type = "checkbox";
    input.checked = checked;
    input.addEventListener("change", () => action(input));
    row.append(label, input);
    return row;
  }
  function toggleRow(text, checked, action) {
    const row = el("div", "ns-check-row ns-toggle-row"),
      label = el("span", "", text),
      control = el("label", "toggle-switch"),
      input = el("input"),
      slider = el("span", "toggle-slider");
    input.type = "checkbox";
    input.checked = checked;
    input.setAttribute("aria-label", text);
    input.addEventListener("change", () => action(input));
    control.append(input, slider);
    row.append(label, control);
    row.addEventListener("click", (event) => {
      if (!control.contains(event.target) && !input.disabled) input.click();
    });
    return row;
  }
  async function save(next, surface = dialog) {
    if (busy) return false;
    busy = true;
    surface.querySelectorAll("input").forEach((n) => (n.disabled = true));
    appDialog
      ?.querySelectorAll("input[type=checkbox]")
      .forEach((n) => (n.disabled = true));
    const hint = surface.querySelector(".ns-save-status");
    hint.textContent = "正在保存…";
    try {
      info = await invoke("notification_settings", { settings: next });
      config = info.settings;
      hint.textContent = android ? "" : "已保存";
      if (appDialog?.open)
        appDialog.querySelector(".ns-save-status").textContent = android ? "" : "已保存";
      renderDevices();
      if (android) window.showMessageActionToast?.("设置完成✅", 1000);
      return true;
    } catch (e) {
      hint.textContent = `保存失败：${getErrorMessage(e)}`;
      if (appDialog?.open)
        appDialog.querySelector(".ns-save-status").textContent =
          hint.textContent;
      if (android) window.showMessageActionToast?.("设置失败❌", 1000);
      return false;
    } finally {
      busy = false;
      surface.querySelectorAll("input").forEach((n) => (n.disabled = false));
      appDialog
        ?.querySelectorAll("input[type=checkbox]")
        .forEach((n) => (n.disabled = false));
    }
  }
  function updateSet(field, value, checked) {
    const values = new Set(config[field]);
    checked ? values.add(value) : values.delete(value);
    return { ...config, [field]: [...values] };
  }
  async function systemAction(action, surface = dialog) {
    try {
      await invoke("notification_action", { action });
    } catch (e) {
      const message = getErrorMessage(e);
      surface.querySelector(".ns-save-status").textContent = message;
      if (android && action === "access") {
        pendingAccessAuthorization = false;
        window.showMessageActionToast?.("设置失败❌", 1000);
      }
    }
  }
  function filteredApps() {
    const query = appDialog.querySelector("#android-push-apps-search").value.trim().toLocaleLowerCase();
    return appOptions.filter((app) => {
      const categoryMatches = appCategory === "all" ||
        (appCategory === "system" ? app.is_system : !app.is_system);
      const text = `${app.name || ""}\n${app.package || ""}`.toLocaleLowerCase();
      return categoryMatches && text.includes(query);
    });
  }
  function updateAppSelectionControls() {
    const visible = filteredApps();
    const count = visible.filter((app) => appSelection.has(app.package)).length;
    const all = appDialog.querySelector("#android-push-apps-select-all");
    all.checked = visible.length > 0 && count === visible.length;
    all.indeterminate = count > 0 && count < visible.length;
    all.disabled = appLoading || busy || !visible.length;
    appDialog.querySelector("#android-push-apps-search").disabled = appLoading || busy;
    appDialog.querySelector("#android-push-apps-category").disabled = appLoading || busy;
    appDialog.querySelector("#android-push-apps-save-btn").disabled = appLoading || busy;
    appDialog.querySelector("#android-push-apps-select-all-label").textContent =
      appDialog.querySelector("#android-push-apps-search").value.trim() ? "全选搜索结果" : "全选";
    appDialog.querySelector("#android-push-apps-count").textContent = `已选 ${appSelection.size} 个`;
  }
  function renderAppPicker() {
    if (!appDialog) return;
    const list = appDialog.querySelector("#android-push-apps-list");
    const empty = appDialog.querySelector("#android-push-apps-empty");
    list.replaceChildren();
    const visible = filteredApps();
    updateAppSelectionControls();
    empty.hidden = visible.length > 0;
    if (!visible.length) {
      empty.textContent = appOptions.length ? "当前分类下没有匹配的应用" : "未发现可选择的应用";
      return;
    }
    const fragment = document.createDocumentFragment();
    for (const app of visible) {
      const row = el("label", "android-push-app-row");
      const icon = el("span", "android-push-app-icon");
      if (app.icon) {
        const image = document.createElement("img");
        image.alt = "";
        image.src = `data:image/png;base64,${app.icon}`;
        image.loading = "lazy";
        image.decoding = "async";
        icon.append(image);
      } else {
        icon.textContent = Array.from(app.name || app.package || "应用")[0];
      }
      const identity = el("span", "android-push-app-identity");
      const name = el("span", "android-push-app-name", app.name || app.package);
      const details = el("span", "android-push-app-package", app.package);
      if (app.is_system) details.append(el("span", "android-push-app-system-tag", "系统"));
      identity.append(name, details);
      const input = document.createElement("input");
      input.type = "checkbox";
      input.checked = appSelection.has(app.package);
      input.setAttribute("aria-label", `选择 ${app.name || app.package}`);
      input.addEventListener("change", () => {
        input.checked
          ? appSelection.add(app.package)
          : appSelection.delete(app.package);
        updateAppSelectionControls();
      });
      row.append(icon, identity, input);
      fragment.append(row);
    }
    list.append(fragment);
  }
  async function chooseApps() {
    if (!appDialog || busy || appDialog.classList.contains("is-open")) return;
    appLoading = true;
    appOptions = [];
    appSelection = new Set(config.allowed_packages);
    appDialog.querySelector("#android-push-apps-search").value = "";
    appCategory = "all";
    appDialog.querySelector("#android-push-apps-category").value = appCategory;
    updateAppSelectionControls();
    appDialog.classList.add("is-open");
    appDialog.scrollTop = 0;
    appDialog.querySelector("#android-push-apps-list").replaceChildren();
    const empty = appDialog.querySelector("#android-push-apps-empty");
    empty.hidden = false;
    empty.textContent = "正在读取应用…";
    appDialog.querySelector("#android-push-apps-status").textContent = "";
    if (location.hash !== "#push-apps")
      history.pushState({ pushApps: true }, "", "#push-apps");
    try {
      await window.AndroidAppLoading.show("push-apps");
      const result = await invoke("notification_action", { action: "apps" });
      appOptions = result.apps || [];
      appSelection = new Set(config.allowed_packages);
      appLoading = false;
      renderAppPicker();
    } catch (e) {
      empty.hidden = false;
      empty.textContent = getErrorMessage(e);
    } finally {
      appLoading = false;
      window.AndroidAppLoading.hide("push-apps");
    }
  }
  function closeAppPicker() {
    window.AndroidAppLoading?.hide("push-apps");
    appDialog?.classList.remove("is-open");
  }
  async function saveAppPicker() {
    if (!appDialog || busy || appLoading) return;
    const saveButton = appDialog.querySelector("#android-push-apps-save-btn");
    const status = appDialog.querySelector("#android-push-apps-status");
    busy = true;
    updateAppSelectionControls();
    appDialog.querySelectorAll(".android-push-app-row input").forEach((input) => { input.disabled = true; });
    saveButton.disabled = true;
    document.querySelector(".message-action-toast")?.remove();
    status.textContent = "正在保存…";
    try {
      const result = await invoke("notification_action", {
        action: "replace_allowed",
        payload: { packages: [...appSelection] },
      });
      info = { ...info, ...result };
      config = result.settings || config;
      renderSettings();
      status.textContent = "";
      showMessageActionToast("设置完成✅", 1000);
      if (appDialog.classList.contains("is-open")) {
        closeAppPicker();
        if (location.hash === "#push-apps") history.back();
      }
    } catch (e) {
      status.textContent = `保存失败：${getErrorMessage(e)}`;
      showMessageActionToast("设置失败❌", 1000);
    } finally {
      busy = false;
      updateAppSelectionControls();
      appDialog.querySelectorAll(".android-push-app-row input").forEach((input) => { input.disabled = false; });
    }
  }
  function renderReceiveSettings(surface, showHeading = true) {
    let content = surface.querySelector(".ns-settings-content");
    if (!content) {
      content = el("div", "ns-settings-content");
      const status = el("p", "ns-save-status");
      status.setAttribute("role", "status");
      surface.append(content, status);
    }
    content.replaceChildren();
    if (showHeading) content.append(el("h3", "", "信息接收"));
    content.append(
      (android ? toggleRow : checkRow)(
        "允许接收其他设备推送",
        config.receive_enabled,
        async (input) => {
          if (!(await save({ ...config, receive_enabled: input.checked }, surface)))
            input.checked = !input.checked;
        },
      ),
      button(
        `系统通知设置${info.permission === "blocked" ? " · 已受阻" : ""}`,
        () => systemAction("permission", surface),
      ),
      el(
        "p",
        "ns-hint",
        "双方需先手动启动 LQChat。在发送端的信息推送设置中勾选本机；目标离线时直接丢弃，不补发。",
      ),
      el(
        "p",
        "ns-hint",
        "沿用局域网信任模型，请仅允许自有可信设备推送。通知查看记录保留七天，不用于重试或补发。",
      ),
    );
  }
  function renderPushDetails() {
    if (!pushDetailsDialog) return;
    const content = pushDetailsDialog.querySelector(".ns-settings-content");
    content.replaceChildren();
      const choices = el("div", "ns-target-options");
      const ids = peers
        .filter((peer) => peer.id !== localId && !peer.is_offline && peer.notification_receive_enabled === true)
        .map((peer) => peer.id);
      for (const id of ids) {
        const p = peerFor(id);
        const row = toggleRow(
          "",
          config.target_device_ids.includes(id),
          async (input) => {
            if (
              !(await save(updateSet("target_device_ids", id, input.checked), pushDetailsDialog))
            )
              input.checked = !input.checked;
          },
        );
        const copy = row.firstElementChild;
        row.querySelector("input").setAttribute("aria-label", `向 ${nameFor(id)} 推送通知`);
        copy.className = "ns-target-copy";
        copy.append(
          el("strong", "", nameFor(id)),
          el("small", "", `${p?.addr || "地址未知"} · ${p && !p.is_offline ? "在线" : "离线"}`),
        );
        choices.append(row);
      }
      if (!ids.length)
        choices.append(
          el("p", "ns-hint", "暂无已开启“允许接收其他设备推送”的在线设备。"),
        );
      const appPushRow = toggleRow("应用信息推送", config.push_enabled, async (input) => {
          if (!(await save({ ...config, push_enabled: input.checked }, pushDetailsDialog)))
            input.checked = !input.checked;
        });
      const appPickerEntry = button(`应用信息推送（${config.allowed_packages.length}）`, (event) => {
        event.stopPropagation();
        void chooseApps();
      }, "ns-app-picker-entry");
      appPushRow.firstElementChild.replaceWith(appPickerEntry);
      content.append(
        appPushRow,
        toggleRow("自定义通知推送", config.lq_reminder_push_enabled, async (input) => {
          if (!(await save({ ...config, lq_reminder_push_enabled: input.checked }, pushDetailsDialog)))
            input.checked = !input.checked;
        }),
        choices,
        toggleRow("电量推送", config.lq_battery_push_enabled, async (input) => {
          if (!(await save({ ...config, lq_battery_push_enabled: input.checked }, pushDetailsDialog)))
            input.checked = !input.checked;
        }),
        button("发送测试通知", async (event) => {
          const b = event.currentTarget;
          const hint = pushDetailsDialog.querySelector(".ns-save-status");
          const availableTargets = ids.filter((id) => config.target_device_ids.includes(id));
          if (!config.push_enabled) {
            hint.textContent = "请先启用信息推送";
            return;
          }
          if (!availableTargets.length) {
            hint.textContent = "请至少选择一台已开启接收的在线设备";
            return;
          }
          b.disabled = true;
          hint.textContent = "正在发送测试通知…";
          try {
            await invoke("notification_test");
            hint.textContent = `测试通知已发送到 ${availableTargets.length} 台设备`;
            await refreshRecords();
          } catch (e) {
            hint.textContent = getErrorMessage(e);
          } finally {
            b.disabled = false;
          }
        }, "ns-test-button"),
      );
  }
  function openPushDetails() {
    if (!pushDetailsDialog || busy) return;
    renderPushDetails();
    pushDetailsDialog.classList.add("is-open");
    pushDetailsDialog.scrollTop = 0;
    pushDetailsDialog.querySelector(".ns-save-status").textContent = "";
    if (location.hash !== "#push-details")
      history.pushState({ pushDetails: true }, "", "#push-details");
  }
  function closePushDetails() {
    pushDetailsDialog?.classList.remove("is-open");
  }
  function renderSettings() {
    const content = dialog.querySelector(".ns-settings-content");
    content.replaceChildren();
    if (android) {
      if (pushDetailsDialog?.classList.contains("is-open")) renderPushDetails();
      renderNotificationAccess();
      const receiveToggle = document.getElementById("android-receive-toggle");
      if (receiveToggle) receiveToggle.checked = config.receive_enabled;
      return;
    }
    renderReceiveSettings(dialog);
  }
  async function openReceiveSettings() {
    if (!enabled || !android || !receiveDialog) return;
    receiveDialog.style.display = "block";
    if (location.hash !== "#receive-settings")
      history.pushState({ receiveSettings: true }, "", "#receive-settings");
    try {
      info = await invoke("notification_settings");
      config = info.settings;
      renderReceiveSettings(receiveDialog, false);
      renderDevices();
      receiveDialog.querySelector(".ns-save-status").textContent = "";
    } catch (e) {
      receiveDialog.querySelector(".ns-save-status").textContent =
        `读取设置失败：${getErrorMessage(e)}`;
    }
  }
  function closeReceiveSettings() {
    if (receiveDialog) receiveDialog.style.display = "none";
  }
  async function openSettings() {
    if (!enabled) return;
    try {
      info = await invoke("notification_settings");
      config = info.settings;
      renderSettings();
      renderDevices();
      dialog.querySelector(".ns-save-status").textContent = "";
      if (android) {
        const settingsPanel = document.getElementById("settings-panel");
        if (settingsPanel?.style.display !== "block")
          document.getElementById("settings-btn")?.click();
      } else if (!dialog.open) {
        dialog.showModal();
      }
    } catch (e) {
      dialog.querySelector(".ns-save-status").textContent =
        `读取设置失败：${getErrorMessage(e)}`;
      if (!android && !dialog.open) dialog.showModal();
    }
  }
  async function init(options) {
    if (!window.__TAURI__ || enabled) return;
    try {
      info = await invoke("notification_settings");
    } catch (error) {
      console.warn("[NotificationSync] 初始化未完成", getErrorMessage(error));
      if (!document.getElementById("notification-init-retry")) {
        const retry = button("通知功能尚未就绪，点击重试", () => init(options));
        retry.id = "notification-init-retry";
        document.querySelector(".user-list-container")?.append(retry);
      }
      return;
    }
    document.getElementById("notification-init-retry")?.remove();
    if (!["android", "windows"].includes(info.platform)) return;
    enabled = true;
    android = info.platform === "android";
    config = info.settings;
    localId = await apiGetMyId();
    document.body.classList.add("notification-enabled");
    if (!android) document.body.classList.add("windows-app");
    const main = document.querySelector(".main-content"),
      sidebar = document.querySelector(".user-list-container"),
      chatList = document.getElementById("user-list");
    if (android) {
      receiveList = document.getElementById("android-receive-list");
      welcome = null;
    } else {
      const chatGroup = el("details", "ns-chat-group");
      chatGroup.open = true;
      chatGroup.append(el("summary", "", "局域网聊天设备"));
      chatGroup.append(button("清除", () => window.clearOfflinePeerDevices?.(), "ns-clear-offline-peers"));
      sidebar.insertBefore(chatGroup, document.getElementById("android-listening"));
      chatGroup.append(chatList);
      const incoming = group("信息接收设备", false);
      receiveList = incoming.list;
      sidebar.insertBefore(
        incoming.box,
        document.getElementById("android-listening"),
      );
      welcome = el("section", "ns-welcome");
      welcome.append(
        el("span", "ns-welcome-mark", "L"),
        el("p", "ns-eyebrow", "LQ CHAT · 局域网互联"),
        el("h2", "", "手机上的消息，在电脑上接收"),
        el("p", "ns-welcome-hint"),
        button("设置信息接收", openSettings),
      );
      main.append(welcome);
    }
    panel = el("section", "ns-detail");
    panel.hidden = true;
    const head = el("header", "ns-detail-header"),
      titles = el("div");
    titles.append(el("h2", "ns-detail-title"), el("p", "ns-detail-status"));
    head.append(button("‹ 返回", close, "ns-text-button"), titles);
    if (!android) {
      head.append(button("设置", openSettings));
    }
    panel.append(
      head,
      el("div", "ns-cards"),
      button(
        "有新通知",
        () => {
          panel.querySelector(".ns-cards").scrollTop = 0;
          panel.querySelector(".ns-new").hidden = true;
        },
        "ns-new ns-button",
      ),
      el(
        "footer",
        "ns-footer",
        "通知查看记录保留七天 · 此视图不能发送聊天或文件",
      ),
    );
    panel.querySelector(".ns-new").hidden = true;
    main.append(panel);
    new ResizeObserver(() => {
      panel.querySelectorAll(".ns-card").forEach(card => {
        const body = card.querySelector(".ns-body");
        card.querySelector(".ns-more").hidden = !card.classList.contains("expanded") && body.scrollHeight <= body.clientHeight + 1;
      });
    }).observe(panel.querySelector(".ns-cards"));
    if (android) {
      dialog = document.getElementById("android-notification-settings");
      pushDetailsDialog = document.getElementById("android-push-details-panel");
      appDialog = document.getElementById("android-push-apps-panel");
      pushSourcesPanel = document.getElementById("android-push-sources-panel");
      receiveDialog = document.getElementById("android-receive-settings-panel");
      dialog.replaceChildren(
        el("div", "ns-settings-content"),
        el("p", "ns-save-status"),
      );
      dialog.querySelector(".ns-save-status").setAttribute("role", "status");
      document
        .getElementById("android-receive-toggle")
        ?.addEventListener("change", async (event) => {
          const input = event.currentTarget;
          if (!(await save({ ...config, receive_enabled: input.checked })))
            input.checked = !input.checked;
        });
      document
        .getElementById("android-notification-access-toggle")
        ?.addEventListener("change", (event) => {
          event.currentTarget.checked = !!info.access;
          pendingAccessAuthorization = true;
          void systemAction("access");
        });
      const pushEntry = document.getElementById("android-push-settings-btn");
      pushEntry?.addEventListener("click", openPushDetails);
      pushEntry?.closest(".android-permission-row")?.addEventListener("click", (event) => {
        if (pushEntry.contains(event.target) || event.target.closest(".toggle-switch")) return;
        pushEntry.click();
      });
      document
        .getElementById("android-push-details-back-btn")
        ?.addEventListener("click", () => {
          if (location.hash === "#push-details") history.back();
          else closePushDetails();
        });
      document
        .getElementById("android-push-apps-back-btn")
        ?.addEventListener("click", () => {
          if (location.hash === "#push-apps") history.back();
          else closeAppPicker();
        });
      document
        .getElementById("android-push-apps-save-btn")
        ?.addEventListener("click", saveAppPicker);
      document.getElementById("android-push-apps-search")
        ?.addEventListener("input", renderAppPicker);
      document.getElementById("android-push-apps-category")
        ?.addEventListener("change", (event) => {
          appCategory = event.target.value;
          renderAppPicker();
        });
      document.getElementById("android-push-apps-select-all")
        ?.addEventListener("change", (event) => {
          for (const app of filteredApps()) {
            event.target.checked ? appSelection.add(app.package) : appSelection.delete(app.package);
          }
          renderAppPicker();
        });
      document
        .getElementById("android-push-sources-back-btn")
        ?.addEventListener("click", () => {
          if (location.hash === "#push-sources") history.back();
          else closePushSources();
        });
      document
        .getElementById("android-receive-settings-back-btn")
        ?.addEventListener("click", () => {
          if (location.hash === "#receive-settings") history.back();
          else closeReceiveSettings();
        });
      renderSettings();
    } else {
      dialog = el("dialog", "ns-dialog");
      const dialogHead = el("header", "ns-dialog-header");
      dialogHead.append(
        el("h2", "", "信息接收设置"),
        button("关闭", () => dialog.close(), "ns-text-button"),
      );
      dialog.append(
        dialogHead,
        el("div", "ns-settings-content"),
        el("p", "ns-save-status"),
      );
      dialog.querySelector(".ns-save-status").setAttribute("role", "status");
      document.body.append(dialog);
      const receiveSettingsButton = button("信息接收设置", openSettings);
      receiveSettingsButton.classList.add("desktop-receive-settings");
      const status = document.querySelector("#settings-panel > .settings-content > .settings-auto-status");
      if (status) status.before(receiveSettingsButton);
      else document.querySelector("#settings-panel .settings-content")?.append(receiveSettingsButton);
      if (!document.querySelector("#settings-panel .ns-button"))
        document
          .getElementById("settings-panel")
          ?.append(button("信息接收设置", openSettings));
    }
    window.addEventListener("popstate", () => {
      const inSettings = android && ["#settings", "#device-settings", "#push-details", "#push-apps", "#notification-options", "#background-options", "#download-options"].includes(location.hash);
      const inReceiveSettings = android && location.hash === "#receive-settings";
      if (!inSettings && !inReceiveSettings && location.hash !== "#notifications") leave();
      if (location.hash !== "#push-apps") closeAppPicker();
      if (!["#push-details", "#push-apps"].includes(location.hash)) closePushDetails();
      if (location.hash !== "#push-sources") closePushSources();
      if (!inReceiveSettings) closeReceiveSettings();
    });
    window.addEventListener("focus", () => {
      refreshSoon();
      if (android) refreshNotificationAccess();
      else if (dialog.open && !busy) openSettings();
    });
    document.addEventListener("visibilitychange", () => {
      if (document.visibilityState === "visible") refreshNotificationAccess();
    });
    await apiListen("notification-records-changed", refreshSoon);
    window.addEventListener("synced-notification-tapped", (event) =>
      handleSyncedActivation(event.detail),
    );
    await apiListen("synced-notification-tapped", (event) =>
      handleSyncedActivation(event.payload),
    );
    try {
      const pendingActivation = await invoke("notification_pending_activation");
      if (pendingActivation) await handleSyncedActivation(pendingActivation);
    } catch (_) {}
    await apiListen("core-state-changed", (event) => {
      refreshSoon();
    });
    peers = ((await apiGetPeers()) || []).filter((p) => p.id !== localId);
    await refreshRecords();
    renderDevices();
  }
  function onPeers(value) {
    if (!enabled) return;
    const previousTargets = peers.filter((peer) => !peer.is_offline && peer.notification_receive_enabled === true)
      .map((peer) => `${peer.id}:${peer.name}:${peer.addr}`).sort().join("|");
    peers = value.filter((p) => p.id !== localId);
    const nextTargets = peers.filter((peer) => !peer.is_offline && peer.notification_receive_enabled === true)
      .map((peer) => `${peer.id}:${peer.name}:${peer.addr}`).sort().join("|");
    if (android && dialog && !busy && previousTargets !== nextTargets) renderSettings();
    renderDevices();
  }
  async function refreshSettings() {
    if (!enabled) return;
    info = await invoke("notification_settings");
    config = info.settings;
    renderSettings();
    renderDevices();
  }
  async function setReceiveEnabled(enabled) {
    if (!android || !dialog) return false;
    if (config.receive_enabled === enabled) return true;
    const saved = await save({ ...config, receive_enabled: enabled }, dialog);
    const toggle = document.getElementById("android-receive-toggle");
    if (toggle) toggle.checked = config.receive_enabled;
    return saved;
  }
  async function refresh() {
    if (!enabled) return;
    peers = ((await apiGetPeers()) || []).filter((p) => p.id !== localId);
    await refreshRecords();
    if (android) renderSettings();
    renderDevices();
  }
  return {
    init,
    onPeers,
    leave,
    openSettings,
    open,
    openPushSources,
    refreshSettings,
    setReceiveEnabled,
    refresh,
  };
})();
