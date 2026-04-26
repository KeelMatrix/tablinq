export const scenarioRegistry = {
  success: {},
  reload: {},
  timeout: {},
  "done-early": {},
  "stall-recover": {},
  "stall-recover-transient": {},
  "followup-stale": {},
  "followup-rich-text": {},
  "thinking-artifact-messages": {},
  "recover-complete": { recovery: true },
  "recover-message-successor": { recovery: true },
  "recover-done-transient": { recovery: true },
  "recover-rich-markdown": { recovery: true },
  "recover-mismatch": { recovery: true },
  "recover-wrong-conversation": { recovery: true },
  "recover-running-transient": { recovery: true },
  "recover-same-message-rewrite": { recovery: true },
  "submit-upload-trap": { composerPlusTrap: true },
  "submit-upload-trap-no-keyboard": { composerPlusTrap: true, keyboardSubmit: false },
  "submit-upload-stale-recover": { staleSubmitRecovery: true, keyboardSubmit: false },
  "upload-processing-delay": { delayedUploadProcessing: true },
  "upload-processing-stuck": { stuckUploadProcessing: true },
  "submit-never-ready": { blockedSubmitAfterUpload: true, keyboardSubmit: false },
  "research-success": { deepResearchFrame: true },
  "research-failure": { deepResearchFrame: true },
  "research-reload": { deepResearchFrame: true },
  "research-starting-shell": { deepResearchFrame: true, deepResearchStartingShell: true },
  "research-deep-landing": { deepResearchLanding: true },
  "research-deep-landing-submit-fail": { deepResearchLanding: true, researchSubmissionFails: true },
  "provider-generation-failure-recover": { providerGenerationFailureRecovery: true },
  "provider-generation-failure-after-success": { providerGenerationFailureAfterSuccess: true },
  "provider-generation-failure-twice": { providerGenerationFailureTwice: true },
};

export function createScenarioState(window) {
  const params = new URLSearchParams(window.location.search);
  const requestedVariant = params.get("variant");
  const variant =
    requestedVariant === "aria" ||
    requestedVariant === "live" ||
    requestedVariant === "menu"
      ? requestedVariant
      : "classic";
  const scenario = params.get("scenario") || "success";
  const definition = scenarioRegistry[scenario] ?? {};
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

  return {
    params,
    variant,
    scenario,
    definition,
    usesComposerPlusTrap: Boolean(definition.composerPlusTrap),
    usesStaleSubmitRecovery: Boolean(definition.staleSubmitRecovery),
    hasDelayedUploadProcessing: Boolean(definition.delayedUploadProcessing),
    hasStuckUploadProcessing: Boolean(definition.stuckUploadProcessing),
    hasBlockedSubmitAfterUpload: Boolean(definition.blockedSubmitAfterUpload),
    usesDeepResearchStartingShell: Boolean(definition.deepResearchStartingShell),
    usesDeepResearchLanding: Boolean(definition.deepResearchLanding),
    researchSubmissionFails: Boolean(definition.researchSubmissionFails),
    usesDeepResearchFrame:
      variant === "live" && Boolean(definition.deepResearchFrame),
    usesProviderGenerationFailureRecovery: Boolean(
      definition.providerGenerationFailureRecovery,
    ),
    usesProviderGenerationFailureAfterSuccess: Boolean(
      definition.providerGenerationFailureAfterSuccess,
    ),
    usesProviderGenerationFailureTwice: Boolean(
      definition.providerGenerationFailureTwice,
    ),
    keyboardSubmitEnabled: definition.keyboardSubmit !== false,
    initialModel,
    initialEffort,
    missingControls: new Set(
      (params.get("missing") || "")
        .split(",")
        .map((value) => value.trim().toLowerCase())
        .filter(Boolean),
    ),
  };
}

export function initializeScenarioDataset(document, window, config) {
  document.body.dataset.uiVariant = config.variant;
  document.body.dataset.streamState = "idle";
  document.body.dataset.selectedModel = config.initialModel;
  document.body.dataset.selectedEffort = config.initialEffort;
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
  document.body.dataset.keyboardSubmitReady = config.keyboardSubmitEnabled
    ? "true"
    : "false";
  document.body.dataset.researchSubmitted = "false";
  document.body.dataset.researchLandingVisited = "false";
}

export function pathnameSegments(pathname) {
  return pathname.split("/").filter(Boolean);
}

export function pathnameFromSegments(segments) {
  return segments.length > 0 ? "/" + segments.join("/") : "/";
}

export function conversationIdFromPathname(pathname) {
  const segments = pathnameSegments(pathname);
  if (
    segments.length >= 2 &&
    segments[segments.length - 2]?.toLowerCase() === "c"
  ) {
    return segments[segments.length - 1] || null;
  }
  return null;
}

export function conversationWorkspacePathname(pathname) {
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

export function conversationPathname(pathname, conversationId) {
  const segments = pathnameSegments(conversationWorkspacePathname(pathname));
  segments.push("c", conversationId);
  return pathnameFromSegments(segments);
}

export function escapeHtml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

export function deepResearchPathname(pathname) {
  const segments = pathnameSegments(conversationWorkspacePathname(pathname));
  segments.push("deep-research");
  return pathnameFromSegments(segments);
}
