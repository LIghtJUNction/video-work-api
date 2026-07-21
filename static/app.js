const translations = {
  zh: {
    title: "视频工作 API",
    workspaceEyebrow: "VOICE WORKSPACE",
    workspaceTitle: "把声音，变成可复用的创作资产。",
    workspaceLead: "导入有明确授权的参考音频，保存精确逐字稿，然后生成自然、稳定的语音。",
    endpointLabel: "MCP SERVER",
    mcpHint: "MCP 服务器运行在此端点，使用 Bearer Token 认证。",
    logout: "退出",
    docs: "文档",
    downloadModel: "下载模型",
    downloadingModel: "下载中…",
    confirmModelDownload: "模型下载约需 10 GB 网络流量和磁盘空间，确认开始吗？",
    modelDownloadFailed: "模型下载失败，请查看服务日志后重试。",
    setupTitle: "首次设置",
    setupHelp: "运行 vwactl init 后，将一次性令牌和新密码填入此处。",
    token: "一次性令牌",
    password: "管理员密码（至少 12 位）",
    passwordShort: "密码",
    finishSetup: "完成设置",
    loginTitle: "管理员登录",
    login: "登录",
    or: "或",
    passkeyLogin: "使用 Passkey 登录",
    passkeysTitle: "Passkey 管理",
    passkeysLead: "添加本机或安全密钥，下次可免输密码登录。",
    passkeyName: "设备名称",
    addPasskey: "添加 Passkey",
    noPasskeys: "尚未添加 Passkey。",
    passkeyUnsupported: "当前浏览器或连接不支持 Passkey；远程访问请使用 HTTPS 域名。",
    passkeyIpUnsupported:
      "IP 地址页面不支持 Passkey；请改用 http://localhost:<端口>，远程访问请使用 HTTPS 域名。",
    passwordRecovery: "管理员密码会保留作为恢复登录方式。",
    libraryEyebrow: "VOICE LIBRARY",
    library: "音色库",
    libraryLead: "管理说话人和已授权的参考音色。",
    speakerName: "说话人名称",
    addSpeaker: "添加说话人",
    generateEyebrow: "GENERATE",
    generateTitle: "生成语音",
    generateLead: "选择音色，输入文案，马上试听结果。",
    voiceStyle: "说话人 / 语气",
    targetText: "目标文案",
    speed: "语速",
    generateButton: "生成音频",
    download: "下载 WAV",
    resultReady: "生成完成",
    deleteSpeaker: "删除说话人",
    renameSpeaker: "重命名",
    addProfile: "添加参考音频",
    styleName: "语气名称",
    audioFile: "音频（推荐 8–15 秒）",
    transcript: "参考录音逐字稿",
    rights: "我确认拥有克隆及使用该声音的明确权利和同意。",
    upload: "上传并转换",
    renameProfile: "重命名",
    deleteProfile: "删除",
    renameSave: "保存",
    renameCancel: "取消",
    renameEmpty: "名称不能为空。",
    renameTooLong: "名称最多 100 个字符。",
    noProfiles: "尚无参考音频",
    modelReady: "模型已就绪",
    modelWarm: "模型已热启动",
    modelNeedsSetup: "权重已就绪 · 请运行 setup",
    modelMissing: "模型未下载",
    modelIdle: "模型未就绪",
    working: "处理中…",
    failed: "操作失败",
    confirmDelete: "确定删除吗？",
    alreadyConfigured: "已完成首次设置，请使用密码登录。",
    subtitlesEyebrow: "SUBTITLES",
    subtitlesTitle: "视频字幕提取",
    subtitlesLead: "输入 videos 目录中的视频文件名，提取带时间码的字幕。",
    videoPath: "视频文件名",
    subtitlesHint: "首次提取会下载 ASR 模型，长视频可能需要几分钟。",
    extractSubtitles: "提取字幕",
    downloadSrt: "下载 SRT",
    subtitlesEmpty: "未识别到字幕片段。",
  },
  en: {
    title: "Video Work API",
    workspaceEyebrow: "VOICE WORKSPACE",
    workspaceTitle: "Turn every voice into a reusable creative asset.",
    workspaceLead: "Import an explicitly authorized reference, keep its exact transcript, and generate natural, consistent speech.",
    endpointLabel: "MCP SERVER",
    mcpHint: "The MCP server runs at this endpoint with Bearer Token authentication.",
    logout: "Sign out",
    docs: "Docs",
    downloadModel: "Download model",
    downloadingModel: "Downloading…",
    confirmModelDownload: "The model download uses roughly 10 GB of network traffic and disk space. Start now?",
    modelDownloadFailed: "Model download failed. Check the service logs, then retry.",
    setupTitle: "First-time setup",
    setupHelp: "Run vwactl init, then enter the one-time token and a new password.",
    token: "One-time token",
    password: "Admin password (12+ characters)",
    passwordShort: "Password",
    finishSetup: "Complete setup",
    loginTitle: "Admin sign in",
    login: "Sign in",
    or: "or",
    passkeyLogin: "Sign in with a passkey",
    passkeysTitle: "Passkeys",
    passkeysLead: "Add this device or a security key for passwordless sign-in.",
    passkeyName: "Device name",
    addPasskey: "Add passkey",
    noPasskeys: "No passkeys registered.",
    passkeyUnsupported:
      "This browser or connection cannot use passkeys; use an HTTPS domain for remote access.",
    passkeyIpUnsupported:
      "Passkeys do not support IP address pages; use http://localhost:<port>, or an HTTPS domain for remote access.",
    passwordRecovery: "The admin password remains available for account recovery.",
    libraryEyebrow: "VOICE LIBRARY",
    library: "Voice library",
    libraryLead: "Manage speakers and authorized reference voices.",
    speakerName: "Speaker name",
    addSpeaker: "Add speaker",
    generateEyebrow: "GENERATE",
    generateTitle: "Generate speech",
    generateLead: "Choose a voice, write your copy, and preview the result.",
    voiceStyle: "Speaker / style",
    targetText: "Target text",
    speed: "Speed",
    generateButton: "Generate audio",
    download: "Download WAV",
    resultReady: "Generation complete",
    deleteSpeaker: "Delete speaker",
    renameSpeaker: "Rename",
    addProfile: "Add reference audio",
    styleName: "Style name",
    audioFile: "Audio (8–15 seconds recommended)",
    transcript: "Exact reference transcript",
    rights: "I confirm I have explicit rights and consent to clone and use this voice.",
    upload: "Upload and convert",
    renameProfile: "Rename",
    deleteProfile: "Delete",
    renameSave: "Save",
    renameCancel: "Cancel",
    renameEmpty: "Name cannot be empty.",
    renameTooLong: "Name must be at most 100 characters.",
    noProfiles: "No reference audio yet",
    modelReady: "Model ready",
    modelWarm: "Model warm",
    modelNeedsSetup: "Weights ready · run setup",
    modelMissing: "Model not downloaded",
    modelIdle: "Model not ready",
    working: "Working…",
    failed: "Operation failed",
    confirmDelete: "Delete this item?",
    alreadyConfigured: "Setup is already complete. Please sign in with your password.",
    subtitlesEyebrow: "SUBTITLES",
    subtitlesTitle: "Video Subtitles",
    subtitlesLead: "Enter a filename from the videos directory to extract time-coded subtitles.",
    videoPath: "Video filename",
    subtitlesHint: "The first extraction downloads the ASR model; long videos may take a few minutes.",
    extractSubtitles: "Extract subtitles",
    downloadSrt: "Download SRT",
    subtitlesEmpty: "No subtitle segments detected.",
  },
};

