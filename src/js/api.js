// 防御性获取 Tauri 接口
const getTauri = () => window.__TAURI__;
const getErrorMessage = (error) => {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === "string" && error.trim()) return error;
  try {
    return JSON.stringify(error) || "未知错误";
  } catch (_) {
    return String(error ?? "未知错误");
  }
};

async function apiGetMyName() {
  const tauri = getTauri();

  if (tauri) {
    // 桌面端环境
    try {
      console.log("[JS-API] 正在通过 Tauri 调用 get_my_name");
      return await tauri.core.invoke("get_my_name");
    } catch (e) {
      console.error("[JS-API] Tauri 调用失败:", e);
      return "Tauri错误";
    }
  } else {
    // Web 端环境
    try {
      console.log("[JS-API] 正在通过 HTTP 调用 get_my_name");
      const resp = await fetch("/api/get_my_name");
      const data = await resp.json();
      return data.name;
    } catch (e) {
      console.error("[JS-API] Web 请求失败:", e);
      return "Web未连接";
    }
  }
}

async function apiGetMyId() {
  const tauri = getTauri();

  if (tauri) {
    // 桌面端环境
    try {
      console.log("[JS-API] 正在通过 Tauri 调用 get_my_id");
      return await tauri.core.invoke("get_my_id");
    } catch (e) {
      console.error("[JS-API] Tauri 调用失败:", e);
      throw new Error("获取 ID 失败: " + e);
    }
  } else {
    // Web 端环境
    try {
      console.log("[JS-API] 正在通过 HTTP 调用 get_my_id");
      const resp = await fetch("/api/get_my_id");
      const data = await resp.json();
      return data.id;
    } catch (e) {
      console.error("[JS-API] Web 请求失败:", e);
      throw new Error("获取 ID 失败: " + e);
    }
  }
}

// 获取设置
async function apiGetSettings() {
  const tauri = getTauri();

  if (tauri) {
    // 桌面端
    try {
      console.log("[JS-API] 通过 Tauri 获取设置");
      return await tauri.core.invoke("get_settings");
    } catch (e) {
      console.error("[JS-API] 获取设置失败:", e);
      throw new Error("获取设置失败: " + e);
    }
  } else {
    // Web 端
    try {
      console.log("[JS-API] 通过 HTTP 获取设置");
      const resp = await fetch("/api/get_settings");
      const data = await resp.json();
      return data;
    } catch (e) {
      console.error("[JS-API] 获取设置失败:", e);
      throw new Error("获取设置失败: " + e);
    }
  }
}

// 更新设置
async function apiUpdateSettings(downloadPath, port, dbPath, autoDownload, closeToTray, startMinimized) {
  const tauri = getTauri();

  if (tauri) {
    // 桌面端
    try {
      console.log("[JS-API] 通过 Tauri 更新设置");
      return await tauri.core.invoke("update_settings", {
        downloadPath,
        port,
        dbPath,
        autoDownload,
        closeToTray,
        startMinimized,
      });
    } catch (e) {
      console.error("[JS-API] Failed to update settings:", e);
      throw new Error("Failed to update settings: " + e);
    }
  } else {
    // Web 端
    try {
      console.log("[JS-API] 通过 HTTP 更新设置");
      const resp = await fetch("/api/update_settings", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          download_path: downloadPath,
          port: port,
          db_path: dbPath,
          auto_download: autoDownload,
          close_to_tray: closeToTray,
          start_minimized: startMinimized,
        }),
      });
      const data = await resp.json();
      if (data.error) {
        throw new Error(data.error);
      }
      return data;
    } catch (e) {
      console.error("[JS-API] Failed to update settings:", e);
      throw new Error("Failed to update settings: " + e);
    }
  }
}

// 获取默认下载路径
async function apiGetDefaultDownloadPath() {
  const tauri = getTauri();

  if (tauri) {
    try {
      return await tauri.core.invoke("get_default_download_path");
    } catch (e) {
      console.error("[JS-API] 获取默认路径失败:", e);
      return "";
    }
  } else {
    return "/tmp/LQChat";
  }
}

