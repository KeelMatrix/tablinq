const root = document.querySelector("#app-root");
const note = document.querySelector("#variant-note");
const params = new URLSearchParams(window.location.search);
const requestedVariant = params.get("variant");
const variant =
  requestedVariant === "aria" ||
  requestedVariant === "live" ||
  requestedVariant === "menu"
    ? requestedVariant
    : "classic";
const scenario = params.get("scenario") || "success";
const usesComposerPlusTrap =
  scenario === "submit-upload-trap" ||
  scenario === "submit-upload-trap-no-keyboard";
const usesStaleSubmitRecovery = scenario === "submit-upload-stale-recover";
const hasDelayedUploadProcessing = scenario === "upload-processing-delay";
const hasStuckUploadProcessing = scenario === "upload-processing-stuck";
const hasBlockedSubmitAfterUpload = scenario === "submit-never-ready";
const usesDeepResearchStartingShell = scenario === "research-starting-shell";
const usesDeepResearchLanding =
  scenario === "research-deep-landing" ||
  scenario === "research-deep-landing-submit-fail";
const researchSubmissionFails =
  scenario === "research-deep-landing-submit-fail";
const usesDeepResearchFrame =
  variant === "live" &&
  (scenario === "research-success" ||
    scenario === "research-failure" ||
    scenario === "research-reload" ||
    usesDeepResearchStartingShell);
const usesProviderGenerationFailureRecovery =
  scenario === "provider-generation-failure-recover";
const usesProviderGenerationFailureAfterSuccess =
  scenario === "provider-generation-failure-after-success";
const usesProviderGenerationFailureTwice =
  scenario === "provider-generation-failure-twice";
const keyboardSubmitEnabled =
  scenario !== "submit-upload-trap-no-keyboard" &&
  scenario !== "submit-never-ready" &&
  scenario !== "submit-upload-stale-recover";
const requestedInitialModel = params.get("initial_model");
const initialModel =
  requestedInitialModel === "instant" || requestedInitialModel === "thinking"
    ? requestedInitialModel
    : "instant";
const requestedInitialEffort = params.get("initial_effort");
const initialEffort =
  requestedInitialEffort === "standard" || requestedInitialEffort === "extended"
    ? requestedInitialEffort
    : "standard";
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
  if (segments[segments.length - 1]?.toLowerCase() === "deep-research") {
    return pathnameFromSegments(segments.slice(0, -1));
  }
  return pathnameFromSegments(segments);
}

function conversationPathname(pathname, conversationId) {
  const segments = pathnameSegments(conversationWorkspacePathname(pathname));
  segments.push("c", conversationId);
  return pathnameFromSegments(segments);
}

function escapeHtml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

function deepResearchPathname(pathname) {
  const segments = pathnameSegments(conversationWorkspacePathname(pathname));
  segments.push("deep-research");
  return pathnameFromSegments(segments);
}

document.body.dataset.uiVariant = variant;
document.body.dataset.streamState = "idle";
document.body.dataset.selectedModel = initialModel;
document.body.dataset.selectedEffort = initialEffort;
document.body.dataset.selectedMode = "normal";
document.body.dataset.uploadedFiles = "";
document.body.dataset.uploadProcessing = "false";
document.body.dataset.researchState = "idle";
document.body.dataset.researchStatusMessage = "";
document.body.dataset.conversationId =
  conversationIdFromPathname(window.location.pathname) || "";
document.body.dataset.lastMessageId = "";
document.body.dataset.uploadPopoverOpen = "false";
document.body.dataset.lastSubmissionMethod = "";
document.body.dataset.staleSubmitClicks = "0";
document.body.dataset.retryClicks = "0";
document.body.dataset.keyboardSubmitReady = keyboardSubmitEnabled
  ? "true"
  : "false";
document.body.dataset.researchSubmitted = "false";
document.body.dataset.researchLandingVisited = "false";