const recordTranslations = {
  zh: {
    recordPreferred: "推荐：直接录制 8–15 秒",
    startRecording: "开始录音",
    stopRecording: "停止",
    discardRecording: "丢弃重录",
    recording: "正在录音",
    recordReady: "录音已就绪，确认逐字稿后点击保存。",
    micDenied: "无法使用麦克风，请检查权限。",
    insecureMic: "手机或远程电脑录音需要 HTTPS；localhost 可直接使用。",
    audioRequired: "请先录音或选择音频文件。",
  },
  en: {
    recordPreferred: "Preferred: record 8–15 seconds now",
    startRecording: "Start recording",
    stopRecording: "Stop",
    discardRecording: "Discard / re-record",
    recording: "Recording",
    recordReady: "Recording ready. Verify the transcript, then save.",
    micDenied: "The microphone is unavailable. Check browser permission.",
    insecureMic:
      "Recording from a phone or remote computer requires HTTPS; localhost works directly.",
    audioRequired: "Record or choose an audio file first.",
  },
};

let language = localStorage.getItem("vwa-language") || "zh";
let state = { speakers: [], passkeys: [] };
const recorders = new WeakMap();
const liveStreams = new Set();
let modelDownloadPoll = null;

const $ = (selector) => document.querySelector(selector);

function t(key) {
  return translations[language][key] || recordTranslations[language][key] || key;
}

/** FNV-1a over UTF-8 → 4 lowercase hex digits (matches Rust short_hash4). */
function shortHash4(text) {
  const bytes = new TextEncoder().encode(String(text || ""));
  let h = 2166136261 >>> 0;
  for (let i = 0; i < bytes.length; i++) {
    h ^= bytes[i];
    h = Math.imul(h, 16777619) >>> 0;
  }
  return (h & 0xffff).toString(16).padStart(4, "0");
}

