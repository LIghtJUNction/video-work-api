"use strict";

const fs = require("node:fs");
const path = require("node:path");
const crypto = require("node:crypto");
const { spawnSync } = require("node:child_process");
const { chromium } = require("playwright");

async function main() {
  const baseUrl = process.env.VWA_SMOKE_BASE_URL;
  const dataDir = process.env.VWA_SMOKE_DATA_DIR;
  const setupTokenFile = process.env.VWA_SMOKE_SETUP_TOKEN_FILE;
  const screenshotPath = process.env.VWA_SMOKE_SCREENSHOT;
  if (!baseUrl || !dataDir || !setupTokenFile || !screenshotPath) {
    throw new Error("browser smoke environment is incomplete");
  }
  const setupToken = fs.readFileSync(setupTokenFile, "utf8").trim();
  const password = `Smoke-${crypto.randomUUID()}-A9!`;
  let stage = "launch";
  const browser = await chromium.launch({
    headless: true,
    executablePath: "/usr/bin/google-chrome-stable",
    args: ["--no-sandbox", "--disable-dev-shm-usage"],
  });
  try {
    const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
    const consoleErrors = [];
    const failedRequests = [];
    page.on("console", (message) => {
      if (message.type() === "error") consoleErrors.push(message.text());
    });
    page.on("requestfailed", (request) => {
      const failure = request.failure();
      if (!failure || !failure.errorText.includes("ERR_ABORTED")) {
        failedRequests.push(new URL(request.url()).pathname);
      }
    });

    stage = "open setup";
    await page.goto(`${baseUrl}/`, { waitUntil: "networkidle" });
    await page.locator("#setupForm input[name=token]").fill(setupToken);
    await page.locator("#setupForm input[name=password]").fill(password);
    const setupResponse = page.waitForResponse((response) => response.url().endsWith("/api/setup"));
    await page.locator("#setupForm button").click();
    const setupResult = await setupResponse;
    if (!setupResult.ok()) {
      const payload = await setupResult.json();
      throw new Error(`setup failed: ${setupResult.status()} ${payload.error?.code || "unknown"}`);
    }
    await page.locator("#loginView:not(.hidden)").waitFor();
    await page.locator("#loginForm input[name=password]").fill(password);
    const loginResponse = page.waitForResponse((response) =>
      response.url().endsWith("/api/auth/login")
    );
    await page.locator("#loginForm button").click();
    const loginResult = await loginResponse;
    if (!loginResult.ok()) {
      const payload = await loginResult.json();
      throw new Error(`login failed: ${loginResult.status()} ${payload.error?.code || "unknown"}`);
    }
    await page.locator("#studioView:not(.hidden)").waitFor();

    stage = "open editor";
    await page.goto(`${baseUrl}/editor`, { waitUntil: "domcontentloaded" });
    await page.locator("#projectList").waitFor();
    await page.locator("#newProjectButton").click();
    await page.locator("#projectSlug").fill("browser-smoke");
    await page.locator("#createProjectForm button[type=submit]").click();
    const editor = page.locator("#sourceEditor");
    await editor.waitFor({ state: "visible" });
    await page.waitForFunction(() => !document.querySelector("#sourceEditor").disabled);

    const asset = path.join(dataDir, "video-projects", "browser-smoke", "assets", "source.mp4");
    const ffmpeg = spawnSync(
      "/usr/bin/ffmpeg",
      [
        "-hide_banner", "-loglevel", "error",
        "-f", "lavfi", "-i", "color=c=black:size=160x90:rate=10",
        "-t", "1.2", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-an", "-y", asset,
      ],
      { stdio: "ignore" },
    );
    if (ffmpeg.status !== 0) throw new Error("browser smoke fixture generation failed");

    const revisionTwo = `project "Browser Smoke" {
  canvas 160x90 @ 10fps
  source main = "assets/source.mp4"

  timeline {
    track main {
      clip main source 00:00:00.000..00:00:01.000 at 00:00:00.000
    }
  }

  marker "Opening hook" at 00:00:00.500
  variant "EN" aspect 16:9 cta "Review"
}
`;
    stage = "save revision 2";
    await editor.fill(revisionTwo);
    await page.locator("#saveButton").click();
    await page.waitForFunction(() => document.querySelector("#revisionStatus").textContent === "Revision 2");
    stage = "validate revision 2";
    await page.locator("#validateButton").click();
    await page.waitForFunction(() => document.querySelector("#validationBadge").textContent === "Validated");
    stage = "export revision 2";
    await page.locator("#exportButton").click();
    await page.locator("#queueInspector .queue-job").first().waitFor();
    await page.waitForFunction(() => {
      const value = document.querySelector("#queueInspector .queue-job-state")?.textContent;
      return value === "succeeded" || value === "failed" || value === "canceled";
    }, null, { timeout: 45000 });
    const exportState = await page.locator("#queueInspector .queue-job-state").first().textContent();
    if (exportState !== "succeeded") {
      throw new Error(`export finished as ${exportState}`);
    }

    const revisionThree = revisionTwo.replace("Browser Smoke", "Browser Remote");
    stage = "clean remote revision 3";
    await page.evaluate(async (content) => {
    const response = await fetch("/api/editor", {
      method: "POST",
      credentials: "same-origin",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        action: "write_file",
        project: "browser-smoke",
        path: "project.vpe",
        content,
        expected_revision: 2,
      }),
    });
    if (!response.ok) throw new Error("remote write failed");
    }, revisionThree);
    await page.waitForFunction(() => document.querySelector("#revisionStatus").textContent === "Revision 3");
    await page.waitForFunction(() => document.querySelector("#sourceEditor").value.includes("Browser Remote"));

    stage = "dirty remote revision 4";
    await editor.fill(`${revisionThree}\nLOCAL UNSAVED`);
    const revisionFour = revisionThree.replace("Browser Remote", "Browser Remote Two");
    await page.evaluate(async (content) => {
    const response = await fetch("/api/editor", {
      method: "POST",
      credentials: "same-origin",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        action: "write_file",
        project: "browser-smoke",
        path: "project.vpe",
        content,
        expected_revision: 3,
      }),
    });
    if (!response.ok) throw new Error("second remote write failed");
    }, revisionFour);
    await page.locator("#remoteBanner:not(.hidden)").waitFor();
    if (!(await editor.inputValue()).includes("LOCAL UNSAVED")) {
      throw new Error("dirty buffer was replaced by a remote revision");
    }

    fs.mkdirSync(path.dirname(screenshotPath), { recursive: true });
    await page.screenshot({ path: screenshotPath, fullPage: true });
    const queueState = await page.locator("#queueInspector .queue-job-state").first().textContent();
    if (consoleErrors.length || failedRequests.length) {
      throw new Error(
        `browser console/network errors: console=${consoleErrors.length} network=${failedRequests.length}`,
      );
    }
    process.stdout.write(JSON.stringify({
      screenshot: screenshotPath,
      create: true,
      save_revision: 2,
      validate: true,
      export_job_state: queueState,
      clean_remote_reload: true,
      dirty_remote_preserved: true,
      console_errors: 0,
      network_errors: 0,
    }) + "\n");
  } catch (error) {
    throw new Error(`${stage}: ${error.message}`);
  } finally {
    await browser.close();
  }
}

main().catch((error) => {
  process.stderr.write(`${error.message}\n`);
  process.exitCode = 1;
});