async function apiUpdateMyName(newName) {
  const tauri = getTauri();

  if (tauri) {
    // 桌面端环境
    try {
      console.log("[JS-API] 正在通过 Tauri 调用 update_my_name");
      return await tauri.core.invoke("update_my_name", { newName });
    } catch (e) {
      console.error("[JS-API] Tauri 调用失败:", e);
      throw new Error("更新失败: " + e);
    }
  } else {
    // Web 端环境
    try {
      console.log("[JS-API] 正在通过 HTTP 调用 update_my_name");
      const resp = await fetch("/api/update_my_name", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ name: newName }),
      });
      const data = await resp.json();
      if (data.error) {
        throw new Error(data.error);
      }
      return data.name;
    } catch (e) {
      console.error("[JS-API] Web 请求失败:", e);
      throw new Error("更新失败: " + e);
    }
  }
}

// 导出监听函数，同样要做环境判断
async function apiListen(eventName, callback) {
  console.log("[JS-API] 尝试监听事件:", eventName);
  const tauri = getTauri();
  if (tauri) {
    console.log("[JS-API] ✓ Tauri 环境，注册事件监听器:", eventName);
    const unlisten = await tauri.event.listen(eventName, callback);
    console.log("[JS-API] ✓ 事件监听器注册成功:", eventName);
    return unlisten;
  } else {
    console.warn(`[JS-API] ✗ 当前环境不支持监听事件: ${eventName}`);
    return () => {}; // 返回空函数
  }
}

// 获取在线用户列表（仅 Web 端使用）
async function apiGetPeers() {
  const tauri = getTauri();

  if (tauri) {
    // 桌面端通过 Tauri 命令获取
    try {
      return await tauri.core.invoke("get_peers");
    } catch (e) {
      console.error("[JS-API] 桌面端获取用户列表失败:", e);
      return [];
    }
  } else {
    // Web 端通过 HTTP 轮询
    try {
      const resp = await fetch("/api/get_peers");

      if (!resp.ok) {
        console.error("[JS-API] HTTP 错误:", resp.status, resp.statusText);
        return [];
      }

      const text = await resp.text();

      if (!text) {
        console.warn("[JS-API] 响应为空");
        return [];
      }

      const peers = JSON.parse(text);
      return peers;
    } catch (e) {
      console.error("[JS-API] 获取用户列表失败:", e);
      return [];
    }
  }
}

// 发送文本消息
async function apiSendMessage(peerId, peerAddr, content) {
  const tauri = getTauri();

  if (tauri) {
    // 桌面端
    try {
      console.log("[JS-API] 通过 Tauri 发送消息");
      return await tauri.core.invoke("send_message", {
        peerId,
        peerAddr,
        content,
      });
    } catch (e) {
      console.error("[JS-API] 发送消息失败:", e);
      throw new Error("发送失败: " + e);
    }
  } else {
    // Web 端
    try {
      console.log("[JS-API] 通过 HTTP 发送消息");
      const resp = await fetch("/api/send_message", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          peer_id: peerId, // 添加接收者ID
          peer_addr: peerAddr,
          content,
        }),
      });
      const data = await resp.json();
      if (data.error) {
        throw new Error(data.error);
      }
      return data;
    } catch (e) {
      console.error("[JS-API] 发送消息失败:", e);
      throw new Error("发送失败: " + e);
    }
  }
}

// 获取聊天历史（支持分页）
async function apiGetChatHistory(peerId, limit = 10, offset = 0) {
  const tauri = getTauri();

  if (tauri) {
    // 桌面端
    try {
      // 始终使用分页命令，确保 offset=0 时也不会丢失调用方传入的 limit。
      return await tauri.core.invoke("get_chat_history_with_offset", {
        peerId,
        limit,
        offset,
      });
    } catch (e) {
      console.error("[JS-API] 获取历史消息失败:", e);
      throw new Error("获取聊天记录失败: " + e);
    }
  } else {
    // Web 端
    try {
      // 始终传递 limit 和 offset 参数
      const url = `/api/chat_history/${peerId}?limit=${limit}&offset=${offset}`;

      const resp = await fetch(url);

      if (!resp.ok) {
        throw new Error(`HTTP ${resp.status}: ${resp.statusText}`);
      }

      const text = await resp.text();

      if (!text) {
        console.warn("[JS-API] 响应为空");
        return [];
      }

      const data = JSON.parse(text);
      return data.messages || [];
    } catch (e) {
      console.error("[JS-API] 获取历史消息失败:", e);
      throw new Error("获取聊天记录失败: " + e);
    }
  }
}