/** Browser download basename: 前几个字…末尾几个字-xxxx.wav */
function downloadNameFromText(text) {
  const raw = String(text || "").trim();
  const cleaned = raw
    .replace(/[\u0000-\u001f\\/:*?"<>|]/g, "")
    .replace(/\s+/g, " ")
    .trim();
  const chars = Array.from(cleaned);
  const head = 8;
  const tail = 6;
  let label;
  if (!cleaned) {
    label = "speech";
  } else if (chars.length <= head + tail + 1) {
    label = cleaned;
  } else {
    label = chars.slice(0, head).join("") + "…" + chars.slice(-tail).join("");
  }
  label = label.replace(/^[\s。，、；：！？.,!?;:]+|[\s。，、；：！？.,!?;:]+$/g, "");
  if (!label) label = "speech";
  return `${label}-${shortHash4(raw)}.wav`;
}

function translate() {
  document.documentElement.lang = language === "zh" ? "zh-CN" : "en";
  document.querySelectorAll("[data-i18n]").forEach((node) => {
    node.textContent = t(node.dataset.i18n);
  });
  const languageButton = $("#language");
  if (languageButton) {
    languageButton.textContent = language === "zh" ? "English" : "中文";
  }
  const passkeyLoginSupport = $("#passkeyLoginSupport");
  if (passkeyLoginSupport) {
    passkeyLoginSupport.textContent = passkeyUnsupportedMessage();
  }
}

function notice(message) {
  const box = $("#notice");
  if (!box) return;
  box.textContent = message || "";
  box.classList.toggle("hidden", !message);
}

function safeReset(form) {
  if (form && typeof form.reset === "function") {
    form.reset();
  }
}

function stopModelDownloadPolling() {
  if (modelDownloadPoll !== null) clearTimeout(modelDownloadPoll);
  modelDownloadPoll = null;
}

async function refreshModelDownloadStatus() {
  const button = $("#modelDownload");
  if (!button || button.classList.contains("hidden")) return;
  try {
    const status = await api("/api/model/download");
    const running = status.state === "running";
    button.disabled = running;
    button.textContent = running ? t("downloadingModel") : t("downloadModel");
    if (status.model_present) {
      stopModelDownloadPolling();
      await boot();
    } else if (status.state === "failed" || status.state === "succeeded") {
      stopModelDownloadPolling();
      notice(t("modelDownloadFailed"));
    } else if (running) {
      stopModelDownloadPolling();
      modelDownloadPoll = setTimeout(refreshModelDownloadStatus, 2000);
    }
  } catch (error) {
    stopModelDownloadPolling();
    notice(error.message);
  }
}

function isIpLiteralHostname(hostname = window.location.hostname) {
  const value = String(hostname).replace(/^\[|\]$/g, "");
  if (value.includes(":")) return true;
  const parts = value.split(".");
  return (
    parts.length === 4 &&
    parts.every(
      (part) => /^\d+$/.test(part) && Number(part) >= 0 && Number(part) <= 255,
    )
  );
}

function passkeysSupported() {
  return Boolean(
    window.PublicKeyCredential &&
      navigator.credentials &&
      window.isSecureContext &&
      !isIpLiteralHostname(),
  );
}

function passkeyUnsupportedMessage() {
  return t(isIpLiteralHostname() ? "passkeyIpUnsupported" : "passkeyUnsupported");
}

function base64urlToBuffer(value) {
  const padded = String(value).replace(/-/g, "+").replace(/_/g, "/");
  const binary = atob(padded + "=".repeat((4 - (padded.length % 4)) % 4));
  return Uint8Array.from(binary, (char) => char.charCodeAt(0)).buffer;
}

function bufferToBase64url(value) {
  if (value === null || value === undefined) return null;
  const bytes = new Uint8Array(value);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "");
}

function creationOptions(publicKey) {
  return {
    ...publicKey,
    challenge: base64urlToBuffer(publicKey.challenge),
    user: { ...publicKey.user, id: base64urlToBuffer(publicKey.user.id) },
    excludeCredentials: (publicKey.excludeCredentials || []).map((item) => ({
      ...item,
      id: base64urlToBuffer(item.id),
    })),
  };
}

function requestOptions(publicKey) {
  return {
    ...publicKey,
    challenge: base64urlToBuffer(publicKey.challenge),
    allowCredentials: (publicKey.allowCredentials || []).map((item) => ({
      ...item,
      id: base64urlToBuffer(item.id),
    })),
  };
}

function serializeCredential(credential) {
  const response = credential.response;
  const serialized = {
    id: credential.id,
    rawId: bufferToBase64url(credential.rawId),
    type: credential.type,
    extensions: credential.getClientExtensionResults(),
    response: {
      clientDataJSON: bufferToBase64url(response.clientDataJSON),
    },
  };
  if (response.attestationObject) {
    serialized.response.attestationObject = bufferToBase64url(
      response.attestationObject,
    );
    if (typeof response.getTransports === "function") {
      serialized.response.transports = response.getTransports();
    }
  } else {
    serialized.response.authenticatorData = bufferToBase64url(
      response.authenticatorData,
    );
    serialized.response.signature = bufferToBase64url(response.signature);
    serialized.response.userHandle = bufferToBase64url(response.userHandle);
  }
  return serialized;
}

async function api(path, options = {}) {
  const response = await fetch(path, {
    ...options,
    credentials: "same-origin",
    headers: {
      ...(options.body instanceof FormData
        ? {}
        : { "Content-Type": "application/json" }),
      ...(options.headers || {}),
    },
  });
  if (!response.ok) {
    let body = {};
    try {
      body = await response.json();
    } catch {
      /* ignore non-JSON error bodies */
    }
    const error = new Error(body.error?.message || `${response.status}`);
    error.code = body.error?.code || null;
    error.status = response.status;
    throw error;
  }
  if (response.status === 204) return null;
  return response.json();
}

function setView(name) {
  ["setupView", "loginView", "studioView"].forEach((id) => {
    const node = $("#" + id);
    if (node) node.classList.toggle("hidden", id !== name);
  });
  const logout = $("#logout");
  if (logout) logout.classList.toggle("hidden", name !== "studioView");
}

async function boot() {
  translate();
  try {
    const status = await api("/api/status");
    const modelDownload = $("#modelDownload");
    if (modelDownload) {
      modelDownload.classList.toggle(
        "hidden",
        !status.authenticated || status.model_present !== false,
      );
    }
    const modelState = $("#modelState");
    if (modelState) {
      // model_loaded = warm after first generation in-process (lazy).
      // model_present = weights on disk. model_ready = present + Python venv.
      if (status.model_loaded) {
        modelState.textContent = t("modelWarm");
      } else if (status.model_ready) {
        modelState.textContent = t("modelReady");
      } else if (status.model_present) {
        modelState.textContent = t("modelNeedsSetup");
      } else if (status.model_present === false) {
        modelState.textContent = t("modelMissing");
      } else {
        // Older servers that only send model_loaded
        modelState.textContent = status.model_loaded
          ? t("modelReady")
          : t("modelIdle");
      }
    }
    if (!status.configured) {
      stopModelDownloadPolling();
      setView("setupView");
    } else if (!status.authenticated) {
      stopModelDownloadPolling();
      const passkeyLogin = $("#passkeyLogin");
      if (passkeyLogin) {
        passkeyLogin.classList.toggle(
          "hidden",
          !status.passkey_login_available || !passkeysSupported(),
        );
      }
      const passkeySupport = $("#passkeyLoginSupport");
      if (passkeySupport) {
        passkeySupport.textContent = passkeyUnsupportedMessage();
        passkeySupport.classList.toggle(
          "hidden",
          !status.passkey_login_available || passkeysSupported(),
        );
      }
      setView("loginView");
    } else {
      setView("studioView");
      await refresh();
      if (status.model_present === false) await refreshModelDownloadStatus();
    }
  } catch (error) {
    notice(error.message);
  }
}

async function refresh() {
  const [speakers, passkeys] = await Promise.all([
    api("/api/speakers"),
    api("/api/auth/passkeys"),
  ]);
  state = {
    speakers: Array.isArray(speakers?.speakers) ? speakers.speakers : [],
    passkeys: Array.isArray(passkeys?.passkeys) ? passkeys.passkeys : [],
  };
  renderSpeakers();
  renderPasskeys();
}

function renderPasskeys() {
  const root = $("#passkeyList");
  const form = $("#passkeyForm");
  const support = $("#passkeySupport");
  if (!root) return;
  const supported = passkeysSupported();
  if (form) form.classList.toggle("hidden", !supported);
  if (support) {
    support.textContent = supported ? "" : passkeyUnsupportedMessage();
    support.classList.toggle("hidden", supported);
  }
  root.replaceChildren();
  if (!state.passkeys.length) {
    const empty = document.createElement("p");
    empty.className = "hint";
    empty.textContent = t("noPasskeys");
    root.appendChild(empty);
    return;
  }
  for (const passkey of state.passkeys) {
    const row = document.createElement("div");
    row.className = "passkey-row";
    const details = document.createElement("div");
    const name = document.createElement("strong");
    name.textContent = passkey.name;
    const date = document.createElement("span");
    date.textContent = new Date(passkey.created_at * 1000).toLocaleDateString();
    details.append(name, date);
    const button = document.createElement("button");
    button.type = "button";
    button.className = "danger compact";
    button.textContent = t("deleteProfile");
    button.onclick = async () => {
      if (!confirm(t("confirmDelete"))) return;
      try {
        await api(`/api/auth/passkeys/${passkey.id}`, { method: "DELETE" });
        await refresh();
        notice("");
      } catch (error) {
        notice(error.message);
      }
    };
    row.append(details, button);
    root.appendChild(row);
  }
}

function renderSpeakers() {
  const root = $("#speakers");
  const select = $("#profileSelect");
  if (!root || !select) return;

  const selected = select.value;
  liveStreams.forEach(stopStream);
  root.replaceChildren();
  select.replaceChildren();

  (state.speakers || []).forEach((speaker) => {
    const fragment = $("#speakerTemplate").content.cloneNode(true);
    const article = fragment.querySelector("article");
    article.dataset.id = speaker.id;
    const title = article.querySelector("h3");
    title.textContent = speaker.name;
    title.classList.add("speaker-title");
    title.title = t("renameSpeaker");

    const renameSpeakerBtn = article.querySelector(".rename-speaker");
    const deleteSpeakerBtn = article.querySelector(".delete-speaker");
    const beginSpeakerRename = () =>
      startInlineRename({
        displayEl: title,
        current: speaker.name,
        path: `/api/speakers/${speaker.id}`,
        bodyKey: "name",
        maxLength: 100,
        extraHide: [renameSpeakerBtn, deleteSpeakerBtn].filter(Boolean),
      });
    if (renameSpeakerBtn) {
      renameSpeakerBtn.onclick = beginSpeakerRename;
    }
    title.onclick = beginSpeakerRename;
    if (deleteSpeakerBtn) {
      deleteSpeakerBtn.onclick = () => remove(`/api/speakers/${speaker.id}`);
    }

    const profiles = article.querySelector(".profiles");
    if (!speaker.profiles.length) {
      profiles.textContent = t("noProfiles");
    }
    speaker.profiles.forEach((profile) => {
      const row = document.createElement("div");
      row.className = "profile";

      const label = document.createElement("span");
      label.className = "profile-label";
      label.dataset.style = profile.style_name;
      label.title = t("renameProfile");
      renderProfileLabel(label, profile);

      const actions = document.createElement("div");
      actions.className = "profile-actions";

      const renameBtn = document.createElement("button");
      renameBtn.type = "button";
      renameBtn.className = "secondary compact";
      renameBtn.textContent = t("renameProfile");

      const deleteBtn = document.createElement("button");
      deleteBtn.type = "button";
      deleteBtn.className = "danger";
      deleteBtn.textContent = t("deleteProfile");
      deleteBtn.onclick = () => remove(`/api/profiles/${profile.id}`);

      const beginProfileRename = () =>
        startInlineRename({
          displayEl: label,
          current: profile.style_name,
          path: `/api/profiles/${profile.id}`,
          bodyKey: "style_name",
          maxLength: 100,
          extraHide: [renameBtn, deleteBtn],
          onRenderDisplay: (el, value) => {
            profile.style_name = value;
            renderProfileLabel(el, profile);
          },
        });
      renameBtn.onclick = beginProfileRename;
      label.onclick = beginProfileRename;

      actions.append(renameBtn, deleteBtn);
      row.append(label, actions);
      profiles.append(row);

      const option = document.createElement("option");
      option.value = `${speaker.id}|${profile.id}`;
      option.textContent = `${speaker.name} — ${profile.style_name}`;
      select.append(option);
    });

    const form = article.querySelector(".profile-form");
    form.onsubmit = (event) => uploadProfile(event, speaker.id);
    initializeRecorder(form);
    root.append(fragment);
  });

  if ([...select.options].some((option) => option.value === selected)) {
    select.value = selected;
  }
  translate();
}

function renderProfileLabel(el, profile) {
  const secs =
    typeof profile.duration_seconds === "number"
      ? profile.duration_seconds.toFixed(1)
      : "—";
  el.textContent = `${profile.style_name} · ${secs}s`;
}

async function remove(path) {
  if (!confirm(t("confirmDelete"))) return;
  try {
    await api(path, { method: "DELETE" });
    await refresh();
  } catch (error) {
    notice(error.message);
  }
}

/** Inline rename editor (speaker name or profile style). */
function startInlineRename({
  displayEl,
  current,
  path,
  bodyKey,
  maxLength = 100,
  extraHide = [],
  onRenderDisplay,
}) {
  if (!displayEl || displayEl.dataset.editing === "1") return;
  displayEl.dataset.editing = "1";

  const parent = displayEl.parentElement;
  if (!parent) return;

  const editor = document.createElement("div");
  editor.className = "inline-rename";
  editor.setAttribute("role", "group");

  const input = document.createElement("input");
  input.type = "text";
  input.className = "inline-rename-input";
  input.maxLength = maxLength;
  input.value = current || "";
  input.setAttribute("aria-label", t("renameSpeaker"));

  const saveBtn = document.createElement("button");
  saveBtn.type = "button";
  saveBtn.className = "compact";
  saveBtn.textContent = t("renameSave");

  const cancelBtn = document.createElement("button");
  cancelBtn.type = "button";
  cancelBtn.className = "secondary compact";
  cancelBtn.textContent = t("renameCancel");

  editor.append(input, saveBtn, cancelBtn);
  displayEl.classList.add("hidden");
  extraHide.forEach((node) => node && node.classList.add("hidden"));
  parent.insertBefore(editor, displayEl.nextSibling);
  input.focus();
  input.select();

  let closed = false;
  const cleanup = () => {
    if (closed) return;
    closed = true;
    editor.remove();
    displayEl.classList.remove("hidden");
    delete displayEl.dataset.editing;
    extraHide.forEach((node) => node && node.classList.remove("hidden"));
  };

  const cancel = () => {
    cleanup();
  };

  const save = async () => {
    const name = input.value.trim();
    if (!name) {
      notice(t("renameEmpty"));
      input.focus();
      return;
    }
    if (name.length > maxLength) {
      notice(t("renameTooLong"));
      input.focus();
      return;
    }
    if (name === current) {
      cleanup();
      return;
    }
    saveBtn.disabled = true;
    cancelBtn.disabled = true;
    input.disabled = true;
    try {
      await api(path, {
        method: "PATCH",
        body: JSON.stringify({ [bodyKey]: name }),
      });
      notice("");
      cleanup();
      await refresh();
    } catch (error) {
      notice(error.message);
      saveBtn.disabled = false;
      cancelBtn.disabled = false;
      input.disabled = false;
      input.focus();
    }
  };

  saveBtn.onclick = (event) => {
    event.preventDefault();
    event.stopPropagation();
    save();
  };
  cancelBtn.onclick = (event) => {
    event.preventDefault();
    event.stopPropagation();
    cancel();
  };
  input.onkeydown = (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      save();
    } else if (event.key === "Escape") {
      event.preventDefault();
      cancel();
    }
  };
  // Avoid bubbling into row/title click handlers while editing.
  editor.onclick = (event) => event.stopPropagation();
  editor.onmousedown = (event) => event.stopPropagation();
}