note.textContent =
  variant === "aria"
    ? "ARIA-first variant with moved controls, renamed labels, and semantic groups."
    : variant === "menu"
      ? "Current menu variant with a ChatGPT trigger, portal menus, and composer-adjacent effort controls."
      : variant === "live"
        ? usesComposerPlusTrap
          ? "Current live variant with a ChatGPT trigger, portal menus, and a composer plus button trap."
          : "Current live variant with a ChatGPT trigger, portal menus, and composer-adjacent effort controls."
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
    <div class="pill">
      <button
        type="button"
        id="model-switcher-trigger"
        data-testid="model-switcher-dropdown-button"
        aria-label="Model selector"
        aria-haspopup="menu"
        aria-controls="tablinq-model-menu"
        aria-expanded="false"
      >
        ChatGPT
      </button>
    </div>
  `;
}

function menuModelControls() {
  return liveModelControls();
}

function liveModeControls() {
  return hasControl("search")
    ? `<button type="button" data-mode="search" aria-label="Search" aria-pressed="false">Search</button>`
    : "";
}

function liveSidebarNav() {
  if (variant !== "live" && variant !== "menu") {
    return "";
  }
  return `
    <nav class="stacked-toolbar" aria-label="workspace navigation">
      <a href="/apps" data-testid="apps-button">Apps</a>
      ${
        hasControl("research")
          ? `<a href="${deepResearchPathname(window.location.pathname)}${window.location.search}" data-testid="deep-research-sidebar-item" id="deep-research-nav">Deep research</a>`
          : ""
      }
      <a href="/codex/cloud">Codex</a>
      <a href="/gpts" data-testid="explore-gpts-button">GPTs</a>
      <button type="button" id="projects-nav-button">Projects</button>
    </nav>
  `;
}

function liveEffortControls() {
  if (!hasControl("effort")) {
    return "";
  }
  return `
    <button
      type="button"
      id="thinking-effort-trigger"
      class="pill __composer-pill"
      aria-label="Thinking"
      aria-haspopup="menu"
      aria-controls="tablinq-effort-menu"
      aria-expanded="false"
      hidden
    >
      Thinking
    </button>
  `;
}

function menuEffortControls() {
  return liveEffortControls();
}

function currentPortalMenus() {
  if (variant !== "live" && variant !== "menu") {
    return "";
  }
  return `
    <div data-slot="portal-root">
      ${
        hasControl("model")
          ? `
      <div id="tablinq-model-menu" role="menu" hidden>
        <div class="pill" aria-hidden="true">Latest</div>
        ${
          hasControl("instant-model")
            ? `
        <div id="model-option-instant" role="menuitemradio" data-testid="model-switcher-gpt-5-3" hidden>
          <div>Instant</div>
          <div>For everyday chats</div>
        </div>
      `
            : ""
        }
        ${
          hasControl("thinking-model")
            ? `
        <div id="model-option-thinking" role="menuitemradio" data-testid="model-switcher-gpt-5-4-thinking" hidden>
          <div>Thinking</div>
          <div>For complex questions</div>
        </div>
      `
            : ""
        }
        <div id="model-option-pro" role="menuitemradio" data-testid="model-switcher-gpt-5-4-pro" hidden>
          <div>Pro</div>
          <div>Research-grade intelligence</div>
        </div>
        <div id="model-option-configure" role="menuitem" data-testid="model-configure-modal" hidden>
          Configure...
        </div>
      </div>
      `
          : ""
      }
      ${
        hasControl("effort")
          ? `
      <div id="tablinq-effort-menu" role="menu" hidden>
        <div class="pill" aria-hidden="true">Thinking effort</div>
        <div id="effort-option-standard" role="menuitemradio" hidden>Standard</div>
        <div id="effort-option-extended" role="menuitemradio" hidden>Extended</div>
      </div>
      `
          : ""
      }
    </div>
  `;
}

function liveUploadControl() {
  if (!hasControl("upload")) {
    return "";
  }
  if (usesComposerPlusTrap) {
    return `
      <input id="live-upload-input" type="file" hidden multiple />
    `;
  }
  return `
    <button type="button" data-testid="upload-button" aria-label="Add photos and files">Add files</button>
    <input id="live-upload-input" type="file" hidden multiple />
  `;
}

function liveComposerActions() {
  if (usesComposerPlusTrap) {
    return `
      <div class="pill">
        ${liveEffortControls()}
        <button
          type="button"
          id="composer-plus-btn"
          aria-label="Add photos and files"
          aria-haspopup="dialog"
        >
          +
        </button>
        <button type="button" id="composer-send-glyph">
          <span aria-hidden="true">></span>
        </button>
        <button
          type="button"
          data-testid="stop-button"
          aria-label="Stop generating"
          hidden
        >
          Stop
        </button>
      </div>
    `;
  }
  return `
    <div class="pill">
      ${liveEffortControls()}
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
  `;
}

function liveDecoySubmitControl() {
  if (!usesStaleSubmitRecovery) {
    return "";
  }
  return `
    <section class="stacked-toolbar" aria-label="decoy controls">
      <button
        type="button"
        id="stale-send-button"
        data-testid="send-button"
        aria-label="Send prompt"
        hidden
      >
        Send
      </button>
    </section>
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
        ${liveSidebarNav()}
        <section class="stacked-toolbar">
          ${liveModelControls()}
          ${liveModeControls()}
          ${liveUploadControl()}
        </section>
        ${liveDecoySubmitControl()}
        <form data-testid="composer-root" class="stacked-composer">
          <textarea
            id="prompt-textarea"
            aria-label="Message ChatGPT"
            placeholder="Message ChatGPT"
          ></textarea>
          ${liveComposerActions()}
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
            ${
              scenario === "followup-rich-text" ||
              scenario === "recover-rich-markdown"
                ? ""
                : `
            <a
              href="#"
              data-testid="artifact-download"
              aria-label="Download report"
            >
              Download report
            </a>
            `
            }
          </article>
        </section>
        <section class="notice-panel" aria-label="workspace notices">
          <article class="message" data-slot="notice-card">
            <p>Workspace notices appear here while ChatGPT is thinking.</p>
          </article>
        </section>
        ${
          usesDeepResearchFrame
            ? `
        <iframe
          id="deep-research-sandbox"
          title="internal://deep-research"
        ></iframe>
        `
            : ""
        }
        ${currentPortalMenus()}
      </main>
    `
      : variant === "menu"
        ? `
      <main aria-label="chatgpt workspace">
        ${liveSidebarNav()}
        <section class="stacked-toolbar">
          ${menuModelControls()}
          ${liveModeControls()}
          ${liveUploadControl()}
        </section>
        <form data-testid="composer-root" class="stacked-composer">
          <textarea
            id="prompt-textarea"
            aria-label="Message ChatGPT"
            placeholder="Message ChatGPT"
          ></textarea>
          ${liveComposerActions()}
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
        ${currentPortalMenus()}
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
  "[data-testid='composer-root'] [data-testid='send-button'], [data-testid='composer-root'] [aria-label='Send prompt'], [data-action='submit'], [aria-label='Submit prompt'], [aria-label='Send prompt']",
);
const staleSendButton = document.querySelector("#stale-send-button");
const composerPlusButton = document.querySelector("#composer-plus-btn");
const stopButton = document.querySelector("[data-testid='stop-button']");
const uploadEntry = document.querySelector(
  "[data-slot='upload-entry'], [data-testid='upload-button']",
);
const uploadInput = document.querySelector("input[type='file']");
const uploadList = document.querySelector("[data-slot='upload-list']");
const messageArticle = document.querySelector(
  "[data-slot='assistant-message'], [data-message-author-role='assistant']",
);
const messageMarkdown = document.querySelector(
  "[data-message-author-role='assistant'] .markdown",
);
const message = document.querySelector(
  "[data-slot='assistant-message'] p, [data-message-author-role='assistant'] .markdown p",
);
const artifactLink = document.querySelector(
  "[data-slot='artifact-link'], [data-testid='artifact-download']",
);
const deepResearchFrame = document.querySelector("#deep-research-sandbox");
let activeTimer = null;
let uploadProcessingTimer = null;

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

