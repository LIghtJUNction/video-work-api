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
    setupTitle: "首次设置",
    setupHelp: "运行 vwactl init 后，将一次性令牌和新密码填入此处。",
    token: "一次性令牌",
    password: "管理员密码（至少 12 位）",
    passwordShort: "密码",
    finishSetup: "完成设置",
    loginTitle: "管理员登录",
    login: "登录",
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
    addProfile: "添加参考音频",
    styleName: "语气名称",
    audioFile: "音频（推荐 8–15 秒）",
    transcript: "参考录音逐字稿",
    rights: "我确认拥有克隆及使用该声音的明确权利和同意。",
    upload: "上传并转换",
    deleteProfile: "删除",
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
    setupTitle: "First-time setup",
    setupHelp: "Run vwactl init, then enter the one-time token and a new password.",
    token: "One-time token",
    password: "Admin password (12+ characters)",
    passwordShort: "Password",
    finishSetup: "Complete setup",
    loginTitle: "Admin sign in",
    login: "Sign in",
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
    addProfile: "Add reference audio",
    styleName: "Style name",
    audioFile: "Audio (8–15 seconds recommended)",
    transcript: "Exact reference transcript",
    rights: "I confirm I have explicit rights and consent to clone and use this voice.",
    upload: "Upload and convert",
    deleteProfile: "Delete",
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
let state = { speakers: [] };
const recorders = new WeakMap();
const liveStreams = new Set();

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
      setView("setupView");
    } else if (!status.authenticated) {
      setView("loginView");
    } else {
      setView("studioView");
      await refresh();
    }
  } catch (error) {
    notice(error.message);
  }
}

async function refresh() {
  state = await api("/api/speakers");
  if (!state || !Array.isArray(state.speakers)) {
    state = { speakers: [] };
  }
  renderSpeakers();
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
    article.querySelector("h3").textContent = speaker.name;
    article.querySelector(".delete-speaker").onclick = () =>
      remove(`/api/speakers/${speaker.id}`);

    const profiles = article.querySelector(".profiles");
    if (!speaker.profiles.length) {
      profiles.textContent = t("noProfiles");
    }
    speaker.profiles.forEach((profile) => {
      const row = document.createElement("div");
      row.className = "profile";
      const label = document.createElement("span");
      label.textContent = `${profile.style_name} · ${profile.duration_seconds.toFixed(1)}s`;
      const button = document.createElement("button");
      button.type = "button";
      button.className = "danger";
      button.textContent = t("deleteProfile");
      button.onclick = () => remove(`/api/profiles/${profile.id}`);
      row.append(label, button);
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

async function remove(path) {
  if (!confirm(t("confirmDelete"))) return;
  try {
    await api(path, { method: "DELETE" });
    await refresh();
  } catch (error) {
    notice(error.message);
  }
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
      setView("studioView");
      await refresh();
      notice("");
    } catch (error) {
      notice(error.message);
    }
  };
}

const logoutButton = $("#logout");
if (logoutButton) {
  logoutButton.onclick = async () => {
    try {
      await api("/api/auth/logout", { method: "POST", body: "{}" });
      setView("loginView");
    } catch (error) {
      notice(error.message);
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
window.addEventListener("beforeunload", () => liveStreams.forEach(stopStream));
