const test = require("node:test");
const assert = require("node:assert/strict");
const {
  buildAgentPrompt,
  createPromptAccessController,
} = require("../static/agent-prompt.js");

test("agent prompt uses the concise cross-client MCP configuration template", () => {
  const url = "https://voice.example.com/mcp";
  const token = "test-token-value";
  const prompt = buildAgentPrompt("zh", url, token);
  assert.equal(prompt, [
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
    `- URL："${url}"`,
    `- HTTP Header：Authorization = "Bearer ${token}"`,
    "",
    "完成后检查配置格式，并验证 MCP 是否注册成功。",
    "",
    "输出：",
    "- 修改的配置文件",
    "- 添加后的 MCP 配置",
    "- 验证结果",
  ].join("\n"));
});

test("agent prompt safely quotes dynamic HTTP MCP values", () => {
  const prompt = buildAgentPrompt("en", "https://example.com/a\"b", "test\"token");
  assert.ok(prompt.includes('- URL："https://example.com/a\\\"b"'));
  assert.ok(prompt.includes('- HTTP Header：Authorization = "Bearer test\\\"token"'));
});

test("agent prompt protects the bearer token and project config", () => {
  const prompt = buildAgentPrompt("zh", "https://voice.example.com/mcp", "secret");
  assert.ok(prompt.includes("不得在终端、日志或最终回复中回显"));
  assert.ok(prompt.includes("输出配置时必须脱敏"));
  assert.ok(prompt.includes("确认配置文件不会被 Git 跟踪或提交"));
  assert.ok(prompt.includes("否则停止并提示用户"));
});

test("starts hidden and refuses unauthenticated retrieval", () => {
  const visibility = [];
  const access = createPromptAccessController((value) => visibility.push(value));
  assert.equal(access.isAvailable(), false);
  assert.equal(access.begin(), null);
  assert.deepEqual(visibility, []);
});

test("authenticated configured state allows one current token attempt", () => {
  const visibility = [];
  const access = createPromptAccessController((value) => visibility.push(value));
  access.setAvailable(true);
  const attempt = access.begin();
  assert.equal(access.isCurrent(attempt.id), true);
  assert.equal(attempt.signal.aborted, false);
  assert.equal(access.storeToken(attempt.id, "test-token"), true);
  assert.equal(access.takeToken(attempt.id), "test-token");
  access.finish(attempt.id);
  assert.equal(access.isCurrent(attempt.id), false);
  assert.deepEqual(visibility, [true]);
});

test("logout invalidates and aborts an in-flight retrieval", () => {
  const visibility = [];
  const access = createPromptAccessController((value) => visibility.push(value));
  access.setAvailable(true);
  const attempt = access.begin();
  access.invalidate();
  assert.equal(attempt.signal.aborted, true);
  assert.equal(access.isCurrent(attempt.id), false);
  assert.equal(access.storeToken(attempt.id, "stale-token"), false);
  assert.equal(access.takeToken(attempt.id), null);
  assert.equal(access.isAvailable(), false);
  assert.deepEqual(visibility, [true, false]);
});

test("401 invalidation prevents stale retrieval callbacks from becoming current", () => {
  const access = createPromptAccessController();
  access.setAvailable(true);
  const stale = access.begin();
  access.invalidate();
  access.setAvailable(true);
  const current = access.begin();
  assert.equal(access.isCurrent(stale.id), false);
  assert.equal(access.storeToken(stale.id, "stale-token"), false);
  assert.equal(access.isCurrent(current.id), true);
});

test("no-config state remains hidden and unavailable", () => {
  const visibility = [];
  const access = createPromptAccessController((value) => visibility.push(value));
  access.setAvailable(true);
  access.setAvailable(false);
  assert.equal(access.isAvailable(), false);
  assert.equal(access.begin(), null);
  assert.deepEqual(visibility, [true, false]);
});