function deepResearchFrameDocument() {
  if (!(deepResearchFrame instanceof HTMLIFrameElement)) {
    return null;
  }
  const doc = deepResearchFrame.contentDocument;
  if (!doc) {
    return null;
  }
  if (doc.body?.dataset.tablinqFrame === "true") {
    return doc;
  }
  doc.open();
  doc.write(
    `<!doctype html><html><body data-tablinq-frame="true"></body></html>`,
  );
  doc.close();
  return deepResearchFrame.contentDocument;
}

function renderDeepResearchFrameIdle() {
  const doc = deepResearchFrameDocument();
  if (!doc?.body) {
    return;
  }
  doc.body.innerHTML = "";
}

function renderDeepResearchPlanner(state) {
  const doc = deepResearchFrameDocument();
  if (!doc?.body) {
    return;
  }
  doc.body.innerHTML = `
    <main data-deep-research-root="true">
      <h1>Deep research</h1>
      <p>Review the proposed research plan before continuing.</p>
      <section data-plan>${escapeHtml(state.promptText)}</section>
      <div>
        <button type="button" data-action="edit">Edit</button>
        <button type="button" data-action="cancel">Cancel</button>
        <button type="button" data-action="start">Start</button>
      </div>
    </main>
  `;
  doc.querySelector("[data-action='cancel']")?.addEventListener("click", () => {
    setStreamState("idle");
    setResearchState("failed", "research cancelled");
    setStreamingControls(false);
    clearResearchState(state.conversationId);
    renderDeepResearchFrameIdle();
  });
  doc.querySelector("[data-action='start']")?.addEventListener("click", () => {
    launchDeepResearch(state);
  });
}

function renderDeepResearchFrameReport(state) {
  const doc = deepResearchFrameDocument();
  if (!doc?.body) {
    return;
  }
  const text = state.reportText.slice(0, state.index);
  const statusText =
    state.phase === "waiting" ? "gathering sources" : "drafting report";
  doc.body.innerHTML = `
    <main data-deep-research-root="true">
      <h1>Deep research</h1>
      <p data-deep-research-status="true">${escapeHtml(statusText)}</p>
      <pre data-deep-research-text="true">${escapeHtml(text)}</pre>
    </main>
  `;
}

function renderDeepResearchStartingShell(state) {
  const doc = deepResearchFrameDocument();
  if (!doc?.body) {
    return;
  }
  doc.body.innerHTML = `
    <main data-deep-research-root="true">
      <h1>Deep research</h1>
      <p>Providing a comprehensive report for the user...</p>
      <section>
        <p>Browser vs Hosted AI for Enterprise</p>
        <p>Update</p>
        <ul>
          <li>Collect vendor documentation for browser-backed AI and hosted API providers.</li>
          <li>Survey enterprise security and compliance guidance on authentication reuse and data residency.</li>
          <li>Synthesize comparisons into a concise matrix highlighting tradeoffs and recommendations.</li>
        </ul>
      </section>
      <div>
        <button type="button">Stop research</button>
      </div>
    </main>
  `;
}

function launchDeepResearch(state) {
  setStreamState("running");
  setResearchState("waiting_for_sources", "gathering sources");
  setStreamingControls(true);
  state.phase = "waiting";
  state.waitTicks = 0;
  state.index = 0;
  if (usesDeepResearchStartingShell) {
    renderDeepResearchStartingShell(state);
  } else {
    renderDeepResearchFrameReport(state);
  }
  persistResearchState(state);
  activeTimer = setTimeout(() => tickResearch(state), 80);
}

function clearComposer() {
  if (!composer) {
    return;
  }
  if ("value" in composer) {
    composer.value = "";
    composer.dispatchEvent(new Event("input", { bubbles: true }));
    composer.dispatchEvent(new Event("change", { bubbles: true }));
    return;
  }
  composer.textContent = "";
  composer.dispatchEvent(new InputEvent("input", { bubbles: true }));
}

function enterDeepResearchLanding() {
  const url = new URL(window.location.href);
  url.pathname = deepResearchPathname(url.pathname);
  history.replaceState({}, "", url);
  document.body.dataset.conversationId = "";
  document.body.dataset.lastMessageId = "";
  document.body.dataset.researchSubmitted = "false";
  document.body.dataset.researchLandingVisited = "true";
  setStreamState("idle");
  setResearchState("idle", "");
  setStreamingControls(false);
  clearComposer();
  if (messageArticle instanceof HTMLElement) {
    messageArticle.hidden = true;
    messageArticle.removeAttribute("data-message-id");
    messageArticle.removeAttribute("data-message-author-role");
    messageArticle.setAttribute("data-stream-state", "idle");
  }
  if (message) {
    message.textContent = "";
  }
  renderDeepResearchFrameIdle();
}

function onDeepResearchLanding() {
  return pathnameSegments(window.location.pathname).includes("deep-research");
}

