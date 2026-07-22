(function (root, factory) {
  const prompt = factory();
  if (typeof module === "object" && module.exports) module.exports = prompt;
  else root.AgentPrompt = prompt;
})(typeof globalThis !== "undefined" ? globalThis : this, function () {
  "use strict";

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
    const jsonToken = JSON.stringify(`Bearer ${token}`);
    const promptUrl = JSON.stringify(String(url));
    return [
      "请为当前项目添加 MCP 配置。",
      "",
      "要求：",
      "1. 自动识别当前环境：",
      "   - Codex：添加项目级 MCP 到 .codex/config.toml",
      "   - Claude Code：添加项目级 MCP 到 .mcp.json",
      "",
      "2. 如果需要全局配置，则：",
      "   - Codex 使用用户级 MCP 配置",
      "   - Claude Code 使用 user/global scope MCP 配置",
      "",
      "3. 不覆盖已有 MCP，只新增或更新指定 MCP。",
      "",
      "4. Authorization/Bearer Token 属于密钥，不得在终端、日志或最终回复中回显；输出配置时必须脱敏。写入项目级配置前，确认配置文件不会被 Git 跟踪或提交；否则停止并提示用户。",
      "",
      "MCP 信息：",
      "名称：video-work-api",
      "Command：无（HTTP MCP）",
      "Args：",
      `- URL：${promptUrl}`,
      `- HTTP Header：Authorization = ${jsonToken}`,
      "",
      "完成后检查配置格式，并验证 MCP 是否注册成功。",
      "",
      "输出：",
      "- 修改的配置文件",
      "- 添加后的 MCP 配置",
      "- 验证结果",
    ].join("\n");
  }

  return { buildAgentPrompt, createPromptAccessController };
});
