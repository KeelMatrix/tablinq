function createResearchRuntimeModule({
  textUtils,
  domUtils,
  composerRuntime,
  messageRuntime,
  markdownRuntime,
  conversationRuntime,
}) {
  const { normalizeText, collapseText, datasetValue, hasToken } = textUtils;
  const { queryFirstVisible, latestVisible } = domUtils;
  const {
    findComposer,
    readComposerText,
    composerDisabled,
    readSubmitControlState,
    stopButtonVisible,
  } = composerRuntime;
  const { findLatestAssistantMessage, readMessageIdentity } = messageRuntime;
  const { collectText } = markdownRuntime;
  const { parseConversationIdFromUrl, currentPathname } = conversationRuntime;

  const readResearchStatusMessage = () =>
    datasetValue(document.body.dataset.researchStatusMessage) ??
    normalizeText(
      queryFirstVisible([
        "[data-slot='research-status']",
        "[data-testid='research-status']",
        "[aria-live='polite']",
      ])?.textContent,
    ) ??
    null;

  const onDeepResearchLanding = () =>
    /(^|\/)deep-research$/.test(currentPathname()) &&
    parseConversationIdFromUrl() === null;

  const isPlaceholderResearchText = (text) =>
    hasToken(text, ["streaming", "output", "lands", "here"]) ||
    hasToken(text, ["workspace", "notices", "appear", "thinking"]);

  const normalizeResearchReportText = (text) => {
    const collapsed = collapseText(text);
    return collapsed && !isPlaceholderResearchText(collapsed) ? collapsed : "";
  };

  const findResearchReportArticle = () => {
    const explicit = latestVisible([
      "[data-slot='research-report']",
      "[data-message-author-role='assistant'][data-message-id]",
      "[data-slot='assistant-message'][data-message-id]",
    ]);
    if (explicit) {
      return explicit;
    }

    const latest = findLatestAssistantMessage();
    if (
      latest &&
      (datasetValue(latest.getAttribute("data-message-id")) ||
        latest.getAttribute("data-stream-state") === "running")
    ) {
      return latest;
    }
    return null;
  };

  const inferResearchStatus = (
    text,
    statusMessage = readResearchStatusMessage(),
  ) => {
    const explicitState = datasetValue(document.body.dataset.researchState);
    if (explicitState && explicitState !== "idle") {
      return explicitState;
    }

    const statusText = normalizeText(statusMessage);

    if (
      hasToken(statusText, ["failed", "error", "retry"]) &&
      !hasToken(statusText, ["running"])
    ) {
      return "failed";
    }
    if (hasToken(statusText, ["source", "sources", "gathering", "finding"])) {
      return "waiting_for_sources";
    }
    if (hasToken(statusText, ["running", "drafting", "working"])) {
      return "running";
    }
    if (stopButtonVisible()) {
      return "running";
    }
    if (text) {
      return "done";
    }
    return "idle";
  };

  const readResearchSubmissionState = () => {
    const article = findResearchReportArticle();
    const reportText = normalizeResearchReportText(
      article ? collectText(article) : "",
    );
    const statusMessage = readResearchStatusMessage();
    const status = inferResearchStatus(reportText, statusMessage);
    const controlState = readSubmitControlState();
    const conversationId =
      datasetValue(document.body.dataset.conversationId) ??
      parseConversationIdFromUrl();
    const messageId =
      readMessageIdentity(article) ??
      datasetValue(document.body.dataset.lastMessageId) ??
      null;
    const submitted =
      status !== "idle" || Boolean(conversationId) || Boolean(messageId);

    return {
      composerReady: Boolean(findComposer()),
      composerText: readComposerText(),
      composerDisabled: composerDisabled(),
      ...controlState,
      onDeepResearchLanding: onDeepResearchLanding(),
      status,
      statusMessage,
      reportText,
      conversationId,
      messageId,
      currentUrl: window.location.href,
      submitted,
    };
  };

  const researchSubmissionStarted = () =>
    readResearchSubmissionState().submitted;

  return {
    readResearchStatusMessage,
    onDeepResearchLanding,
    normalizeResearchReportText,
    findResearchReportArticle,
    inferResearchStatus,
    readResearchSubmissionState,
    researchSubmissionStarted,
  };
}
