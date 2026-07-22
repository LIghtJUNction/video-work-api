const test = require("node:test");
const assert = require("node:assert/strict");
const {
  buildAgentPrompt,
  createPromptAccessController,
} = require("../static/agent-prompt.js");

test("prefers Codex CLI and includes the supplied token", () => {
  const url = "https://voice.example.com/mcp";
  const token = "test-token-value";
  const prompt = buildAgentPrompt("zh", url, token);
  const command = `codex mcp add video-work-api --url ${url} --bearer-token-env-var VWA_MCP_TOKEN`;

  assert.ok(prompt.indexOf("首选 Codex CLI") < prompt.indexOf("opencode 配置"));
  assert.ok(prompt.includes(command));
  assert.ok(prompt.includes(token));
  assert.ok(!prompt.includes("<VWA_MCP_TOKEN>"));
  assert.ok(prompt.includes("只会保存环境变量名"));
  assert.ok(prompt.includes("不是持久配置"));
  assert.ok(prompt.includes("[mcp_servers.video-work-api]"));
  assert.ok(prompt.includes(`http_headers = { Authorization = "Bearer ${token}" }`));
});

test("shell-quotes tokens used in the Codex environment export", () => {
  const prompt = buildAgentPrompt(
    "en",
    "http://localhost:7860/mcp",
    "test'quote",
  );
  assert.ok(prompt.includes(`export VWA_MCP_TOKEN='test'"'"'quote'`));
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