function stopStream(stream) {
  if (stream) {
    stream.getTracks().forEach((track) => track.stop());
    liveStreams.delete(stream);
  }
}

function initializeRecorder(form) {
  if (!form) return;
  const start = form.querySelector(".record-start");
  const stop = form.querySelector(".record-stop");
  const discard = form.querySelector(".record-discard");
  const timer = form.querySelector(".record-timer");
  const status = form.querySelector(".record-state");
  const preview = form.querySelector(".record-preview");
  const file = form.elements.namedItem("audio");
  if (!start || !stop || !discard || !timer || !status || !preview || !file) {
    return;
  }

  const data = {
    blob: null,
    url: null,
    stream: null,
    media: null,
    interval: null,
    started: 0,
    mime: "audio/webm",
  };
  recorders.set(form, data);

  const resetRecorder = () => {
    if (data.media?.state === "recording") data.media.stop();
    stopStream(data.stream);
    clearInterval(data.interval);
    data.stream = null;
    data.media = null;
    data.blob = null;
    if (data.url) URL.revokeObjectURL(data.url);
    data.url = null;
    preview.removeAttribute("src");
    preview.classList.add("hidden");
    discard.classList.add("hidden");
    stop.classList.add("hidden");
    start.classList.remove("hidden");
    timer.textContent = "00:00";
    status.textContent = "";
  };

  discard.onclick = resetRecorder;
  file.onchange = () => {
    if (file.files?.length) resetRecorder();
  };

  start.onclick = async () => {
    if (!window.isSecureContext || !navigator.mediaDevices?.getUserMedia) {
      status.textContent = t("insecureMic");
      return;
    }
    try {
      const stream = await navigator.mediaDevices.getUserMedia({
        audio: {
          channelCount: 1,
          echoCancellation: true,
          noiseSuppression: true,
        },
        video: false,
      });
      data.stream = stream;
      liveStreams.add(stream);
      const choices = [
        "audio/webm;codecs=opus",
        "audio/webm",
        "audio/ogg;codecs=opus",
        "audio/mp4",
      ];
      data.mime =
        choices.find((type) => MediaRecorder.isTypeSupported(type)) || "";
      data.media = new MediaRecorder(
        stream,
        data.mime ? { mimeType: data.mime } : undefined
      );
      const chunks = [];
      data.media.ondataavailable = (event) => {
        if (event.data.size) chunks.push(event.data);
      };
      data.media.onerror = () => {
        status.textContent = t("micDenied");
        clearInterval(data.interval);
        stopStream(data.stream);
      };
      data.media.onstop = () => {
        stopStream(data.stream);
        clearInterval(data.interval);
        data.blob = new Blob(chunks, {
          type: data.media.mimeType || data.mime || "audio/webm",
        });
        data.url = URL.createObjectURL(data.blob);
        preview.src = data.url;
        preview.classList.remove("hidden");
        discard.classList.remove("hidden");
        stop.classList.add("hidden");
        start.classList.remove("hidden");
        status.textContent = t("recordReady");
      };
      data.started = Date.now();
      data.media.start(250);
      file.value = "";
      start.classList.add("hidden");
      stop.classList.remove("hidden");
      status.textContent = t("recording");
      data.interval = setInterval(() => {
        const seconds = Math.floor((Date.now() - data.started) / 1000);
        timer.textContent = `${String(Math.floor(seconds / 60)).padStart(2, "0")}:${String(seconds % 60).padStart(2, "0")}`;
      }, 250);
    } catch {
      clearInterval(data.interval);
      stopStream(data.stream);
      status.textContent = window.isSecureContext
        ? t("micDenied")
        : t("insecureMic");
    }
  };

  stop.onclick = () => {
    if (data.media?.state === "recording") data.media.stop();
  };
}

