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
    const codexCommand = `codex mcp add video-work-api --url ${url}`;
    const jsonToken = JSON.stringify(`Bearer ${token}`);
    const tomlUrl = JSON.stringify(String(url));
    const tomlBlock = [
      "[mcp_servers.video-work-api]",
      `url = ${tomlUrl}`,
      `http_headers = { Authorization = ${jsonToken} }`,
    ].join("\n");

    if (language === "zh") {
      return [
        "请帮我为 Codex 安装 Video Work API MCP 服务器（语音生成 / 字幕提取工具）。",
        "",
        "强制交互规则：在执行任何命令、写文件或其他变更之前，只问我这一个问题，文字必须完全一致：",
        "安装到当前项目还是全局？",
        "问完后停止并等待我的回答。收到回答后只执行对应的一个分支，不要再问选择问题，也不要同时执行两个分支。",
        "",
        "如果回答“当前项目”：",
        "1. 在排除路径或写入 token 前，先定位仓库根目录并确认它是用户信任的目标项目；Codex 只加载受信任项目中的 .codex/config.toml。用 git ls-files 同时预检 `.codex` 目录路径、`.codex/config.toml` 和固定临时路径 `.codex/.config.toml.video-work-api.tmp`；若 `.codex/` 下已有任何 tracked 条目，或最终/临时路径已被跟踪，立即拒绝修改并报告风险。",
        "2. 若处于 Git 仓库，先把 `/.codex/config.toml` 和 `/.codex/.config.toml.video-work-api.tmp` 两行安全合并进该仓库的 .git/info/exclude（不要修改已跟踪的 .gitignore），并对最终路径和临时路径分别执行 git check-ignore，确认两者都已本地排除；任何验证失败都必须在写入 token 前停止。",
        "3. 在任何 chmod、读取或写入 `.codex` 内容前，先 lstat `.codex`：若不存在才安全创建为 mode 0700；若存在，必须是真实的非符号链接目录且由当前用户所有，否则拒绝。校验通过后才可 chmod 0700。随后 lstat `.codex/config.toml`：若不存在，安全独占创建 mode 0600 空文件；若存在，必须是真实的非符号链接普通文件且由当前用户所有，否则拒绝；只有校验通过后才可 chmod 0600、读取或改写。",
        "4. 使用 TOML 解析器原子合并：保留目标表之外的全部配置，但完整替换 [mcp_servers.video-work-api]，使该表只包含下面两个键（不得保留 command、args、bearer_token_env_var、env_http_headers、auth、旧 headers、子表或任何其他冲突键）：",
        tomlBlock,
        "5. 固定使用 `.codex/.config.toml.video-work-api.tmp`：先用 lstat 确认它不存在（若存在则拒绝，不得跟随或覆盖），再以 O_CREAT|O_EXCL|O_NOFOLLOW 和 mode 0600 创建空临时文件。创建空文件后、插入 token 前，再次用 git check-ignore 分别验证 `.codex/config.toml` 与该临时路径。只有两者都被排除后，才通过已打开的文件描述符写入完整 TOML、fsync，并原子 rename 为 `.codex/config.toml`，保持最终权限 0600。不要把 token 放进命令参数、终端输出或日志。",
        "6. 验证 TOML 可解析、目标表精确等于上述两键、固定临时文件已不存在且未被暂存，并确认 `git status --short --untracked-files=all` 不显示最终路径或临时路径。",
        "",
        "如果回答“全局”：",
        "1. 在任何 chmod、读取或写入前，先 lstat ~/.codex：若不存在才安全创建为 mode 0700；若存在，必须是真实的非符号链接目录且由当前用户所有，否则拒绝。校验通过后才可 chmod 0700。随后 lstat ~/.codex/config.toml：若不存在，安全独占创建 mode 0600 空文件；若存在，必须是真实的非符号链接普通文件且由当前用户所有，否则拒绝；只有校验通过后才可 chmod 0600、读取或改写。",
        `2. 首选当前 Codex CLI，然后运行：${codexCommand}。若它因为同名服务器已存在而失败，保留现有其他配置并继续下一步的精确规范化。`,
        "3. 使用 TOML 解析器原子合并：保留目标表之外的全部配置，但完整替换 [mcp_servers.video-work-api]，使该表只包含下面两个键（不得保留 command、args、bearer_token_env_var、env_http_headers、auth、旧 headers、子表或任何其他冲突键）：",
        tomlBlock,
        "4. 固定使用 ~/.codex/.config.toml.video-work-api.tmp；先用 lstat 确认不存在，再以 O_CREAT|O_EXCL|O_NOFOLLOW 和 mode 0600 创建，通过已打开的文件描述符写入、fsync 并原子 rename 覆盖 config.toml。保持最终权限 0600，确认临时文件已不存在，并验证 TOML 与目标表。不要使用临时 export 或 bearer_token_env_var，也不要输出 token。",
        "",
        "共同验收：不要在终端、日志或回复中打印 token。先验证配置，然后重启 Codex 或在目标目录开启新会话使配置生效；通过 MCP initialize/tools/list（或新会话中的 /mcp）实际确认 video-work-api 已连接且工具可见。可用工具应包括 get_status、list_speakers、create_speaker、generate_speech、extract_video_subtitles。实际连接未成功前不要声称安装完成。以后若服务端轮换 Token，必须重新登录网页复制新提示词、再次执行同一个安装分支替换静态 Token，然后重启/新建 Codex 会话并重新验证工具；仅重启不够。",
      ].join("\n");
    }

    return [
      "Install the Video Work API MCP server for Codex (speech generation / subtitle extraction tools).",
      "",
      "Mandatory interaction rule: before running any command, writing any file, or making any other mutation, ask exactly this one question:",
      "Install for the current project or globally?",
      "Stop and wait for my answer. After I answer, execute exactly one matching branch; do not ask another scope question and do not execute both branches.",
      "",
      "If I answer current project:",
      "1. Before excluding paths or writing the token, locate the repository root and confirm it is the intended trusted project; Codex loads project .codex/config.toml only for trusted projects. Use git ls-files to preflight the `.codex` directory path, `.codex/config.toml`, and deterministic temporary path `.codex/.config.toml.video-work-api.tmp`; if any entry under `.codex/` or either final/temporary path is tracked, refuse to mutate and report the risk.",
      "2. In a Git repository, first safely merge both `/.codex/config.toml` and `/.codex/.config.toml.video-work-api.tmp` as separate lines into that repository's .git/info/exclude (do not change a tracked .gitignore). Run git check-ignore separately for the final and temporary paths and confirm both local exclusions before any token write; stop before writing the token if either check fails.",
      "3. Before any chmod, read, or write of `.codex` content, lstat `.codex`: only if absent may it be safely created with mode 0700; if present, require a real non-symlink directory owned by the current user, otherwise refuse. Only after validation may it be chmodded to 0700. Then lstat `.codex/config.toml`: if absent, securely and exclusively create an empty mode-0600 file; if present, require a real non-symlink regular file owned by the current user, otherwise refuse. Only after validation may it be chmodded to 0600, read, or changed.",
      "4. Use a TOML parser for an atomic merge: preserve everything outside the target table, but completely replace [mcp_servers.video-work-api] so it contains exactly the following two keys (remove command, args, bearer_token_env_var, env_http_headers, auth, old headers, subtables, and every other conflicting key):",
      tomlBlock,
      "5. Use the deterministic path `.codex/.config.toml.video-work-api.tmp`: first lstat it and require that it does not exist (refuse rather than follow or overwrite it), then create it empty with O_CREAT|O_EXCL|O_NOFOLLOW and mode 0600. After creating the empty file but before inserting the token, rerun git check-ignore separately for `.codex/config.toml` and the temporary path. Only when both are excluded, write the complete TOML through the already-open file descriptor, fsync it, and atomically rename it to `.codex/config.toml`, retaining final mode 0600. Never place the token in command arguments, terminal output, or logs.",
      "6. Verify the TOML parses, the target table exactly matches those two keys, the deterministic temporary file no longer exists or is staged, and `git status --short --untracked-files=all` exposes neither the final nor temporary path.",
      "",
      "If I answer globally:",
      "1. Before any chmod, read, or write, lstat ~/.codex: only if absent may it be safely created with mode 0700; if present, require a real non-symlink directory owned by the current user, otherwise refuse. Only after validation may it be chmodded to 0700. Then lstat ~/.codex/config.toml: if absent, securely and exclusively create an empty mode-0600 file; if present, require a real non-symlink regular file owned by the current user, otherwise refuse. Only after validation may it be chmodded to 0600, read, or changed.",
      `2. Prefer the current Codex CLI, then run: ${codexCommand}. If it fails because that server name already exists, preserve all other configuration and continue with exact normalization below.`,
      "3. Use a TOML parser for an atomic merge: preserve everything outside the target table, but completely replace [mcp_servers.video-work-api] so it contains exactly the following two keys (remove command, args, bearer_token_env_var, env_http_headers, auth, old headers, subtables, and every other conflicting key):",
      tomlBlock,
      "4. Use the deterministic ~/.codex/.config.toml.video-work-api.tmp path: lstat and require it to be absent, create it with O_CREAT|O_EXCL|O_NOFOLLOW and mode 0600, write through the opened descriptor, fsync, and atomically rename it over config.toml. Retain final mode 0600, confirm the temporary file is gone, and verify the TOML and target table. Do not use a transient export or bearer_token_env_var, and never print the token.",
      "",
      "Acceptance for either branch: never print the token in terminal output, logs, or your reply. Verify the config, then restart Codex or open a new session in the target directory so it reloads configuration; use MCP initialize/tools/list (or /mcp in the new session) to prove video-work-api connects and its tools are available. Expected tools include get_status, list_speakers, create_speaker, generate_speech, and extract_video_subtitles. Do not claim completion until the live connection succeeds. After any server-side token rotation, sign in to the web UI and copy the NEW prompt, rerun the same chosen install branch to replace the static token, then restart/open a new Codex session and verify tools again; restart alone is insufficient.",
    ].join("\n");
  }

  return { buildAgentPrompt, createPromptAccessController };
});
