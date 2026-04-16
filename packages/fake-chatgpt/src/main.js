const root = document.querySelector("#app-root");
const note = document.querySelector("#variant-note");
const params = new URLSearchParams(window.location.search);
const requestedVariant = params.get("variant");
const variant =
  requestedVariant === "aria" || requestedVariant === "live"
    ? requestedVariant
    : "classic";
const scenario = params.get("scenario") || "success";
const missingControls = new Set(
  (params.get("missing") || "")
    .split(",")
    .map((value) => value.trim().toLowerCase())
    .filter(Boolean),
);

function pathnameSegments(pathname) {
  return pathname.split("/").filter(Boolean);
}

function pathnameFromSegments(segments) {
  return segments.length > 0 ? `/${segments.join("/")}` : "/";
}

function conversationIdFromPathname(pathname) {
  const segments = pathnameSegments(pathname);
  if (
    segments.length >= 2 &&
    segments[segments.length - 2]?.toLowerCase() === "c"
  ) {
    return segments[segments.length - 1] || null;
  }
  return null;
}

function conversationWorkspacePathname(pathname) {
  const segments = pathnameSegments(pathname);
  if (
    segments.length >= 2 &&
    segments[segments.length - 2]?.toLowerCase() === "c"
  ) {
    return pathnameFromSegments(segments.slice(0, -2));
  }
  return pathnameFromSegments(segments);
}

function conversationPathname(pathname, conversationId) {
  const segments = pathnameSegments(conversationWorkspacePathname(pathname));
  segments.push("c", conversationId);
  return pathnameFromSegments(segments);
}

document.body.dataset.uiVariant = variant;
document.body.dataset.streamState = "idle";
document.body.dataset.selectedModel = "thinking";
document.body.dataset.selectedEffort = "standard";
document.body.dataset.selectedMode = "normal";
document.body.dataset.uploadedFiles = "";
document.body.dataset.researchState = "idle";
document.body.dataset.researchStatusMessage = "";
document.body.dataset.conversationId =
  conversationIdFromPathname(window.location.pathname) || "";
document.body.dataset.lastMessageId = "";

note.textContent =
  variant === "aria"
    ? "ARIA-first variant with moved controls, renamed labels, and semantic groups."
    : variant === "live"
      ? "Live-like variant with prompt-textarea, send/stop buttons, upload chips, and download controls."
      : "Classic variant with grouped controls and direct toolbar buttons.";

function hasControl(name) {
  return !missingControls.has(name);
}

function classicModelControls() {
  if (!hasControl("model")) {
    return "";
  }
  return `
    <div class="pill" data-slot="model-picker" role="group" aria-label="Model picker">
      <button type="button" data-model="instant" aria-pressed="false">GPT-5.3 Instant</button>
      <button type="button" data-model="thinking" aria-pressed="true">GPT-5.4 Thinking</button>
    </div>
  `;
}

function classicModeControls() {
  return [
    hasControl("search")
      ? `<button type="button" data-mode="search" aria-pressed="false">Search</button>`
      : "",
    hasControl("research")
      ? `<button type="button" data-mode="research" aria-pressed="false">Research</button>`
      : "",
  ].join("");
}

function classicEffortControls() {
  if (!hasControl("effort")) {
    return "";
  }
  return `
    <div class="pill" data-slot="effort-toggle" role="group" aria-label="Thinking effort">
      <button type="button" data-effort="standard" aria-pressed="true">Standard</button>
      <button type="button" data-effort="extended" aria-pressed="false">Extended</button>
    </div>
  `;
}

function classicUploadControl() {
  if (!hasControl("upload")) {
    return "";
  }
  return `
    <button type="button" data-slot="upload-entry">Attach</button>
    <input id="classic-upload-input" type="file" hidden multiple />
  `;
}

function ariaModelControls() {
  if (!hasControl("model")) {
    return "";
  }
  return `
    <div class="pill" role="group" aria-label="Choose engine">
      <button type="button" data-model="instant" aria-pressed="false">Instant answers</button>
      <button type="button" data-model="thinking" aria-pressed="true">Deep thinking</button>
    </div>
  `;
}