async function uploadProfile(event, speakerId) {
  event.preventDefault();
  const form = event.currentTarget;
  if (!form) return;
  const data = new FormData(form);
  const recording = recorders.get(form);
  if (recording?.blob) {
    const type = recording.blob.type;
    const extension = type.includes("ogg")
      ? "ogg"
      : type.includes("mp4")
        ? "m4a"
        : "webm";
    data.set("audio", recording.blob, `recording.${extension}`);
  } else if (!form.elements.namedItem("audio")?.files?.length) {
    notice(t("audioRequired"));
    return;
  }
  data.set("consent", form.consent?.checked ? "true" : "false");
  try {
    notice(t("working"));
    await api(`/api/speakers/${speakerId}/profiles`, {
      method: "POST",
      body: data,
    });
    stopStream(recording?.stream);
    safeReset(form);
    await refresh();
    notice("");
  } catch (error) {
    notice(error.message);
  }
}

const languageButton = $("#language");
if (languageButton) {
  languageButton.onclick = () => {
    language = language === "zh" ? "en" : "zh";
    localStorage.setItem("vwa-language", language);
    translate();
    // Only re-render speaker cards when studio data is already loaded.
    if (!$("#studioView")?.classList.contains("hidden")) {
      renderSpeakers();
      renderPasskeys();
    }
  };
}