// 发送文件
// 每段收到确认后才更新进度，和原生发送路径保持一致。
function calculateOptimalChunkSize(_fileSize) {
  return 10 * 1024 * 1024;
}

async function apiSendFile(peerId, peerAddr, file, filePath) {
  const tauri = getTauri();

  if (tauri) {
    // 桌面端/移动端 - 使用 Tauri 对话框选择文件或使用提供的路径
    try {
      console.log("[JS-API] Tauri 环境发送文件");

      let selectedPath = filePath;

      // 如果没有提供文件路径，使用对话框选择
      if (!selectedPath) {
        const selected = await tauri.dialog.open({
          multiple: true,
          title: "选择要发送的文件",
        });

        if (!selected) {
          throw new Error("未选择文件");
        }

        const selectedPaths = Array.isArray(selected) ? selected : [selected];
        if (selectedPaths.length > 1) {
          const results = [];
          for (const path of selectedPaths) {
            try {
              results.push({ path, ok: true, result: await apiSendFile(peerId, peerAddr, null, path) });
            } catch (error) {
              results.push({ path, ok: false, error: error.message || String(error) });
            }
          }
          return { success: true, results };
        }
        selectedPath = selectedPaths[0];
      }

      console.log("[JS-API] 文件路径:", selectedPath);

      // 监听后端传来的进度事件
      let unlistenProgress;
      let unlistenStarted;
      if (tauri.event) {
        unlistenStarted = await tauri.event.listen("file_upload_started", (event) => {
          if (event.payload.peer_id !== peerId) return;
          const target = captureChatTarget();
          if (target?.peer.id === peerId) refreshCapturedChat(target).catch(console.error);
        });
        unlistenProgress = await tauri.event.listen(
          "upload_progress",
          (event) => {
            const p = event.payload;
            updateFileTransferById(p.sender_msg_id,
              p.transfer_status === "chunk_failed" ? "chunk_failed" : p.transfer_status === "failed" ? "failed" : "uploading",
              p.transferred, p.total, p.speed_mb_s);
          },
        );
      }

      try {
        // 直接调用后端统一命令，后端会自动处理 content URI 和普通路径
        const result = await tauri.core.invoke("send_file", {
          peerId,
          peerAddr,
          filePath: selectedPath,
        });
        console.log("[JS-API] 文件发送成功:", result);

        return result;
      } finally {
        // 取消事件监听
        if (unlistenProgress) {
          unlistenProgress();
        }
        if (unlistenStarted) unlistenStarted();
      }
    } catch (e) {
      console.error("[JS-API] 文件发送失败:", e);
      throw new Error("发送失败: " + getErrorMessage(e));
    }
  } else {
    // Web 端 - 通过 HTTP 上传（使用分块协议）
    const fileName = file.name;
    const fileSize = file.size;

    // 统一创建上传记录，后端返回 is_online 和 file_status
    const createResp = await fetch("/api/create_upload_record", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        file_name: fileName,
        file_size: fileSize,
        timestamp: Math.floor(Date.now() / 1000),
        receiver_id: peerId,
      }),
    });
    const createData = await createResp.json();
    if (!createData.success) throw new Error(createData.error || "创建记录失败");

    const msgId = createData.msg_id;

    // 保存 File 引用（供接收端关闭自动下载时的手动下载使用）
    if (!window.__pendingUploads) window.__pendingUploads = {};
    window.__pendingUploads[msgId] = {
      file,
      peerId,
      peerAddr,
      fileName,
      fileSize,
    };

    // 后端根据 peer_manager 判断在线状态；离线不会创建记录。
    if (!createData.is_online) {
      delete window.__pendingUploads[msgId];
      throw new Error("对方不在线");
    }

    if (createData.file_status === "offering") {
      // 在线但 auto_download OFF → 发 file_offer
      const myId = await apiGetMyId();
      const offerResp = await fetch("/api/offer_file", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          peer_addr: peerAddr,
          file_name: fileName,
          file_size: fileSize,
          sender_msg_id: msgId,
        }),
      });
      if (!offerResp.ok) throw new Error("发送 file_offer 失败");

      return {
        success: true,
        status: "offered",
        msg_id: msgId,
        file_name: fileName,
      };
    }

    // 在线 + auto_download ON → 直接分块上传
    try {
      const myId = await apiGetMyId();

      console.log("[JS-API] Web 端分块上传");
      console.log("[JS-API] 文件信息:", fileName, fileSize, "字节");
      console.log("[JS-API] sender_id (我的ID):", myId);

      // 根据设备内存动态计算分块大小
      const chunkSize = calculateOptimalChunkSize(fileSize);
      const totalChunks = Math.ceil(fileSize / chunkSize);

      console.log(
        "[JS-API] 计算的分块大小:",
        Math.round(chunkSize / (1024 * 1024)),
        "MB, 总分块数:",
        totalChunks,
      );

      const uploadUrl = `http://${peerAddr}/api/upload`;
      console.log("[JS-API] 上传地址:", uploadUrl);

      let offset = 0;
      let chunkIndex = 0;
      const startTime = Date.now();
      let lastLogTime = startTime;

      while (offset < fileSize) {
        const size = Math.min(chunkSize, fileSize - offset);
        const chunk = file.slice(offset, offset + size);

        // 构造 FormData 上传这一块
        const formData = new FormData();
        formData.append("peer_id", myId);
        formData.append("file_name", fileName);
        formData.append("file_size", fileSize.toString());
        formData.append("chunk_index", chunkIndex.toString());
        formData.append("chunk_total", totalChunks.toString());
        formData.append("sender_msg_id", msgId.toString());
        const elapsed = (Date.now() - startTime) / 1000;
        const speed = chunkIndex > 0 && elapsed > 0 ? offset / (1024 * 1024) / elapsed : 0;
        formData.append("speed_mb_s", speed.toFixed(1));
        formData.append("chunk", chunk, "chunk");

        console.log("[JS-API] 上传分块", chunkIndex + 1, "大小:", size, "字节");

        const resp = await fetch(uploadUrl, {
          method: "POST",
          body: formData,
          mode: "cors",
        });

        if (!resp.ok) {
          const errorText = await resp.text();
          console.error("[JS-API] ✗ 上传分块失败，状态码:", resp.status);
          console.error("[JS-API] ✗ 错误响应:", errorText);
          throw new Error(`HTTP ${resp.status}: ${errorText}`);
        }

        // 检查秒传命中（仅第一块）
        if (chunkIndex === 0) {
          const respData = await resp.json();
          if (respData.status === "already_exists") {
            console.log("[JS-API] ✓ 秒传命中，接收端已有完整文件，停止上传");
            updateFileTransferById(msgId, "sent", fileSize, fileSize, 0, true);
            // 标记发送端本地记录为 accepted（await 确保 loadChatHistory 前已更新）
            await fetch("/api/mark_upload_complete", {
              method: "POST",
              headers: { "Content-Type": "application/json" },
              body: JSON.stringify({ msg_id: createData.msg_id, status: "accepted" }),
            }).catch(e => console.warn("[JS-API] 标记秒传完成失败:", e));
            return {
              success: true,
              file_name: fileName,
              file_size: fileSize,
              instant_transfer: true,
            };
          }
        }

        offset += size;
        chunkIndex++;
        updateFileTransferById(msgId, "uploading", offset, fileSize,
          offset / Math.max(0.001, (Date.now() - startTime) / 1000) / (1024 * 1024));

        // 每秒打印一次进度并更新 UI
        const now = Date.now();
        if (now - lastLogTime > 1000) {
          const elapsed = (now - startTime) / 1000;
          const speed = offset / (1024 * 1024) / elapsed;
          console.log(
            "[JS-API] 已上传:",
            Math.round(offset / 1024 / 1024),
            "MB, 速度:",
            Math.round(speed),
            "MB/s",
          );

          lastLogTime = now;
        }
      }

      const totalTime = (Date.now() - startTime) / 1000;
      const avgSpeed = (fileSize / (1024 * 1024)) / totalTime;
      console.log(
        "[JS-API] ✓ 文件上传完成，共",
        chunkIndex,
        "块，耗时:",
        totalTime.toFixed(2),
        "秒，平均速度:",
        avgSpeed.toFixed(2),
        "MB/s",
      );

      // 更新发送端记录状态为 sent
      if (createData && createData.success) {
        await fetch("/api/mark_upload_complete", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            msg_id: msgId,
            status: "sent",
          }),
        });
      }
      updateFileTransferById(msgId, "sent", fileSize, fileSize, 0, true);

      return {
        success: true,
        file_name: fileName,
        file_size: fileSize,
      };
    } catch (e) {
      console.error("[JS-API] 文件上传失败:", e);
      fetch("/api/mark_upload_complete", {
        method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ msg_id: msgId, status: "failed" }),
      }).catch(console.warn);
      updateFileTransferById(msgId, "failed", 0, fileSize, 0, true);
      throw new Error("上传失败: " + getErrorMessage(e));
    }
  }
}
// 主题相关 API
async function apiGetThemeList() {
  const tauri = window.__TAURI__;
  if (tauri) {
    // 桌面端
    return await tauri.core.invoke("get_theme_list");
  } else {
    // Web 端
    const response = await fetch("/api/get_theme_list");
    if (!response.ok) {
      throw new Error("获取主题列表失败");
    }
    return await response.json();
  }
}

