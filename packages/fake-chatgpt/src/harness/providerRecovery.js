export function createProviderRecoveryRuntime({
  document,
  localStorage,
  scenario,
  refs,
  helpers,
}) {
  const { messageArticle, messageMarkdown, message } = refs;
  const {
    currentConversationId,
    uniqueId,
    writeConversationState,
    clearState,
    setStreamState,
    setStreamingControls,
    escapeHtml,
  } = helpers;

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

  function renderProviderStoppedThinking(promptText) {
    const conversationId = currentConversationId();
    const messageId = uniqueId("msg");

    writeConversationState(conversationId, messageId);
    localStorage.setItem(providerFailureStorageKey("priorPrompt"), promptText);
    clearState(conversationId);
    clearAskFailureState();
    const existingStoppedThinking =
      messageArticle instanceof HTMLElement &&
      /stopped thinking/i.test(messageArticle.textContent || "");
    if (existingStoppedThinking && messageArticle.parentElement) {
      const article = document.createElement("article");
      article.className = "message";
      article.setAttribute("data-message-author-role", "assistant");
      article.setAttribute("data-message-id", messageId);
      article.innerHTML =
        '<div class="markdown"><p><button type="button">Stopped thinking</button></p></div>';
      messageArticle.parentElement.appendChild(article);
      setStreamState("done");
      setStreamingControls(false);
      return;
    }
    if (messageArticle instanceof HTMLElement) {
      messageArticle.hidden = false;
      messageArticle.setAttribute("data-message-author-role", "assistant");
      messageArticle.setAttribute("data-message-id", messageId);
    }
    if (message) {
      message.innerHTML = '<button type="button">Stopped thinking</button>';
    } else if (messageMarkdown instanceof HTMLElement) {
      messageMarkdown.innerHTML =
        '<p><button type="button">Stopped thinking</button></p>';
    }
    setStreamState("done");
    setStreamingControls(false);
  }

  function retryButtonClicked() {
    document.body.dataset.retryClicks = String(
      Number.parseInt(document.body.dataset.retryClicks || "0", 10) + 1,
    );
  }

  return {
    clearAskFailureState,
    nextProviderFailureAttempt,
    renderProviderGenerationFailure,
    renderProviderStoppedThinking,
    retryButtonClicked,
  };
}