function ariaModeControls() {
  return [
    hasControl("search")
      ? `<button type="button" data-mode="search" aria-pressed="false">Browse the web</button>`
      : "",
    hasControl("research")
      ? `<button type="button" data-mode="research" aria-pressed="false">Deep research</button>`
      : "",
  ].join("");
}

function ariaEffortControls() {
  if (!hasControl("effort")) {
    return "";
  }
  return `
    <div class="pill" role="group" aria-label="Reasoning effort">
      <button type="button" data-effort="standard" aria-pressed="true">Standard reasoning</button>
      <button type="button" data-effort="extended" aria-pressed="false">Extended reasoning</button>
    </div>
  `;
}

function ariaUploadControl() {
  if (!hasControl("upload")) {
    return "";
  }
  return `
    <label class="pill" for="upload-input">Add files</label>
    <input id="upload-input" type="file" hidden multiple />
  `;
}

function liveModelControls() {
  if (!hasControl("model")) {
    return "";
  }
  return `
    <div class="pill" role="group" aria-label="Choose a model">
      <button type="button" data-model="instant" aria-pressed="false">GPT-5.3 Instant</button>
      <button type="button" data-model="thinking" aria-pressed="true">GPT-5.4 Thinking</button>
    </div>
  `;
}

function liveModeControls() {
  return [
    hasControl("search")
      ? `<button type="button" data-mode="search" aria-label="Search" aria-pressed="false">Search</button>`
      : "",
    hasControl("research")
      ? `<button type="button" data-mode="research" aria-label="Deep research" aria-pressed="false">Deep research</button>`
      : "",
  ].join("");
}

function liveEffortControls() {
  if (!hasControl("effort")) {
    return "";
  }
  return `
    <div class="pill" role="group" aria-label="Reasoning effort">
      <button type="button" data-effort="standard" aria-label="Standard reasoning" aria-pressed="true">Standard</button>
      <button type="button" data-effort="extended" aria-label="Extended reasoning" aria-pressed="false">Extended</button>
    </div>
  `;
}

function liveUploadControl() {
  if (!hasControl("upload")) {
    return "";
  }
  return `
    <button type="button" data-testid="upload-button" aria-label="Add photos and files">Add files</button>
    <input id="live-upload-input" type="file" hidden multiple />
  `;
}

root.innerHTML =
  variant === "aria"
    ? `
      <section class="stacked-shell" aria-label="assistant workspace">
        <div class="stacked-toolbar">
          ${ariaModelControls()}
          ${ariaModeControls()}
          ${ariaEffortControls()}
          ${ariaUploadControl()}
        </div>
        <div class="stacked-composer">
          <div
            role="textbox"
            aria-label="Message ChatGPT"
            contenteditable="true"
            spellcheck="false"
          ></div>
          <button type="button" aria-label="Submit prompt">Go</button>
        </div>
        <div class="stacked-toolbar" data-slot="upload-list"></div>
        <div class="transcript">
          <article
            class="message"
            data-slot="assistant-message"
            data-stream-state="idle"
          >
            <p>Streaming output lands here.</p>
            <a
              href="/artifact.txt"
              download="artifact.txt"
              data-slot="artifact-link"
              data-artifact-ready="false"
            >
              Download artifact.txt
            </a>
          </article>
        </div>
      </section>
    `
    : variant === "live"
      ? `
      <main aria-label="chatgpt workspace">
        <section class="stacked-toolbar">
          ${liveModelControls()}
          ${liveModeControls()}
          ${liveEffortControls()}
          ${liveUploadControl()}
        </section>
        <form data-testid="composer-root" class="stacked-composer">
          <textarea
            id="prompt-textarea"
            aria-label="Message ChatGPT"
            placeholder="Message ChatGPT"
          ></textarea>
          <div class="pill">
            <button type="button" data-testid="send-button" aria-label="Send prompt">Send</button>
            <button
              type="button"
              data-testid="stop-button"
              aria-label="Stop generating"
              hidden
            >
              Stop
            </button>
          </div>
        </form>
        <div class="stacked-toolbar" data-slot="upload-list"></div>
        <section class="transcript">
          <article
            class="message"
            data-message-author-role="assistant"
            data-stream-state="idle"
          >
            <div class="markdown">
              <p>Streaming output lands here.</p>
            </div>
            <a
              href="#"
              data-testid="artifact-download"
              aria-label="Download report"
            >
              Download report
            </a>
          </article>
        </section>
        <section class="notice-panel" aria-label="workspace notices">
          <article class="message" data-slot="notice-card">
            <p>Workspace notices appear here while ChatGPT is thinking.</p>
          </article>
        </section>
      </main>
    `
      : `
      <section class="shell" aria-label="fake-chatgpt-shell">
        <div class="toolbar">
          ${classicModelControls()}
          ${classicModeControls()}
          ${classicEffortControls()}
          ${classicUploadControl()}
        </div>
        <div class="composer">
          <textarea
            aria-label="Prompt composer"
            data-slot="composer"
            placeholder="Ask fake ChatGPT something"
          ></textarea>
          <button type="button" data-action="submit">Send</button>
        </div>
        <div class="toolbar" data-slot="upload-list"></div>
        <div class="transcript">
          <article
            class="message"
            data-slot="assistant-message"
            data-stream-state="idle"
          >
            <p>Streaming output lands here.</p>
            <a
              href="/artifact.txt"
              download="artifact.txt"
              data-slot="artifact-link"
              data-artifact-ready="false"
            >
              artifact.txt
            </a>
          </article>
        </div>
      </section>
    `;