async function apiGetThemeCss(themeName) {
  const tauri = window.__TAURI__;
  if (tauri) {
    // 桌面端
    return await tauri.core.invoke("get_theme_css", { themeName });
  } else {
    // Web 端
    const response = await fetch(`/api/get_theme_css/${themeName}`);
    if (!response.ok) {
      throw new Error("获取主题CSS失败");
    }
    return await response.text();
  }
}

async function apiSaveCurrentTheme(themeName) {
  const tauri = window.__TAURI__;
  if (tauri) {
    // 桌面端
    return await tauri.core.invoke("save_current_theme", { themeName });
  } else {
    // Web 端
    const response = await fetch("/api/save_current_theme", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ theme_name: themeName }),
    });
    if (!response.ok) {
      throw new Error("保存主题失败");
    }
    return await response.json();
  }
}

async function apiGetCurrentTheme() {
  const tauri = window.__TAURI__;
  if (tauri) {
    // 桌面端
    return await tauri.core.invoke("get_current_theme");
  } else {
    // Web 端
    const response = await fetch("/api/get_current_theme");
    if (!response.ok) {
      throw new Error("获取当前主题失败");
    }
    const result = await response.json();
    return result.theme;
  }
}

// 获取分享文件
async function apiGetAndroidSharedFiles() {
  console.log("[JS-API] apiGetAndroidSharedFiles 被调用");

  // 直接从全局保险箱里拿数据！(不仅有数据，还完美自带原生层分配好的 fd)
  if (
    window.__ANDROID_SHARED_FILES__ &&
    window.__ANDROID_SHARED_FILES__.length > 0
  ) {
    console.log(
      "[JS-API] 成功截获原生层空投的分享数据:",
      window.__ANDROID_SHARED_FILES__,
    );
    // 返回深拷贝的数据，避免引用污染
    return JSON.parse(JSON.stringify(window.__ANDROID_SHARED_FILES__));
  }

  const tauri = getTauri();
  if (tauri && navigator.userAgent.includes("Android")) {
    const files = await tauri.core.invoke("get_android_shared_files");
    if (Array.isArray(files) && files.length > 0) {
      window.__ANDROID_SHARED_FILES__ = files;
      console.log("[JS-API] 已从 Android 领取分享文件:", files.length);
      return JSON.parse(JSON.stringify(files));
    }
  }

  console.log("[JS-API] 全局变量为空，没有分享文件");
  return [];
}