function setUploadPopoverOpen(open) {
  document.body.dataset.uploadPopoverOpen = open ? "true" : "false";
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
  const chips = fileNames
    .map((fileName) =>
      variant === "live"
        ? `<span class="pill" data-testid="upload-chip" aria-label="Attached file ${fileName}">${fileName}</span>`
        : `<span class="pill" data-slot="uploaded-file">${fileName}</span>`,
    )
    .join("");
  const processingIndicator =
    document.body.dataset.uploadProcessing === "true"
      ? `<span class="pill" data-slot="upload-processing" data-upload-processing="true" aria-busy="true">Processing upload</span>`
      : "";
  uploadList.innerHTML = `${chips}${processingIndicator}`;
}

function syncSubmissionControls() {
  const running = document.body.dataset.streamState === "running";
  const uploadProcessing = document.body.dataset.uploadProcessing === "true";
  const submitBlocked =
    hasBlockedSubmitAfterUpload && uploadedFiles().length > 0 && !running;
  if (sendButton instanceof HTMLElement) {
    sendButton.hidden = running;
    sendButton.setAttribute(
      "aria-disabled",
      uploadProcessing || submitBlocked ? "true" : "false",
    );
    sendButton.setAttribute("aria-busy", uploadProcessing ? "true" : "false");
    sendButton.dataset.disabled =
      uploadProcessing || submitBlocked ? "true" : "false";
    sendButton.dataset.busy = uploadProcessing ? "true" : "false";
    if ("disabled" in sendButton) {
      sendButton.disabled = uploadProcessing || submitBlocked;
    }
  }
  if (composerPlusButton instanceof HTMLElement) {
    composerPlusButton.disabled = running;
  }
  if (staleSendButton instanceof HTMLElement) {
    staleSendButton.hidden = uploadedFiles().length === 0 || running;
  }
  if (stopButton instanceof HTMLElement) {
    stopButton.hidden = !running;
  }
  document.body.dataset.keyboardSubmitReady =
    !running && !uploadProcessing && keyboardSubmitEnabled ? "true" : "false";
}

function setStreamingControls(running) {
  syncSubmissionControls();
}

function stopUploadProcessingTimer() {
  if (uploadProcessingTimer) {
    clearTimeout(uploadProcessingTimer);
    uploadProcessingTimer = null;
  }
}

function setUploadProcessing(processing) {
  document.body.dataset.uploadProcessing = processing ? "true" : "false";
  renderUploadedFiles(uploadedFiles());
  syncSubmissionControls();
}

function resetArtifactLink() {
  if (!artifactLink) {
    return;
  }
  if (variant === "live") {
    artifactLink.removeAttribute("href");
    artifactLink.removeAttribute("download");
  } else {
    artifactLink.setAttribute("href", "/artifact.txt");
    artifactLink.setAttribute("download", "artifact.txt");
  }
  artifactLink.setAttribute("data-artifact-ready", "false");
}

function clearAskFailureState() {
  delete document.body.dataset.askFailureCode;
  delete document.body.dataset.askFailureMessage;
}

function providerGenerationFailureMessage() {
  return "Something went wrong while generating the response. If this issue persists please contact us through our help center.";
}

function providerFailureStorageKey(name) {
  return `tablinq-provider-failure:${scenario}:${name}`;
}

function nextProviderFailureAttempt() {
  const key = providerFailureStorageKey("attempt");
  const next = Number.parseInt(localStorage.getItem(key) || "0", 10) + 1;
  localStorage.setItem(key, String(next));
  return next;
}

function renderProviderGenerationFailure(promptText) {
  const conversationId = currentConversationId();
  const messageId = uniqueId("msg");
  const failureMessage = providerGenerationFailureMessage();
  const retryMarkup =
    '<button type="button" data-testid="retry-button">Retry</button>';

  writeConversationState(conversationId, messageId);
  localStorage.setItem(
    providerFailureStorageKey("priorConversationId"),
    conversationId,
  );
  localStorage.setItem(providerFailureStorageKey("priorPrompt"), promptText);
  clearState(conversationId);
  clearAskFailureState();
  document.body.dataset.askFailureCode =
    "provider_generation_failed_nonretryable";
  document.body.dataset.askFailureMessage = failureMessage;
  if (messageArticle instanceof HTMLElement) {
    messageArticle.hidden = false;
    messageArticle.setAttribute("data-message-author-role", "assistant");
    messageArticle.setAttribute("data-message-id", messageId);
  }
  if (messageMarkdown instanceof HTMLElement) {
    messageMarkdown.innerHTML = `<p>${escapeHtml(failureMessage)}</p>${retryMarkup}`;
  } else if (message) {
    message.innerHTML = `<p>${escapeHtml(failureMessage)}</p>${retryMarkup}`;
  }
  setStreamState("failed");
  setStreamingControls(false);
}

function retryButtonClicked() {
  document.body.dataset.retryClicks = String(
    Number.parseInt(document.body.dataset.retryClicks || "0", 10) + 1,
  );
}

function submitPrompt(method) {
  if (!composer || !message) {
    return;
  }
  if (document.body.dataset.uploadProcessing === "true") {
    return;
  }

  stopTimer();
  clearAskFailureState();
  setUploadPopoverOpen(false);
  document.body.dataset.lastSubmissionMethod = method;
  const promptText =
    "value" in composer ? composer.value.trim() : composer.textContent.trim();
  if (
    document.body.dataset.selectedMode === "research" &&
    onDeepResearchLanding()
  ) {
    if (researchSubmissionFails) {
      return;
    }
    startResearch(promptText);
    return;
  }
  startStream(promptText);
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
    const key = element.getAttribute("data-mode");
    const selected = key === selectedValue;
    element.setAttribute("aria-pressed", selected ? "true" : "false");
    if (element.getAttribute("role") === "menuitemradio") {
      element.setAttribute("aria-checked", selected ? "true" : "false");
    }
    element.dataset.selected = selected ? "true" : "false";
  });
}