const setupForm = $("#setupForm");
if (setupForm) {
  setupForm.onsubmit = async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    try {
      await api("/api/setup", {
        method: "POST",
        body: JSON.stringify(Object.fromEntries(new FormData(form))),
      });
      safeReset(form);
      setView("loginView");
      notice("");
    } catch (error) {
      if (error.code === "already_configured" || error.status === 409) {
        setView("loginView");
        notice(t("alreadyConfigured"));
        return;
      }
      notice(error.message);
    }
  };
}

const loginForm = $("#loginForm");
if (loginForm) {
  loginForm.onsubmit = async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    try {
      await api("/api/auth/login", {
        method: "POST",
        body: JSON.stringify(Object.fromEntries(new FormData(form))),
      });
      safeReset(form);
      await boot();
      notice("");
    } catch (error) {
      notice(error.message);
    }
  };
}

const passkeyLoginButton = $("#passkeyLogin");
if (passkeyLoginButton) {
  passkeyLoginButton.onclick = async () => {
    try {
      if (!passkeysSupported()) throw new Error(passkeyUnsupportedMessage());
      const start = await api("/api/auth/passkeys/login/start", {
        method: "POST",
        body: "{}",
      });
      const credential = await navigator.credentials.get({
        publicKey: requestOptions(start.publicKey),
      });
      if (!credential) throw new Error(t("failed"));
      await api("/api/auth/passkeys/login/finish", {
        method: "POST",
        body: JSON.stringify({
          transaction_id: start.transaction_id,
          credential: serializeCredential(credential),
        }),
      });
      await boot();
      notice("");
    } catch (error) {
      notice(error.message);
    }
  };
}

