import {
  createScenarioState,
  escapeHtml,
  initializeScenarioDataset,
} from "./harness/scenarioState.js";
import { renderHarnessShell } from "./harness/uiVariants.js";
import { createUploadRuntime } from "./harness/uploads.js";
import { createProviderRecoveryRuntime } from "./harness/providerRecovery.js";
import {
  createConversationPersistence,
  uniqueId,
} from "./harness/conversationPersistence.js";
import { createAskRuntime } from "./harness/askRuntime.js";
import { createDeepResearchRuntime } from "./harness/deepResearchRuntime.js";
import { createMenuRuntime } from "./harness/menus.js";

const root = document.querySelector("#app-root");
const note = document.querySelector("#variant-note");
const scenarioState = createScenarioState(window);
const {
  variant,
  scenario,
  usesDeepResearchLanding,
  researchSubmissionFails,
  hasDelayedUploadProcessing,
  hasStuckUploadProcessing,
  keyboardSubmitEnabled,
} = scenarioState;

initializeScenarioDataset(document, window, scenarioState);
renderHarnessShell({ root, note, window, config: scenarioState });
if (scenarioState.usesDelayedConversationLoadFailure) {
  window.setTimeout(() => {
    document
      .querySelector("[data-delayed-conversation-load-failure]")
      ?.removeAttribute("hidden");
  }, 900);
}

function hasControl(name) {
  return !scenarioState.missingControls.has(name);
}

function queryFirst(selectors) {
  for (const selector of selectors) {
    const match = document.querySelector(selector);
    if (match) {
      return match;
    }
  }
  return null;
}

const composer = queryFirst([
  "#prompt-textarea",
  "[data-slot='composer']",
  "form [role='textbox']",
  "[role='textbox']",
]);
const sendButton = queryFirst([
  "[data-testid='composer-root'] [data-testid='send-button']",
  "[data-testid='composer-root'] [aria-label='Send prompt']",
  "[data-testid='composer-root'] [aria-label='Send message']",
  "form [data-testid='send-button']",
  "form [aria-label='Send prompt']",
  "form [aria-label='Send message']",
  "form [data-action='submit']",
  "[data-action='submit']",
  "[aria-label='Submit prompt']",
  "[aria-label='Send message']",
  "[aria-label='Send prompt']",
]);
const staleSendButton = document.querySelector("#stale-send-button");
const composerPlusButton = document.querySelector("#composer-plus-btn");
const stopButton = document.querySelector("[data-testid='stop-button']");
const uploadEntry = document.querySelector(
  "[data-slot='upload-entry'], [data-testid='upload-button'], [aria-label='Add photos and files']",
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
  "[data-slot='artifact-link'], [data-testid='artifact-download'], [aria-label='Download report']",
);
const deepResearchFrame = document.querySelector("#deep-research-sandbox");

let activeTimer = null;
const timers = {
  stop() {
    if (activeTimer) {
      clearTimeout(activeTimer);
      activeTimer = null;
    }
  },
  set(callback, delayMs) {
    activeTimer = setTimeout(callback, delayMs);
  },
};

const conversation = createConversationPersistence({
  document,
  window,
  localStorage,
});

function setStreamState(state) {
  document.body.dataset.streamState = state;
  messageArticle?.setAttribute("data-stream-state", state);
}

