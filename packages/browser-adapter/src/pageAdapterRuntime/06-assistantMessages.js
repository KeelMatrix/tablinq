function createAssistantMessageRuntimeModule({
  textUtils,
  domUtils,
  messageFilters,
  markdownRuntime,
  composerRuntime,
  conversationRuntime,
}) {
  const {
    normalizeText,
    collapseText,
    uniqueCollapsed,
    datasetValue,
    hasToken,
  } = textUtils;
  const { queryFirstVisible, queryVisible, visibleUnique, findByAliases } =
    domUtils;
  const {
    isReasoningArtifactMessage,
    explicitAssistantRole,
    readMessageIdentity,
    thoughtDividerBeforeMessage,
  } = messageFilters;
  const {
    collectText,
    hasTransientRenderedWidgets,
    materializeCodePreviewSources,
  } = markdownRuntime;
  const { stopButtonVisible } = composerRuntime;
  const { parseConversationIdFromUrl } = conversationRuntime;

  const isModernAssistantTurn = (element) => {
    if (!(element instanceof HTMLElement)) {
      return false;
    }
    const testId = datasetValue(element.getAttribute("data-testid"));
    if (!testId || !/^conversation-turn(?:-[a-z0-9_-]+)?$/i.test(testId)) {
      return false;
    }
    if (element.querySelector(".markdown, [data-message-content]")) {
      return true;
    }
    const labels = uniqueCollapsed([
      element.getAttribute("aria-label"),
      ...Array.from(
        element.querySelectorAll(
          "h1, h2, h3, h4, h5, h6, .sr-only, [aria-label]",
        ),
      ).map((candidate) =>
        candidate instanceof HTMLElement
          ? (candidate.getAttribute("aria-label") ??
            candidate.textContent ??
            "")
          : "",
      ),
    ]);
    return labels.some((value) => hasToken(value, ["chatgpt", "assistant"]));
  };

  const isAssistantMessageElement = (element) => {
    if (!(element instanceof HTMLElement)) {
      return false;
    }
    if (isReasoningArtifactMessage(element)) {
      return false;
    }
    if (explicitAssistantRole(element)) {
      return true;
    }
    const slot = normalizeText(
      datasetValue(element.getAttribute("data-slot")) ?? "",
    );
    if (slot === "assistant-message") {
      return true;
    }
    if (element.matches("article.assistant")) {
      return true;
    }
    return isModernAssistantTurn(element);
  };

  const findAssistantMessages = () => {
    const direct = visibleUnique(
      document.querySelectorAll(
        [
          "[data-slot='assistant-message']",
          "[data-message-author-role='assistant']",
          "article.assistant",
          "section[data-testid^='conversation-turn']",
        ].join(", "),
      ),
    ).filter((element) => isAssistantMessageElement(element));
    if (direct.length > 0) {
      return direct.filter(
        (element) => !isInlineTextDocumentCreatedMarker(element),
      );
    }

    const transcript = queryFirstVisible([
      "[data-testid='conversation-turns']",
      "[data-slot='conversation-thread']",
      ".transcript",
      "main",
    ]);
    if (!(transcript instanceof Element)) {
      return [];
    }
    return visibleUnique(
      transcript.querySelectorAll(
        [
          "article[data-message-id]",
          "article[data-stream-state]",
          "article.message",
          "article",
          "section[data-testid^='conversation-turn']",
        ].join(", "),
      ),
    ).filter((element) => isAssistantMessageElement(element));
  };

  const isInlineTextDocumentCreatedMarker = (element) => {
    if (!(element instanceof HTMLElement)) {
      return false;
    }
    if (!explicitAssistantRole(element)) {
      return false;
    }
    const text = normalizeText(
      collectText(element) || element.textContent || "",
    );
    if (text && text !== "created.") {
      return false;
    }
    const turn = element.closest("section[data-testid^='conversation-turn']");
    return Boolean(turn?.querySelector?.("[id^='textdoc-message-']"));
  };

  const findLatestAssistantMessage = () => {
    const messages = findAssistantMessages();
    return messages[messages.length - 1] ?? null;
  };

  const resolveAskArticle = (baseline) => {
    const messages = findAssistantMessages();
    if (messages.length === 0) {
      return null;
    }
    if (!baseline) {
      return messages[messages.length - 1] ?? null;
    }

    const latest = messages[messages.length - 1] ?? null;
    const latestMessageId = readMessageIdentity(latest);
    const latestText = latest ? collectText(latest) : "";
    const currentConversationId =
      datasetValue(document.body.dataset.conversationId) ??
      parseConversationIdFromUrl();
    const streamState =
      latest?.getAttribute("data-stream-state") ??
      document.body.dataset.streamState;

    if (
      latest &&
      latestText &&
      baseline.conversationId !== currentConversationId &&
      currentConversationId
    ) {
      return latest;
    }
    if (messages.length > Math.max(0, baseline.assistantMessageCount ?? 0)) {
      return latest;
    }
    if (latest && latestMessageId && latestMessageId !== baseline.messageId) {
      return latest;
    }
    if (
      latest &&
      latestText &&
      latestText !== (baseline.text ?? "") &&
      streamState &&
      streamState !== "idle"
    ) {
      return latest;
    }
    return null;
  };

  const PROVIDER_GENERATION_FAILURE_CODE =
    "provider_generation_failed_nonretryable";
  const PROVIDER_GENERATION_FAILURE_MARKER =
    "something went wrong while generating the response";
  const PROVIDER_GENERATION_FAILURE_HELP_CENTER = "help center";

  const readAskFailureState = (article) => {
    const explicitCode = datasetValue(document.body.dataset.askFailureCode);
    const explicitMessage = datasetValue(
      document.body.dataset.askFailureMessage,
    );
    const retryVisible = Boolean(
      findByAliases({
        aliases: ["retry"],
        fallbackTokens: ["retry"],
      }),
    );

    if (explicitCode === PROVIDER_GENERATION_FAILURE_CODE) {
      return {
        failureCode: explicitCode,
        failureMessage:
          explicitMessage ??
          "Something went wrong while generating the response. If this issue persists please contact us through our help center.",
        retryVisible,
      };
    }

    const candidateTexts = uniqueCollapsed([
      explicitMessage,
      article?.textContent ?? "",
      ...queryVisible([
        "[role='alert']",
        "[aria-live='assertive']",
        "[data-slot='error']",
        "[data-testid*='error']",
      ]).map((element) => element.textContent ?? ""),
      document.body.innerText ?? "",
    ]);
    const failureMessage =
      candidateTexts.find((value) => {
        const normalized = normalizeText(value);
        return (
          normalized.includes(PROVIDER_GENERATION_FAILURE_MARKER) &&
          normalized.includes(PROVIDER_GENERATION_FAILURE_HELP_CENTER)
        );
      }) ?? null;

    return {
      failureCode: failureMessage ? PROVIDER_GENERATION_FAILURE_CODE : null,
      failureMessage,
      retryVisible,
    };
  };

  const inferAskState = (text, article) => {
    const failure = readAskFailureState(article);
    if (failure.failureCode) {
      return "failed";
    }
    const explicitState =
      article?.getAttribute("data-stream-state") ??
      document.body.dataset.streamState;
    if (explicitState && explicitState !== "idle") {
      return explicitState;
    }
    if (stopButtonVisible()) {
      return "running";
    }
    if (text) {
      return "done";
    }
    return explicitState || "idle";
  };

  return {
    findAssistantMessages,
    findLatestAssistantMessage,
    resolveAskArticle,
    readAskFailureState,
    inferAskState,
    readMessageIdentity,
    collectText,
    hasTransientRenderedWidgets,
    materializeCodePreviewSources,
    thoughtDividerBeforeMessage,
  };
}