// 清理分享文件（并安全释放底层资源）
async function apiClearAndroidSharedFiles() {
  console.log("[JS-API] 准备清除前端分享缓存...");

  if (
    window.__ANDROID_SHARED_FILES__ &&
    window.__ANDROID_SHARED_FILES__.length > 0
  ) {
    const tauri = getTauri();
    if (tauri) {
      // 遍历所有待发送的文件，通知 Rust 释放文件描述符
      for (const file of window.__ANDROID_SHARED_FILES__) {
        if (file.fd !== undefined && file.fd >= 0) {
          try {
            await tauri.core.invoke("close_android_fd", { fd: file.fd });
            console.log("[JS-API] 已通知 Rust 安全关闭 FD:", file.fd);
          } catch (e) {
            console.error("[JS-API] 通知 Rust 关闭 FD 失败:", e);
          }
        }
      }
    }
  }

  // 释放完毕，彻底清空前端全局变量
  window.__ANDROID_SHARED_FILES__ = null;
  console.log("[JS-API] 前端分享缓存已清空");
}

async function apiSendFileFromAndroidUri(peerId, peerAddr, fileInfo) {
  const tauri = getTauri();

  if (!tauri) {
    throw new Error("Tauri 不可用");
  }

  try {
    console.log("[JS-API] 从 Android URI 发送文件:", fileInfo);
    console.log("[JS-API] peerId:", peerId, "peerAddr:", peerAddr);

    // 确保文件名不为空
    let fileName = fileInfo.fileName;
    if (!fileName || fileName.trim() === "") {
      // 根据 MIME 类型生成默认文件名
      const timestamp = Date.now();
      const ext = fileInfo.mimeType
        ? fileInfo.mimeType.split("/")[1] || "dat"
        : "dat";
      fileName = `shared_file_${timestamp}.${ext}`;
      console.log("[JS-API] 文件名为空，生成默认文件名:", fileName);
    }

    // 检查是否有文件描述符
    if (fileInfo.fd && fileInfo.fd >= 0) {
      console.log("[JS-API] 使用文件描述符发送: fd=" + fileInfo.fd);

      // 使用 FD 发送文件
      const params = {
        peerId: peerId,
        peerAddr: peerAddr,
        fileName: fileName,
        fileSize: fileInfo.fileSize,
        fd: fileInfo.fd,
        originalUri: fileInfo.uri || null, // 传递原始 URI
      };

      console.log(
        "[JS-API] 调用 send_file_from_fd，参数:",
        JSON.stringify(params),
      );

      const result = await tauri.core.invoke("send_file_from_fd", params);
      console.log("[JS-API] 文件发送成功:", result);
      return result;
    } else {
      // 没有 FD，无法发送
      console.error("[JS-API] 没有有效的文件描述符: fd=" + fileInfo.fd);
      throw new Error("无法获取文件描述符，无法发送文件");
    }
  } catch (e) {
    console.error("[JS-API] 从 Android URI 发送文件失败:", e);
    throw e;
  }
}