function setResearchState(state, statusMessage = "") {
  document.body.dataset.researchState = state;
  document.body.dataset.researchStatusMessage = statusMessage;
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

function resetToNewChat() {
  timers.stop();
  delete document.body.dataset.conversationId;
  delete document.body.dataset.lastMessageId;
  setStreamState("idle");
  setStreamingControls(false);
  const url = new URL(window.location.href);
  url.pathname = "/";
  window.history.replaceState({}, "", url);
  if (message) {
    message.textContent = "Streaming output lands here.";
  }
  messageArticle?.removeAttribute("data-message-id");
  clearComposer();
}

function incrementDatasetCounter(name) {
  document.body.dataset[name] = String(
    Number.parseInt(document.body.dataset[name] || "0", 10) + 1,
  );
}

const uploadRuntime = createUploadRuntime({
  document,
  variant,
  config: scenarioState,
  refs: {
    uploadList,
    sendButton,
    staleSendButton,
    stopButton,
    artifactLink,
  },
});
const {
  setUploadPopoverOpen,
  uploadedFiles,
  renderUploadedFiles,
  syncSubmissionControls,
  setStreamingControls,
  stopUploadProcessingTimer,
  scheduleUploadProcessingClear,
  setUploadProcessing,
  resetArtifactLink,
  setUploadedFiles,
  prepareArtifact,
} = uploadRuntime;

const providerRecoveryRuntime = createProviderRecoveryRuntime({
  document,
  localStorage,
  scenario,
  refs: {
    messageArticle,
    messageMarkdown,
    message,
  },
  helpers: {
    currentConversationId: conversation.currentConversationId,
    uniqueId,
    writeConversationState: conversation.writeConversationState,
    clearState: conversation.clearStreamState,
    setStreamState,
    setStreamingControls,
    escapeHtml,
  },
});

const deepResearchRuntime = createDeepResearchRuntime({
  document,
  window,
  config: scenarioState,
  refs: {
    messageArticle,
    message,
    deepResearchFrame,
  },
  timers,
  conversation,
  uploads: {
    uploadedFiles,
    setStreamingControls,
    prepareArtifact,
  },
  clearComposer,
  setStreamState,
  setResearchState,
});

const askRuntime = createAskRuntime({
  document,
  window,
  config: scenarioState,
  refs: {
    messageArticle,
    messageMarkdown,
    message,
  },
  timers,
  conversation,
  uploads: {
    uploadedFiles,
    setStreamingControls,
    prepareArtifact,
  },
  providerRecovery: providerRecoveryRuntime,
  setStreamState,
  escapeHtml,
});

const menuRuntime = createMenuRuntime({
  document,
  config: scenarioState,
  hasControl,
  isDeepResearchLanding: deepResearchRuntime.onDeepResearchLanding,
  enterDeepResearchLanding: deepResearchRuntime.enterDeepResearchLanding,
});

function submitPrompt(method) {
  if (!composer || !message) {
    return;
  }
  if (document.body.dataset.uploadProcessing === "true") {
    return;
  }

  timers.stop();
  providerRecoveryRuntime.clearAskFailureState();
  setUploadPopoverOpen(false);
  document.body.dataset.lastSubmissionMethod = method;
  document.body.dataset.lastSubmittedModel =
    document.body.dataset.selectedModel || "";
  document.body.dataset.lastSubmittedEffort =
    document.body.dataset.selectedEffort || "";
  const promptText =
    "value" in composer ? composer.value.trim() : composer.textContent.trim();
  if (
    document.body.dataset.selectedMode === "research" &&
    deepResearchRuntime.onDeepResearchLanding()
  ) {
    if (researchSubmissionFails) {
      return;
    }
    deepResearchRuntime.startResearch(promptText);
    return;
  }
  askRuntime.startStream(promptText);
}

menuRuntime.registerSelectionEvents();

uploadEntry?.addEventListener("click", () => {
  if (uploadInput instanceof HTMLElement) {
    uploadInput.click();
  }
});

composerPlusButton?.addEventListener("click", () => {
  if (document.querySelector("#tablinq-tools-menu")) {
    return;
  }
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
    scheduleUploadProcessingClear(2200);
  }
});

window.__tablinqCancelStream = () => {
  askRuntime.cancelStream();
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

document.addEventListener("keydown", (event) => {
  if (!(event.ctrlKey && event.shiftKey && event.key.toLowerCase() === "o")) {
    return;
  }
  event.preventDefault();
  incrementDatasetCounter("newChatShortcut");
  resetToNewChat();
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
    providerRecoveryRuntime.retryButtonClicked();
    return;
  }
  if (target.closest("[data-testid='create-new-chat-button']")) {
    event.preventDefault();
    incrementDatasetCounter("newChatClicks");
    resetToNewChat();
  }
});

if (deepResearchRuntime.onDeepResearchLanding()) {
  document.body.dataset.selectedMode = "research";
  deepResearchRuntime.enterDeepResearchLanding();
}

window.syncSelectedState = menuRuntime.syncSelectedState;
menuRuntime.syncSelectedState();
renderUploadedFiles(uploadedFiles());
resetArtifactLink();
syncSubmissionControls();
deepResearchRuntime.renderDeepResearchFrameIdle();
deepResearchRuntime.resumeResearchIfNeeded();
askRuntime.resumeStreamIfNeeded();