const composer = document.querySelector(
  "#prompt-textarea, [data-slot='composer'], [role='textbox']",
);
const sendButton = document.querySelector(
  "[data-action='submit'], [aria-label='Submit prompt'], [data-testid='send-button'], [aria-label='Send prompt']",
);
const stopButton = document.querySelector("[data-testid='stop-button']");
const uploadEntry = document.querySelector(
  "[data-slot='upload-entry'], [data-testid='upload-button']",
);
const uploadInput = document.querySelector("input[type='file']");
const uploadList = document.querySelector("[data-slot='upload-list']");
const messageArticle = document.querySelector(
  "[data-slot='assistant-message'], [data-message-author-role='assistant']",
);
const message = document.querySelector(
  "[data-slot='assistant-message'] p, [data-message-author-role='assistant'] .markdown p",
);
const artifactLink = document.querySelector(
  "[data-slot='artifact-link'], [data-testid='artifact-download']",
);
let activeTimer = null;

function uniqueId(prefix) {
  return `${prefix}-${Date.now()}-${Math.floor(Math.random() * 1_000_000)}`;
}

function currentConversationId() {
  const url = new URL(window.location.href);
  return (
    document.body.dataset.conversationId ||
    conversationIdFromPathname(url.pathname) ||
    uniqueId("conv")
  );
}

function streamStorageKey(conversationId) {
  return `tablinq-stream:${conversationId}`;
}

function askCountStorageKey(conversationId) {
  return `tablinq-ask-count:${conversationId}`;
}

function researchStorageKey(conversationId) {
  return `tablinq-research:${conversationId}`;
}

function nextAskNumber(conversationId) {
  const key = askCountStorageKey(conversationId);
  const next = Number.parseInt(localStorage.getItem(key) || "0", 10) + 1;
  localStorage.setItem(key, String(next));
  return next;
}

function writeConversationState(conversationId, messageId) {
  document.body.dataset.conversationId = conversationId;
  document.body.dataset.lastMessageId = messageId;
  const url = new URL(window.location.href);
  url.pathname = conversationPathname(url.pathname, conversationId);
  history.replaceState({}, "", url);
}

function setStreamState(state) {
  document.body.dataset.streamState = state;
  messageArticle?.setAttribute("data-stream-state", state);
}

function setResearchState(state, statusMessage = "") {
  document.body.dataset.researchState = state;
  document.body.dataset.researchStatusMessage = statusMessage;
}

function uploadedFiles() {
  return (document.body.dataset.uploadedFiles || "")
    .split("|")
    .map((value) => value.trim())
    .filter(Boolean);
}

function renderUploadedFiles(fileNames) {
  if (!uploadList) {
    return;
  }
  uploadList.innerHTML = fileNames
    .map((fileName) =>
      variant === "live"
        ? `<span class="pill" data-testid="upload-chip" aria-label="Attached file ${fileName}">${fileName}</span>`
        : `<span class="pill" data-slot="uploaded-file">${fileName}</span>`,
    )
    .join("");
}

