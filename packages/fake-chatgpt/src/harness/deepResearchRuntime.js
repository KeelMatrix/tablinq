import {
  deepResearchPathname,
  escapeHtml,
  pathnameSegments,
} from "./scenarioState.js";
import { createResearchState } from "./research.js";
import { uniqueId } from "./conversationPersistence.js";

export function createDeepResearchRuntime({
  document,
  window,
  config,
  refs,
  timers,
  conversation,
  uploads,
  clearComposer,
  setStreamState,
  setResearchState,
}) {
  const { scenario, usesDeepResearchStartingShell, usesDeepResearchFrame } =
    config;
  const { messageArticle, message, deepResearchFrame } = refs;
  const { uploadedFiles, setStreamingControls, prepareArtifact } = uploads;
  const {
    currentConversationId,
    writeConversationState,
    readResearchState,
    persistResearchState,
    clearResearchState,
  } = conversation;

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
    doc
      .querySelector("[data-action='cancel']")
      ?.addEventListener("click", () => {
        setStreamState("idle");
        setResearchState("failed", "research cancelled");
        setStreamingControls(false);
        clearResearchState(state.conversationId);
        renderDeepResearchFrameIdle();
      });
    doc
      .querySelector("[data-action='start']")
      ?.addEventListener("click", () => {
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
    timers.set(() => tickResearch(state), 80);
  }

  function enterDeepResearchLanding() {
    const url = new URL(window.location.href);
    url.pathname = deepResearchPathname(url.pathname);
    window.history.replaceState({}, "", url);
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
    timers.stop();
    state.index = state.reportText.length;
    renderResearchChunk(state);
    setResearchState("done", "research completed");
    setStreamState("done");
    setStreamingControls(false);
    prepareArtifact(state.promptText);
    clearResearchState(state.conversationId);
  }

  function tickResearch(state) {
    timers.stop();
    if (state.phase === "waiting") {
      state.waitTicks += 1;
      setResearchState("waiting_for_sources", "gathering sources");
      persistResearchState(state);
      if (state.waitTicks >= 10) {
        state.phase = "running";
      }
      timers.set(() => tickResearch(state), 80);
      return;
    }

    setResearchState("running", "drafting report");
    state.index = Math.min(state.reportText.length, state.index + 12);
    renderResearchChunk(state);

    if (
      scenario === "research-reload" &&
      !state.didReload &&
      state.index >= 24
    ) {
      state.didReload = true;
      persistResearchState(state);
      window.location.reload();
      return;
    }

    if (scenario === "research-timeout-reload-success" && state.index >= 24) {
      persistResearchState(state);
      timers.stop();
      return;
    }

    if (scenario === "research-failure" && state.index >= 24) {
      setResearchState("failed", "temporary upstream issue; retryable");
      setStreamingControls(false);
      persistResearchState(state);
      timers.stop();
      return;
    }

    if (state.index >= state.reportText.length) {
      finishResearch(state);
      return;
    }

    persistResearchState(state);
    timers.set(() => tickResearch(state), 80);
  }

  function startResearch(promptText) {
    if (!message || !messageArticle) {
      if (!usesDeepResearchFrame) {
        return;
      }
    }

    const conversationId = currentConversationId();
    const messageId = uniqueId("msg");
    const state = createResearchState({
      conversationId,
      messageId,
      promptText,
      uploadedFiles,
      usesDeepResearchFrame,
    });

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
      timers.set(() => tickResearch(state), 80);
    }
  }

  function resumeResearchIfNeeded() {
    const conversationId = currentConversationId();
    const state = readResearchState(conversationId);
    if (!state) {
      return;
    }

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
    if (scenario === "research-timeout-reload-success") {
      finishResearch(state);
      return;
    }
    setStreamingControls(true);
    timers.set(() => tickResearch(state), 80);
  }

  return {
    enterDeepResearchLanding,
    onDeepResearchLanding,
    renderDeepResearchFrameIdle,
    resumeResearchIfNeeded,
    startResearch,
  };
}
