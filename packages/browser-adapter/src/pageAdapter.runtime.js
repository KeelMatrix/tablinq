function createTextUtilsModule(deps) {
  return {
    normalizeText: deps.normalizeText,
    collapseText: deps.collapseText,
    unique: deps.unique,
    uniqueCollapsed: deps.uniqueCollapsed,
    datasetValue: deps.datasetValue,
    hasToken: deps.hasToken,
    collapseInlineText: deps.collapseInlineText,
    normalizeInlineOutput: deps.normalizeInlineOutput,
    indentLines: deps.indentLines,
  };
}

function createDomUtilsModule(deps) {
  return {
    getComputed: deps.getComputed,
    isVisible: deps.isVisible,
    firstVisible: deps.firstVisible,
    accessibleName: deps.accessibleName,
    queryFirstVisible: deps.queryFirstVisible,
    queryVisible: deps.queryVisible,
    actionableSelector: deps.actionableSelector,
    actionableElements: deps.actionableElements,
    latestVisible: deps.latestVisible,
    visibleUnique: deps.visibleUnique,
    isSelected: deps.isSelected,
    isDisabled: deps.isDisabled,
    findByAliases: deps.findByAliases,
    clickControl: deps.clickControl,
  };
}

function createComposerRuntimeModule(deps) {
  return {
    findComposer: deps.findComposer,
    composerContainers: deps.composerContainers,
    isComposerAdjacent: deps.isComposerAdjacent,
    findComposerAdjacentControl: deps.findComposerAdjacentControl,
    readComposerText: deps.readComposerText,
    composerDisabled: deps.composerDisabled,
    findSubmitButton: deps.findSubmitButton,
    readSubmitControlState: deps.readSubmitControlState,
    keyboardSubmitReady: deps.keyboardSubmitReady,
    submitButtonEnabled: deps.submitButtonEnabled,
    submitReady: deps.submitReady,
    stopButtonVisible: deps.stopButtonVisible,
    readSubmitState: deps.readSubmitState,
    submitStarted: deps.submitStarted,
    attachmentsStillProcessing: deps.attachmentsStillProcessing,
    setComposerValue: deps.setComposerValue,
    uploadAcknowledged: deps.uploadAcknowledged,
  };
}

function createArtifactRuntimeModule(deps) {
  return {
    isReasoningArtifactMessage: deps.isReasoningArtifactMessage,
    isInsideReasoningArtifactMessage: deps.isInsideReasoningArtifactMessage,
    isMermaidPreviewBlock: deps.isMermaidPreviewBlock,
    serializeMermaidPreviewBlock: deps.serializeMermaidPreviewBlock,
    collectMermaidSvgLabels: deps.collectMermaidSvgLabels,
  };
}

function createMessageRuntimeModule(deps) {
  return {
    artifactRuntime: deps.artifactRuntime,
    findAssistantMessages: deps.findAssistantMessages,
    findLatestAssistantMessage: deps.findLatestAssistantMessage,
    resolveAskArticle: deps.resolveAskArticle,
    readAskFailureState: deps.readAskFailureState,
    inferAskState: deps.inferAskState,
    readMessageIdentity: deps.readMessageIdentity,
    collectText: deps.collectText,
    thoughtDividerBeforeMessage: deps.thoughtDividerBeforeMessage,
  };
}

function createCapabilityRuntimeModule(deps) {
  return {
    expandAliases: deps.expandAliases,
    controlStateDetail: deps.controlStateDetail,
    throwCapabilityMissing: deps.throwCapabilityMissing,
    throwUiDrift: deps.throwUiDrift,
    mergeAliasBuckets: deps.mergeAliasBuckets,
    ensureMode: deps.ensureMode,
    ensureModelClass: deps.ensureModelClass,
    ensureEffort: deps.ensureEffort,
    discoverVisibleModelAliases: deps.discoverVisibleModelAliases,
    discoverVisibleEffortAliases: deps.discoverVisibleEffortAliases,
  };
}

function createConversationRuntimeModule(deps) {
  return {
    parseConversationIdFromUrl: deps.parseConversationIdFromUrl,
    currentPathname: deps.currentPathname,
    readResearchStatusMessage: deps.readResearchStatusMessage,
    onDeepResearchLanding: deps.onDeepResearchLanding,
    normalizeResearchReportText: deps.normalizeResearchReportText,
    findResearchReportArticle: deps.findResearchReportArticle,
    inferResearchStatus: deps.inferResearchStatus,
    readResearchSubmissionState: deps.readResearchSubmissionState,
    researchSubmissionStarted: deps.researchSubmissionStarted,
  };
}