const passkeyForm = $("#passkeyForm");
if (passkeyForm) {
  passkeyForm.onsubmit = async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    const button = form.querySelector("button");
    try {
      if (!passkeysSupported()) throw new Error(passkeyUnsupportedMessage());
      if (button) button.disabled = true;
      const name = String(new FormData(form).get("name") || "").trim();
      const start = await api("/api/auth/passkeys/register/start", {
        method: "POST",
        body: JSON.stringify({ name }),
      });
      const credential = await navigator.credentials.create({
        publicKey: creationOptions(start.publicKey),
      });
      if (!credential) throw new Error(t("failed"));
      await api("/api/auth/passkeys/register/finish", {
        method: "POST",
        body: JSON.stringify({
          transaction_id: start.transaction_id,
          credential: serializeCredential(credential),
        }),
      });
      safeReset(form);
      await refresh();
      notice("");
    } catch (error) {
      notice(error.message);
    } finally {
      if (button) button.disabled = false;
    }
  };
}

const logoutButton = $("#logout");
if (logoutButton) {
  logoutButton.onclick = async () => {
    try {
      await api("/api/auth/logout", { method: "POST", body: "{}" });
      stopModelDownloadPolling();
      await boot();
      notice("");
    } catch (error) {
      notice(error.message);
    }
  };
}

const modelDownloadButton = $("#modelDownload");
if (modelDownloadButton) {
  modelDownloadButton.onclick = async () => {
    if (!confirm(t("confirmModelDownload"))) return;
    try {
      modelDownloadButton.disabled = true;
      modelDownloadButton.textContent = t("downloadingModel");
      await api("/api/model/download", { method: "POST", body: "{}" });
      notice("");
      await refreshModelDownloadStatus();
    } catch (error) {
      modelDownloadButton.disabled = false;
      modelDownloadButton.textContent = t("downloadModel");
      if (error.code === "model_download_running") {
        await refreshModelDownloadStatus();
      } else {
        notice(error.message);
      }
    }
  };
}

