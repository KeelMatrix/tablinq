import { createAskStreamState } from "./streaming.js";
import { uniqueId } from "./conversationPersistence.js";

export function createAskRuntime({
  document,
  window,
  config,
  refs,
  timers,
  conversation,
  uploads,
  providerRecovery,
  setStreamState,
  escapeHtml,
}) {
  const { scenario, variant } = config;
  const {
    usesProviderGenerationFailureRecovery,
    usesProviderGenerationFailureAfterSuccess,
    usesProviderGenerationFailureTwice,
    usesProviderStoppedThinkingOnce,
    usesProviderStoppedThinkingTwice,
  } = config;
  const { messageArticle, messageMarkdown, message } = refs;
  const { uploadedFiles, setStreamingControls, prepareArtifact } = uploads;
  const {
    currentConversationId,
    nextAskNumber,
    writeConversationState,
    readStreamState,
    persistStreamState,
    clearStreamState,
  } = conversation;
  const {
    clearAskFailureState,
    nextProviderFailureAttempt,
    renderProviderGenerationFailure,
    renderProviderStoppedThinking,
  } = providerRecovery;

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
    return (
      messageArticle?.parentElement ?? document.querySelector(".transcript")
    );
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

  function scheduleNextTick(state) {
    timers.set(() => {
      tickStream(state);
    }, 60);
  }

  function finishStream(state) {
    timers.stop();
    state.index = state.responseText.length;
    renderChunk(state);
    if (
      scenario === "sticky-stop-after-complete" ||
      scenario === "sticky-stop-after-reload"
    ) {
      setStreamState("running");
      setStreamingControls(true);
      persistStreamState(state);
      return;
    }
    setStreamState("done");
    setStreamingControls(false);
    prepareArtifact(state.promptText);
    clearStreamState(state.conversationId);
  }

  function tickStream(state) {
    timers.stop();
    if (
      scenario === "thinking-artifact-messages" &&
      variant === "live" &&
      Array.isArray(state.reasoningArtifacts)
    ) {
      if (state.phase === "prelude-1") {
        state.phase = "prelude-2";
        renderChunk(state);
        persistStreamState(state);
        timers.set(() => tickStream(state), 220);
        return;
      }
      if (state.phase === "prelude-2") {
        state.phase = "answer";
        state.index = Math.min(state.responseText.length, state.index + 6);
        renderChunk(state);
        persistStreamState(state);
        scheduleNextTick(state);
        return;
      }
    }
    state.index = Math.min(state.responseText.length, state.index + 6);
    renderChunk(state);

    if (scenario === "reload" && !state.didReload && state.index >= 12) {
      state.didReload = true;
      persistStreamState(state);
      window.location.reload();
      return;
    }

    if (
      (scenario === "timeout" || scenario === "timeout-reload-success") &&
      state.index >= 18
    ) {
      setStreamState("running");
      persistStreamState(state);
      return;
    }

    if (scenario === "stream-stall-reload-advance" && state.index >= 18) {
      setStreamState("running");
      persistStreamState(state);
      return;
    }

    if (scenario === "stall-recover" && !state.didStall && state.index >= 18) {
      state.didStall = true;
      persistStreamState(state);
      timers.set(() => tickStream(state), 2400);
      return;
    }

    if (
      scenario === "stall-recover-transient" &&
      !state.didStall &&
      state.index >= 18
    ) {
      state.didStall = true;
      persistStreamState(state);
      setStreamState("idle");
      setStreamingControls(false);
      message.textContent = "";
      messageArticle?.removeAttribute("data-message-author-role");
      messageArticle?.removeAttribute("data-message-id");
      document.body.dataset.lastMessageId = "";
      timers.set(() => {
        writeConversationState(state.conversationId, state.messageId);
        messageArticle?.setAttribute("data-message-author-role", "assistant");
        setStreamState("running");
        setStreamingControls(true);
        renderChunk(state);
        scheduleNextTick(state);
      }, 3800);
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
      persistStreamState(state);
      timers.set(() => finishStream(state), 180);
      return;
    }

    if (state.index >= state.responseText.length) {
      finishStream(state);
      return;
    }

    persistStreamState(state);
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
    if (usesProviderStoppedThinkingOnce || usesProviderStoppedThinkingTwice) {
      const attempt = nextProviderFailureAttempt();
      if (usesProviderStoppedThinkingTwice || attempt === 1) {
        renderProviderStoppedThinking(promptText);
        return;
      }
    }

    const messageId = uniqueId("msg");
    const state = createAskStreamState({
      scenario,
      variant,
      conversationId,
      messageId,
      promptText,
      askNumber,
      uploadedFiles,
      uniqueId,
    });

    setStreamState("running");
    setStreamingControls(true);
    if (scenario === "followup-stale" && askNumber > 1) {
      state.pendingStart = true;
      persistStreamState(state);
      timers.set(() => {
        state.pendingStart = false;
        writeConversationState(state.conversationId, state.messageId);
        renderChunk(state);
        persistStreamState(state);
        scheduleNextTick(state);
      }, 250);
      return;
    }

    writeConversationState(
      conversationId,
      state.reasoningArtifacts
        ? state.reasoningArtifactMessageIds[0]
        : messageId,
    );
    clearAskFailureState();
    message.textContent = "";
    renderChunk(state);
    persistStreamState(state);
    scheduleNextTick(state);
  }

  function resumeStreamIfNeeded() {
    const conversationId = currentConversationId();
    const state = readStreamState(conversationId);
    if (!state) {
      return;
    }

    writeConversationState(state.conversationId, state.messageId);
    if (scenario === "recover-complete") {
      finishStream(state);
      return;
    }
    if (scenario === "sticky-stop-after-complete") {
      state.index = state.responseText.length;
      renderChunk(state);
      setStreamState("done");
      setStreamingControls(false);
      prepareArtifact(state.promptText);
      clearStreamState(state.conversationId);
      return;
    }
    if (scenario === "sticky-stop-after-reload") {
      state.index = state.responseText.length;
      renderChunk(state);
      setStreamState("running");
      setStreamingControls(true);
      persistStreamState(state);
      return;
    }
    if (scenario === "recover-message-successor") {
      state.messageId = uniqueId("msg");
      state.index = state.responseText.length;
      writeConversationState(state.conversationId, state.messageId);
      renderChunk(state);
      setStreamState("done");
      setStreamingControls(false);
      persistStreamState(state);
      return;
    }
    if (scenario === "recover-done-transient") {
      state.index = state.responseText.length;
      renderPlaceholderDrift(state);
      setStreamState("done");
      setStreamingControls(false);
      persistStreamState(state);
      timers.set(() => finishStream(state), 180);
      return;
    }
    if (scenario === "recover-rich-markdown") {
      state.index = state.responseText.length;
      renderRichRecoveryChunk(state);
      setStreamState("done");
      setStreamingControls(false);
      persistStreamState(state);
      return;
    }
    if (scenario === "recover-mismatch") {
      state.responseText = "Unexpected response after interruption";
      state.index = state.responseText.length;
      renderChunk(state);
      setStreamState("done");
      setStreamingControls(false);
      persistStreamState(state);
      return;
    }
    if (scenario === "recover-wrong-conversation") {
      const originalConversationId = state.conversationId;
      state.conversationId = uniqueId("conv");
      state.messageId = uniqueId("msg");
      state.responseText = "Unexpected response after interruption";
      state.index = state.responseText.length;
      clearStreamState(originalConversationId);
      writeConversationState(state.conversationId, state.messageId);
      renderChunk(state);
      setStreamState("done");
      setStreamingControls(false);
      persistStreamState(state);
      return;
    }
    if (scenario === "recover-running-transient") {
      setStreamState("running");
      setStreamingControls(true);
      renderPlaceholderDrift(state);
      persistStreamState(state);
      timers.set(() => {
        renderChunk(state);
        persistStreamState(state);
        scheduleNextTick(state);
      }, 180);
      return;
    }
    if (scenario === "recover-same-message-rewrite") {
      setStreamState("running");
      setStreamingControls(true);
      renderThinkingRewrite(state);
      persistStreamState(state);
      timers.set(() => {
        renderChunk(state);
        persistStreamState(state);
        scheduleNextTick(state);
      }, 180);
      return;
    }
    setStreamState("running");
    if (state.pendingStart) {
      state.pendingStart = false;
    }
    renderChunk(state);

    if (scenario === "timeout-reload-success") {
      finishStream(state);
      return;
    }
    if (scenario === "timeout") {
      persistStreamState(state);
      return;
    }
    if (scenario === "stream-stall-reload-advance") {
      state.reloadResumeCount = (state.reloadResumeCount || 0) + 1;
      state.index = Math.min(state.responseText.length - 1, state.index + 6);
      setStreamState("running");
      setStreamingControls(true);
      renderChunk(state);
      persistStreamState(state);
      return;
    }

    setStreamingControls(true);
    scheduleNextTick(state);
  }

  function cancelStream() {
    timers.stop();
    const conversationId = currentConversationId();
    clearStreamState(conversationId);
    setStreamState("cancelled");
    setStreamingControls(false);
  }

  return {
    startStream,
    resumeStreamIfNeeded,
    cancelStream,
  };
}
