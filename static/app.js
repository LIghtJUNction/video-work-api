const translations = {
  zh: {
    title: "视频工作 API",
    workspaceEyebrow: "VOICE WORKSPACE",
    workspaceTitle: "把声音，变成可复用的创作资产。",
    workspaceLead: "导入有明确授权的参考音频，保存精确逐字稿，然后生成自然、稳定的语音。",
    endpointLabel: "MCP SERVER",
    mcpHint: "MCP 服务器运行在此端点，使用 Bearer Token 认证。",
    copyAgentPrompt: "复制 Agent 提示词",
    agentPromptCopied: "已复制 ✓",
    agentPromptCopyFailed: "复制失败，请手动复制",
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
    generateLead: "每个非空行生成一个独立 WAV，最多 50 条。",
    voiceStyle: "说话人 / 语气",
    targetText: "目标文案（每个非空行一条）",
    speed: "语速",
    generateButton: "生成音频",
    generateSingleButton: "仅生成一条",
    download: "下载 WAV",
    resultReady: "生成完成",
    itemCount: "{count} 条 · 共 {chars} 字符",
    generateItems: "生成 {count} 条",
    emptyGeneration: "请至少输入一条非空文案。",
    profileRequired: "请先选择说话人 / 语气。",
    tooManyItems: "一次最多处理 50 条。",
    textTooLong: "第 {index} 条超过 1200 字符（当前 {chars}）。",
    batchProgress: "完成 {done} / {total} · 失败 {failed}",
    pending: "等待中",
    running: "处理中",
    complete: "已完成",
    failedStatus: "失败",
    retryItem: "重试此项",
    retryFailures: "重试失败项（{count}）",
    itemPreview: "文案",
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
    subtitlesLead: "可合并目录文件名和本地上传，按顺序提取字幕。",
    videoPath: "视频文件名（每个非空行一项）",
    videoUpload: "也可上传视频文件（每个 ≤2 GB）",
    videoPathOrFileRequired: "请输入文件名或选择视频文件。",
    chooseVideo: "选择视频",
    noFileChosen: "未选择文件",
    filesChosen: "已选 {count} 个：{names}",
    clear: "清空",
    clearFiles: "清空已选文件",
    subtitleItemCount: "共 {count} 项（路径 {paths} · 上传 {files}）",
    extractSubtitleItems: "提取 {count} 项字幕",
    fileTooLarge: "文件“{name}”超过 2 GB。",
    pathTooLong: "第 {index} 个路径超过 500 字符（当前 {chars}）。",
    subtitlesHint: "首次提取会下载 ASR 模型，长视频可能需要几分钟。",
    extractSubtitles: "提取字幕",
    downloadSrt: "下载 SRT",
    subtitlesEmpty: "未识别到字幕片段。",
    subtitlePreview: "字幕片段预览",
  },
  en: {
    title: "Video Work API",
    workspaceEyebrow: "VOICE WORKSPACE",
    workspaceTitle: "Turn every voice into a reusable creative asset.",
    workspaceLead: "Import an explicitly authorized reference, keep its exact transcript, and generate natural, consistent speech.",
    endpointLabel: "MCP SERVER",
    mcpHint: "The MCP server runs at this endpoint with Bearer Token authentication.",
    copyAgentPrompt: "Copy agent prompt",
    agentPromptCopied: "Copied ✓",
    agentPromptCopyFailed: "Copy failed — please copy manually",
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
    generateLead: "Each non-empty line creates a separate WAV, up to 50 items.",
    voiceStyle: "Speaker / style",
    targetText: "Target text (one non-empty line per item)",
    speed: "Speed",
    generateButton: "Generate audio",
    generateSingleButton: "Generate one",
    download: "Download WAV",
    resultReady: "Generation complete",
    itemCount: "{count} items · {chars} characters total",
    generateItems: "Generate {count} items",
    emptyGeneration: "Enter at least one non-empty line.",
    profileRequired: "Select a speaker / style first.",
    tooManyItems: "A batch can contain at most 50 items.",
    textTooLong: "Item {index} exceeds 1,200 characters ({chars}).",
    batchProgress: "Completed {done} / {total} · Failed {failed}",
    pending: "Pending",
    running: "Running",
    complete: "Complete",
    failedStatus: "Failed",
    retryItem: "Retry item",
    retryFailures: "Retry failed ({count})",
    itemPreview: "Copy",
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
    subtitlesLead: "Combine filenames from the videos directory with local uploads and process them in order.",
    videoPath: "Video filenames (one non-empty line per item)",
    videoUpload: "You can also upload videos (≤2 GB each)",
    videoPathOrFileRequired: "Enter a filename or choose a video file.",
    chooseVideo: "Choose video",
    noFileChosen: "No file chosen",
    filesChosen: "{count} selected: {names}",
    clear: "Clear",
    clearFiles: "Clear selected files",
    subtitleItemCount: "{count} total ({paths} paths · {files} uploads)",
    extractSubtitleItems: "Extract {count} subtitle items",
    fileTooLarge: "“{name}” exceeds 2 GB.",
    pathTooLong: "Path {index} exceeds 500 characters ({chars}).",
    subtitlesHint: "The first extraction downloads the ASR model; long videos may take a few minutes.",
    extractSubtitles: "Extract subtitles",
    downloadSrt: "Download SRT",
    subtitlesEmpty: "No subtitle segments detected.",
    subtitlePreview: "Subtitle segment preview",
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
let generationRunning = false;
let subtitleRunning = false;
let generationJobs = [];
let subtitleJobs = [];
let generationEpoch = 0;
let subtitleEpoch = 0;
let logoutInProgress = false;

const {
  characterCount,
  parseNonEmptyLines,
  parseWholeTextItem,
  validateItems,
  runSequential,
} = window.BatchCore;

const $ = (selector) => document.querySelector(selector);

function t(key) {
  return translations[language][key] || recordTranslations[language][key] || key;
}

function tf(key, values = {}) {
  return Object.entries(values).reduce(
    (text, [name, value]) => text.replaceAll(`{${name}}`, String(value)),
    t(key),
  );
}

const agentPromptAccess = window.AgentPrompt.createPromptAccessController(
  (available) => {
    const button = $("#copyAgentPrompt");
    if (!button) return;
    button.classList.toggle("hidden", !available);
    if (!available) {
      button.disabled = false;
      button.textContent = t("copyAgentPrompt");
    }
  },
);

function shortPreview(value, limit = 72) {
  const chars = Array.from(String(value || "").replace(/\s+/g, " ").trim());
  return chars.length <= limit ? chars.join("") : `${chars.slice(0, limit).join("")}…`;
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
  document.querySelectorAll("[data-i18n-aria-label]").forEach((node) => {
    node.setAttribute("aria-label", t(node.dataset.i18nAriaLabel));
  });
  const languageButton = $("#language");
  if (languageButton) {
    languageButton.textContent = language === "zh" ? "English" : "中文";
  }
  const passkeyLoginSupport = $("#passkeyLoginSupport");
  if (passkeyLoginSupport) {
    passkeyLoginSupport.textContent = passkeyUnsupportedMessage();
  }
  syncGenerationInput();
  syncSubtitleInput();
  localizeGenerationJobsInPlace();
  localizeSubtitleJobsInPlace();
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

function setAgentPromptAvailable(available) {
  agentPromptAccess.setAvailable(Boolean(available));
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
    if (response.status === 401) setAgentPromptAvailable(false);
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
  if (name !== "studioView") setAgentPromptAvailable(false);
}

async function boot() {
  translate();
  setAgentPromptAvailable(false);
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
      setAgentPromptAvailable(status.mcp?.configured === true);
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
    if (logoutInProgress) return;
    logoutInProgress = true;
    logoutButton.disabled = true;
    setAgentPromptAvailable(false);
    clearBatchState(false);
    try {
      await api("/api/auth/logout", { method: "POST", body: "{}" });
      clearBatchState(false);
      stopModelDownloadPolling();
      await boot();
      clearBatchState(true);
      notice("");
    } catch (error) {
      clearBatchState(true);
      notice(error.message);
    } finally {
      logoutInProgress = false;
      logoutButton.disabled = false;
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

const MAX_BATCH_ITEMS = 50;
const MAX_GENERATION_CHARS = 1200;
const MAX_VIDEO_BYTES = 2 * 1024 * 1024 * 1024;

function revokeSubtitleUrls(jobs = subtitleJobs) {
  jobs.forEach((job) => {
    if (!job.downloadUrl) return;
    URL.revokeObjectURL(job.downloadUrl);
    job.downloadUrl = "";
  });
}

function clearBatchState(unlock = true) {
  generationEpoch += 1;
  subtitleEpoch += 1;
  revokeSubtitleUrls();
  generationJobs = [];
  subtitleJobs = [];
  generationRunning = false;
  subtitleRunning = false;
  setGenerationControlsDisabled(!unlock);
  setSubtitleControlsDisabled(!unlock);
  $("#generationJobs")?.replaceChildren();
  $("#subtitleJobs")?.replaceChildren();
  updateBatchProgress("generation", generationJobs);
  updateBatchProgress("subtitle", subtitleJobs);
}

function statusText(status) {
  return t(status === "failed" ? "failedStatus" : status);
}

function localizeGenerationJobsInPlace() {
  generationJobs.forEach((job) => {
    if (!job.row || !job.statusNode) return;
    job.statusNode.textContent = statusText(job.status);
    const download = job.content?.querySelector("a[download]");
    if (download) download.textContent = t("download");
    const retry = job.content?.querySelector(".retry-item");
    if (retry) retry.textContent = t("retryItem");
  });
  updateBatchProgress("generation", generationJobs);
}

function localizeSubtitleJobsInPlace() {
  subtitleJobs.forEach((job) => {
    if (!job.row || !job.statusNode) return;
    job.statusNode.textContent = statusText(job.status);
    const download = job.content?.querySelector("a[download]");
    if (download) download.textContent = t("downloadSrt");
    const retry = job.content?.querySelector(".retry-item");
    if (retry) retry.textContent = t("retryItem");
    const summary = job.content?.querySelector("details > summary");
    if (summary) {
      summary.textContent = `${t("subtitlePreview")} (${job.result?.segments?.length || 0})`;
    }
    const empty = job.content?.querySelector(".segment-empty");
    if (empty) empty.textContent = t("subtitlesEmpty");
  });
  updateBatchProgress("subtitle", subtitleJobs);
}

function batchCounts(jobs) {
  return {
    done: jobs.filter((job) => job.status === "complete" || job.status === "failed").length,
    failed: jobs.filter((job) => job.status === "failed").length,
  };
}

function updateBatchProgress(kind, jobs) {
  const prefix = kind === "generation" ? "generation" : "subtitle";
  const box = $(`#${prefix}Batch`);
  const progress = $(`#${prefix}Progress`);
  const text = $(`#${prefix}ProgressText`);
  const retry = $(`#retry${kind === "generation" ? "Generation" : "Subtitle"}Failures`);
  if (!box || !progress || !text || !retry) return;
  box.classList.toggle("hidden", jobs.length === 0);
  const counts = batchCounts(jobs);
  progress.max = Math.max(jobs.length, 1);
  progress.value = counts.done;
  text.textContent = tf("batchProgress", { ...counts, total: jobs.length });
  retry.textContent = tf("retryFailures", { count: counts.failed });
  retry.classList.toggle("hidden", counts.failed === 0 || jobs.some((job) => job.status === "running"));
}

function setGenerationControlsDisabled(disabled) {
  [
    $("#profileSelect"),
    $("#targetText"),
    $("#speed"),
    $("#generateButton"),
    $("#generateSingleButton"),
  ].forEach((node) => {
    if (node) node.disabled = disabled;
  });
  document.querySelectorAll("#generationJobs .retry-item").forEach((node) => {
    node.disabled = disabled;
  });
}

function setGenerationLocked(locked) {
  generationRunning = locked;
  setGenerationControlsDisabled(locked);
}

function generationValidation(lines = parseNonEmptyLines($("#targetText")?.value)) {
  const validation = validateItems(lines, {
    maxItems: MAX_BATCH_ITEMS,
    maxChars: MAX_GENERATION_CHARS,
  });
  if (!validation) return "";
  if (validation.type === "empty") return t("emptyGeneration");
  if (validation.type === "too_many") return t("tooManyItems");
  return tf("textTooLong", {
    index: validation.index + 1,
    chars: validation.count,
  });
}

function setGenerationError(message, field = "text") {
  const input = $("#targetText");
  const profile = $("#profileSelect");
  const error = $("#generationError");
  if (error) error.textContent = message;
  if (input) input.setAttribute("aria-invalid", "false");
  if (profile) profile.setAttribute("aria-invalid", "false");
  const invalid = field === "profile" ? profile : input;
  if (message && invalid) invalid.setAttribute("aria-invalid", "true");
}

function syncGenerationInput() {
  const input = $("#targetText");
  const count = $("#generationCount");
  const error = $("#generationError");
  const button = $("#generateButton");
  if (!input || !count || !error || !button) return;
  const lines = parseNonEmptyLines(input.value);
  const chars = lines.reduce((sum, line) => sum + characterCount(line), 0);
  count.textContent = tf("itemCount", { count: lines.length, chars });
  const validation = generationValidation(lines);
  setGenerationError(validation, "text");
  button.textContent = tf("generateItems", { count: lines.length });
}

function updateGenerationJob(job) {
  if (!job.row) return;
  job.row.className = `job-row status-${job.status}`;
  job.statusNode.textContent = statusText(job.status);
  job.content.replaceChildren();
  if (job.status === "complete" && job.result?.audio_url) {
    job.content.className = "job-content generation-output";
    const audio = document.createElement("audio");
    audio.controls = true;
    audio.preload = "none";
    audio.src = job.result.audio_url;
    const download = document.createElement("a");
    download.className = "button secondary compact";
    download.href = job.result.audio_url;
    download.download = job.result.download_name || downloadNameFromText(job.text);
    download.textContent = t("download");
    job.content.append(audio, download);
  } else if (job.status === "failed") {
    job.content.className = "job-content";
    const error = document.createElement("p");
    error.className = "job-error";
    error.textContent = job.error;
    const retry = document.createElement("button");
    retry.type = "button";
    retry.className = "secondary compact retry-item";
    retry.textContent = t("retryItem");
    retry.disabled = generationRunning;
    retry.onclick = () => runGenerationJobs([job]);
    job.content.append(error, retry);
  }
  updateBatchProgress("generation", generationJobs);
}

function renderGenerationJobs() {
  const root = $("#generationJobs");
  if (!root) return;
  root.replaceChildren();
  generationJobs.forEach((job, index) => {
    const row = document.createElement("article");
    const head = document.createElement("div");
    head.className = "job-head";
    const title = document.createElement("strong");
    title.textContent = `#${index + 1} · ${shortPreview(job.text)}`;
    title.title = job.text;
    const badge = document.createElement("span");
    badge.className = "job-status";
    head.append(title, badge);
    const content = document.createElement("div");
    row.append(head, content);
    job.row = row;
    job.statusNode = badge;
    job.content = content;
    root.appendChild(row);
    updateGenerationJob(job);
  });
  updateBatchProgress("generation", generationJobs);
}

async function runGenerationJobs(jobs) {
  if (generationRunning || logoutInProgress || !jobs.length) return;
  const epoch = generationEpoch;
  setGenerationLocked(true);
  await runSequential(
    jobs,
    (job) =>
      api("/api/generations", {
        method: "POST",
        body: JSON.stringify(job.request),
      }),
    updateGenerationJob,
    () => epoch !== generationEpoch,
  );
  if (epoch !== generationEpoch) return;
  setGenerationLocked(false);
  syncGenerationInput();
  updateBatchProgress("generation", generationJobs);
}

async function startGeneration(items) {
  if (generationRunning || logoutInProgress) return;
  const error = generationValidation(items);
  if (error) {
    setGenerationError(error, "text");
    return;
  }
  const profile = $("#profileSelect")?.value;
  if (!profile) {
    setGenerationError(t("profileRequired"), "profile");
    return;
  }
  const [speaker_id, profile_id] = profile.split("|");
  const speedSnapshot = Number($("#speed")?.value || 1);
  setGenerationError("");
  generationEpoch += 1;
  generationJobs = items.map((text) => ({
    text,
    status: "pending",
    error: "",
    result: null,
    request: { speaker_id, profile_id, target_text: text, speed: speedSnapshot },
  }));
  renderGenerationJobs();
  notice("");
  await runGenerationJobs(generationJobs);
}

const targetText = $("#targetText");
if (targetText) targetText.addEventListener("input", syncGenerationInput);
const profileSelect = $("#profileSelect");
if (profileSelect) profileSelect.addEventListener("change", syncGenerationInput);

const generateForm = $("#generateForm");
if (generateForm) {
  generateForm.onsubmit = async (event) => {
    event.preventDefault();
    await startGeneration(parseNonEmptyLines($("#targetText")?.value));
  };
}

const generateSingleButton = $("#generateSingleButton");
if (generateSingleButton) {
  generateSingleButton.onclick = () =>
    startGeneration(parseWholeTextItem($("#targetText")?.value));
}

const retryGenerationFailures = $("#retryGenerationFailures");
if (retryGenerationFailures) {
  retryGenerationFailures.onclick = () =>
    runGenerationJobs(generationJobs.filter((job) => job.status === "failed"));
}

const videoFileInput = $("#videoFileInput");
const videoFileButton = $("#videoFileButton");
const videoFileName = $("#videoFileName");
const videoFileClear = $("#videoFileClear");

function subtitleSources() {
  return {
    paths: parseNonEmptyLines($("#videoPaths")?.value),
    files: Array.from(videoFileInput?.files || []),
  };
}

function subtitleValidation(sources = subtitleSources()) {
  const total = sources.paths.length + sources.files.length;
  if (!total) return t("videoPathOrFileRequired");
  if (total > MAX_BATCH_ITEMS) return t("tooManyItems");
  const pathValidation = sources.paths.length
    ? validateItems(sources.paths, { maxItems: MAX_BATCH_ITEMS, maxChars: 500 })
    : null;
  if (pathValidation?.type === "too_long") {
    return tf("pathTooLong", {
      index: pathValidation.index + 1,
      chars: pathValidation.count,
    });
  }
  const oversized = sources.files.find((file) => file.size > MAX_VIDEO_BYTES);
  return oversized ? tf("fileTooLarge", { name: oversized.name }) : "";
}

function syncSubtitleInput() {
  const { paths, files } = subtitleSources();
  const summary = $("#videoFileName");
  const clear = $("#videoFileClear");
  const count = $("#subtitleCount");
  const error = $("#subtitleError");
  const button = $("#subtitleButton");
  const names = files.slice(0, 3).map((file) => file.name).join(", ");
  if (summary) {
    summary.textContent = files.length
      ? tf("filesChosen", { count: files.length, names: `${names}${files.length > 3 ? "…" : ""}` })
      : t("noFileChosen");
    summary.classList.toggle("has-file", files.length > 0);
  }
  if (clear) clear.classList.toggle("hidden", files.length === 0);
  if (count) count.textContent = tf("subtitleItemCount", { count: paths.length + files.length, paths: paths.length, files: files.length });
  const validation = subtitleValidation({ paths, files });
  if (error) error.textContent = validation;
  const pathsInput = $("#videoPaths");
  if (pathsInput) pathsInput.setAttribute("aria-invalid", String(Boolean(validation)));
  if (videoFileInput) videoFileInput.setAttribute("aria-invalid", String(Boolean(validation)));
  if (button) button.textContent = tf("extractSubtitleItems", { count: paths.length + files.length });
}

function setSubtitleControlsDisabled(disabled) {
  [$("#videoPaths"), videoFileInput, videoFileButton, videoFileClear, $("#subtitleButton")].forEach((node) => {
    if (node) node.disabled = disabled;
  });
  document.querySelectorAll("#subtitleJobs .retry-item").forEach((node) => {
    node.disabled = disabled;
  });
}

function setSubtitleLocked(locked) {
  subtitleRunning = locked;
  setSubtitleControlsDisabled(locked);
}

function subtitleDownloadName(label) {
  const base = String(label || "subtitles").replace(/\.[^.]+$/, "").replace(/[^\w.-]+/g, "_") || "subtitles";
  return `${base}.srt`;
}

function appendSegments(root, segments) {
  if (!segments.length) {
    const empty = document.createElement("p");
    empty.className = "hint segment-empty";
    empty.textContent = t("subtitlesEmpty");
    root.appendChild(empty);
    return;
  }
  segments.forEach((seg) => {
    const row = document.createElement("div");
    row.className = "segment";
    const time = document.createElement("span");
    time.className = "segment-time";
    time.textContent = `${seg.start} → ${seg.end}`;
    const text = document.createElement("span");
    text.className = "segment-text";
    text.textContent = seg.text || "";
    row.append(time, text);
    root.appendChild(row);
  });
}

function updateSubtitleJob(job) {
  if (!job.row) return;
  job.row.className = `job-row status-${job.status}`;
  job.statusNode.textContent = statusText(job.status);
  job.content.replaceChildren();
  if (job.status === "complete") {
    job.content.className = "job-content subtitle-output";
    if (job.result?.srt) {
      if (!job.downloadUrl) {
        job.downloadUrl = URL.createObjectURL(
          new Blob([job.result.srt], { type: "application/x-subrip" }),
        );
      }
      const download = document.createElement("a");
      download.className = "button secondary compact";
      download.href = job.downloadUrl;
      download.download = subtitleDownloadName(job.label);
      download.textContent = t("downloadSrt");
      job.content.appendChild(download);
    }
    const details = document.createElement("details");
    const summary = document.createElement("summary");
    summary.textContent = `${t("subtitlePreview")} (${job.result?.segments?.length || 0})`;
    details.appendChild(summary);
    details.addEventListener("toggle", () => {
      if (!details.open || details.dataset.materialized) return;
      const segments = document.createElement("div");
      segments.className = "segments";
      appendSegments(
        segments,
        Array.isArray(job.result?.segments) ? job.result.segments : [],
      );
      details.appendChild(segments);
      details.dataset.materialized = "true";
    });
    job.content.appendChild(details);
  } else if (job.status === "failed") {
    job.content.className = "job-content";
    const error = document.createElement("p");
    error.className = "job-error";
    error.textContent = job.error;
    const retry = document.createElement("button");
    retry.type = "button";
    retry.className = "secondary compact retry-item";
    retry.textContent = t("retryItem");
    retry.disabled = subtitleRunning;
    retry.onclick = () => runSubtitleJobs([job]);
    job.content.append(error, retry);
  }
  updateBatchProgress("subtitle", subtitleJobs);
}

function renderSubtitleJobs() {
  const root = $("#subtitleJobs");
  if (!root) return;
  root.replaceChildren();
  subtitleJobs.forEach((job, index) => {
    const row = document.createElement("article");
    const head = document.createElement("div");
    head.className = "job-head";
    const title = document.createElement("strong");
    title.textContent = `#${index + 1} · ${shortPreview(job.label)}`;
    title.title = job.label;
    const badge = document.createElement("span");
    badge.className = "job-status";
    head.append(title, badge);
    const content = document.createElement("div");
    row.append(head, content);
    job.row = row;
    job.statusNode = badge;
    job.content = content;
    root.appendChild(row);
    updateSubtitleJob(job);
  });
  updateBatchProgress("subtitle", subtitleJobs);
}

async function runSubtitleJobs(jobs) {
  if (subtitleRunning || logoutInProgress || !jobs.length) return;
  const epoch = subtitleEpoch;
  setSubtitleLocked(true);
  jobs.forEach((job) => {
    if (job.downloadUrl) {
      URL.revokeObjectURL(job.downloadUrl);
      job.downloadUrl = "";
    }
  });
  await runSequential(
    jobs,
    async (job) => {
      if (job.kind === "file") {
        const body = new FormData();
        body.append("video", job.file);
        return api("/api/videos/subtitles/upload", { method: "POST", body });
      }
      return api("/api/videos/subtitles", {
        method: "POST",
        body: JSON.stringify({ video_path: job.path }),
      });
    },
    updateSubtitleJob,
    () => epoch !== subtitleEpoch,
  );
  if (epoch !== subtitleEpoch) return;
  setSubtitleLocked(false);
  syncSubtitleInput();
  updateBatchProgress("subtitle", subtitleJobs);
}

if (videoFileInput && videoFileButton && videoFileClear) {
  videoFileButton.addEventListener("click", () => videoFileInput.click());
  videoFileInput.addEventListener("change", syncSubtitleInput);
  videoFileClear.addEventListener("click", () => {
    videoFileInput.value = "";
    syncSubtitleInput();
  });
}
const videoPaths = $("#videoPaths");
if (videoPaths) videoPaths.addEventListener("input", syncSubtitleInput);

const subtitleForm = $("#subtitleForm");
if (subtitleForm) {
  subtitleForm.onsubmit = async (event) => {
    event.preventDefault();
    if (subtitleRunning || logoutInProgress) return;
    const sources = subtitleSources();
    const error = subtitleValidation(sources);
    syncSubtitleInput();
    if (error) return;
    revokeSubtitleUrls();
    subtitleEpoch += 1;
    subtitleJobs = [
      ...sources.paths.map((path) => ({ kind: "path", path, label: path, status: "pending", error: "", result: null })),
      ...sources.files.map((file) => ({ kind: "file", file, label: file.name, status: "pending", error: "", result: null })),
    ];
    renderSubtitleJobs();
    notice("");
    await runSubtitleJobs(subtitleJobs);
  };
}

const retrySubtitleFailures = $("#retrySubtitleFailures");
if (retrySubtitleFailures) {
  retrySubtitleFailures.onclick = () => runSubtitleJobs(subtitleJobs.filter((job) => job.status === "failed"));
}

async function copyTextToClipboard(text) {
  if (navigator.clipboard && window.isSecureContext) {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      /* fall through to legacy path */
    }
  }
  const area = document.createElement("textarea");
  area.value = text;
  area.setAttribute("readonly", "");
  area.style.position = "fixed";
  area.style.top = "-9999px";
  document.body.appendChild(area);
  area.select();
  let ok = false;
  try {
    ok = document.execCommand("copy");
  } catch {
    ok = false;
  }
  area.remove();
  return ok;
}

const copyAgentPromptButton = $("#copyAgentPrompt");
if (copyAgentPromptButton) {
  let resetTimer = null;
  copyAgentPromptButton.addEventListener("click", async () => {
    const attempt = agentPromptAccess.begin();
    if (!attempt) return;
    let promptText = null;
    let receivedToken = null;
    let token = null;
    try {
      copyAgentPromptButton.disabled = true;
      const response = await api("/api/auth/mcp-token", {
        method: "POST",
        body: "{}",
        signal: attempt.signal,
      });
      receivedToken = response.token;
      response.token = null;
      if (!agentPromptAccess.storeToken(attempt.id, receivedToken)) {
        receivedToken = null;
        return;
      }
      receivedToken = null;
      if (!agentPromptAccess.isCurrent(attempt.id)) return;
      token = agentPromptAccess.takeToken(attempt.id);
      if (!token || !agentPromptAccess.isCurrent(attempt.id)) return;
      promptText = window.AgentPrompt.buildAgentPrompt(
        language,
        `${window.location.origin}/mcp`,
        token,
      );
      token = null;
      if (!agentPromptAccess.isCurrent(attempt.id)) return;
      const ok = await copyTextToClipboard(promptText);
      if (!agentPromptAccess.isCurrent(attempt.id)) return;
      if (!ok) {
        if (!agentPromptAccess.isCurrent(attempt.id)) return;
        window.prompt(t("agentPromptCopyFailed"), promptText);
      }
      if (!agentPromptAccess.isCurrent(attempt.id)) return;
      copyAgentPromptButton.textContent = ok
        ? t("agentPromptCopied")
        : t("copyAgentPrompt");
      if (resetTimer) clearTimeout(resetTimer);
      const feedbackEpoch = attempt.id;
      resetTimer = setTimeout(() => {
        if (agentPromptAccess.currentEpoch() !== feedbackEpoch) return;
        copyAgentPromptButton.textContent = t("copyAgentPrompt");
        copyAgentPromptButton.disabled = false;
        resetTimer = null;
      }, 1600);
    } catch (error) {
      if (!agentPromptAccess.isCurrent(attempt.id) || error.name === "AbortError") {
        return;
      }
      if (error.code === "mcp_not_configured") {
        setAgentPromptAvailable(false);
      }
      notice(error.message);
      copyAgentPromptButton.disabled = false;
    } finally {
      receivedToken = null;
      token = null;
      promptText = null;
      agentPromptAccess.finish(attempt.id);
    }
  });
}

boot();
window.addEventListener("beforeunload", (event) => {
  if (generationRunning || subtitleRunning) {
    event.preventDefault();
    event.returnValue = "";
  }
});
window.addEventListener("pagehide", (event) => {
  if (event.persisted) return;
  stopModelDownloadPolling();
  liveStreams.forEach(stopStream);
  revokeSubtitleUrls();
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