const speakerForm = $("#speakerForm");
if (speakerForm) {
  speakerForm.onsubmit = async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    try {
      await api("/api/speakers", {
        method: "POST",
        body: JSON.stringify(Object.fromEntries(new FormData(form))),
      });
      // Capture form before await; currentTarget may be null afterwards.
      safeReset(form);
      await refresh();
      notice("");
    } catch (error) {
      notice(error.message);
    }
  };
}

const speed = $("#speed");
const speedValue = $("#speedValue");
if (speed && speedValue) {
  speed.oninput = (event) => {
    speedValue.textContent = `${Number(event.target.value).toFixed(2)}×`;
  };
}

const subtitleForm = $("#subtitleForm");if (subtitleForm) {
  subtitleForm.onsubmit = async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    const button = $("#subtitleButton");
    const resultBox = $("#subtitleResult");
    const segmentsBox = $("#subtitleSegments");
    const download = $("#subtitleDownload");
    if (!resultBox || !segmentsBox || !download) return;
    if (button) {
      button.disabled = true;
      button.textContent = t("working");
    }
    notice("");
    try {
      const data = await api("/api/videos/subtitles", {
        method: "POST",
        body: JSON.stringify(Object.fromEntries(new FormData(form))),
      });
      const segments = Array.isArray(data?.segments) ? data.segments : [];
      segmentsBox.textContent = "";
      if (!segments.length) {
        const empty = document.createElement("p");
        empty.className = "hint segment-empty";
        empty.textContent = t("subtitlesEmpty");
        segmentsBox.appendChild(empty);
      } else {
        for (const seg of segments) {
          const row = document.createElement("div");
          row.className = "segment";
          const time = document.createElement("span");
          time.className = "segment-time";
          time.textContent = `${seg.start} → ${seg.end}`;
          const text = document.createElement("span");
          text.className = "segment-text";
          text.textContent = seg.text || "";
          row.append(time, text);
          segmentsBox.appendChild(row);
        }
      }
      if (data?.srt) {
        if (download.dataset.url) URL.revokeObjectURL(download.dataset.url);
        const url = URL.createObjectURL(
          new Blob([data.srt], { type: "application/x-subrip" }),
        );
        download.dataset.url = url;
        download.href = url;
        const raw = String(new FormData(form).get("video_path") || "subtitles");
        const base =
          raw.replace(/\.[^.]+$/, "").replace(/[^\w.-]+/g, "_") || "subtitles";
        download.download = `${base}.srt`;
        download.classList.remove("hidden");
      } else {
        download.classList.add("hidden");
      }
      resultBox.classList.remove("hidden");
    } catch (error) {
      resultBox.classList.add("hidden");
      notice(error.message);
    } finally {
      if (button) {
        button.disabled = false;
        button.textContent = t("extractSubtitles");
      }
    }
  };
}

const generateForm = $("#generateForm");
if (generateForm) {
  generateForm.onsubmit = async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    const profileSelect = $("#profileSelect");
    if (!profileSelect?.value) {
      notice(t("failed"));
      return;
    }
    const [speaker_id, profile_id] = profileSelect.value.split("|");
    const data = Object.fromEntries(new FormData(form));
    data.speaker_id = speaker_id;
    data.profile_id = profile_id;
    data.speed = Number(data.speed);
    const button = $("#generateButton");
    try {
      if (button) button.disabled = true;
      notice(t("working"));
      const result = await api("/api/generations", {
        method: "POST",
        body: JSON.stringify(data),
      });
      const player = $("#player");
      const download = $("#download");
      const resultBox = $("#result");
      if (player) player.src = result.audio_url;
      if (download) {
        download.href = result.audio_url;
        // Prefer server-provided 前缀…后缀.wav; fall back to local abbreviate.
        const name =
          result.download_name ||
          downloadNameFromText(String(data.target_text || ""));
        download.setAttribute("download", name);
      }
      if (resultBox) resultBox.classList.remove("hidden");
      notice("");
    } catch (error) {
      notice(error.message);
    } finally {
      if (button) button.disabled = false;
    }
  };
}

boot();
window.addEventListener("beforeunload", () => {
  stopModelDownloadPolling();
  liveStreams.forEach(stopStream);
});

const studioLayout = document.querySelector(".studio");
const libraryToggle = $("#libraryToggle");
if (studioLayout && libraryToggle) {
  const key = "vwa-library-collapsed";
  const narrow = matchMedia("(max-width: 1024px)");
  const apply = (collapsed) =>
    studioLayout.classList.toggle("library-collapsed", collapsed);
  const savedPreference = () => {
    try {
      return localStorage.getItem(key);
    } catch {
      return null;
    }
  };
  const saved = savedPreference();
  apply(saved === null ? narrow.matches : saved === "1");
  libraryToggle.addEventListener("click", () => {
    const collapsed = !studioLayout.classList.contains("library-collapsed");
    apply(collapsed);
    try {
      localStorage.setItem(key, collapsed ? "1" : "0");
    } catch {
      /* storage unavailable */
    }
  });
  narrow.addEventListener?.("change", (event) => {
    if (savedPreference() === null) apply(event.matches);
  });
}