function createCommandRuntime({
  textUtils,
  domUtils,
  composerRuntime,
  messageRuntime,
  capabilityRuntime,
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
      if (request.modelClass) {
        await capabilityRuntime.ensureModelClass(report, request.modelClass);
      }
      if (request.effort) {
        await capabilityRuntime.ensureEffort(report, request.effort);
      }
      if (request.mode) {
        capabilityRuntime.ensureMode(report, request.mode);
      }
      return true;
    },
    uploadAcknowledged(payload) {
      return composerRuntime.uploadAcknowledged(payload);
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
    readResearchSnapshot() {
      const submission = conversationRuntime.readResearchSubmissionState();
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
      return conversationRuntime.readResearchSubmissionState();
    },
    researchSubmissionStarted() {
      return conversationRuntime.researchSubmissionStarted();
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

export function createTablinqPageAdapterRuntime() {
  const normalizeText = (value) =>
    (value ?? "").replace(/\s+/g, " ").trim().toLowerCase();
  const collapseText = (value) => (value ?? "").replace(/\s+/g, " ").trim();

  const unique = (values) => {
    const seen = new Set();
    const output = [];
    for (const value of values) {
      const normalized = normalizeText(value);
      if (!normalized || seen.has(normalized)) {
        continue;
      }
      seen.add(normalized);
      output.push(normalized);
    }
    return output;
  };

  const uniqueCollapsed = (values) => {
    const seen = new Set();
    const output = [];
    for (const value of values) {
      const collapsed = collapseText(value);
      const normalized = normalizeText(collapsed);
      if (!normalized || seen.has(normalized)) {
        continue;
      }
      seen.add(normalized);
      output.push(collapsed);
    }
    return output;
  };

  const datasetValue = (value) => {
    const trimmed = (value ?? "").trim();
    return trimmed ? trimmed : null;
  };

  const hasToken = (value, tokens) => {
    const normalized = normalizeText(value);
    return tokens.some((token) =>
      new RegExp(
        `(^|[^a-z0-9])${token.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}([^a-z0-9]|$)`,
      ).test(normalized),
    );
  };

  const getComputed = (element) => {
    if (
      typeof window === "undefined" ||
      typeof window.getComputedStyle !== "function"
    ) {
      return null;
    }
    try {
      return window.getComputedStyle(element);
    } catch {
      return null;
    }
  };

  const isVisible = (element) => {
    if (!(element instanceof HTMLElement)) {
      return false;
    }
    if (element.hidden || element.getAttribute("aria-hidden") === "true") {
      return false;
    }
    if (
      element instanceof HTMLInputElement &&
      element.type === "hidden" &&
      !element.isConnected
    ) {
      return false;
    }
    const computed = getComputed(element);
    if (!computed) {
      return true;
    }
    return (
      computed.display !== "none" &&
      computed.visibility !== "hidden" &&
      computed.opacity !== "0"
    );
  };

  const firstVisible = (elements) => {
    for (const element of Array.from(elements)) {
      if (isVisible(element)) {
        return element;
      }
    }
    return null;
  };

  const accessibleName = (element) => {
    if (!(element instanceof HTMLElement)) {
      return "";
    }
    const ariaLabel = element.getAttribute("aria-label");
    if (ariaLabel) {
      return normalizeText(ariaLabel);
    }

    const labelledBy = element.getAttribute("aria-labelledby");
    if (labelledBy) {
      const labels = labelledBy
        .split(/\s+/)
        .map((id) => document.getElementById(id))
        .filter(Boolean)
        .map((value) => normalizeText(value.textContent))
        .filter(Boolean);
      if (labels.length > 0) {
        return labels.join(" ");
      }
    }

    if (
      element instanceof HTMLInputElement ||
      element instanceof HTMLTextAreaElement
    ) {
      if (element.labels?.length) {
        const text = Array.from(element.labels)
          .map((label) => normalizeText(label.textContent))
          .filter(Boolean)
          .join(" ");
        if (text) {
          return text;
        }
      }
      if (element.placeholder) {
        return normalizeText(element.placeholder);
      }
      if (element.value) {
        return normalizeText(element.value);
      }
    }

    const title = element.getAttribute("title");
    if (title) {
      return normalizeText(title);
    }

    return normalizeText(element.textContent);
  };

  const queryFirstVisible = (selectors) => {
    for (const selector of selectors) {
      const match = firstVisible(document.querySelectorAll(selector));
      if (match) {
        return match;
      }
    }
    return null;
  };

  const queryVisible = (selectors) =>
    visibleUnique(
      selectors.flatMap((selector) =>
        Array.from(document.querySelectorAll(selector)),
      ),
    );

  const actionableSelector = [
    "button",
    "[role='button']",
    "[role='tab']",
    "[role='switch']",
    "[role='option']",
    "[role='menuitem']",
    "[role='menuitemradio']",
    "[aria-haspopup]",
    "label[for]",
    "a",
  ].join(", ");

  const actionableElements = () =>
    Array.from(document.querySelectorAll(actionableSelector)).filter(isVisible);

  const latestVisible = (selectors) => {
    const matches = [];
    for (const selector of selectors) {
      matches.push(...Array.from(document.querySelectorAll(selector)));
    }
    const visible = matches.filter(isVisible);
    if (visible.length === 0) {
      return null;
    }
    return visible[visible.length - 1] ?? null;
  };

  const visibleUnique = (elements) => {
    const seen = new Set();
    const output = [];
    for (const element of Array.from(elements)) {
      if (!isVisible(element) || seen.has(element)) {
        continue;
      }
      seen.add(element);
      output.push(element);
    }
    return output;
  };

  const expandAliases = (report, bucket, key, defaults) =>
    unique([...(report?.aliases?.[bucket]?.[key] ?? []), ...defaults]);

  const isSelected = (element) =>
    element.getAttribute("aria-pressed") === "true" ||
    element.getAttribute("aria-checked") === "true" ||
    element.getAttribute("aria-selected") === "true" ||
    element.dataset.selected === "true" ||
    element.dataset.active === "true";

  const isDisabled = (element) =>
    element.getAttribute("aria-disabled") === "true" ||
    element.hasAttribute("disabled") ||
    element.dataset.disabled === "true";

  const findByAliases = ({
    selectors = [],
    aliases = [],
    fallbackTokens = [],
    include,
    exclude,
  }) => {
    const selectorMatch = queryFirstVisible(selectors);
    if (
      selectorMatch &&
      !exclude?.(selectorMatch) &&
      (!include || include(selectorMatch))
    ) {
      return selectorMatch;
    }

    const normalizedAliases = unique(aliases);
    for (const element of actionableElements()) {
      if (exclude?.(element) || (include && !include(element))) {
        continue;
      }
      const name = accessibleName(element);
      if (!name) {
        continue;
      }
      if (
        normalizedAliases.some(
          (alias) =>
            name === alias || name.includes(alias) || alias.includes(name),
        )
      ) {
        return element;
      }
    }

    for (const element of actionableElements()) {
      if (exclude?.(element) || (include && !include(element))) {
        continue;
      }
      if (hasToken(accessibleName(element), fallbackTokens)) {
        return element;
      }
    }

    return null;
  };

  const clickControl = (element) => {
    if (!(element instanceof HTMLElement)) {
      return false;
    }
    if (isDisabled(element)) {
      return false;
    }
    if (element.tagName.toLowerCase() === "label") {
      const inputId = element.getAttribute("for");
      if (inputId) {
        const input = document.getElementById(inputId);
        if (input instanceof HTMLElement) {
          input.click();
          return true;
        }
      }
    }
    element.focus?.();
    try {
      element.click();
      return true;
    } catch {}
    const rect = element.getBoundingClientRect();
    const eventInit = {
      bubbles: true,
      cancelable: true,
      composed: true,
      button: 0,
      buttons: 1,
      clientX:
        rect.left + Math.max(1, Math.min(rect.width / 2, rect.width - 1)),
      clientY:
        rect.top + Math.max(1, Math.min(rect.height / 2, rect.height - 1)),
    };
    if (typeof PointerEvent === "function") {
      element.dispatchEvent(
        new PointerEvent("pointerdown", {
          ...eventInit,
          pointerId: 1,
          pointerType: "mouse",
          isPrimary: true,
        }),
      );
    }
    element.dispatchEvent(new MouseEvent("mousedown", eventInit));
    if (typeof PointerEvent === "function") {
      element.dispatchEvent(
        new PointerEvent("pointerup", {
          ...eventInit,
          pointerId: 1,
          pointerType: "mouse",
          isPrimary: true,
        }),
      );
    }
    element.dispatchEvent(new MouseEvent("mouseup", eventInit));
    element.dispatchEvent(new MouseEvent("click", eventInit));
    return true;
  };

  const parseConversationIdFromUrl = () => {
    try {
      const url = new URL(window.location.href);
      const match = url.pathname.match(/\/c\/([^/?#]+)/i);
      return match?.[1] ?? null;
    } catch {
      return null;
    }
  };

  const currentPathname = () => {
    try {
      return new URL(window.location.href).pathname.toLowerCase();
    } catch {
      return "";
    }
  };

  const findComposer = () =>
    queryFirstVisible([
      "#prompt-textarea",
      "[data-slot='composer']",
      "[data-testid='composer-root'] textarea",
      "[data-testid='composer-root'] [contenteditable='true']",
      "textarea[aria-label*='Message']",
      "textarea[placeholder*='message']",
      "[role='textbox'][aria-label*='Message']",
      "[contenteditable='true'][aria-label*='Message']",
      "textarea",
      "[role='textbox']",
      "[contenteditable='true']",
    ]);

  const composerContainers = () => {
    const composer = findComposer();
    const seen = new Set();
    const containers = [];
    for (const container of [
      composer?.closest("[data-testid='composer-root']"),
      composer?.closest("form"),
      composer?.parentElement,
      composer?.closest("section"),
      composer?.closest("main"),
    ]) {
      if (!(container instanceof HTMLElement) || seen.has(container)) {
        continue;
      }
      seen.add(container);
      containers.push(container);
    }
    return containers;
  };

  const isComposerAdjacent = (element) =>
    element instanceof HTMLElement &&
    composerContainers().some(
      (container) => container === element || container.contains(element),
    );

  const findComposerAdjacentControl = ({
    selectors = [],
    tokens = [],
    requirePopup = false,
    include,
    exclude,
  }) => {
    for (const container of composerContainers()) {
      for (const selector of selectors) {
        const match = firstVisible(container.querySelectorAll(selector));
        if (
          match &&
          !exclude?.(match) &&
          (!include || include(match)) &&
          (!requirePopup || match.getAttribute("aria-haspopup") !== null)
        ) {
          return match;
        }
      }

      for (const element of visibleUnique(
        container.querySelectorAll(actionableSelector),
      )) {
        if (
          exclude?.(element) ||
          (include && !include(element)) ||
          (requirePopup && element.getAttribute("aria-haspopup") === null)
        ) {
          continue;
        }
        if (hasToken(accessibleName(element), tokens)) {
          return element;
        }
      }
    }
    return null;
  };

  const readComposerText = () => {
    const composer = findComposer();
    if (!(composer instanceof HTMLElement)) {
      return "";
    }
    if (
      composer instanceof HTMLTextAreaElement ||
      composer instanceof HTMLInputElement
    ) {
      return collapseText(composer.value);
    }
    return collapseText(composer.textContent);
  };

  const composerDisabled = () => {
    const composer = findComposer();
    if (!(composer instanceof HTMLElement)) {
      return false;
    }
    return (
      composer.getAttribute("aria-disabled") === "true" ||
      composer.hasAttribute("disabled") ||
      composer.getAttribute("contenteditable") === "false"
    );
  };

  const controlFingerprint = (element) =>
    normalizeText(
      [
        accessibleName(element),
        element?.id ?? "",
        element?.getAttribute?.("data-testid") ?? "",
        element?.getAttribute?.("data-slot") ?? "",
        element?.getAttribute?.("name") ?? "",
        typeof element?.className === "string" ? element.className : "",
      ].join(" "),
    );

  const isUploadLikeControl = (element) => {
    if (!(element instanceof HTMLElement)) {
      return false;
    }
    if (
      element.matches(
        [
          "[data-slot='upload-entry']",
          "[data-testid='upload-button']",
          "label[for]",
          "input[type='file']",
        ].join(", "),
      )
    ) {
      return true;
    }
    const fingerprint = controlFingerprint(element);
    if (
      hasToken(fingerprint, [
        "plus",
        "add",
        "upload",
        "file",
        "files",
        "photo",
        "photos",
        "attach",
        "attachment",
        "paperclip",
      ])
    ) {
      return true;
    }
    return (
      element.getAttribute("aria-haspopup") !== null &&
      !hasToken(fingerprint, ["send", "submit", "ask"])
    );
  };

  const submitCandidateScore = (element, { allowDisabled = false } = {}) => {
    if (!(element instanceof HTMLElement)) {
      return Number.NEGATIVE_INFINITY;
    }
    if (!allowDisabled && isDisabled(element)) {
      return Number.NEGATIVE_INFINITY;
    }
    if (isUploadLikeControl(element)) {
      return Number.NEGATIVE_INFINITY;
    }

    let score = 0;
    const composer = findComposer();
    const fingerprint = controlFingerprint(element);
    if (
      element.matches(
        [
          "[data-action='submit']",
          "[data-testid='send-button']",
          "button[aria-label='Submit prompt']",
          "button[aria-label*='Send']",
          "input[type='submit']",
        ].join(", "),
      )
    ) {
      score += 10;
    }
    if (hasToken(fingerprint, ["send", "submit", "ask"])) {
      score += 6;
    }
    if (
      element.tagName.toLowerCase() === "input" &&
      element.getAttribute("type") === "submit"
    ) {
      score += 4;
    }
    if (composer) {
      if (element.closest("form") === composer.closest("form")) {
        score += 8;
      }
      if (element.parentElement === composer.parentElement) {
        score += 4;
      }
      if (element.closest("main") === composer.closest("main")) {
        score += 1;
      }
    }
    if (isDisabled(element)) {
      score -= 3;
    }
    if (hasToken(fingerprint, ["stop", "cancel"])) {
      score -= 10;
    }
    if (element.getAttribute("aria-haspopup") !== null) {
      score -= 4;
    }
    return score;
  };

  const submitCandidatesNearComposer = () => {
    const composer = findComposer();
    const seen = new Set();
    const candidates = [];
    for (const container of [
      composer?.closest("form"),
      composer?.parentElement,
      composer?.closest("section"),
      composer?.closest("main"),
    ]) {
      if (!container) {
        continue;
      }
      for (const candidate of visibleUnique(
        container.querySelectorAll(
          "button, [role='button'], input[type='submit']",
        ),
      )) {
        if (seen.has(candidate)) {
          continue;
        }
        seen.add(candidate);
        candidates.push(candidate);
      }
    }
    return candidates;
  };

  const findSubmitButton = ({ allowDisabled = false } = {}) => {
    const explicit = queryVisible([
      "[data-action='submit']",
      "[data-testid='send-button']",
      "button[aria-label='Submit prompt']",
      "button[aria-label*='Send']",
      "input[type='submit']",
    ])
      .map((element) => ({
        element,
        score: submitCandidateScore(element, { allowDisabled }),
      }))
      .filter((entry) => entry.score > 0)
      .sort((left, right) => right.score - left.score);
    if (explicit[0]?.element) {
      return explicit[0].element;
    }

    const nearby = submitCandidatesNearComposer()
      .map((element) => ({
        element,
        score: submitCandidateScore(element, { allowDisabled }),
      }))
      .filter((entry) => entry.score > 0)
      .sort((left, right) => right.score - left.score);
    if (nearby[0]?.element) {
      return nearby[0].element;
    }

    return findByAliases({
      fallbackTokens: ["send", "submit", "ask"],
      exclude: (element) =>
        submitCandidateScore(element, { allowDisabled }) <= 0,
    });
  };

  const submitButtonBusy = (element) =>
    element instanceof HTMLElement &&
    (element.getAttribute("aria-busy") === "true" ||
      element.dataset.busy === "true" ||
      element.dataset.loading === "true");

  const readSubmitControlState = () => {
    const button = findSubmitButton({ allowDisabled: true });
    const visible = button instanceof HTMLElement;
    const enabled = visible && !isDisabled(button);
    const busy = visible && submitButtonBusy(button);
    return {
      submitVisible: visible,
      submitEnabled: enabled,
      submitBusy: busy,
    };
  };

  const uploadAttachmentSelectors = [
    "[data-slot='uploaded-file']",
    "[data-slot='upload-entry-name']",
    "[data-testid='upload-chip']",
    "[data-testid*='attachment']",
    "[aria-label*='Attached file']",
    "[aria-label*='attachment']",
  ].join(", ");

  const visibleAttachmentElements = () =>
    visibleUnique(document.querySelectorAll(uploadAttachmentSelectors));

  const hasAcknowledgedAttachments = () =>
    unique((document.body.dataset.uploadedFiles ?? "").split("|")).length > 0 ||
    visibleAttachmentElements().length > 0;

  const attachmentRegions = () => {
    const seen = new Set();
    const roots = [];
    const add = (element) => {
      if (!(element instanceof HTMLElement) || seen.has(element)) {
        return;
      }
      seen.add(element);
      roots.push(element);
    };

    add(queryFirstVisible(["[data-slot='upload-list']"]));
    for (const container of composerContainers()) {
      add(container);
    }
    for (const element of visibleAttachmentElements()) {
      add(element.closest("[data-slot='upload-list']"));
      add(element.closest("[data-testid='composer-root']"));
      add(element.parentElement);
    }
    if (roots.length === 0) {
      add(document.body);
    }
    return roots;
  };

  const attachmentProcessingSelectors = [
    "[data-slot='upload-processing']",
    "[data-upload-processing='true']",
    "[data-upload-state='processing']",
    "[data-upload-state='uploading']",
    "[data-testid*='upload-progress']",
    "[data-testid*='attachment-progress']",
    "[data-testid*='upload-processing']",
    "[role='progressbar']",
    "[aria-busy='true']",
  ];

  const attachmentsStillProcessing = () => {
    if (
      document.body.dataset.uploadProcessing === "true" ||
      document.body.dataset.attachmentsProcessing === "true"
    ) {
      return true;
    }
    if (!hasAcknowledgedAttachments()) {
      return false;
    }

    for (const root of attachmentRegions()) {
      for (const selector of attachmentProcessingSelectors) {
        const match = firstVisible(root.querySelectorAll(selector));
        if (match) {
          return true;
        }
      }

      for (const element of visibleUnique(
        root.querySelectorAll("span, div, li, p, button"),
      )) {
        const fingerprint = controlFingerprint(element);
        if (
          hasToken(fingerprint, [
            "upload",
            "attachment",
            "file",
            "processing",
            "uploading",
            "indexing",
            "analyzing",
          ]) &&
          (element.getAttribute("aria-busy") === "true" ||
            hasToken(fingerprint, [
              "processing",
              "uploading",
              "indexing",
              "analyzing",
            ]))
        ) {
          return true;
        }
      }
    }

    const submitState = readSubmitControlState();
    return submitState.submitVisible && submitState.submitBusy;
  };

  const keyboardSubmitReady = () => {
    const explicit = document.body.dataset.keyboardSubmitReady;
    if (explicit === "true") {
      return true;
    }
    if (explicit === "false") {
      return false;
    }
    const composer = findComposer();
    return (
      composer instanceof HTMLElement &&
      !composerDisabled() &&
      !readSubmitControlState().submitVisible
    );
  };

  const submitButtonEnabled = () => {
    const state = readSubmitControlState();
    return state.submitVisible && state.submitEnabled && !state.submitBusy;
  };

  const submitReady = () =>
    !attachmentsStillProcessing() &&
    (submitButtonEnabled() || keyboardSubmitReady());

  const stopButtonVisible = () =>
    Boolean(
      findByAliases({
        selectors: [
          "[data-testid='stop-button']",
          "button[aria-label*='Stop']",
        ],
        fallbackTokens: ["stop", "cancel"],
      }),
    );

  const readSubmitState = () => ({
    composerText: readComposerText(),
    composerDisabled: composerDisabled(),
    ...readSubmitControlState(),
    keyboardReady: keyboardSubmitReady(),
    attachmentsProcessing: attachmentsStillProcessing(),
    stopVisible: stopButtonVisible(),
    assistantMessageCount: findAssistantMessages().length,
    streamState:
      document.body.dataset.streamState ??
      findLatestAssistantMessage()?.getAttribute("data-stream-state") ??
      "idle",
  });

  const submitStarted = ({ baseline }) => {
    const current = readSubmitState();
    if (!baseline) {
      return current.stopVisible || current.streamState === "running";
    }
    return (
      (current.stopVisible && !baseline.stopVisible) ||
      current.streamState === "running" ||
      (current.streamState !== "idle" &&
        current.streamState !== (baseline.streamState ?? "idle")) ||
      current.assistantMessageCount > (baseline.assistantMessageCount ?? 0) ||
      (current.submitBusy && !baseline.submitBusy) ||
      (baseline.composerText &&
        current.composerText !== baseline.composerText) ||
      (current.composerDisabled && !baseline.composerDisabled) ||
      (baseline.submitEnabled &&
        !current.submitEnabled &&
        (current.stopVisible ||
          current.streamState !== "idle" ||
          current.submitBusy))
    );
  };

  const collapseInlineText = (value) =>
    (value ?? "").replace(/\s+/g, " ").trim();

  const normalizeInlineOutput = (value) =>
    (value ?? "")
      .replace(/[ \t]+\n/g, "\n")
      .replace(/\n[ \t]+/g, "\n")
      .replace(/[ \t]{2,}/g, " ")
      .trim();

  const isTransientAskStatusText = (value) =>
    /^thought for\b/i.test(collapseText(value));

  const isTransientAskStatusElement = (element) => {
    if (!(element instanceof HTMLElement)) {
      return false;
    }
    const text = collapseText(element.textContent);
    if (!isTransientAskStatusText(text)) {
      return false;
    }
    if (element.tagName.toLowerCase() === "button") {
      return true;
    }
    const button = element.querySelector("button");
    if (!(button instanceof HTMLElement)) {
      return false;
    }
    return (
      isTransientAskStatusText(button.textContent) &&
      Array.from(
        element.querySelectorAll(
          [
            ".markdown",
            "p",
            "pre",
            "code",
            "table",
            "ol",
            "ul",
            "blockquote",
            "h1",
            "h2",
            "h3",
            "h4",
            "h5",
            "h6",
          ].join(", "),
        ),
      ).every((candidate) => candidate === button || button.contains(candidate))
    );
  };

  const isThoughtDividerElement = (element) => {
    if (!(element instanceof HTMLElement)) {
      return false;
    }
    const text = collapseText(element.textContent);
    if (!isTransientAskStatusText(text)) {
      return false;
    }
    return (
      Boolean(element.querySelector("button")) || element.tagName === "BUTTON"
    );
  };

  const explicitAssistantRole = (element) =>
    normalizeText(
      datasetValue(element?.getAttribute?.("data-message-author-role")) ?? "",
    ) === "assistant";

  const visibleSibling = (element, direction) => {
    let current =
      direction === "previous"
        ? element?.previousElementSibling
        : element?.nextElementSibling;
    while (current) {
      if (isVisible(current)) {
        return current;
      }
      current =
        direction === "previous"
          ? current.previousElementSibling
          : current.nextElementSibling;
    }
    return null;
  };

  const thoughtDividerBeforeMessage = (element) =>
    isThoughtDividerElement(visibleSibling(element, "previous"));

  const hasFollowingThoughtDivider = (element) => {
    let current = visibleSibling(element, "next");
    while (current) {
      if (isThoughtDividerElement(current)) {
        return true;
      }
      current = visibleSibling(current, "next");
    }
    return false;
  };

  const isReasoningArtifactMessage = (element) => {
    if (!(element instanceof HTMLElement)) {
      return false;
    }
    if (
      isTransientAskStatusElement(element) ||
      isThoughtDividerElement(element)
    ) {
      return true;
    }
    if (!explicitAssistantRole(element)) {
      return false;
    }
    return (
      Boolean(element.querySelector(".markdown, [data-message-content]")) &&
      hasFollowingThoughtDivider(element)
    );
  };

  const isInsideReasoningArtifactMessage = (element) => {
    if (!(element instanceof HTMLElement)) {
      return false;
    }
    const message = element.closest("[data-message-author-role='assistant']");
    return (
      message instanceof HTMLElement && isReasoningArtifactMessage(message)
    );
  };

  const findMessageContentRoot = (root) => {
    if (!(root instanceof HTMLElement)) {
      return null;
    }
    const candidates = Array.from(
      root.querySelectorAll(".markdown, [data-message-content]"),
    ).filter((candidate) => candidate instanceof HTMLElement);
    for (const candidate of candidates) {
      if (
        !isTransientAskStatusElement(candidate) &&
        !isInsideReasoningArtifactMessage(candidate)
      ) {
        return candidate;
      }
    }
    if (candidates.length > 0) {
      return null;
    }
    if (isTransientAskStatusElement(root) || isReasoningArtifactMessage(root)) {
      return null;
    }
    return root;
  };

  const readMessageIdentity = (element) => {
    if (!(element instanceof HTMLElement)) {
      return null;
    }
    return (
      datasetValue(element.getAttribute("data-message-id")) ??
      datasetValue(element.getAttribute("data-testid")) ??
      datasetValue(element.id) ??
      null
    );
  };

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

  const collectCodeBlockText = (element) => {
    if (!(element instanceof HTMLElement)) {
      return "";
    }
    const explicitCode = element.querySelector("code");
    if (explicitCode instanceof HTMLElement) {
      return (
        explicitCode.innerText ||
        explicitCode.textContent ||
        ""
      ).trimEnd();
    }
    const editorContent = element.querySelector(".cm-content");
    if (editorContent instanceof HTMLElement) {
      return (
        editorContent.innerText ||
        editorContent.textContent ||
        ""
      ).trimEnd();
    }
    return (element.innerText || element.textContent || "").trimEnd();
  };

  const collectCodeBlockLanguage = (element) => {
    if (!(element instanceof HTMLElement)) {
      return "";
    }
    const explicit =
      element.getAttribute("data-language") ??
      element.getAttribute("data-code-language") ??
      element.dataset.language ??
      element.dataset.codeLanguage;
    if (explicit) {
      return collapseInlineText(explicit).toLowerCase();
    }

    const candidates = Array.from(
      element.querySelectorAll(
        [
          "[class*='language']",
          "[class*='font-medium']",
          "[data-language]",
          "[data-code-language]",
        ].join(", "),
      ),
    );
    for (const candidate of candidates) {
      if (!(candidate instanceof HTMLElement) || candidate.closest("button")) {
        continue;
      }
      const label = collapseInlineText(candidate.textContent);
      if (
        label &&
        !hasToken(label, ["copy", "run", "code", "table"]) &&
        label.length <= 32
      ) {
        return label.toLowerCase();
      }
    }
    return "";
  };

  const decodeDataUrlText = (url) => {
    if (typeof url !== "string" || !url.toLowerCase().startsWith("data:")) {
      return "";
    }
    const commaIndex = url.indexOf(",");
    if (commaIndex < 0) {
      return "";
    }
    const metadata = url.slice("data:".length, commaIndex);
    const payload = url.slice(commaIndex + 1);
    const isBase64 = metadata
      .split(";")
      .map((segment) => segment.trim().toLowerCase())
      .includes("base64");
    try {
      if (isBase64) {
        const binary = atob(payload);
        const bytes = Uint8Array.from(binary, (value) => value.charCodeAt(0));
        return new TextDecoder().decode(bytes);
      }
      return decodeURIComponent(payload);
    } catch {
      return "";
    }
  };

  const collectMermaidSvgLabels = (svgText) => {
    if (!svgText || typeof DOMParser !== "function") {
      return [];
    }
    try {
      const parsed = new DOMParser().parseFromString(svgText, "image/svg+xml");
      return uniqueCollapsed(
        Array.from(
          parsed.querySelectorAll(
            "text, tspan, foreignObject p, foreignObject span, .nodeLabel, .edgeLabel",
          ),
        )
          .map((candidate) => candidate.textContent ?? "")
          .filter(Boolean),
      ).filter((label) => normalizeText(label) !== "mermaid");
    } catch {
      return [];
    }
  };

  const isMermaidPreviewBlock = (element) => {
    if (!(element instanceof HTMLElement)) {
      return false;
    }
    if (
      element.querySelector("code") instanceof HTMLElement ||
      element.querySelector(".cm-content") instanceof HTMLElement
    ) {
      return false;
    }
    const hasPreviewSurface = Boolean(
      element.querySelector(
        "img[src^='data:image/'], img[src^='blob:'], canvas",
      ),
    );
    if (!hasPreviewSurface) {
      return false;
    }
    const text = collapseInlineText(element.textContent);
    return (
      hasToken(text, ["mermaid"]) ||
      (Boolean(element.querySelector("button[aria-label='Code']")) &&
        Boolean(element.querySelector("button[aria-label='Preview']")))
    );
  };

  const serializeMermaidPreviewBlock = (element) => {
    if (!isMermaidPreviewBlock(element)) {
      return "";
    }
    const previewImage = element.querySelector(
      "img[src^='data:image/svg+xml']",
    );
    const svgText =
      previewImage instanceof HTMLImageElement
        ? decodeDataUrlText(previewImage.currentSrc || previewImage.src)
        : "";
    const labels = collectMermaidSvgLabels(svgText);
    if (labels.length === 0) {
      return "Mermaid diagram preview.";
    }
    const summary = labels.slice(0, 12).join("; ");
    return `Mermaid diagram preview: ${summary}${labels.length > 12 ? "; ..." : "."}`;
  };

  const isIgnoredMessageChrome = (element) => {
    if (!(element instanceof HTMLElement)) {
      return false;
    }
    if (
      element.matches(
        "[data-testid='artifact-download'], [data-slot='artifact-link']",
      )
    ) {
      return true;
    }
    const ariaLabel = collapseInlineText(element.getAttribute("aria-label"));
    const text = collapseInlineText(element.textContent);
    return (
      element.tagName.toLowerCase() === "a" &&
      ((Boolean(ariaLabel) && hasToken(ariaLabel, ["download", "artifact"])) ||
        (element.hasAttribute("download") &&
          Boolean(text) &&
          hasToken(text, ["artifact", "download"])))
    );
  };

  const serializeInlineNode = (node) => {
    if (node.nodeType === Node.TEXT_NODE) {
      return (node.textContent ?? "").replace(/\s+/g, " ");
    }
    if (!(node instanceof HTMLElement)) {
      return "";
    }
    if (!isVisible(node)) {
      return "";
    }
    if (isIgnoredMessageChrome(node)) {
      return "";
    }
    const tag = node.tagName.toLowerCase();
    if (tag === "button") {
      return "";
    }
    if (tag === "br") {
      return "\n";
    }
    if (tag === "code" && node.closest("pre") === null) {
      return `\`${collapseInlineText(node.textContent)}\``;
    }
    if (tag === "strong") {
      const content = serializeInlineChildren(node);
      return content ? `**${content}**` : "";
    }
    if (tag === "em") {
      const content = serializeInlineChildren(node);
      return content ? `*${content}*` : "";
    }
    if (tag === "a") {
      const text = serializeInlineChildren(node);
      const href = collapseInlineText(node.getAttribute("href"));
      if (text && href) {
        return `[${text}](${href})`;
      }
      return text;
    }
    return serializeInlineChildren(node);
  };

  const serializeInlineChildren = (element) =>
    normalizeInlineOutput(
      Array.from(element.childNodes)
        .map((node) => serializeInlineNode(node))
        .join(""),
    );

  const indentLines = (value, prefix, blankPrefix = prefix) =>
    value
      .split("\n")
      .map((line) => (line ? `${prefix}${line}` : blankPrefix.trimEnd()))
      .join("\n");

  const serializeMarkdownTableCell = (value) =>
    normalizeInlineOutput(value)
      .replace(/[ \t]*\n[ \t]*/g, "<br>")
      .replace(/\|/g, "\\|");

  const serializeTable = (table) => {
    if (!(table instanceof HTMLElement)) {
      return "";
    }
    const rows = Array.from(table.querySelectorAll("tr")).map((row) =>
      Array.from(row.children)
        .filter((cell) => cell instanceof HTMLElement)
        .map((cell) =>
          serializeMarkdownTableCell(serializeInlineChildren(cell)),
        ),
    );
    const header = rows[0] ?? [];
    if (header.length === 0) {
      return "";
    }
    const body = rows.slice(1);
    const separator = header.map(() => "---");
    return [
      `| ${header.join(" | ")} |`,
      `| ${separator.join(" | ")} |`,
      ...body.map((row) => `| ${row.join(" | ")} |`),
    ].join("\n");
  };

  const serializeListItem = (item, prefix, options = {}) => {
    if (!(item instanceof HTMLElement)) {
      return "";
    }
    const childElements = Array.from(item.children).filter(
      (child) =>
        child instanceof HTMLElement &&
        child.tagName.toLowerCase() !== "button" &&
        !isIgnoredMessageChrome(child),
    );
    const nestedLists = childElements.filter((child) =>
      ["ul", "ol"].includes(child.tagName.toLowerCase()),
    );
    const contentElements = childElements.filter(
      (child) => !["ul", "ol"].includes(child.tagName.toLowerCase()),
    );
    const content =
      contentElements.length > 0
        ? contentElements
            .map((child, index) =>
              serializeBlock(child, {
                ...options,
                isLastBlock:
                  nestedLists.length === 0 &&
                  index === contentElements.length - 1 &&
                  options.isLastBlock,
              }),
            )
            .filter(Boolean)
            .join("\n\n")
        : serializeInlineChildren(item);
    const normalized = normalizeInlineOutput(content);
    if (!normalized) {
      return "";
    }
    const lines = normalized.split("\n");
    const firstLine = `${prefix}${lines[0]}`;
    const continuationPrefix = " ".repeat(prefix.length);
    const rest = lines
      .slice(1)
      .map((line) => `${continuationPrefix}${line}`)
      .join("\n");
    const nested = nestedLists
      .map((list, index) =>
        serializeBlock(list, {
          ...options,
          isLastBlock: index === nestedLists.length - 1 && options.isLastBlock,
        }),
      )
      .filter(Boolean)
      .map((value) =>
        indentLines(value, continuationPrefix, continuationPrefix),
      )
      .join("\n");
    return [firstLine, rest, nested].filter(Boolean).join("\n");
  };

  const serializeBlock = (element, options = {}) => {
    if (!(element instanceof HTMLElement)) {
      return "";
    }
    if (isIgnoredMessageChrome(element)) {
      return "";
    }
    const tag = element.tagName.toLowerCase();
    if (tag === "button") {
      return "";
    }
    if (
      tag === "h1" ||
      tag === "h2" ||
      tag === "h3" ||
      tag === "h4" ||
      tag === "h5" ||
      tag === "h6"
    ) {
      const level = Number(tag.slice(1));
      return `${"#".repeat(level)} ${serializeInlineChildren(element)}`.trim();
    }
    if (tag === "p") {
      return serializeInlineChildren(element);
    }
    if (tag === "blockquote") {
      const content = serializeBlockChildren(element);
      return content ? indentLines(content, "> ", ">") : "";
    }
    if (tag === "pre") {
      const mermaidPreview = serializeMermaidPreviewBlock(element);
      if (mermaidPreview) {
        return mermaidPreview;
      }
      const code = collectCodeBlockText(element);
      if (!code) {
        return "";
      }
      const language = collectCodeBlockLanguage(element);
      if (options.streamRunning && options.isLastBlock) {
        return `\`\`\`${language}\n${code}`;
      }
      return `\`\`\`${language}\n${code}\n\`\`\``;
    }
    if (tag === "ul") {
      return Array.from(element.children)
        .map((child, index, items) =>
          serializeListItem(child, "- ", {
            ...options,
            isLastBlock: index === items.length - 1 && options.isLastBlock,
          }),
        )
        .filter(Boolean)
        .join("\n");
    }
    if (tag === "ol") {
      return Array.from(element.children)
        .map((child, index, items) =>
          serializeListItem(child, `${index + 1}. `, {
            ...options,
            isLastBlock: index === items.length - 1 && options.isLastBlock,
          }),
        )
        .filter(Boolean)
        .join("\n");
    }
    if (tag === "table") {
      return serializeTable(element);
    }
    return serializeBlockChildren(element, options);
  };

  const serializeBlockChildren = (element, options = {}) => {
    if (!(element instanceof HTMLElement)) {
      return "";
    }
    const blocks = Array.from(element.children)
      .map((child, index, items) => ({
        tag: child instanceof HTMLElement ? child.tagName.toLowerCase() : "",
        text: serializeBlock(child, {
          ...options,
          isLastBlock: index === items.length - 1 && options.isLastBlock,
        }),
      }))
      .filter((entry) => Boolean(entry.text));
    if (blocks.length > 0) {
      return blocks.reduce((output, entry, index) => {
        if (index === 0) {
          return entry.text;
        }
        const previousTag = blocks[index - 1]?.tag ?? "";
        const joiner =
          /^h[1-6]$/.test(previousTag) && /^h[1-6]$/.test(entry.tag)
            ? "\n"
            : "\n\n";
        return `${output}${joiner}${entry.text}`;
      }, "");
    }
    return serializeInlineChildren(element);
  };

  const collectText = (root) => {
    const contentRoot = findMessageContentRoot(root);
    if (!(contentRoot instanceof HTMLElement)) {
      return "";
    }
    if (isTransientAskStatusElement(contentRoot)) {
      return "";
    }
    const streamRunning =
      (root?.getAttribute?.("data-stream-state") ??
        document.body.dataset.streamState ??
        "idle") === "running";
    const serialized = serializeBlockChildren(contentRoot, {
      isLastBlock: true,
      streamRunning,
    });
    if (serialized && !isTransientAskStatusText(serialized)) {
      return serialized;
    }

    const directText = uniqueCollapsed(
      Array.from(contentRoot.childNodes)
        .filter((node) => node.nodeType === Node.TEXT_NODE)
        .map((node) => node.textContent),
    );
    return directText
      .filter((value) => !isTransientAskStatusText(value))
      .join("\n");
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
      return direct;
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

  const setComposerValue = (element, value) => {
    if (!(element instanceof HTMLElement)) {
      throw new Error("composer not found");
    }

    element.focus();
    if (
      element instanceof HTMLTextAreaElement ||
      element instanceof HTMLInputElement
    ) {
      const descriptor =
        Object.getOwnPropertyDescriptor(
          Object.getPrototypeOf(element),
          "value",
        ) ??
        Object.getOwnPropertyDescriptor(
          HTMLTextAreaElement.prototype,
          "value",
        ) ??
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value");
      if (descriptor?.set) {
        descriptor.set.call(element, value);
      } else {
        element.value = value;
      }
      element.dispatchEvent(new Event("input", { bubbles: true }));
      element.dispatchEvent(new Event("change", { bubbles: true }));
      return;
    }

    if (element.isContentEditable) {
      const selection = window.getSelection();
      const range = document.createRange();
      range.selectNodeContents(element);
      range.deleteContents();
      selection?.removeAllRanges();
      selection?.addRange(range);
      if (typeof document.execCommand === "function") {
        document.execCommand("insertText", false, value);
      } else {
        element.textContent = value;
      }
      element.dispatchEvent(
        new InputEvent("input", { bubbles: true, data: value }),
      );
      return;
    }

    element.textContent = value;
    element.dispatchEvent(
      new InputEvent("input", { bubbles: true, data: value }),
    );
  };

  const controlStateDetail = (aliases, requested) => {
    const entries = Object.entries(aliases ?? {})
      .map(([key, labels]) => `${key}=[${labels.join(", ")}]`)
      .join("; ");
    if (!entries) {
      return `requested=${requested}; discovered=none`;
    }
    return `requested=${requested}; discovered=${entries}`;
  };

  const throwCapabilityMissing = (capability, requested, aliases) => {
    throw new Error(
      `capability missing: ${capability}; ${controlStateDetail(aliases, requested)}`,
    );
  };

  const throwUiDrift = (detail) => {
    throw new Error(`ui drift: ${detail}`);
  };

  const mergeAliasBuckets = (...buckets) => {
    const merged = {};
    for (const bucket of buckets) {
      for (const [key, labels] of Object.entries(bucket ?? {})) {
        const current = merged[key] ?? [];
        merged[key] = unique([...current, ...(labels ?? [])]);
      }
    }
    return merged;
  };

  const modelDefaults = (modelClass) => [
    modelClass,
    modelClass === "instant" ? "for everyday chats" : "for complex questions",
  ];

  const modelFallbackTokens = (modelClass) =>
    modelClass === "instant" ? ["instant"] : ["thinking"];

  const effortDefaults = (effort) => [
    effort,
    effort === "standard" ? "thinking" : "extended thinking",
  ];

  const effortFallbackTokens = (effort) =>
    effort === "standard" ? ["standard"] : ["extended"];

  const menuOptionSelector =
    "[role='menuitemradio'], [role='menuitem'], [role='option']";
  const popupRootSelector = "[role='menu'], [role='listbox']";
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const elementRole = (element) => element?.getAttribute?.("role") ?? "";
  const isPopupOption = (element) =>
    element instanceof HTMLElement &&
    ["menuitemradio", "menuitem", "option"].includes(elementRole(element));

  const isModelPickerTrigger = (element) =>
    element instanceof HTMLElement &&
    element.getAttribute("aria-haspopup") !== null &&
    (element.matches("button[aria-label='Model selector']") ||
      element.matches("button[aria-label='Switch model']") ||
      element.matches("button[data-testid='model-switcher-dropdown-button']") ||
      hasToken(accessibleName(element), ["chatgpt", "model", "selector"]));

  const isEffortPickerTrigger = (element) =>
    element instanceof HTMLElement &&
    element.getAttribute("aria-haspopup") !== null &&
    isComposerAdjacent(element) &&
    (element.matches("button.__composer-pill") ||
      hasToken(accessibleName(element), [
        "thinking",
        "standard",
        "extended",
        "effort",
      ]));

  const isModelOption = (element) =>
    element instanceof HTMLElement &&
    !isModelPickerTrigger(element) &&
    isPopupOption(element) &&
    element.closest(popupRootSelector) !== null &&
    hasToken(accessibleName(element), ["instant", "thinking", "pro"]);

  const isEffortOption = (element) =>
    element instanceof HTMLElement &&
    !isEffortPickerTrigger(element) &&
    isPopupOption(element) &&
    element.closest(popupRootSelector) !== null &&
    hasToken(accessibleName(element), ["standard", "extended"]);

  const elementPathname = (element) => {
    if (!(element instanceof HTMLElement)) {
      return "";
    }
    const href =
      element.getAttribute("href") ??
      (element instanceof HTMLAnchorElement ? element.href : "");
    if (!href) {
      return "";
    }
    try {
      return new URL(href, window.location.href).pathname.toLowerCase();
    } catch {
      return "";
    }
  };

  const pathnameHasSegment = (pathname, segment) =>
    pathname.split("/").filter(Boolean).includes(segment);

  const isSearchModeControl = (element) =>
    element instanceof HTMLElement &&
    (element.matches("[data-mode='search']") ||
      element.matches("button[aria-label*='Search']") ||
      element.matches("button[aria-label*='web']") ||
      hasToken(accessibleName(element), ["search", "browse the web"]));

  const isResearchModeControl = (element) => {
    if (!(element instanceof HTMLElement)) {
      return false;
    }
    const pathname = elementPathname(element);
    return (
      element.getAttribute("data-testid") === "deep-research-sidebar-item" ||
      (element.tagName.toLowerCase() === "a" &&
        pathnameHasSegment(pathname, "deep-research"))
    );
  };

  const discoverVisibleAliases = ({ keys, exclude }) => {
    const discovered = {};
    for (const element of actionableElements()) {
      if (exclude?.(element)) {
        continue;
      }
      const name = accessibleName(element);
      if (!name) {
        continue;
      }
      for (const [key, tokens] of Object.entries(keys)) {
        if (hasToken(name, tokens)) {
          const current = discovered[key] ?? [];
          discovered[key] = unique([...current, name]);
        }
      }
    }
    return discovered;
  };

  const discoverVisibleModelAliases = () =>
    discoverVisibleAliases({
      keys: {
        instant: ["instant"],
        thinking: ["thinking"],
      },
      exclude: (element) =>
        isModelPickerTrigger(element) || !isModelOption(element),
    });

  const discoverVisibleEffortAliases = () =>
    discoverVisibleAliases({
      keys: {
        standard: ["standard"],
        extended: ["extended"],
        none: ["none", "auto"],
      },
      exclude: (element) =>
        isEffortPickerTrigger(element) || !isEffortOption(element),
    });

  const ensurePickerOpen = async (trigger) => {
    if (!(trigger instanceof HTMLElement)) {
      return [];
    }
    const resolveRoots = () => {
      const roots = [];
      const controlledId = trigger.getAttribute("aria-controls");
      if (controlledId) {
        const controlled = document.getElementById(controlledId);
        if (controlled instanceof HTMLElement && isVisible(controlled)) {
          roots.push(controlled);
        }
      }
      for (const root of visibleUnique(
        document.querySelectorAll(popupRootSelector),
      )) {
        if (
          !roots.includes(root) &&
          visibleUnique(root.querySelectorAll(menuOptionSelector)).some(
            (element) => isModelOption(element) || isEffortOption(element),
          )
        ) {
          roots.push(root);
        }
      }
      return roots;
    };
    const initialRoots = resolveRoots();
    if (initialRoots.length > 0) {
      return initialRoots;
    }
    if (!clickControl(trigger)) {
      return [];
    }
    const deadline = Date.now() + 1500;
    while (Date.now() < deadline) {
      const roots = resolveRoots();
      if (roots.length > 0) {
        return roots;
      }
      await sleep(50);
    }
    return resolveRoots();
  };

  const findModelPickerTrigger = (aliases, fallbackTokens) =>
    findByAliases({
      selectors: [
        "button[aria-label='Model selector']",
        "button[aria-label='Switch model']",
        "button[data-testid='model-switcher-dropdown-button']",
      ],
      aliases: unique(["chatgpt", ...aliases]),
      fallbackTokens: unique([
        "chatgpt",
        "model",
        "selector",
        "engine",
        "gpt",
        ...fallbackTokens,
      ]),
      include: isModelPickerTrigger,
      exclude: (element) => !isModelPickerTrigger(element),
    });

  const findEffortPickerTrigger = (aliases, fallbackTokens) =>
    findComposerAdjacentControl({
      selectors: [
        "button.__composer-pill[aria-haspopup]",
        "button[aria-haspopup][aria-label*='Thinking']",
        "button[aria-haspopup][aria-label*='thinking']",
      ],
      aliases,
      fallbackTokens: unique([
        "effort",
        "reasoning",
        "thinking",
        ...fallbackTokens,
      ]),
      include: isEffortPickerTrigger,
      requirePopup: true,
      exclude: (element) => !isEffortPickerTrigger(element),
    });

  const visibleRoots = (selectors) =>
    visibleUnique(
      selectors.flatMap((selector) =>
        Array.from(document.querySelectorAll(selector)),
      ),
    );

  const queryFirstVisibleInRoots = (roots, selectors) => {
    for (const root of roots) {
      for (const selector of selectors) {
        const match = firstVisible(root.querySelectorAll(selector));
        if (match) {
          return match;
        }
      }
    }
    return null;
  };

  const findByAliasesInRoots = ({
    roots,
    selectors = [],
    aliases = [],
    fallbackTokens = [],
    include,
    exclude,
  }) => {
    const selectorMatch = queryFirstVisibleInRoots(roots, selectors);
    if (
      selectorMatch &&
      !exclude?.(selectorMatch) &&
      (!include || include(selectorMatch))
    ) {
      return selectorMatch;
    }

    const normalizedAliases = unique(aliases);
    for (const root of roots) {
      for (const element of visibleUnique(
        root.querySelectorAll(actionableSelector),
      )) {
        if (exclude?.(element) || (include && !include(element))) {
          continue;
        }
        const name = accessibleName(element);
        if (!name) {
          continue;
        }
        if (
          normalizedAliases.some(
            (alias) =>
              name === alias || name.includes(alias) || alias.includes(name),
          )
        ) {
          return element;
        }
      }
    }

    for (const root of roots) {
      for (const element of visibleUnique(
        root.querySelectorAll(actionableSelector),
      )) {
        if (exclude?.(element) || (include && !include(element))) {
          continue;
        }
        if (hasToken(accessibleName(element), fallbackTokens)) {
          return element;
        }
      }
    }

    return null;
  };

  const openPopupRootsFor = async (trigger, include) => {
    const roots = await ensurePickerOpen(trigger);
    return roots.filter((root) =>
      visibleUnique(root.querySelectorAll(menuOptionSelector)).some((element) =>
        include(element),
      ),
    );
  };

  const visiblePopupRootsFor = (include, trigger = null) => {
    const roots = [];
    if (trigger instanceof HTMLElement) {
      const controlledId = trigger.getAttribute("aria-controls");
      if (controlledId) {
        const controlled = document.getElementById(controlledId);
        if (controlled instanceof HTMLElement && isVisible(controlled)) {
          roots.push(controlled);
        }
      }
    }
    for (const root of visibleUnique(
      document.querySelectorAll(popupRootSelector),
    )) {
      if (
        !roots.includes(root) &&
        visibleUnique(root.querySelectorAll(menuOptionSelector)).some(
          (element) => include(element),
        )
      ) {
        roots.push(root);
      }
    }
    return roots;
  };

  const findConcreteModelControl = (report, modelClass, roots = []) =>
    roots.length > 0
      ? findByAliasesInRoots({
          roots,
          aliases: expandAliases(
            report,
            "model_classes",
            modelClass,
            modelDefaults(modelClass),
          ),
          fallbackTokens: modelFallbackTokens(modelClass),
          include: isModelOption,
          exclude: isModelPickerTrigger,
        })
      : findByAliases({
          aliases: expandAliases(
            report,
            "model_classes",
            modelClass,
            modelDefaults(modelClass),
          ),
          fallbackTokens: modelFallbackTokens(modelClass),
          include: isModelOption,
          exclude: isModelPickerTrigger,
        });

  const findOpenModelMenuControl = (report, modelClass) =>
    findByAliasesInRoots({
      roots: visiblePopupRootsFor(isModelOption),
      aliases: expandAliases(
        report,
        "model_classes",
        modelClass,
        modelDefaults(modelClass),
      ),
      fallbackTokens: modelFallbackTokens(modelClass),
      include: isModelOption,
      exclude: isModelPickerTrigger,
    });

  const findConcreteEffortControl = (report, effort, roots = []) =>
    roots.length > 0
      ? findByAliasesInRoots({
          roots,
          aliases: expandAliases(
            report,
            "efforts",
            effort,
            effortDefaults(effort),
          ),
          fallbackTokens: effortFallbackTokens(effort),
          include: isEffortOption,
          exclude: isEffortPickerTrigger,
        })
      : findByAliases({
          aliases: expandAliases(
            report,
            "efforts",
            effort,
            effortDefaults(effort),
          ),
          fallbackTokens: effortFallbackTokens(effort),
          include: isEffortOption,
          exclude: isEffortPickerTrigger,
        });

  const findOpenEffortMenuControl = (report, effort) =>
    findByAliasesInRoots({
      roots: visiblePopupRootsFor(isEffortOption),
      aliases: expandAliases(report, "efforts", effort, effortDefaults(effort)),
      fallbackTokens: effortFallbackTokens(effort),
      include: isEffortOption,
      exclude: isEffortPickerTrigger,
    });

  const ensureMode = (report, mode) => {
    if (mode === "normal") {
      return;
    }

    const capabilityVisible =
      mode === "search"
        ? report.matrix.search_mode_visible
        : report.matrix.research_mode_visible;
    if (!capabilityVisible) {
      throwCapabilityMissing(`${mode} mode`, mode, report.aliases.modes);
    }

    const aliases = expandAliases(report, "modes", mode, [
      mode,
      mode === "search" ? "browse the web" : "deep research",
    ]);
    const control = findByAliases({
      selectors:
        mode === "search"
          ? [
              "[data-mode='search']",
              "button[aria-label*='Search']",
              "button[aria-label*='web']",
            ]
          : [
              "a[data-testid='deep-research-sidebar-item']",
              "a[href='/deep-research']",
              "a[href$='/deep-research']",
            ],
      aliases,
      fallbackTokens:
        mode === "search" ? ["search", "browse", "web"] : ["deep research"],
      include: mode === "search" ? isSearchModeControl : isResearchModeControl,
    });
    if (!control) {
      throwUiDrift(`could not find ${mode} mode control after probing it`);
    }
    if (!isSelected(control) && !clickControl(control)) {
      throwUiDrift(`could not activate ${mode} mode control`);
    }
  };

  const ensureModelClass = async (report, modelClass) => {
    if (!report.matrix.model_picker_visible) {
      throwCapabilityMissing(
        "model picker",
        modelClass,
        report.aliases.model_classes,
      );
    }
    let control = findConcreteModelControl(report, modelClass);
    if (!control) {
      const trigger = findModelPickerTrigger(
        modelDefaults(modelClass),
        modelFallbackTokens(modelClass),
      );
      if (trigger) {
        const roots = await openPopupRootsFor(trigger, isModelOption);
        control =
          findConcreteModelControl(report, modelClass, roots) ??
          findOpenModelMenuControl(report, modelClass) ??
          findConcreteModelControl(report, modelClass);
      }
    }
    if (!control) {
      const discoveredAliases = mergeAliasBuckets(
        report.aliases.model_classes,
        discoverVisibleModelAliases(),
      );
      if ((discoveredAliases[modelClass] ?? []).length === 0) {
        throwCapabilityMissing("model class", modelClass, discoveredAliases);
      }
      throwUiDrift(
        `could not find a concrete control for model=${modelClass} after alias discovery`,
      );
    }
    if (!isSelected(control) && !clickControl(control)) {
      throwUiDrift(`could not activate model=${modelClass}`);
    }
    await sleep(150);
  };

  const ensureEffort = async (report, effort) => {
    if (effort === "none") {
      return;
    }
    let control = findConcreteEffortControl(report, effort);
    if (!control) {
      const trigger = findEffortPickerTrigger(
        effortDefaults(effort),
        effortFallbackTokens(effort),
      );
      if (trigger) {
        const roots = await openPopupRootsFor(trigger, isEffortOption);
        control =
          findConcreteEffortControl(report, effort, roots) ??
          findOpenEffortMenuControl(report, effort) ??
          findConcreteEffortControl(report, effort);
      }
    }
    if (!control) {
      const discoveredAliases = mergeAliasBuckets(
        report.aliases.efforts,
        discoverVisibleEffortAliases(),
      );
      if ((discoveredAliases[effort] ?? []).length === 0) {
        throwCapabilityMissing("thinking effort", effort, discoveredAliases);
      }
      throwUiDrift(
        `could not find a concrete control for effort=${effort} after alias discovery`,
      );
    }
    if (!isSelected(control) && !clickControl(control)) {
      throwUiDrift(`could not activate effort=${effort}`);
    }
    await sleep(150);
  };

  const uploadAcknowledged = ({ names }) => {
    const expected = unique(names);
    const fromDataset = unique(
      (document.body.dataset.uploadedFiles ?? "").split("|"),
    );
    if (expected.every((name) => fromDataset.includes(normalizeText(name)))) {
      return true;
    }

    const candidates = unique(
      Array.from(
        document.querySelectorAll(
          [
            "[data-slot='uploaded-file']",
            "[data-slot='upload-entry-name']",
            "[data-testid='upload-chip']",
            "[data-testid*='attachment']",
            "[aria-label*='Attached file']",
            "[aria-label*='attachment']",
            "button",
            "span",
            "div",
            "li",
          ].join(", "),
        ),
      )
        .filter(isVisible)
        .flatMap((element) => [
          accessibleName(element),
          normalizeText(element.textContent),
        ]),
    );

    return expected.every(
      (name) =>
        candidates.includes(normalizeText(name)) ||
        candidates.some((candidate) => candidate.includes(normalizeText(name))),
    );
  };

  const textUtils = createTextUtilsModule({
    normalizeText,
    collapseText,
    unique,
    uniqueCollapsed,
    datasetValue,
    hasToken,
    collapseInlineText,
    normalizeInlineOutput,
    indentLines,
  });
  const domUtils = createDomUtilsModule({
    getComputed,
    isVisible,
    firstVisible,
    accessibleName,
    queryFirstVisible,
    queryVisible,
    actionableSelector,
    actionableElements,
    latestVisible,
    visibleUnique,
    isSelected,
    isDisabled,
    findByAliases,
    clickControl,
  });
  const composerRuntime = createComposerRuntimeModule({
    findComposer,
    composerContainers,
    isComposerAdjacent,
    findComposerAdjacentControl,
    readComposerText,
    composerDisabled,
    findSubmitButton,
    readSubmitControlState,
    keyboardSubmitReady,
    submitButtonEnabled,
    submitReady,
    stopButtonVisible,
    readSubmitState,
    submitStarted,
    attachmentsStillProcessing,
    setComposerValue,
    uploadAcknowledged,
  });
  const artifactRuntime = createArtifactRuntimeModule({
    isReasoningArtifactMessage,
    isInsideReasoningArtifactMessage,
    isMermaidPreviewBlock,
    serializeMermaidPreviewBlock,
    collectMermaidSvgLabels,
  });
  const messageRuntime = createMessageRuntimeModule({
    artifactRuntime,
    findAssistantMessages,
    findLatestAssistantMessage,
    resolveAskArticle,
    readAskFailureState,
    inferAskState,
    readMessageIdentity,
    collectText,
    thoughtDividerBeforeMessage,
  });
  const capabilityRuntime = createCapabilityRuntimeModule({
    expandAliases,
    controlStateDetail,
    throwCapabilityMissing,
    throwUiDrift,
    mergeAliasBuckets,
    ensureMode,
    ensureModelClass,
    ensureEffort,
    discoverVisibleModelAliases,
    discoverVisibleEffortAliases,
  });
  const conversationRuntime = createConversationRuntimeModule({
    parseConversationIdFromUrl,
    currentPathname,
    readResearchStatusMessage,
    onDeepResearchLanding,
    normalizeResearchReportText,
    findResearchReportArticle,
    inferResearchStatus,
    readResearchSubmissionState,
    researchSubmissionStarted,
  });

  return createCommandRuntime({
    textUtils,
    domUtils,
    composerRuntime,
    messageRuntime,
    capabilityRuntime,
    conversationRuntime,
  });
}
