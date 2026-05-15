function createCommandRuntime({
  textUtils,
  domUtils,
  composerRuntime,
  messageRuntime,
  capabilityRuntime,
  researchRuntime,
  conversationRuntime,
}) {
  return {
    composerReady() {
      return Boolean(composerRuntime.findComposer());
    },
    fillComposer({ prompt }) {
      composerRuntime.setComposerValue(composerRuntime.findComposer(), prompt);
      return true;
    },
    clickSubmit() {
      const button = composerRuntime.findSubmitButton();
      if (button && domUtils.clickControl(button)) {
        return true;
      }
      return false;
    },
    async applyAskConfiguration({ report, request }) {
      if (
        !request.modelClass &&
        !request.effort &&
        (!request.mode || request.mode === "normal")
      ) {
        return true;
      }
      if (!report) {
        throw new Error(
          "capability missing: no capability report is available for model or mode selection",
        );
      }
      if (request.mode) {
        await capabilityRuntime.ensureMode(report, request.mode);
      }
      if (request.modelClass) {
        await capabilityRuntime.ensureModelClass(report, request.modelClass);
      }
      if (request.effort) {
        await capabilityRuntime.ensureEffort(report, request.effort);
      }
      return true;
    },
    uploadAcknowledged(payload) {
      return composerRuntime.uploadAcknowledged(payload);
    },
    readComposerAttachmentState() {
      return composerRuntime.readComposerAttachmentState();
    },
    readAskBaseline() {
      const article = messageRuntime.findLatestAssistantMessage();
      const messages = messageRuntime.findAssistantMessages();
      return {
        assistantMessageCount: messages.length,
        conversationId:
          textUtils.datasetValue(document.body.dataset.conversationId) ??
          conversationRuntime.parseConversationIdFromUrl(),
        messageId: messageRuntime.readMessageIdentity(article),
        text: article ? messageRuntime.collectText(article) : "",
      };
    },
    readAskSnapshot({ baseline } = {}) {
      const article = messageRuntime.resolveAskArticle(baseline ?? null);
      const text = article ? messageRuntime.collectText(article) : "";
      const failure = messageRuntime.readAskFailureState(article);
      return {
        hasAssistantMessage: Boolean(article),
        text,
        state: messageRuntime.inferAskState(text, article),
        failureCode: failure.failureCode,
        failureMessage: failure.failureMessage,
        retryVisible: failure.retryVisible,
        hasTransientWidgets: article
          ? messageRuntime.hasTransientRenderedWidgets(article)
          : false,
        conversationId:
          textUtils.datasetValue(document.body.dataset.conversationId) ??
          conversationRuntime.parseConversationIdFromUrl(),
        messageId:
          messageRuntime.readMessageIdentity(article) ??
          (article
            ? textUtils.datasetValue(document.body.dataset.lastMessageId)
            : null) ??
          null,
        thoughtDividerBeforeMessage:
          messageRuntime.thoughtDividerBeforeMessage(article),
        currentUrl: window.location.href,
      };
    },
    async materializeAskCodePreviewSources({ baseline } = {}) {
      const article = messageRuntime.resolveAskArticle(baseline ?? null);
      return article
        ? await messageRuntime.materializeCodePreviewSources(article)
        : 0;
    },
    readResearchSnapshot() {
      const submission = researchRuntime.readResearchSubmissionState();
      return {
        status: submission.status,
        statusMessage: submission.statusMessage,
        reportText: submission.reportText,
        conversationId: submission.conversationId,
        messageId: submission.messageId,
        currentUrl: submission.currentUrl,
      };
    },
    readResearchSubmissionState() {
      return researchRuntime.readResearchSubmissionState();
    },
    researchSubmissionStarted() {
      return researchRuntime.researchSubmissionStarted();
    },
    readSubmitState() {
      return composerRuntime.readSubmitState();
    },
    submitReady() {
      return composerRuntime.submitReady();
    },
    attachmentsStillProcessing() {
      return composerRuntime.attachmentsStillProcessing();
    },
    submitStarted(payload) {
      return composerRuntime.submitStarted(payload ?? {});
    },
  };
}