function setStreamingControls(running) {
  if (sendButton instanceof HTMLElement) {
    sendButton.hidden = running;
  }
  if (stopButton instanceof HTMLElement) {
    stopButton.hidden = !running;
  }
}

function resetArtifactLink() {
  if (!artifactLink) {
    return;
  }
  if (variant === "live") {
    artifactLink.setAttribute("href", "#");
    artifactLink.removeAttribute("download");
    artifactLink.removeAttribute("data-artifact-ready");
  } else {
    artifactLink.setAttribute("href", "/artifact.txt");
    artifactLink.setAttribute("download", "artifact.txt");
    artifactLink.setAttribute("data-artifact-ready", "false");
  }
}

function setUploadedFiles(fileNames) {
  document.body.dataset.uploadedFiles = fileNames.join("|");
  renderUploadedFiles(fileNames);
  resetArtifactLink();
}

function buildArtifactPayload(promptText) {
  const uploads = uploadedFiles();
  return [
    `Prompt: ${promptText || "(empty)"}`,
    `Mode: ${document.body.dataset.selectedMode}`,
    `Model: ${document.body.dataset.selectedModel}`,
    `Effort: ${document.body.dataset.selectedEffort}`,
    `Uploads: ${uploads.length ? uploads.join(", ") : "none"}`,
  ].join("\n");
}