// 分享文件到其他应用（仅 Android）
async function apiShareFileToOtherApp(filePath) {
  const tauri = getTauri();

  if (!tauri) {
    throw new Error("仅支持 Android 端");
  }

  try {
    console.log("[JS-API] 分享文件到其他应用:", filePath);
    await tauri.core.invoke("share_file_to_other_app", { filePath });
    console.log("[JS-API] 分享成功");
  } catch (e) {
    console.error("[JS-API] 分享文件失败:", e);
    throw e;
  }
}

// 媒体 Token 缓存（避免每次渲染图片都 invoke）
let _mediaTokenCache = null;
let _mediaTokenRequest = null;
async function apiGetMediaToken() {
  if (_mediaTokenCache) return _mediaTokenCache;
  if (_mediaTokenRequest) return _mediaTokenRequest;
  const tauri = getTauri();
  if (!tauri) return "";
  _mediaTokenRequest = tauri.core.invoke("get_media_token")
    .then((token) => {
      _mediaTokenCache = token;
      console.log("[JS-API] 获取媒体 Token 成功");
      return token;
    })
    .catch((e) => {
      console.error("[JS-API] 获取媒体 Token 失败:", e);
      return "";
    })
    .finally(() => {
      _mediaTokenRequest = null;
    });
  return _mediaTokenRequest;
}

