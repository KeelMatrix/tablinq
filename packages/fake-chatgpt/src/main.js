const root = document.querySelector("#app-root");
const note = document.querySelector("#variant-note");
const params = new URLSearchParams(window.location.search);
const variant = params.get("variant") === "aria" ? "aria" : "classic";
const scenario = params.get("scenario") || "success";
const missingControls = new Set(
  (params.get("missing") || "")
    .split(",")
    .map((value) => value.trim().toLowerCase())
    .filter(Boolean),
);

document.body.dataset.uiVariant = variant;
document.body.dataset.streamState = "idle";
document.body.dataset.selectedModel = "thinking";
document.body.dataset.selectedEffort = "standard";
document.body.dataset.selectedMode = "normal";
document.body.dataset.uploadedFiles = "";

note.textContent =
  variant === "aria"
    ? "ARIA-first variant with moved controls, renamed labels, and semantic groups."
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
  "[data-slot='composer'], [role='textbox']",
);
const submit = document.querySelector(
  "[data-action='submit'], [aria-label='Submit prompt']",
);
const uploadEntry = document.querySelector("[data-slot='upload-entry']");
const uploadInput = document.querySelector("input[type='file']");
const uploadList = document.querySelector("[data-slot='upload-list']");
const messageArticle = document.querySelector(
  "[data-slot='assistant-message']",
);
const message = document.querySelector("[data-slot='assistant-message'] p");
const artifactLink = document.querySelector("[data-slot='artifact-link']");
let activeTimer = null;

function currentConversationId() {
  const url = new URL(window.location.href);
  return (
    document.body.dataset.conversationId ||
    url.searchParams.get("conversation") ||
    `conv-${Date.now()}`
  );
}

function streamStorageKey(conversationId) {
  return `tablinq-stream:${conversationId}`;
}

function writeConversationState(conversationId, messageId) {
  document.body.dataset.conversationId = conversationId;
  document.body.dataset.lastMessageId = messageId;
  const url = new URL(window.location.href);
  url.searchParams.set("conversation", conversationId);
  history.replaceState({}, "", url);
}

function setStreamState(state) {
  document.body.dataset.streamState = state;
  messageArticle?.setAttribute("data-stream-state", state);
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
    .map(
      (fileName) =>
        `<span class="pill" data-slot="uploaded-file">${fileName}</span>`,
    )
    .join("");
}

function resetArtifactLink() {
  if (!artifactLink) {
    return;
  }
  artifactLink.setAttribute("href", "/artifact.txt");
  artifactLink.setAttribute("download", "artifact.txt");
  artifactLink.setAttribute("data-artifact-ready", "false");
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
  artifactLink.setAttribute("data-artifact-ready", "true");
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
  sessionStorage.setItem(
    streamStorageKey(state.conversationId),
    JSON.stringify(state),
  );
}

function clearState(conversationId) {
  sessionStorage.removeItem(streamStorageKey(conversationId));
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
  prepareArtifact(state.promptText);
  clearState(state.conversationId);
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
  const messageId = `msg-${Date.now()}`;
  const state = {
    conversationId,
    messageId,
    promptText,
    responseText: promptText
      ? `Fake response for: ${promptText}`
      : "Fake response for an empty prompt.",
    index: 0,
    didReload: false,
  };

  writeConversationState(conversationId, messageId);
  setStreamState("running");
  message.textContent = "";
  renderChunk(state);
  persistState(state);
  scheduleNextTick(state);
}

function resumeStreamIfNeeded() {
  const conversationId = currentConversationId();
  const raw = sessionStorage.getItem(streamStorageKey(conversationId));
  if (!raw) {
    return;
  }

  const state = JSON.parse(raw);
  writeConversationState(state.conversationId, state.messageId);
  setStreamState("running");
  renderChunk(state);

  if (scenario === "timeout") {
    persistState(state);
    return;
  }

  scheduleNextTick(state);
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
};

submit?.addEventListener("click", () => {
  if (!composer || !message) {
    return;
  }

  stopTimer();
  const promptText =
    "value" in composer ? composer.value.trim() : composer.textContent.trim();
  startStream(promptText);
});

syncSelectedState();
renderUploadedFiles(uploadedFiles());
resetArtifactLink();
resumeStreamIfNeeded();