function syncModernMenuOption(selector, selected) {
  const element = document.querySelector(selector);
  if (!(element instanceof HTMLElement)) {
    return;
  }
  element.setAttribute("aria-checked", selected ? "true" : "false");
  element.dataset.selected = selected ? "true" : "false";
}

function syncSelectedState() {
  const selectedMode = document.body.dataset.selectedMode;
  document.querySelectorAll("[data-mode]").forEach((element) => {
    const selected = element.getAttribute("data-mode") === selectedMode;
    element.setAttribute("aria-pressed", selected ? "true" : "false");
    element.dataset.selected = selected ? "true" : "false";
  });
  document.querySelectorAll("[data-model]").forEach((element) => {
    const selected =
      element.getAttribute("data-model") ===
      document.body.dataset.selectedModel;
    element.setAttribute("aria-pressed", selected ? "true" : "false");
    element.dataset.selected = selected ? "true" : "false";
  });
  document.querySelectorAll("[data-effort]").forEach((element) => {
    const selected =
      element.getAttribute("data-effort") ===
      document.body.dataset.selectedEffort;
    element.setAttribute("aria-pressed", selected ? "true" : "false");
    element.dataset.selected = selected ? "true" : "false";
  });
  const deepResearchNav = document.querySelector("#deep-research-nav");
  if (deepResearchNav instanceof HTMLElement) {
    const selected = selectedMode === "research" || onDeepResearchLanding();
    deepResearchNav.dataset.selected = selected ? "true" : "false";
    if (selected) {
      deepResearchNav.setAttribute("aria-current", "page");
    } else {
      deepResearchNav.removeAttribute("aria-current");
    }
  }

  syncModernMenuOption(
    "#model-option-instant",
    document.body.dataset.selectedModel === "instant",
  );
  syncModernMenuOption(
    "#model-option-thinking",
    document.body.dataset.selectedModel === "thinking",
  );
  syncModernMenuOption("#model-option-pro", false);
  syncModernMenuOption(
    "#effort-option-standard",
    document.body.dataset.selectedEffort === "standard",
  );
  syncModernMenuOption(
    "#effort-option-extended",
    document.body.dataset.selectedEffort === "extended",
  );

  const modelMenu = document.querySelector("#tablinq-model-menu");
  const modelTrigger = document.querySelector("#model-switcher-trigger");
  const modelMenuOpen = modelTrigger?.getAttribute("aria-expanded") === "true";
  modelMenu?.toggleAttribute("hidden", !modelMenuOpen);
  modelMenu
    ?.querySelectorAll("[role='menuitemradio'], [role='menuitem']")
    .forEach((element) => {
      element.toggleAttribute("hidden", !modelMenuOpen);
    });

  const currentUiVariant = variant === "live" || variant === "menu";
  const effortVisible =
    hasControl("effort") &&
    (!currentUiVariant || document.body.dataset.selectedModel === "thinking");
  const effortToggle = document.querySelector("#thinking-effort-trigger");
  const effortToggleIsMenuTrigger =
    effortToggle instanceof HTMLElement &&
    effortToggle.getAttribute("aria-haspopup") !== null;
  if (effortToggleIsMenuTrigger) {
    effortToggle.toggleAttribute("hidden", !effortVisible);
    effortToggle.textContent =
      document.body.dataset.selectedEffort === "extended"
        ? "Extended thinking"
        : "Thinking";
    if (!effortVisible) {
      effortToggle.setAttribute("aria-expanded", "false");
    }
  }
  const effortMenu = document.querySelector("#tablinq-effort-menu");
  const effortMenuOpen =
    effortVisible &&
    effortToggleIsMenuTrigger &&
    effortToggle.getAttribute("aria-expanded") === "true";
  effortMenu?.toggleAttribute("hidden", !effortMenuOpen);
  effortMenu?.querySelectorAll("[role='menuitemradio']").forEach((element) => {
    element.toggleAttribute("hidden", !effortMenuOpen);
  });
}

function renderChunk(state) {
  if (!message) {
    return;
  }

  if (
    scenario === "thinking-artifact-messages" &&
    variant === "live" &&
    Array.isArray(state.reasoningArtifacts)
  ) {
    renderThinkingArtifactSequence(state);
    return;
  }

  const text = state.responseText.slice(0, state.index);
  if (
    scenario === "followup-rich-text" &&
    variant === "live" &&
    state.askNumber > 1 &&
    Array.isArray(state.responseBlocks) &&
    messageMarkdown instanceof HTMLElement
  ) {
    let remaining = text.length;
    const blocks = [];
    for (let index = 0; index < state.responseBlocks.length; index += 1) {
      const block = state.responseBlocks[index];
      if (!block || remaining <= 0) {
        break;
      }
      const visibleLength = Math.min(block.length, remaining);
      blocks.push(`<p>${block.slice(0, visibleLength)}</p>`);
      remaining -= visibleLength;
      if (index < state.responseBlocks.length - 1 && remaining > 0) {
        remaining = Math.max(0, remaining - 2);
      }
    }
    messageMarkdown.innerHTML = blocks.join("");
  } else {
    message.textContent = text;
  }
  messageArticle?.setAttribute("data-message-id", state.messageId);
}

function transcriptContainer() {
  return messageArticle?.parentElement ?? document.querySelector(".transcript");
}

function clearThinkingArtifactNodes() {
  transcriptContainer()
    ?.querySelectorAll(
      "[data-tablinq-thinking-artifact], [data-tablinq-thought-divider]",
    )
    .forEach((node) => node.remove());
}

