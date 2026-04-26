import {
  conversationIdFromPathname,
  conversationPathname,
} from "./scenarioState.js";

export function uniqueId(prefix) {
  return `${prefix}-${Date.now()}-${Math.floor(Math.random() * 1_000_000)}`;
}

export function createConversationPersistence({
  document,
  window,
  localStorage,
}) {
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
    window.history.replaceState({}, "", url);
  }

  function readStreamState(conversationId) {
    const raw = localStorage.getItem(streamStorageKey(conversationId));
    return raw ? JSON.parse(raw) : null;
  }

  function persistStreamState(state) {
    localStorage.setItem(
      streamStorageKey(state.conversationId),
      JSON.stringify(state),
    );
  }

  function clearStreamState(conversationId) {
    localStorage.removeItem(streamStorageKey(conversationId));
  }

  function readResearchState(conversationId) {
    const raw = localStorage.getItem(researchStorageKey(conversationId));
    return raw ? JSON.parse(raw) : null;
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

  return {
    currentConversationId,
    nextAskNumber,
    writeConversationState,
    readStreamState,
    persistStreamState,
    clearStreamState,
    readResearchState,
    persistResearchState,
    clearResearchState,
  };
}