function prepareArtifact(promptText) {
  if (!artifactLink) {
    return;
  }
  const uploads = uploadedFiles();
  if (uploads.length === 0) {
    resetArtifactLink();
    return;
  }

  const payload = buildArtifactPayload(promptText);
  const blob = new Blob([payload], { type: "text/plain;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const safeStem = uploads[0]
    .replace(/\.[^.]+$/, "")
    .replace(/[^a-z0-9._-]+/gi, "-")
    .toLowerCase();
  artifactLink.setAttribute("href", url);
  artifactLink.setAttribute(
    "download",
    `${safeStem || "artifact"}-summary.txt`,
  );
  if (variant !== "live") {
    artifactLink.setAttribute("data-artifact-ready", "true");
  }
}

function applyPressedState(selector, selectedValue) {
  document.querySelectorAll(selector).forEach((element) => {
    const key =
      element.getAttribute("data-model") ||
      element.getAttribute("data-effort") ||
      element.getAttribute("data-mode");
    const selected = key === selectedValue;
    element.setAttribute("aria-pressed", selected ? "true" : "false");
    element.dataset.selected = selected ? "true" : "false";
  });
}

function syncSelectedState() {
  applyPressedState("[data-model]", document.body.dataset.selectedModel);
  applyPressedState("[data-effort]", document.body.dataset.selectedEffort);
  const selectedMode = document.body.dataset.selectedMode;
  document.querySelectorAll("[data-mode]").forEach((element) => {
    const selected = element.getAttribute("data-mode") === selectedMode;
    element.setAttribute("aria-pressed", selected ? "true" : "false");
    element.dataset.selected = selected ? "true" : "false";
  });
}

function renderChunk(state) {
  if (!message) {
    return;
  }

  const text = state.responseText.slice(0, state.index);
  message.textContent = text;
  messageArticle?.setAttribute("data-message-id", state.messageId);
}

function persistState(state) {
  localStorage.setItem(
    streamStorageKey(state.conversationId),
    JSON.stringify(state),
  );
}

function clearState(conversationId) {
  localStorage.removeItem(streamStorageKey(conversationId));
}

function persistResearchState(state) {
  localStorage.setItem(
    researchStorageKey(state.conversationId),
    JSON.stringify(state),
  );
}

function clearResearchState(conversationId) {
  localStorage.removeItem(researchStorageKey(conversationId));
}

function stopTimer() {
  if (activeTimer) {
    clearTimeout(activeTimer);
    activeTimer = null;
  }
}

function scheduleNextTick(state) {
  activeTimer = setTimeout(() => {
    tickStream(state);
  }, 60);
}

function finishStream(state) {
  stopTimer();
  state.index = state.responseText.length;
  renderChunk(state);
  setStreamState("done");
  setStreamingControls(false);
  prepareArtifact(state.promptText);
  clearState(state.conversationId);
}

function renderResearchChunk(state) {
  if (!message) {
    return;
  }
  const text = state.reportText.slice(0, state.index);
  message.textContent = text;
  messageArticle?.setAttribute("data-message-id", state.messageId);
}

function finishResearch(state) {
  stopTimer();
  state.index = state.reportText.length;
  renderResearchChunk(state);
  setResearchState("done", "research completed");
  setStreamState("done");
  setStreamingControls(false);
  prepareArtifact(state.promptText);
  clearResearchState(state.conversationId);
}

function tickResearch(state) {
  stopTimer();
  if (state.phase === "waiting") {
    state.waitTicks += 1;
    setResearchState("waiting_for_sources", "gathering sources");
    persistResearchState(state);
    if (state.waitTicks >= 3) {
      state.phase = "running";
    }
    activeTimer = setTimeout(() => tickResearch(state), 80);
    return;
  }

  setResearchState("running", "drafting report");
  state.index = Math.min(state.reportText.length, state.index + 12);
  renderResearchChunk(state);

  if (scenario === "research-failure" && state.index >= 24) {
    setResearchState("failed", "temporary upstream issue; retryable");
    setStreamingControls(false);
    persistResearchState(state);
    stopTimer();
    return;
  }

  if (state.index >= state.reportText.length) {
    finishResearch(state);
    return;
  }

  persistResearchState(state);
  activeTimer = setTimeout(() => tickResearch(state), 80);
}

function startResearch(promptText) {
  if (!message || !messageArticle) {
    return;
  }

  const conversationId = currentConversationId();
  const messageId = uniqueId("msg");
  const state = {
    conversationId,
    messageId,
    promptText,
    reportText: promptText
      ? `Research report for: ${promptText}\n\nSources were reviewed and summarized.\nUploads: ${uploadedFiles().length ? uploadedFiles().join(", ") : "none"}`
      : "Research report for an empty query.",
    index: 0,
    phase: "waiting",
    waitTicks: 0,
  };

  writeConversationState(conversationId, messageId);
  setStreamState("running");
  setResearchState("waiting_for_sources", "gathering sources");
  setStreamingControls(true);
  message.textContent = "";
  renderResearchChunk(state);
  persistResearchState(state);
  activeTimer = setTimeout(() => tickResearch(state), 80);
}

function tickStream(state) {
  stopTimer();
  state.index = Math.min(state.responseText.length, state.index + 6);
  renderChunk(state);

  if (scenario === "reload" && !state.didReload && state.index >= 12) {
    state.didReload = true;
    persistState(state);
    window.location.reload();
    return;
  }

  if (scenario === "timeout" && state.index >= 18) {
    setStreamState("running");
    persistState(state);
    return;
  }

  if (scenario === "stall-recover" && !state.didStall && state.index >= 18) {
    state.didStall = true;
    persistState(state);
    activeTimer = setTimeout(() => tickStream(state), 2400);
    return;
  }

  if (
    scenario === "stall-recover-transient" &&
    !state.didStall &&
    state.index >= 18
  ) {
    state.didStall = true;
    persistState(state);
    setStreamState("idle");
    setStreamingControls(false);
    message.textContent = "";
    messageArticle?.removeAttribute("data-message-author-role");
    messageArticle?.removeAttribute("data-message-id");
    document.body.dataset.lastMessageId = "";
    activeTimer = setTimeout(() => {
      writeConversationState(state.conversationId, state.messageId);
      messageArticle?.setAttribute("data-message-author-role", "assistant");
      setStreamState("running");
      setStreamingControls(true);
      renderChunk(state);
      scheduleNextTick(state);
    }, 2400);
    return;
  }

  if (state.index >= state.responseText.length) {
    finishStream(state);
    return;
  }

  persistState(state);
  scheduleNextTick(state);
}

function startStream(promptText) {
  if (!message || !messageArticle) {
    return;
  }

  const conversationId = currentConversationId();
  const messageId = uniqueId("msg");
  const askNumber = nextAskNumber(conversationId);
  const state = {
    conversationId,
    messageId,
    promptText,
    responseText: promptText
      ? `Fake response for: ${promptText}`
      : "Fake response for an empty prompt.",
    index: 0,
    askNumber,
    didReload: false,
    pendingStart: false,
  };

  setStreamState("running");
  setStreamingControls(true);
  if (scenario === "followup-stale" && askNumber > 1) {
    state.pendingStart = true;
    persistState(state);
    activeTimer = setTimeout(() => {
      state.pendingStart = false;
      writeConversationState(state.conversationId, state.messageId);
      renderChunk(state);
      persistState(state);
      scheduleNextTick(state);
    }, 250);
    return;
  }

  writeConversationState(conversationId, messageId);
  message.textContent = "";
  renderChunk(state);
  persistState(state);
  scheduleNextTick(state);
}

function resumeStreamIfNeeded() {
  const conversationId = currentConversationId();
  const raw = localStorage.getItem(streamStorageKey(conversationId));
  if (!raw) {
    return;
  }

  const state = JSON.parse(raw);
  writeConversationState(state.conversationId, state.messageId);
  if (scenario === "recover-complete") {
    finishStream(state);
    return;
  }
  if (scenario === "recover-mismatch") {
    state.responseText = "Unexpected response after interruption";
    state.index = state.responseText.length;
    renderChunk(state);
    setStreamState("done");
    setStreamingControls(false);
    persistState(state);
    return;
  }
  setStreamState("running");
  if (state.pendingStart) {
    state.pendingStart = false;
  }
  renderChunk(state);

  if (scenario === "timeout") {
    persistState(state);
    return;
  }

  setStreamingControls(true);
  scheduleNextTick(state);
}

function resumeResearchIfNeeded() {
  const conversationId = currentConversationId();
  const raw = localStorage.getItem(researchStorageKey(conversationId));
  if (!raw) {
    return;
  }

  const state = JSON.parse(raw);
  writeConversationState(state.conversationId, state.messageId);
  setStreamState("running");
  setResearchState(
    state.phase === "waiting" ? "waiting_for_sources" : "running",
    state.phase === "waiting" ? "gathering sources" : "drafting report",
  );
  renderResearchChunk(state);
  setStreamingControls(true);
  activeTimer = setTimeout(() => tickResearch(state), 80);
}

document.querySelectorAll("[data-model]").forEach((element) => {
  element.addEventListener("click", () => {
    document.body.dataset.selectedModel = element.getAttribute("data-model");
    syncSelectedState();
  });
});

document.querySelectorAll("[data-effort]").forEach((element) => {
  element.addEventListener("click", () => {
    document.body.dataset.selectedEffort = element.getAttribute("data-effort");
    syncSelectedState();
  });
});

document.querySelectorAll("[data-mode]").forEach((element) => {
  element.addEventListener("click", () => {
    document.body.dataset.selectedMode = element.getAttribute("data-mode");
    syncSelectedState();
  });
});

uploadEntry?.addEventListener("click", () => {
  if (uploadInput instanceof HTMLElement) {
    uploadInput.click();
  }
});

uploadInput?.addEventListener("change", () => {
  const fileNames = Array.from(uploadInput.files || []).map(
    (file) => file.name,
  );
  setUploadedFiles(fileNames);
});

window.__tablinqCancelStream = () => {
  stopTimer();
  const conversationId = currentConversationId();
  clearState(conversationId);
  setStreamState("cancelled");
  setStreamingControls(false);
};

sendButton?.addEventListener("click", () => {
  if (!composer || !message) {
    return;
  }

  stopTimer();
  const promptText =
    "value" in composer ? composer.value.trim() : composer.textContent.trim();
  if (
    document.body.dataset.selectedMode === "research" &&
    scenario.startsWith("research-")
  ) {
    startResearch(promptText);
    return;
  }
  startStream(promptText);
});

stopButton?.addEventListener("click", () => {
  window.__tablinqCancelStream();
});

syncSelectedState();
renderUploadedFiles(uploadedFiles());
resetArtifactLink();
setStreamingControls(false);
resumeResearchIfNeeded();
resumeStreamIfNeeded();