function createThinkingArtifactMessage(text, messageId) {
  const article = document.createElement("div");
  article.dataset.tablinqThinkingArtifact = "true";
  article.setAttribute("data-message-author-role", "assistant");
  article.setAttribute("data-message-id", messageId);
  article.className = "message";
  const markdown = document.createElement("div");
  markdown.className = "markdown";
  const paragraph = document.createElement("p");
  paragraph.textContent = text;
  markdown.appendChild(paragraph);
  article.appendChild(markdown);
  return article;
}

function createThoughtDivider(text) {
  const divider = document.createElement("div");
  divider.dataset.tablinqThoughtDivider = "true";
  const button = document.createElement("button");
  button.type = "button";
  button.textContent = text;
  divider.appendChild(button);
  return divider;
}

function renderThinkingArtifactSequence(state) {
  const transcript = transcriptContainer();
  if (
    !(transcript instanceof HTMLElement) ||
    !(messageArticle instanceof HTMLElement)
  ) {
    return;
  }

  clearThinkingArtifactNodes();
  messageArticle.hidden = state.phase !== "answer";
  if (state.phase === "answer") {
    messageArticle.setAttribute("data-message-author-role", "assistant");
    messageArticle.setAttribute("data-message-id", state.messageId);
    messageArticle.setAttribute("data-turn-start-message", "true");
    message.textContent = state.responseText.slice(0, state.index);
  } else {
    messageArticle.removeAttribute("data-message-author-role");
    messageArticle.removeAttribute("data-message-id");
    messageArticle.removeAttribute("data-turn-start-message");
    message.textContent = "";
  }

  const fragment = document.createDocumentFragment();
  const artifactCount = state.phase === "prelude-1" ? 1 : 2;
  for (let index = 0; index < artifactCount; index += 1) {
    fragment.appendChild(
      createThinkingArtifactMessage(
        state.reasoningArtifacts[index],
        state.reasoningArtifactMessageIds[index],
      ),
    );
  }
  if (state.phase !== "prelude-1") {
    fragment.appendChild(createThoughtDivider("Thought for 59s"));
  }
  transcript.insertBefore(fragment, messageArticle);

  document.body.dataset.lastMessageId =
    state.phase === "answer"
      ? state.messageId
      : state.reasoningArtifactMessageIds[artifactCount - 1];
}

function renderRichRecoveryChunk(state) {
  if (!messageArticle || !(messageMarkdown instanceof HTMLElement)) {
    renderChunk(state);
    return;
  }
  const promptBody =
    state.richRecoveryPrompt || "Draft a reusable release-notes prompt.";
  const outlineItems = Array.isArray(state.richRecoveryOutline)
    ? state.richRecoveryOutline
    : ["Goal", "Inputs", "Output"];
  messageMarkdown.innerHTML = `
    <pre><code>${escapeHtml(promptBody)}</code></pre>
    <p><strong>Outline</strong></p>
    <ol>
      ${outlineItems.map((item) => `<li>${escapeHtml(item)}</li>`).join("")}
    </ol>
  `;
  messageArticle.setAttribute("data-message-id", state.messageId);
}

function renderPlaceholderDrift(state) {
  if (!message) {
    return;
  }
  const visibleLength = Math.max(1, state.index);
  const stableLength = Math.max(0, visibleLength - 1);
  message.textContent = `${state.responseText.slice(0, stableLength)}_`;
  messageArticle?.setAttribute("data-message-id", state.messageId);
}

function renderThinkingRewrite(state) {
  if (!message) {
    return;
  }
  message.innerHTML =
    '<button type="button" disabled data-message-content="true">Thought for 7s</button>';
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
  if (usesDeepResearchFrame) {
    renderDeepResearchFrameReport(state);
    return;
  }
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

  if (scenario === "research-reload" && !state.didReload && state.index >= 24) {
    state.didReload = true;
    persistResearchState(state);
    window.location.reload();
    return;
  }

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
    if (!usesDeepResearchFrame) {
      return;
    }
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
    phase: usesDeepResearchFrame ? "planning" : "waiting",
    waitTicks: 0,
  };

  document.body.dataset.researchSubmitted = "true";
  writeConversationState(conversationId, messageId);
  if (messageArticle instanceof HTMLElement) {
    messageArticle.hidden = false;
    messageArticle.setAttribute("data-message-author-role", "assistant");
    messageArticle.setAttribute("data-message-id", messageId);
  }
  if (message) {
    message.textContent = "";
  }
  setStreamState("idle");
  setResearchState("idle", "");
  setStreamingControls(false);
  renderDeepResearchPlanner(state);
  persistResearchState(state);
  if (!usesDeepResearchFrame) {
    setStreamState("running");
    setResearchState("waiting_for_sources", "gathering sources");
    setStreamingControls(true);
    activeTimer = setTimeout(() => tickResearch(state), 80);
  }
}

