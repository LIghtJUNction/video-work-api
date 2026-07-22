const test = require("node:test");
const assert = require("node:assert/strict");
const {
  buildAgentPrompt,
  createPromptAccessController,
} = require("../static/agent-prompt.js");

test("Chinese prompt asks exactly one scope question before mutation", () => {
  const url = "https://voice.example.com/mcp";
  const token = "test-token-value";
  const prompt = buildAgentPrompt("zh", url, token);
  assert.equal((prompt.match(/安装到当前项目还是全局？/g) || []).length, 1);
  assert.ok(prompt.indexOf("安装到当前项目还是全局？") < prompt.indexOf("如果回答“当前项目”"));
  assert.ok(prompt.includes(`codex mcp add video-work-api --url ${url}`));
  assert.ok(prompt.includes(token));
  assert.ok(!prompt.includes("<VWA_MCP_TOKEN>"));
  assert.ok(prompt.includes("[mcp_servers.video-work-api]"));
  assert.ok(prompt.includes(`http_headers = { Authorization = "Bearer ${token}" }`));
  assert.ok(prompt.includes(".git/info/exclude"));
  assert.ok(prompt.includes("/.codex/.config.toml.video-work-api.tmp"));
  assert.ok(prompt.includes("git ls-files"));
  assert.ok(prompt.includes("git check-ignore"));
  assert.ok(prompt.includes("git status --short --untracked-files=all"));
  assert.ok(prompt.includes("fsync，并原子 rename"));
  assert.ok(prompt.includes("只包含下面两个键"));
  for (const conflicting of ["command", "args", "bearer_token_env_var", "env_http_headers", "auth"]) {
    assert.ok(prompt.includes(conflicting));
  }
  assert.ok(prompt.includes("initialize/tools/list"));
  assert.ok(prompt.includes("仅重启不够"));
  assert.ok(!prompt.includes("opencode"));
  assert.ok(!prompt.includes("Claude"));
  assert.ok(!prompt.includes("export VWA_MCP_TOKEN"));
  assert.ok(!prompt.includes("bearer-token-env-var"));
});

test("project and global instructions order security preflights before token writes", () => {
  const prompt = buildAgentPrompt("zh", "https://voice.example.com/mcp", "test-token-value");
  const project = prompt.slice(prompt.indexOf("如果回答“当前项目”"), prompt.indexOf("如果回答“全局”"));
  assert.ok(project.indexOf("git ls-files") < project.indexOf(".git/info/exclude"));
  assert.ok(project.indexOf(".git/info/exclude") < project.indexOf("先 lstat `.codex`"));
  assert.ok(project.indexOf("先 lstat `.codex`") < project.indexOf("完整替换 [mcp_servers.video-work-api]"));
  assert.ok(project.indexOf("完整替换 [mcp_servers.video-work-api]") < project.indexOf("Bearer test-token-value"));

  const global = prompt.slice(prompt.indexOf("如果回答“全局”"), prompt.indexOf("共同验收"));
  assert.ok(global.indexOf("lstat ~/.codex") < global.indexOf("codex mcp add"));
  assert.ok(global.indexOf("codex mcp add") < global.indexOf("完整替换 [mcp_servers.video-work-api]"));
  assert.ok(global.indexOf("完整替换 [mcp_servers.video-work-api]") < global.indexOf("Bearer test-token-value"));
});

test("both scopes refuse symlinked or foreign-owned config paths before access", () => {
  const zh = buildAgentPrompt("zh", "https://voice.example.com/mcp", "test-token-value");
  const project = zh.slice(zh.indexOf("如果回答“当前项目”"), zh.indexOf("如果回答“全局”"));
  const global = zh.slice(zh.indexOf("如果回答“全局”"), zh.indexOf("共同验收"));
  assert.ok(project.includes("git ls-files 同时预检 `.codex` 目录路径"));
  assert.ok(project.includes("若 `.codex/` 下已有任何 tracked 条目"));
  assert.ok(project.includes("真实的非符号链接目录且由当前用户所有，否则拒绝"));
  assert.ok(project.includes("真实的非符号链接普通文件且由当前用户所有，否则拒绝"));
  assert.ok(project.indexOf("先 lstat `.codex`") < project.indexOf("chmod 0700"));
  assert.ok(project.indexOf("lstat `.codex/config.toml`") < project.indexOf("chmod 0600"));
  assert.ok(global.includes("lstat ~/.codex"));
  assert.ok(global.includes("真实的非符号链接目录且由当前用户所有，否则拒绝"));
  assert.ok(global.includes("真实的非符号链接普通文件且由当前用户所有，否则拒绝"));

  const en = buildAgentPrompt("en", "https://voice.example.com/mcp", "test-token-value");
  assert.ok(en.includes("preflight the `.codex` directory path"));
  assert.ok(en.includes("real non-symlink directory owned by the current user, otherwise refuse"));
  assert.ok(en.includes("real non-symlink regular file owned by the current user, otherwise refuse"));
});

test("project atomic merge excludes and verifies deterministic temp before secret write", () => {
  const prompt = buildAgentPrompt("zh", "https://voice.example.com/mcp", "test-token-value");
  const project = prompt.slice(prompt.indexOf("如果回答“当前项目”"), prompt.indexOf("如果回答“全局”"));
  const finalExclude = "`/.codex/config.toml`";
  const tempExclude = "`/.codex/.config.toml.video-work-api.tmp`";
  assert.ok(project.includes(finalExclude));
  assert.ok(project.includes(tempExclude));
  assert.ok(project.indexOf(finalExclude) < project.indexOf("Bearer test-token-value"));
  assert.ok(project.indexOf(tempExclude) < project.indexOf("Bearer test-token-value"));
  assert.ok(project.indexOf("分别执行 git check-ignore") < project.indexOf("Bearer test-token-value"));
  assert.ok(project.indexOf("O_CREAT|O_EXCL|O_NOFOLLOW") < project.indexOf("再次用 git check-ignore"));
  assert.ok(project.indexOf("再次用 git check-ignore") < project.indexOf("才通过已打开的文件描述符写入"));
  assert.ok(project.includes("固定临时文件已不存在且未被暂存"));
  assert.ok(project.includes("不显示最终路径或临时路径"));
});

test("English prompt has one question and both exclusive branches", () => {
  const prompt = buildAgentPrompt(
    "en",
    "http://localhost:7860/mcp",
    "test'quote",
  );
  assert.equal((prompt.match(/Install for the current project or globally\?/g) || []).length, 1);
  assert.ok(prompt.includes("If I answer current project:"));
  assert.ok(prompt.includes("If I answer globally:"));
  assert.ok(prompt.includes('http_headers = { Authorization = "Bearer test\'quote" }'));
  assert.ok(!prompt.includes("opencode"));
  assert.ok(!prompt.includes("export VWA_MCP_TOKEN"));
  assert.ok(prompt.includes("exactly the following two keys"));
  assert.ok(prompt.includes("restart alone is insufficient"));
  assert.ok(prompt.includes("/.codex/.config.toml.video-work-api.tmp"));
  assert.ok(prompt.includes("O_CREAT|O_EXCL|O_NOFOLLOW"));
  assert.ok(prompt.includes("before inserting the token"));
  assert.ok(prompt.includes("temporary file no longer exists or is staged"));
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
