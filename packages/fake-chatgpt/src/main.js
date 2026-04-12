const root = document.querySelector("#app-root");
const note = document.querySelector("#variant-note");
const params = new URLSearchParams(window.location.search);
const variant = params.get("variant") === "aria" ? "aria" : "classic";
const scenario = params.get("scenario") || "success";

document.body.dataset.uiVariant = variant;
document.body.dataset.streamState = "idle";
note.textContent =
  variant === "aria"
    ? "ARIA-first variant with moved controls and renamed labels."
    : "Classic variant with data-slot anchors and direct toolbar controls.";

root.innerHTML =
  variant === "aria"
    ? `
      <section class="stacked-shell" aria-label="assistant workspace">
        <div class="stacked-toolbar">
          <button type="button" class="pill" aria-haspopup="listbox">
            Choose engine: GPT-5.3 Instant / GPT-5.4 Thinking
          </button>
          <button type="button" class="pill" aria-pressed="false">Browse the web</button>
          <button type="button" class="pill" aria-pressed="false">Deep research</button>
          <button type="button" class="pill" aria-label="Reasoning effort">
            Extended reasoning
          </button>
          <label class="pill" for="upload-input">Add files</label>
          <input id="upload-input" type="file" hidden multiple />
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
        <div class="transcript">
          <article
            class="message"
            data-slot="assistant-message"
            data-stream-state="idle"
          >
            <p>Streaming output lands here.</p>
            <a href="/artifact.txt" download data-slot="artifact-link">Download artifact.txt</a>
          </article>
        </div>
      </section>
    `
    : `
      <section class="shell" aria-label="fake-chatgpt-shell">
        <div class="toolbar">
          <button
            type="button"
            data-slot="model-picker"
            aria-haspopup="listbox"
          >
            GPT-5.4 Thinking
          </button>
          <button type="button" data-mode="search">Search</button>
          <button type="button" data-mode="research">Research</button>
          <button type="button" data-slot="effort-toggle">Standard</button>
          <button type="button" data-slot="upload-entry">Attach</button>
        </div>
        <div class="composer">
          <textarea
            aria-label="Prompt composer"
            data-slot="composer"
            placeholder="Ask fake ChatGPT something"
          ></textarea>
          <button type="button" data-action="submit">Send</button>
        </div>
        <div class="transcript">
          <article
            class="message"
            data-slot="assistant-message"
            data-stream-state="idle"
          >
            <p>Streaming output lands here.</p>
            <a href="/artifact.txt" download data-slot="artifact-link">artifact.txt</a>
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
const messageArticle = document.querySelector(
  "[data-slot='assistant-message']",
);
const message = document.querySelector("[data-slot='assistant-message'] p");
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

resumeStreamIfNeeded();