function tickStream(state) {
  stopTimer();
  if (
    scenario === "thinking-artifact-messages" &&
    variant === "live" &&
    Array.isArray(state.reasoningArtifacts)
  ) {
    if (state.phase === "prelude-1") {
      state.phase = "prelude-2";
      renderChunk(state);
      persistState(state);
      activeTimer = setTimeout(() => tickStream(state), 220);
      return;
    }
    if (state.phase === "prelude-2") {
      state.phase = "answer";
      state.index = Math.min(state.responseText.length, state.index + 6);
      renderChunk(state);
      persistState(state);
      scheduleNextTick(state);
      return;
    }
  }
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

  if (
    scenario === "done-early" &&
    !state.didFinishEarly &&
    state.index >= state.responseText.length - 1
  ) {
    state.didFinishEarly = true;
    state.index = Math.max(0, state.responseText.length - 1);
    renderChunk(state);
    setStreamState("done");
    persistState(state);
    activeTimer = setTimeout(() => finishStream(state), 180);
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
  const askNumber = nextAskNumber(conversationId);

  if (
    usesProviderGenerationFailureRecovery ||
    usesProviderGenerationFailureAfterSuccess ||
    usesProviderGenerationFailureTwice
  ) {
    const shouldCheckFailure =
      !usesProviderGenerationFailureAfterSuccess || askNumber > 1;
    if (shouldCheckFailure) {
      const attempt = nextProviderFailureAttempt();
      const isRecoveryReplay =
        promptText.includes("Recovery note:") &&
        (promptText.includes("Original user request:") ||
          promptText.includes(
            "Answer this request now, using the attachment for prior context when relevant:",
          ));
      if (
        usesProviderGenerationFailureTwice ||
        attempt === 1 ||
        !isRecoveryReplay
      ) {
        renderProviderGenerationFailure(promptText);
        return;
      }
    }
  }

  const messageId = uniqueId("msg");
  const responseBlocks =
    scenario === "followup-rich-text" && askNumber > 1
      ? [
          "I can help with thinking work and doing work.",
          "I can explain concepts, write and edit, summarize, brainstorm, tutor, code, debug, analyze data, compare options, plan projects, draft emails/messages, and turn rough notes into polished documents.",
        ]
      : null;
  const richRecoveryPrompt =
    scenario === "recover-rich-markdown"
      ? "Summarize release notes into a concise update with risks and actions."
      : null;
  const richRecoveryOutline =
    scenario === "recover-rich-markdown"
      ? ["Goal", "Inputs", "Constraints", "Output"]
      : null;
  const reasoningArtifacts =
    scenario === "thinking-artifact-messages" && variant === "live"
      ? [
          "I’m reconstructing the prior context from the recovery note and turning it into a compact comparison that makes the tradeoff explicit rather than just listing features.",
          "The comparison is narrowing to one core distinction: Rust optimizes for correctness under low-level constraints, while TypeScript optimizes for speed and safety of application development. I’m turning that into a decision-oriented visual instead of a feature list.",
        ]
      : null;
  const state = {
    conversationId,
    messageId,
    promptText,
    responseText:
      scenario === "provider-generation-failure-after-success" &&
      promptText.includes("Recovery note:")
        ? `Recovered response for: ${promptText}\nUploads: ${uploadedFiles().length ? uploadedFiles().join(", ") : "none"}`
        : responseBlocks
          ? responseBlocks.join("\n\n")
          : richRecoveryPrompt
            ? `${richRecoveryPrompt}\n\nOutline\n1. Goal\n2. Inputs\n3. Constraints\n4. Output`
            : promptText
              ? `Fake response for: ${promptText}`
              : "Fake response for an empty prompt.",
    responseBlocks,
    richRecoveryPrompt,
    richRecoveryOutline,
    reasoningArtifacts,
    reasoningArtifactMessageIds: reasoningArtifacts
      ? reasoningArtifacts.map(() => uniqueId("msg"))
      : [],
    index: 0,
    askNumber,
    didReload: false,
    pendingStart: false,
    phase: reasoningArtifacts ? "prelude-1" : "answer",
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

  writeConversationState(
    conversationId,
    reasoningArtifacts ? state.reasoningArtifactMessageIds[0] : messageId,
  );
  clearAskFailureState();
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
  if (scenario === "recover-message-successor") {
    state.messageId = uniqueId("msg");
    state.index = state.responseText.length;
    writeConversationState(state.conversationId, state.messageId);
    renderChunk(state);
    setStreamState("done");
    setStreamingControls(false);
    persistState(state);
    return;
  }
  if (scenario === "recover-done-transient") {
    state.index = state.responseText.length;
    renderPlaceholderDrift(state);
    setStreamState("done");
    setStreamingControls(false);
    persistState(state);
    activeTimer = setTimeout(() => finishStream(state), 180);
    return;
  }
  if (scenario === "recover-rich-markdown") {
    state.index = state.responseText.length;
    renderRichRecoveryChunk(state);
    setStreamState("done");
    setStreamingControls(false);
    persistState(state);
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
  if (scenario === "recover-wrong-conversation") {
    const originalConversationId = state.conversationId;
    state.conversationId = uniqueId("conv");
    state.messageId = uniqueId("msg");
    state.responseText = "Unexpected response after interruption";
    state.index = state.responseText.length;
    clearState(originalConversationId);
    writeConversationState(state.conversationId, state.messageId);
    renderChunk(state);
    setStreamState("done");
    setStreamingControls(false);
    persistState(state);
    return;
  }
  if (scenario === "recover-running-transient") {
    setStreamState("running");
    setStreamingControls(true);
    renderPlaceholderDrift(state);
    persistState(state);
    activeTimer = setTimeout(() => {
      renderChunk(state);
      persistState(state);
      scheduleNextTick(state);
    }, 180);
    return;
  }
  if (scenario === "recover-same-message-rewrite") {
    setStreamState("running");
    setStreamingControls(true);
    renderThinkingRewrite(state);
    persistState(state);
    activeTimer = setTimeout(() => {
      renderChunk(state);
      persistState(state);
      scheduleNextTick(state);
    }, 180);
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
  if (usesDeepResearchFrame && state.phase === "planning") {
    setStreamState("idle");
    setResearchState("idle", "");
    setStreamingControls(false);
    renderDeepResearchPlanner(state);
    return;
  }
  setStreamState("running");
  setResearchState(
    state.phase === "waiting" ? "waiting_for_sources" : "running",
    state.phase === "waiting" ? "gathering sources" : "drafting report",
  );
  renderResearchChunk(state);
  setStreamingControls(true);
  activeTimer = setTimeout(() => tickResearch(state), 80);
}

const modelPickerTrigger = document.querySelector("#model-switcher-trigger");
const effortPickerTrigger = document.querySelector("#thinking-effort-trigger");

modelPickerTrigger?.addEventListener("click", () => {
  const expanded = modelPickerTrigger.getAttribute("aria-expanded") === "true";
  modelPickerTrigger.setAttribute("aria-expanded", expanded ? "false" : "true");
  syncSelectedState();
});

document
  .querySelector("#model-option-instant")
  ?.addEventListener("click", () => {
    document.body.dataset.selectedModel = "instant";
    modelPickerTrigger?.setAttribute("aria-expanded", "false");
    effortPickerTrigger?.setAttribute("aria-expanded", "false");
    syncSelectedState();
  });

document
  .querySelector("#model-option-thinking")
  ?.addEventListener("click", () => {
    document.body.dataset.selectedModel = "thinking";
    modelPickerTrigger?.setAttribute("aria-expanded", "false");
    syncSelectedState();
  });

effortPickerTrigger?.addEventListener("click", () => {
  const expanded = effortPickerTrigger.getAttribute("aria-expanded") === "true";
  effortPickerTrigger.setAttribute(
    "aria-expanded",
    expanded ? "false" : "true",
  );
  syncSelectedState();
});

document
  .querySelector("#effort-option-standard")
  ?.addEventListener("click", () => {
    document.body.dataset.selectedEffort = "standard";
    effortPickerTrigger?.setAttribute("aria-expanded", "false");
    syncSelectedState();
  });

document
  .querySelector("#effort-option-extended")
  ?.addEventListener("click", () => {
    document.body.dataset.selectedEffort = "extended";
    effortPickerTrigger?.setAttribute("aria-expanded", "false");
    syncSelectedState();
  });

document.querySelectorAll("[data-model]").forEach((element) => {
  element.addEventListener("click", () => {
    const selectedModel = element.getAttribute("data-model");
    if (!selectedModel) {
      return;
    }
    document.body.dataset.selectedModel = selectedModel;
    syncSelectedState();
  });
});

document.querySelectorAll("[data-effort]").forEach((element) => {
  element.addEventListener("click", () => {
    const selectedEffort = element.getAttribute("data-effort");
    if (!selectedEffort) {
      return;
    }
    document.body.dataset.selectedEffort = selectedEffort;
    syncSelectedState();
  });
});

document.querySelectorAll("[data-mode]").forEach((element) => {
  element.addEventListener("click", () => {
    document.body.dataset.selectedMode = element.getAttribute("data-mode");
    if (
      usesDeepResearchLanding &&
      document.body.dataset.selectedMode === "research"
    ) {
      enterDeepResearchLanding();
    }
    syncSelectedState();
  });
});

document
  .querySelector("#deep-research-nav")
  ?.addEventListener("click", (event) => {
    event.preventDefault();
    document.body.dataset.selectedMode = "research";
    enterDeepResearchLanding();
    syncSelectedState();
  });

uploadEntry?.addEventListener("click", () => {
  if (uploadInput instanceof HTMLElement) {
    uploadInput.click();
  }
});

composerPlusButton?.addEventListener("click", () => {
  setUploadPopoverOpen(true);
  if (uploadInput instanceof HTMLElement) {
    uploadInput.click();
  }
});

staleSendButton?.addEventListener("click", () => {
  document.body.dataset.staleSubmitClicks = String(
    Number.parseInt(document.body.dataset.staleSubmitClicks || "0", 10) + 1,
  );
});

uploadInput?.addEventListener("change", () => {
  const fileNames = Array.from(uploadInput.files || []).map(
    (file) => file.name,
  );
  setUploadedFiles(fileNames);
  stopUploadProcessingTimer();
  if (fileNames.length === 0) {
    setUploadProcessing(false);
    return;
  }
  if (hasDelayedUploadProcessing || hasStuckUploadProcessing) {
    setUploadProcessing(true);
  } else {
    setUploadProcessing(false);
  }
  if (hasDelayedUploadProcessing) {
    uploadProcessingTimer = setTimeout(() => {
      setUploadProcessing(false);
      uploadProcessingTimer = null;
    }, 2200);
  }
});

window.__tablinqCancelStream = () => {
  stopTimer();
  const conversationId = currentConversationId();
  clearState(conversationId);
  setStreamState("cancelled");
  setStreamingControls(false);
};

sendButton?.addEventListener("click", () => {
  submitPrompt("button");
});

composer?.addEventListener("keydown", (event) => {
  if (
    !keyboardSubmitEnabled ||
    event.key !== "Enter" ||
    (!event.ctrlKey && !event.metaKey)
  ) {
    return;
  }
  event.preventDefault();
  submitPrompt("keyboard");
});

stopButton?.addEventListener("click", () => {
  window.__tablinqCancelStream();
});

document.addEventListener("click", (event) => {
  const target = event.target;
  if (!(target instanceof Element)) {
    return;
  }
  if (target.closest("[data-testid='retry-button']")) {
    retryButtonClicked();
  }
});

if (onDeepResearchLanding()) {
  document.body.dataset.selectedMode = "research";
  enterDeepResearchLanding();
}

syncSelectedState();
renderUploadedFiles(uploadedFiles());
resetArtifactLink();
syncSubmissionControls();
renderDeepResearchFrameIdle();
resumeResearchIfNeeded();
resumeStreamIfNeeded();
