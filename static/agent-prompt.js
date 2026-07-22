(function (root, factory) {
  const prompt = factory();
  if (typeof module === "object" && module.exports) module.exports = prompt;
  else root.AgentPrompt = prompt;
})(typeof globalThis !== "undefined" ? globalThis : this, function () {
  "use strict";

  function shellQuote(value) {
    return `'${String(value).replaceAll("'", `'"'"'`)}'`;
  }

  function createPromptAccessController(onAvailabilityChange = function () {}) {
    let available = false;
    let epoch = 0;
    let active = null;

    function invalidate() {
      epoch += 1;
      if (active?.controller) active.controller.abort();
      active = null;
      available = false;
      onAvailabilityChange(false);
    }

    function setAvailable(next) {
      if (!next) {
        invalidate();
        return;
      }
      available = true;
      onAvailabilityChange(true);
    }

    function begin() {
      if (!available) return null;
      epoch += 1;
      if (active?.controller) active.controller.abort();
      const controller =
        typeof AbortController === "function" ? new AbortController() : null;
      active = { id: epoch, controller, token: null };
      return { id: active.id, signal: controller?.signal };
    }

    function isCurrent(id) {
      return available && active?.id === id;
    }

    function storeToken(id, token) {
      if (!isCurrent(id) || typeof token !== "string" || !token) return false;
      active.token = token;
      return true;
    }

    function takeToken(id) {
      if (!isCurrent(id)) return null;
      const token = active.token;
      active.token = null;
      return token;
    }

    function finish(id) {
      if (active?.id === id) active = null;
    }

    return {
      begin,
      currentEpoch: function () { return epoch; },
      finish,
      invalidate,
      isAvailable: function () { return available; },
      isCurrent,
      setAvailable,
      storeToken,
      takeToken,
    };
  }

  function buildAgentPrompt(language, url, token) {
    const exportToken = `export VWA_MCP_TOKEN=${shellQuote(token)}`;
    const codexCommand = `codex mcp add video-work-api --url ${url} --bearer-token-env-var VWA_MCP_TOKEN`;
    const jsonToken = JSON.stringify(`Bearer ${token}`);
    const tomlUrl = JSON.stringify(String(url));

    if (language === "zh") {
      return [
        "请帮我安装 Video Work API 的 MCP 服务器（语音生成 / 字幕提取工具）。",
        "",
        "首选 Codex CLI：",
        "注意：--bearer-token-env-var 只会保存环境变量名，不会保存 token。请通过安全的 shell 配置、服务管理器或密钥注入方式，让 VWA_MCP_TOKEN 对今后启动的 Codex 进程持续可用。",
        "以下 export 只对当前 shell 及其子进程有效，不是持久配置：",
        exportToken,
        codexCommand,
        "",
        "如果无法为 Codex 持久提供环境变量，可直接写入 ~/.codex/config.toml（以下会把 token 以明文保存，请严格限制该文件权限）：",
        "[mcp_servers.video-work-api]",
        `url = ${tomlUrl}`,
        `http_headers = { Authorization = ${jsonToken} }`,
        "",
        "连接信息：",
        "- 传输方式：HTTP（POST，JSON-RPC 2.0）",
        `- 端点：${url}`,
        `- Bearer Token：${token}`,
        "",
        "opencode 配置（opencode.json 的 mcp 节）：",
        '"video-work-api": {',
        '  "type": "remote",',
        `  "url": ${tomlUrl},`,
        `  "headers": { "Authorization": ${jsonToken} }`,
        "}",
        "",
        "可用工具：get_status、list_speakers、create_speaker、delete_speaker、rename_speaker、add_voice_profile、delete_voice_profile、rename_voice_profile、generate_speech、get_generation、extract_video_subtitles。",
        "配置完成后请调用 tools/list 验证。",
      ].join("\n");
    }

    return [
      "Install the Video Work API MCP server (speech generation / subtitle extraction tools).",
      "",
      "Prefer the Codex CLI:",
      "Important: --bearer-token-env-var stores only the environment variable name, not the token. Arrange secure, durable availability of VWA_MCP_TOKEN for future Codex processes through your shell, service manager, or secret injector.",
      "This export affects only the current shell and its child processes; it is not durable configuration:",
      exportToken,
      codexCommand,
      "",
      "If a durable environment variable is unavailable, add this directly to ~/.codex/config.toml (this stores the token in plaintext; strictly restrict the file permissions):",
      "[mcp_servers.video-work-api]",
      `url = ${tomlUrl}`,
      `http_headers = { Authorization = ${jsonToken} }`,
      "",
      "Connection details:",
      "- Transport: HTTP (POST, JSON-RPC 2.0)",
      `- Endpoint: ${url}`,
      `- Bearer token: ${token}`,
      "",
      'opencode config (the "mcp" section of opencode.json):',
      '"video-work-api": {',
      '  "type": "remote",',
      `  "url": ${tomlUrl},`,
      `  "headers": { "Authorization": ${jsonToken} }`,
      "}",
      "",
      "Available tools: get_status, list_speakers, create_speaker, delete_speaker, rename_speaker, add_voice_profile, delete_voice_profile, rename_voice_profile, generate_speech, get_generation, extract_video_subtitles.",
      "After configuring, verify by calling tools/list.",
    ].join("\n");
  }

  return { buildAgentPrompt, createPromptAccessController };
});