// 用对应应用打开文件（仅 Android）
async function apiOpenFileInAndroid(filePath) {
  const tauri = getTauri();

  if (!tauri) {
    throw new Error("仅支持 Android 端");
  }

  try {
    console.log("[JS-API] 打开文件:", filePath);
    await tauri.core.invoke("open_file_in_android", { filePath });
    console.log("[JS-API] 打开文件成功");
  } catch (e) {
    console.error("[JS-API] 打开文件失败:", e);
    throw e;
  }
}

// 检查 Android 文件是否仍可读取（系统目录 URI、应用目录和 FD 路径均支持）
async function apiGetAndroidFileState(filePath) {
  const tauri = getTauri();
  if (!tauri || !navigator.userAgent.includes("Android")) {
    return "NOT_APPLICABLE";
  }

  try {
    return await tauri.core.invoke("get_android_file_state", { filePath });
  } catch (e) {
    console.error("[JS-API] 检查 Android 文件状态失败:", e);
    return "ERROR";
  }
}

// 批量删除消息
async function apiDeleteMessages(msgIds) {
  const tauri = getTauri();

  if (tauri) {
    // 桌面端
    try {
      await tauri.core.invoke("delete_messages", { msgIds });
    } catch (e) {
      console.error("[JS-API] 删除消息失败:", e);
      throw e;
    }
  } else {
    // Web 端
    const resp = await fetch("/api/delete_messages", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ msg_ids: msgIds }),
    });

    if (!resp.ok) {
      throw new Error("删除消息失败: " + resp.status);
    }
  }
}

async function apiClearChatHistory(peerId) {
  const tauri = getTauri();
  if (tauri) {
    return await tauri.core.invoke("clear_chat_history", { peerId });
  } else {
    const resp = await fetch("/api/clear_chat_history", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ peer_id: peerId }),
    });
    return await resp.json();
  }
}

async function apiDeleteUserComplete(peerId) {
  const tauri = getTauri();
  if (tauri) {
    return await tauri.core.invoke("delete_user_complete", { peerId });
  } else {
    const resp = await fetch("/api/delete_user", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ peer_id: peerId }),
    });
    return await resp.json();
  }
}

async function apiClearOfflinePeers() {
  const tauri = getTauri();
  if (tauri) return await tauri.core.invoke("clear_offline_peers");
  const response = await fetch("/api/clear_offline_peers", { method: "POST" });
  if (!response.ok) throw new Error(`清除设备失败：HTTP ${response.status}`);
  const result = await response.json();
  return result.removed || [];
}

async function apiGetCustomPeers() {
  const tauri = getTauri();

  if (tauri) {
    try {
      return await tauri.core.invoke("get_custom_peers");
    } catch (e) {
      console.error("[JS-API] 获取自定义 IP 失败:", e);
      return [];
    }
  } else {
    try {
      const resp = await fetch("/api/get_custom_peers");
      const data = await resp.json();
      return data.peers || [];
    } catch (e) {
      console.error("[JS-API] 获取自定义 IP 失败:", e);
      return [];
    }
  }
}

async function apiAddCustomPeer(peer) {
  const tauri = getTauri();

  if (tauri) {
    try {
      return await tauri.core.invoke("add_custom_peer", { peer });
    } catch (e) {
      console.error("[JS-API] 添加自定义 IP 失败:", e);
      throw e;
    }
  } else {
    const resp = await fetch("/api/add_custom_peer", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ peer }),
    });
    const data = await resp.json();
    if (data.error) throw new Error(data.error);
    return data;
  }
}

