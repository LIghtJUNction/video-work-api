"use strict";

(function editorModule(global) {
  const MAX_DOCUMENT_BYTES = 2 * 1024 * 1024;
  const TERMINAL_JOB_STATES = new Set(["succeeded", "failed", "canceled"]);

  function lineMetrics(value, selectionStart) {
    const safeStart = Math.max(0, Math.min(Number(selectionStart) || 0, value.length));
    const before = value.slice(0, safeStart);
    const lines = before.split("\n");
    return {
      line: lines.length,
      column: Array.from(lines[lines.length - 1]).length + 1,
      lineCount: value.split("\n").length,
    };
  }

  function buildAction(action, fields) {
    return Object.assign({ action }, fields || {});
  }

  function remoteRevisionDecision(buffer, projectEvent) {
    if (!buffer || !projectEvent || projectEvent.slug !== buffer.project) {
      return { reload: false, remoteAvailable: false };
    }
    if (Number(projectEvent.revision) <= Number(buffer.revision)) {
      return { reload: false, remoteAvailable: false };
    }
    return buffer.dirty
      ? { reload: false, remoteAvailable: true }
      : { reload: true, remoteAvailable: false };
  }

  function classifyApiFailure(status, code) {
    if (status === 409 && code === "editor_conflict") return "revision_conflict";
    if (status === 401) return "authentication_required";
    if (status === 403) return "origin_rejected";
    if (status >= 500) return "service_unavailable";
    return "request_failed";
  }

  const publicCore = {
    lineMetrics,
    buildAction,
    remoteRevisionDecision,
    classifyApiFailure,
  };
  global.VideoEditorCore = publicCore;
  if (typeof module !== "undefined" && module.exports) module.exports = publicCore;
  if (typeof document === "undefined") return;

  const state = {
    projects: [],
    trees: new Map(),
    tabs: new Map(),
    activeKey: null,
    activeProject: null,
    jobs: new Map(),
    parsedDocument: null,
    eventSource: null,
    remoteProject: null,
    authenticated: true,
    online: false,
    pollTimer: null,
    projectPollTimer: null,
    toastTimer: null,
  };

  const elements = {
    projectList: document.getElementById("projectList"),
    activeProjectName: document.getElementById("activeProjectName"),
    connectionState: document.getElementById("connectionState"),
    liveStatus: document.getElementById("liveStatus"),
    saveButton: document.getElementById("saveButton"),
    validateButton: document.getElementById("validateButton"),
    exportButton: document.getElementById("exportButton"),
    tabs: document.getElementById("editorTabs"),
    breadcrumbs: document.getElementById("breadcrumbs"),
    remoteBanner: document.getElementById("remoteBanner"),
    reloadRemoteButton: document.getElementById("reloadRemoteButton"),
    keepEditingButton: document.getElementById("keepEditingButton"),
    readOnlyBanner: document.getElementById("readOnlyBanner"),
    editor: document.getElementById("sourceEditor"),
    gutter: document.getElementById("lineGutter"),
    bottomPanel: document.getElementById("bottomPanel"),
    problemsPanel: document.getElementById("problemsPanel"),
    jobsPanel: document.getElementById("jobsPanel"),
    problemCount: document.getElementById("problemCount"),
    jobCount: document.getElementById("jobCount"),
    durationMetric: document.getElementById("durationMetric"),
    mainTrackMetric: document.getElementById("mainTrackMetric"),
    overlayTrackMetric: document.getElementById("overlayTrackMetric"),
    markerMetric: document.getElementById("markerMetric"),
    timeline: document.getElementById("timelineInspector"),
    outline: document.getElementById("outlineInspector"),
    queue: document.getElementById("queueInspector"),
    validationBadge: document.getElementById("validationBadge"),
    dirtyStatus: document.getElementById("dirtyStatus"),
    revisionStatus: document.getElementById("revisionStatus"),
    cursorStatus: document.getElementById("cursorStatus"),
    sizeStatus: document.getElementById("documentSizeStatus"),
    newProjectButton: document.getElementById("newProjectButton"),
    dialog: document.getElementById("createProjectDialog"),
    createForm: document.getElementById("createProjectForm"),
    projectSlug: document.getElementById("projectSlug"),
    createError: document.getElementById("createProjectError"),
    authGate: document.getElementById("authGate"),
    toast: document.getElementById("toast"),
  };

  class ApiFailure extends Error {
    constructor(status, code) {
      super(code || "request_failed");
      this.status = status;
      this.code = code || "request_failed";
      this.kind = classifyApiFailure(status, this.code);
    }
  }

  async function callEditor(action, fields) {
    const response = await fetch("/api/editor", {
      method: "POST",
      credentials: "same-origin",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(buildAction(action, fields)),
    });
    let payload = {};
    try {
      payload = await response.json();
    } catch (_) {
      payload = {};
    }
    if (!response.ok) {
      throw new ApiFailure(response.status, payload.error && payload.error.code);
    }
    return payload;
  }

  function activeBuffer() {
    return state.activeKey ? state.tabs.get(state.activeKey) || null : null;
  }

  function tabKey(project, path) {
    return `${project}\u0000${path}`;
  }

  function setConnection(mode) {
    state.online = mode === "online";
    elements.connectionState.classList.toggle("online", mode === "online");
    elements.connectionState.classList.toggle("offline", mode === "offline");
    elements.connectionState.lastElementChild.textContent =
      mode === "online" ? "Live" : mode === "offline" ? "Offline" : "Connecting";
    elements.liveStatus.textContent = mode === "online" ? "LIVE" : "OFFLINE";
    elements.liveStatus.classList.toggle("online", mode === "online");
  }

  function showToast(message) {
    elements.toast.textContent = message;
    elements.toast.classList.remove("hidden");
    clearTimeout(state.toastTimer);
    state.toastTimer = setTimeout(() => elements.toast.classList.add("hidden"), 4200);
  }

  function showFailure(error, context) {
    if (error && error.kind === "authentication_required") {
      state.authenticated = false;
      elements.authGate.classList.remove("hidden");
      return;
    }
    const messages = {
      revision_conflict: "Save conflict: a newer remote revision is available.",
      origin_rejected: "The request was rejected by the same-origin policy.",
      service_unavailable: "Video Work API is temporarily unavailable.",
      request_failed: `${context} could not be completed.`,
    };
    showToast(messages[error && error.kind] || `${context} could not be completed.`);
  }

  async function refreshProjects(preferredProject) {
    try {
      const payload = await callEditor("list_projects");
      applyProjects(payload.projects || []);
      if (preferredProject) await openProject(preferredProject);
      else if (!state.activeProject && state.projects.length) await openProject(state.projects[0].slug);
    } catch (error) {
      showFailure(error, "Project refresh");
      if (!state.projects.length) renderProjectError();
    }
  }

  function applyProjects(projects) {
    state.projects = projects.slice().sort((a, b) => String(a.slug).localeCompare(String(b.slug)));
    renderProjects();
    refreshValidationState();
  }

  function renderProjectError() {
    const node = document.createElement("div");
    node.className = "empty-state";
    node.textContent = "Projects are unavailable.";
    elements.projectList.replaceChildren(node);
  }

  function renderProjects() {
    if (!state.projects.length) {
      const empty = document.createElement("div");
      empty.className = "empty-state";
      empty.textContent = "No video projects. Create one to begin.";
      elements.projectList.replaceChildren(empty);
      return;
    }
    const fragment = document.createDocumentFragment();
    for (const project of state.projects) {
      const section = document.createElement("section");
      section.className = "project-root";
      section.classList.toggle("active", project.slug === state.activeProject);
      const button = document.createElement("button");
      button.type = "button";
      button.className = "project-button";
      const chevron = document.createElement("span");
      chevron.textContent = project.slug === state.activeProject ? "⌄" : "›";
      const name = document.createElement("b");
      name.textContent = project.slug;
      const revision = document.createElement("span");
      revision.className = "project-meta";
      revision.textContent = `r${project.revision}${project.valid ? " ✓" : ""}`;
      button.append(chevron, name, revision);
      button.addEventListener("click", () => openProject(project.slug));
      section.appendChild(button);
      if (project.slug === state.activeProject) {
        const tree = state.trees.get(project.slug);
        section.appendChild(tree ? renderTree(project.slug, tree.entries || []) : loadingTree());
      }
      fragment.appendChild(section);
    }
    elements.projectList.replaceChildren(fragment);
  }

  function loadingTree() {
    const node = document.createElement("div");
    node.className = "empty-state";
    node.textContent = "Loading files…";
    return node;
  }

  function renderTree(project, entries) {
    const root = { children: new Map(), entry: null };
    for (const entry of entries) {
      let cursor = root;
      for (const part of String(entry.path).split("/")) {
        if (!cursor.children.has(part)) cursor.children.set(part, { children: new Map(), entry: null });
        cursor = cursor.children.get(part);
      }
      cursor.entry = entry;
    }
    const list = document.createElement("ul");
    list.className = "tree-list";
    appendTreeNodes(list, root, project, "");
    return list;
  }

  function appendTreeNodes(list, node, project, prefix) {
    const sorted = Array.from(node.children.entries()).sort((a, b) => {
      const directoryOrder = Number(Boolean(b[1].children.size)) - Number(Boolean(a[1].children.size));
      return directoryOrder || a[0].localeCompare(b[0]);
    });
    for (const [name, child] of sorted) {
      const path = prefix ? `${prefix}/${name}` : name;
      const item = document.createElement("li");
      if (child.children.size) {
        const details = document.createElement("details");
        details.open = path === "assets" || path === "exports";
        const summary = document.createElement("summary");
        summary.className = "tree-row tree-folder read-only";
        const icon = document.createElement("span");
        icon.className = "tree-icon";
        icon.textContent = "▸";
        const label = document.createElement("span");
        label.className = "tree-name";
        label.textContent = name;
        summary.append(icon, label);
        details.appendChild(summary);
        const nested = document.createElement("ul");
        nested.className = "tree-list";
        appendTreeNodes(nested, child, project, path);
        details.appendChild(nested);
        item.appendChild(details);
      } else if (child.entry && child.entry.kind === "file") {
        const row = document.createElement("button");
        row.type = "button";
        row.className = "tree-row";
        row.classList.toggle("read-only", Boolean(child.entry.read_only));
        const buffer = activeBuffer();
        row.classList.toggle(
          "active",
          Boolean(buffer && buffer.project === project && buffer.path === path),
        );
        const icon = document.createElement("span");
        icon.className = "tree-icon";
        icon.textContent = path === "project.vpe" ? "V" : "·";
        const label = document.createElement("span");
        label.className = "tree-name";
        label.textContent = name;
        row.append(icon, label);
        row.addEventListener("click", () => openFile(project, path));
        item.appendChild(row);
      }
      list.appendChild(item);
    }
  }

  async function openProject(project) {
    state.activeProject = project;
    elements.activeProjectName.textContent = project;
    renderProjects();
    if (!state.trees.has(project)) {
      try {
        const tree = await callEditor("get_tree", { project });
        state.trees.set(project, tree);
      } catch (error) {
        showFailure(error, "Project tree");
      }
      renderProjects();
    }
    await openFile(project, "project.vpe");
    renderQueue();
  }

  async function openFile(project, path, forceReload) {
    const key = tabKey(project, path);
    if (!forceReload && state.tabs.has(key)) {
      activateTab(key);
      return;
    }
    setEditorLoading();
    try {
      const file = await callEditor("read_file", { project, path });
      const buffer = {
        project,
        path,
        content: file.content,
        baseContent: file.content,
        revision: Number(file.revision),
        readOnly: Boolean(file.read_only),
        dirty: false,
        remoteAvailable: false,
      };
      state.tabs.set(key, buffer);
      state.activeProject = project;
      state.activeKey = key;
      state.remoteProject = null;
      renderAll();
    } catch (error) {
      showFailure(error, "File open");
      renderAll();
    }
  }

  function setEditorLoading() {
    elements.editor.disabled = true;
    elements.editor.value = "";
    elements.breadcrumbs.textContent = "Loading file…";
    elements.gutter.replaceChildren();
  }

  function activateTab(key) {
    const buffer = state.tabs.get(key);
    if (!buffer) return;
    state.activeKey = key;
    state.activeProject = buffer.project;
    state.remoteProject = buffer.remoteAvailable ? findProject(buffer.project) : null;
    renderAll();
  }

  function renderAll() {
    renderProjects();
    renderTabs();
    renderEditor();
    renderQueue();
    renderBottomJobs();
    refreshValidationState();
  }

  function renderTabs() {
    const fragment = document.createDocumentFragment();
    for (const [key, buffer] of state.tabs) {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "editor-tab";
      button.classList.toggle("active", key === state.activeKey);
      const icon = document.createElement("span");
      icon.textContent = buffer.path === "project.vpe" ? "V" : "·";
      const name = document.createElement("span");
      name.textContent = buffer.path.split("/").pop();
      button.append(icon, name);
      if (buffer.dirty) {
        const dirty = document.createElement("span");
        dirty.className = "tab-dirty";
        dirty.textContent = "●";
        button.appendChild(dirty);
      }
      button.title = `${buffer.project}/${buffer.path}`;
      button.addEventListener("click", () => activateTab(key));
      fragment.appendChild(button);
    }
    elements.tabs.replaceChildren(fragment);
  }

  function renderEditor() {
    const buffer = activeBuffer();
    if (!buffer) {
      elements.editor.disabled = true;
      elements.editor.value = "";
      elements.breadcrumbs.textContent = "No file open";
      elements.readOnlyBanner.classList.add("hidden");
      elements.remoteBanner.classList.add("hidden");
      updateEditorMetrics();
      renderInspector(parseVpePreview(""));
      return;
    }
    elements.editor.disabled = buffer.readOnly;
    if (elements.editor.value !== buffer.content) elements.editor.value = buffer.content;
    elements.breadcrumbs.textContent = `${buffer.project} › ${buffer.path}`;
    elements.readOnlyBanner.classList.toggle("hidden", !buffer.readOnly);
    elements.remoteBanner.classList.toggle("hidden", !buffer.remoteAvailable);
    elements.saveButton.disabled = buffer.readOnly || !buffer.dirty;
    elements.validateButton.disabled = buffer.readOnly || buffer.dirty;
    updateEditorMetrics();
    renderInspector(buffer.path === "project.vpe" ? parseVpePreview(buffer.content) : parseVpePreview(""));
  }

  function updateEditorMetrics() {
    const metrics = lineMetrics(elements.editor.value, elements.editor.selectionStart);
    const fragment = document.createDocumentFragment();
    for (let index = 1; index <= metrics.lineCount; index += 1) {
      const line = document.createElement("div");
      line.className = "gutter-line";
      line.classList.toggle("active", index === metrics.line);
      line.textContent = String(index);
      fragment.appendChild(line);
    }
    elements.gutter.replaceChildren(fragment);
    elements.gutter.scrollTop = elements.editor.scrollTop;
    elements.cursorStatus.textContent = `Ln ${metrics.line}, Col ${metrics.column}`;
    const bytes = new TextEncoder().encode(elements.editor.value).length;
    elements.sizeStatus.textContent = formatBytes(bytes);
    const buffer = activeBuffer();
    elements.dirtyStatus.textContent = buffer && buffer.dirty ? "Modified" : "Clean";
    elements.revisionStatus.textContent = buffer ? `Revision ${buffer.revision}` : "Revision —";
  }

  function formatBytes(bytes) {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
    return `${(bytes / (1024 * 1024)).toFixed(2)} MiB`;
  }

  function findProject(slug) {
    return state.projects.find((project) => project.slug === slug) || null;
  }

  function refreshValidationState() {
    const buffer = activeBuffer();
    const project = buffer ? findProject(buffer.project) : null;
    const valid = Boolean(
      buffer &&
      project &&
      !buffer.dirty &&
      Number(project.validated_revision) === Number(buffer.revision),
    );
    elements.exportButton.disabled = !valid;
    elements.validationBadge.className = `status-badge ${valid ? "valid" : "neutral"}`;
    elements.validationBadge.textContent = valid ? "Validated" : "Not validated";
  }

  async function saveActive() {
    const buffer = activeBuffer();
    if (!buffer || buffer.readOnly || !buffer.dirty) return;
    const bytes = new TextEncoder().encode(buffer.content).length;
    if (bytes > MAX_DOCUMENT_BYTES) {
      showToast("project.vpe exceeds the 2 MiB document limit.");
      return;
    }
    elements.saveButton.disabled = true;
    try {
      const result = await callEditor("write_file", {
        project: buffer.project,
        path: "project.vpe",
        content: buffer.content,
        expected_revision: buffer.revision,
      });
      buffer.revision = Number(result.revision);
      buffer.baseContent = buffer.content;
      buffer.dirty = false;
      buffer.remoteAvailable = false;
      state.remoteProject = null;
      mergeProject({
        slug: buffer.project,
        revision: result.revision,
        validated_revision: result.validated_revision,
        valid: Boolean(result.valid),
        sha256: result.sha256,
      });
      showToast(`Saved Revision ${buffer.revision}.`);
      renderAll();
    } catch (error) {
      if (error.kind === "revision_conflict") {
        buffer.remoteAvailable = true;
        state.remoteProject = findProject(buffer.project);
        renderEditor();
      }
      showFailure(error, "Save");
    } finally {
      renderAll();
    }
  }

  async function validateActive() {
    const buffer = activeBuffer();
    if (!buffer || buffer.readOnly || buffer.dirty) return;
    elements.validateButton.disabled = true;
    clearProblems();
    try {
      const result = await callEditor("validate", { project: buffer.project });
      state.parsedDocument = result.document || null;
      mergeProject({
        slug: buffer.project,
        revision: result.revision,
        validated_revision: result.revision,
        valid: true,
        sha256: result.sha256,
      });
      showToast(`Revision ${result.revision} validated.`);
      renderInspectorFromDocument(result.document);
    } catch (error) {
      elements.validationBadge.className = "status-badge invalid";
      elements.validationBadge.textContent = "Invalid";
      addProblem("Error", "Authoritative validation failed. Review VPE syntax and timeline continuity.");
      showFailure(error, "Validation");
    } finally {
      renderAll();
    }
  }

  async function exportActive() {
    const buffer = activeBuffer();
    const project = buffer && findProject(buffer.project);
    if (!buffer || buffer.dirty || !project || Number(project.validated_revision) !== buffer.revision) {
      showToast("Validate the current saved revision before export.");
      return;
    }
    elements.exportButton.disabled = true;
    try {
      const result = await callEditor("export", { project: buffer.project });
      const job = result.render && result.render.job;
      if (job && job.id) state.jobs.set(job.id, job);
      showToast(`Export queued for Revision ${result.revision}.`);
      renderQueue();
      renderBottomJobs();
    } catch (error) {
      showFailure(error, "Export");
    } finally {
      refreshValidationState();
    }
  }

  function mergeProject(project) {
    const index = state.projects.findIndex((candidate) => candidate.slug === project.slug);
    if (index >= 0) state.projects[index] = Object.assign({}, state.projects[index], project);
    else state.projects.push(project);
    state.projects.sort((a, b) => String(a.slug).localeCompare(String(b.slug)));
  }

  function handleProjectEvent(project) {
    if (!project || !project.slug) return;
    mergeProject(project);
    for (const buffer of state.tabs.values()) {
      const decision = remoteRevisionDecision(buffer, project);
      if (decision.remoteAvailable) {
        buffer.remoteAvailable = true;
        if (buffer === activeBuffer()) state.remoteProject = project;
      }
      if (decision.reload) {
        openFile(buffer.project, buffer.path, true);
      }
    }
    renderAll();
  }

  function applyJob(job) {
    if (!job || !job.id) return;
    state.jobs.set(job.id, job);
    renderQueue();
    renderBottomJobs();
  }

  function connectEvents() {
    if (!global.EventSource || !state.authenticated) {
      setConnection("offline");
      return;
    }
    if (state.eventSource) state.eventSource.close();
    setConnection("connecting");
    const source = new EventSource("/api/editor/events", { withCredentials: true });
    state.eventSource = source;
    source.addEventListener("open", () => setConnection("online"));
    source.addEventListener("snapshot", (event) => {
      const payload = safeEventJson(event.data);
      if (!payload) return;
      applyProjects(payload.projects || []);
      for (const job of payload.jobs || []) applyJob(job);
      if (!state.activeProject && state.projects.length) openProject(state.projects[0].slug);
    });
    source.addEventListener("projects_changed", (event) => {
      const payload = safeEventJson(event.data);
      if (payload) applyProjects(payload.projects || []);
    });
    source.addEventListener("project_changed", (event) => {
      const payload = safeEventJson(event.data);
      if (payload) handleProjectEvent(payload.project);
    });
    source.addEventListener("job_changed", (event) => {
      const payload = safeEventJson(event.data);
      if (payload) applyJob(payload.job);
    });
    source.addEventListener("heartbeat", () => setConnection("online"));
    source.onerror = () => setConnection("offline");
  }

  function safeEventJson(data) {
    try {
      const parsed = JSON.parse(data);
      return parsed && typeof parsed === "object" ? parsed : null;
    } catch (_) {
      return null;
    }
  }

  async function pollKnownJobs() {
    for (const job of Array.from(state.jobs.values())) {
      if (TERMINAL_JOB_STATES.has(job.status)) continue;
      try {
        const payload = await callEditor("get_job", { job_id: job.id });
        applyJob(payload.job);
      } catch (error) {
        if (error.kind === "authentication_required") showFailure(error, "Job refresh");
      }
    }
  }

  function projectJobs() {
    return Array.from(state.jobs.values())
      .filter((job) => !state.activeProject || job.project_id === state.activeProject)
      .sort((a, b) => Number(b.enqueue_seq) - Number(a.enqueue_seq));
  }

  function renderQueue() {
    const jobs = projectJobs();
    if (!jobs.length) {
      const empty = document.createElement("div");
      empty.className = "empty-state";
      empty.textContent = "No project jobs.";
      elements.queue.replaceChildren(empty);
      return;
    }
    const fragment = document.createDocumentFragment();
    for (const job of jobs.slice(0, 24)) {
      const article = document.createElement("article");
      article.className = "queue-job";
      const head = document.createElement("div");
      head.className = "queue-job-head";
      const kind = document.createElement("b");
      kind.textContent = String(job.kind || "job").replaceAll("_", " ");
      const status = document.createElement("span");
      status.className = `queue-job-state ${job.status}`;
      status.textContent = job.status;
      head.append(kind, status);
      const meta = document.createElement("div");
      meta.className = "queue-job-meta";
      meta.textContent = `Revision ${job.project_revision || "—"} · Queue ${job.enqueue_seq}`;
      article.append(head, meta);
      if (job.status === "queued" || job.status === "running") {
        const cancel = document.createElement("button");
        cancel.type = "button";
        cancel.className = "cancel-job";
        cancel.textContent = "Cancel job";
        cancel.addEventListener("click", () => cancelJob(job.id));
        article.appendChild(cancel);
      }
      fragment.appendChild(article);
    }
    elements.queue.replaceChildren(fragment);
  }

  function renderBottomJobs() {
    const jobs = projectJobs();
    elements.jobCount.textContent = String(jobs.length);
    if (!jobs.length) {
      const empty = document.createElement("div");
      empty.className = "empty-state";
      empty.textContent = "No jobs for this project.";
      elements.jobsPanel.replaceChildren(empty);
      return;
    }
    const fragment = document.createDocumentFragment();
    for (const job of jobs) {
      const row = document.createElement("div");
      row.className = "bottom-job-row";
      const status = document.createElement("span");
      status.textContent = job.status;
      const kind = document.createElement("span");
      kind.textContent = String(job.kind || "job").replaceAll("_", " ");
      const revision = document.createElement("span");
      revision.textContent = `r${job.project_revision || "—"}`;
      row.append(status, kind, revision);
      fragment.appendChild(row);
    }
    elements.jobsPanel.replaceChildren(fragment);
  }

  async function cancelJob(id) {
    try {
      const payload = await callEditor("cancel_job", { job_id: id });
      applyJob(payload.job);
      showToast("Cancellation requested.");
    } catch (error) {
      showFailure(error, "Cancellation");
    }
  }

  function secondsFromTimecode(value) {
    const match = /^(\d+):(\d{2}):(\d{2}(?:\.\d+)?)$/.exec(value);
    if (!match) return 0;
    return Number(match[1]) * 3600 + Number(match[2]) * 60 + Number(match[3]);
  }

  function parseVpePreview(content) {
    const preview = {
      duration: 0,
      mainTracks: [],
      overlayTracks: [],
      markers: [],
      transitions: [],
      variants: [],
      gates: [],
      problems: [],
    };
    let currentTrack = null;
    let braceBalance = 0;
    for (const [index, line] of content.split("\n").entries()) {
      braceBalance += (line.match(/{/g) || []).length;
      braceBalance -= (line.match(/}/g) || []).length;
      const track = line.match(/^\s*track\s+(main|overlay)(?:\s+([A-Za-z0-9_-]+))?/);
      if (track) {
        currentTrack = { kind: track[1], name: track[2] || track[1], clips: [] };
        (track[1] === "main" ? preview.mainTracks : preview.overlayTracks).push(currentTrack);
      }
      const clip = line.match(
        /^\s*clip\s+([A-Za-z0-9_-]+)\s+source\s+(\d+:\d{2}:\d{2}(?:\.\d+)?)\.\.(\d+:\d{2}:\d{2}(?:\.\d+)?)\s+at\s+(\d+:\d{2}:\d{2}(?:\.\d+)?)/,
      );
      if (clip && currentTrack) {
        const length = Math.max(0, secondsFromTimecode(clip[3]) - secondsFromTimecode(clip[2]));
        const start = secondsFromTimecode(clip[4]);
        currentTrack.clips.push({ label: clip[1], start, end: start + length });
        preview.duration = Math.max(preview.duration, start + length);
      }
      const hold = line.match(
        /^\s*hold\s+([A-Za-z0-9_-]+)\s+at\s+(\d+:\d{2}:\d{2}(?:\.\d+)?)\.\.(\d+:\d{2}:\d{2}(?:\.\d+)?)/,
      );
      if (hold && currentTrack) {
        currentTrack.clips.push({
          label: `hold ${hold[1]}`,
          start: secondsFromTimecode(hold[2]),
          end: secondsFromTimecode(hold[3]),
        });
      }
      const marker = line.match(/^\s*marker\s+"([^"]+)"\s+at\s+(\d+:\d{2}:\d{2}(?:\.\d+)?)/);
      if (marker) preview.markers.push({ name: marker[1], time: secondsFromTimecode(marker[2]) });
      const transition = line.match(
        /^\s*transition\s+([A-Za-z0-9_-]+)\s+at\s+(\d+:\d{2}:\d{2}(?:\.\d+)?)/,
      );
      if (transition) preview.transitions.push({ name: transition[1], time: secondsFromTimecode(transition[2]) });
      const variant = line.match(/^\s*variant\s+"([^"]+)"\s+aspect\s+([0-9:]+)/);
      if (variant) preview.variants.push({ language: variant[1], aspect: variant[2] });
      const gate = line.match(/^\s*gate\s+([A-Za-z0-9_-]+)\s+require\s+(.+)/);
      if (gate) preview.gates.push({ phase: gate[1], requirements: gate[2] });
      if (braceBalance < 0) preview.problems.push({ line: index + 1, message: "Unexpected closing brace." });
    }
    if (braceBalance !== 0 && content) {
      preview.problems.push({ line: content.split("\n").length, message: "Unbalanced project braces." });
    }
    return preview;
  }

  function renderInspector(preview) {
    state.parsedDocument = null;
    elements.durationMetric.textContent = preview.duration ? `${preview.duration.toFixed(3)} s` : "—";
    elements.mainTrackMetric.textContent = String(preview.mainTracks.length);
    elements.overlayTrackMetric.textContent = String(preview.overlayTracks.length);
    elements.markerMetric.textContent = String(preview.markers.length);
    renderTimelineTracks(preview.mainTracks, preview.overlayTracks, preview.duration);
    renderOutline(preview);
    renderProblems(preview.problems);
  }

  function renderInspectorFromDocument(documentValue) {
    if (!documentValue || !documentValue.timeline) return;
    const timeline = documentValue.timeline;
    const mainTracks = (timeline.main_tracks || []).map((track, index) => ({
      name: track.name || `main ${index + 1}`,
      clips: (track.clips || []).map((clip) => ({
        label: clip.source,
        start: clip.timeline_in,
        end: clip.timeline_out,
      })),
    }));
    const overlayTracks = (timeline.overlay_tracks || []).map((track, index) => {
      const clip = track.clip || track;
      return {
        name: track.track || track.kind || `overlay ${index + 1}`,
        clips: [{
          label: track.kind || "overlay",
          start: clip.timeline_in || 0,
          end: clip.timeline_out || 0,
        }],
      };
    });
    const duration = mainTracks.flatMap((track) => track.clips)
      .reduce((maximum, clip) => Math.max(maximum, Number(clip.end) || 0), 0);
    const preview = {
      duration,
      mainTracks,
      overlayTracks,
      markers: timeline.markers || [],
      transitions: timeline.transitions || [],
      variants: timeline.variants || [],
      gates: documentValue.gates || [],
      problems: [],
    };
    elements.durationMetric.textContent = `${duration.toFixed(3)} s`;
    elements.mainTrackMetric.textContent = String(mainTracks.length);
    elements.overlayTrackMetric.textContent = String(overlayTracks.length);
    elements.markerMetric.textContent = String(preview.markers.length);
    renderTimelineTracks(mainTracks, overlayTracks, duration);
    renderOutline(preview);
  }

  function renderTimelineTracks(mainTracks, overlayTracks, duration) {
    const tracks = mainTracks.map((track) => ({ ...track, overlay: false }))
      .concat(overlayTracks.map((track) => ({ ...track, overlay: true })));
    if (!tracks.length) {
      const empty = document.createElement("div");
      empty.className = "empty-state";
      empty.textContent = "No timeline tracks detected.";
      elements.timeline.replaceChildren(empty);
      return;
    }
    const fragment = document.createDocumentFragment();
    for (const track of tracks) {
      const row = document.createElement("div");
      row.className = "timeline-track";
      const label = document.createElement("span");
      label.className = "track-label";
      label.textContent = track.name;
      const lane = document.createElement("div");
      lane.className = "track-lane";
      for (const clip of track.clips) {
        const node = document.createElement("span");
        node.className = `track-clip${track.overlay ? " overlay" : ""}`;
        const span = Math.max(0.02, (Number(clip.end) - Number(clip.start)) / Math.max(duration, 0.001));
        node.style.flex = `${span} 1 22px`;
        node.textContent = clip.label;
        lane.appendChild(node);
      }
      row.append(label, lane);
      fragment.appendChild(row);
    }
    elements.timeline.replaceChildren(fragment);
  }

  function renderOutline(preview) {
    const nodes = [];
    for (const marker of preview.markers) nodes.push(["MARKER", marker.name || "marker", "✓"]);
    for (const transition of preview.transitions) nodes.push(["TRANSITION", transition.name || transition.kind, "✓"]);
    for (const variant of preview.variants) nodes.push(["VARIANT", `${variant.language} · ${variant.aspect}`, "✓"]);
    for (const gate of preview.gates) nodes.push(["GATE", `${gate.phase}`, "✓"]);
    if (!nodes.length) {
      const empty = document.createElement("div");
      empty.className = "empty-state";
      empty.textContent = "No markers, transitions, variants, or gates detected.";
      elements.outline.replaceChildren(empty);
      return;
    }
    const fragment = document.createDocumentFragment();
    for (const [kind, label, status] of nodes) {
      const row = document.createElement("div");
      row.className = "outline-row";
      const kindNode = document.createElement("span");
      kindNode.className = "outline-kind";
      kindNode.textContent = kind;
      const labelNode = document.createElement("span");
      labelNode.textContent = label;
      const statusNode = document.createElement("span");
      statusNode.className = "outline-state";
      statusNode.textContent = status;
      row.append(kindNode, labelNode, statusNode);
      fragment.appendChild(row);
    }
    elements.outline.replaceChildren(fragment);
  }

  function clearProblems() {
    renderProblems([]);
  }

  function addProblem(severity, message) {
    const current = Array.from(elements.problemsPanel.querySelectorAll(".problem-row")).map((row) => ({
      severity: row.children[0].textContent,
      message: row.children[1].textContent,
      line: row.children[2].textContent,
    }));
    current.push({ severity, message, line: "—" });
    renderProblems(current);
  }

  function renderProblems(problems) {
    elements.problemCount.textContent = String(problems.length);
    if (!problems.length) {
      const empty = document.createElement("div");
      empty.className = "empty-state";
      empty.textContent = "No problems detected.";
      elements.problemsPanel.replaceChildren(empty);
      return;
    }
    const fragment = document.createDocumentFragment();
    for (const problem of problems) {
      const row = document.createElement("div");
      row.className = "problem-row";
      const severity = document.createElement("span");
      severity.className = "problem-severity";
      severity.textContent = problem.severity || "Warning";
      const message = document.createElement("span");
      message.textContent = problem.message;
      const line = document.createElement("span");
      line.textContent = problem.line ? `Ln ${problem.line}` : "—";
      row.append(severity, message, line);
      fragment.appendChild(row);
    }
    elements.problemsPanel.replaceChildren(fragment);
  }

  async function createProject(event) {
    event.preventDefault();
    const slug = elements.projectSlug.value.trim();
    if (!/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(slug)) {
      elements.createError.textContent = "Use lowercase letters, numbers, and single hyphens.";
      return;
    }
    elements.createError.textContent = "";
    try {
      const result = await callEditor("create_project", { slug });
      elements.dialog.close();
      elements.createForm.reset();
      await refreshProjects(result.project);
      showToast(`Created ${result.project}.`);
    } catch (error) {
      elements.createError.textContent =
        error.kind === "revision_conflict" ? "That project already exists." : "Project creation failed.";
      showFailure(error, "Project creation");
    }
  }

  function bindEvents() {
    elements.editor.addEventListener("input", () => {
      const buffer = activeBuffer();
      if (!buffer || buffer.readOnly) return;
      buffer.content = elements.editor.value;
      buffer.dirty = buffer.content !== buffer.baseContent;
      elements.saveButton.disabled = buffer.readOnly || !buffer.dirty;
      elements.validateButton.disabled = buffer.readOnly || buffer.dirty;
      renderTabs();
      updateEditorMetrics();
      renderInspector(parseVpePreview(buffer.content));
      refreshValidationState();
    });
    for (const eventName of ["click", "keyup", "select"]) {
      elements.editor.addEventListener(eventName, updateEditorMetrics);
    }
    elements.editor.addEventListener("scroll", () => {
      elements.gutter.scrollTop = elements.editor.scrollTop;
    });
    document.addEventListener("keydown", (event) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s") {
        event.preventDefault();
        saveActive();
      }
    });
    elements.saveButton.addEventListener("click", saveActive);
    elements.validateButton.addEventListener("click", validateActive);
    elements.exportButton.addEventListener("click", exportActive);
    document.getElementById("refreshJobsButton").addEventListener("click", pollKnownJobs);
    elements.reloadRemoteButton.addEventListener("click", () => {
      const buffer = activeBuffer();
      if (buffer) openFile(buffer.project, buffer.path, true);
    });
    elements.keepEditingButton.addEventListener("click", () => {
      const buffer = activeBuffer();
      if (buffer) buffer.remoteAvailable = false;
      elements.remoteBanner.classList.add("hidden");
    });
    elements.newProjectButton.addEventListener("click", () => {
      elements.createError.textContent = "";
      elements.dialog.showModal();
      elements.projectSlug.focus();
    });
    document.getElementById("closeProjectDialog").addEventListener("click", () => elements.dialog.close());
    document.getElementById("cancelProjectDialog").addEventListener("click", () => elements.dialog.close());
    elements.createForm.addEventListener("submit", createProject);
    document.querySelectorAll(".bottom-tab").forEach((button) => {
      button.addEventListener("click", () => {
        document.querySelectorAll(".bottom-tab").forEach((tab) => tab.classList.remove("active"));
        button.classList.add("active");
        const jobs = button.dataset.panel === "jobs";
        elements.jobsPanel.classList.toggle("hidden", !jobs);
        elements.problemsPanel.classList.toggle("hidden", jobs);
      });
    });
    document.getElementById("collapseBottomButton").addEventListener("click", () => {
      elements.bottomPanel.classList.toggle("collapsed");
    });
  }

  async function start() {
    bindEvents();
    setConnection("connecting");
    renderInspector(parseVpePreview(""));
    await refreshProjects();
    if (state.authenticated) connectEvents();
    state.pollTimer = setInterval(pollKnownJobs, 3000);
    state.projectPollTimer = setInterval(() => {
      if (!state.online) refreshProjects();
    }, 10000);
  }

  start();
})(typeof globalThis !== "undefined" ? globalThis : this);