async function apiRemoveCustomPeer(peer) {
  const tauri = getTauri();

  if (tauri) {
    try {
      return await tauri.core.invoke("remove_custom_peer", { peer });
    } catch (e) {
      console.error("[JS-API] 删除自定义 IP 失败:", e);
      throw e;
    }
  } else {
    const resp = await fetch("/api/remove_custom_peer", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ peer }),
    });
    const data = await resp.json();
    if (data.error) throw new Error(data.error);
    return data;
  }
}
/** 检查对方设备的自动下载是否开启 */
async function apiCheckAutoDownload(peerAddr) {
  try {
    const resp = await fetch(`http://${peerAddr}/api/auto_download`);
    if (resp.ok) {
      const data = await resp.json();
      return data.enabled !== false;
    }
  } catch (_) {}
  return true; // 默认开启
}

/** 发送 file_request 到对方（请求开始发送文件） */
async function apiRequestFile(senderAddr, senderMsgId, myAddr) {
  const payload = { sender_msg_id: senderMsgId, receiver_addr: myAddr };
  // 通过 HTTP 调用发送端的 /api/start_send
  const resp = await fetch(`http://${senderAddr}/api/start_send`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(payload),
  });
  if (!resp.ok) {
    const err = await resp.json().catch(() => ({}));
    if (err.not_found) {
      return { status: "not_found" };
    }
    throw new Error(err.error || "请求发送文件失败");
  }
  return await resp.json();
}

/** 请求通知权限（按需调用，不在初始化时触发以避免 WebKit 警告） */
async function requestNotificationPermission() {
  if (!("Notification" in window)) return;
  if (Notification.permission === "default") {
    await Notification.requestPermission().catch(() => {});
  }
}

/** Android 13+ 通知权限申请（Tauri 端使用插件原生 API） */
let androidNotificationPermissionRequest = null;
window.requestAndroidNotificationPermission = function() {
  if (!window.__TAURI__) return Promise.resolve("granted");
  if (androidNotificationPermissionRequest) return androidNotificationPermissionRequest;
  const tauri = window.__TAURI__;
  let timer;
  const request = (async () => {
    // Notification plugin 2.3.3 does not resolve repeat requests on Android 13+.
    const state = await tauri.core.invoke("get_notification_permission_state");
    if (state === "granted") return state;
    if (tauri.notification) await tauri.notification.requestPermission();
    else await tauri.core.invoke("plugin:notification|request_permission");
    return tauri.core.invoke("get_notification_permission_state");
  })();
  androidNotificationPermissionRequest = Promise.race([
    request,
    new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error("通知授权等待超时，请在系统设置中确认通知权限后重试")), 15000);
    }),
  ]).finally(() => {
    clearTimeout(timer);
    androidNotificationPermissionRequest = null;
  });
  return androidNotificationPermissionRequest;
};

/** 显示桌面通知。Tauri 端调平台命令，Web 端用 Web Notification API */
async function showNotification(title, body, extra = {}) {
  // 存储 from_id 到 localStorage，供 Android 通知点击导航
  if (extra.from_id) {
    try { localStorage.setItem("pendingNotificationFromId", extra.from_id); } catch (_) {}
  }
  if (window.__TAURI__) {
    // Android 与 Windows 的通知都由核心事件层生成，避免 WebView 不存在时
    // 漏通知，也避免前端与后台重复发送同一条系统通知。
    if (navigator.userAgent.includes("Android") || navigator.userAgent.includes("Windows")) return;
    // Tauri 端：先检查通知开关（用缓存避免每次都调 invoke）
    if (window._notificationsEnabled === undefined) {
      try {
        window._notificationsEnabled = await window.__TAURI__.core.invoke("get_notifications_enabled");
      } catch (_) { window._notificationsEnabled = true; }
    }
    if (!window._notificationsEnabled) return;
    // 通过 Rust 命令（Windows→PowerShell, Linux→notify-send, macOS/Android→plugin）
    try {
      await window.__TAURI__.core.invoke("show_notification", { title, body, fromId: extra.from_id || "" });
    } catch (e) {
      console.error("[JS-API] 通知命令失败:", e);
    }
  } else if ("Notification" in window) {
    // Web 端：按需请求权限，仅在已授权时发送
    if (Notification.permission === "default") {
      try { await Notification.requestPermission(); } catch (_) {}
    }
    if (Notification.permission === "granted") {
      try {
        new Notification(title, { body });
      } catch (e) {
        console.error("[JS-API] 通知发送失败:", e);
      }
    }
  }
}
