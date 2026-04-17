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
    element.click();
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
    const explicit = queryFirstVisible([
      "[data-action='submit']",
      "[data-testid='send-button']",
      "button[aria-label='Submit prompt']",
      "button[aria-label*='Send']",
      "input[type='submit']",
    ]);
    if (
      explicit &&
      submitCandidateScore(explicit, { allowDisabled }) > 0
    ) {
      return explicit;
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

  const collectText = (root) => {
    if (!(root instanceof HTMLElement)) {
      return "";
    }
    const blockSelector = [
      "[data-message-content]",
      "p",
      "pre",
      "li",
      "blockquote",
      "h1",
      "h2",
      "h3",
      "h4",
      "h5",
      "h6",
    ].join(", ");
    const leafBlocks = Array.from(root.querySelectorAll(blockSelector)).filter(
      (node) =>
        !Array.from(node.querySelectorAll(blockSelector)).some(
          (child) => child !== node,
        ),
    );
    const lines = uniqueCollapsed(leafBlocks.map((node) => node.textContent));

    if (lines.length > 0) {
      return lines.join("\n\n");
    }

    const directText = uniqueCollapsed(
      Array.from(root.childNodes)
        .filter((node) => node.nodeType === Node.TEXT_NODE)
        .map((node) => node.textContent),
    );
    return directText.join("\n");
  };

  const findAssistantMessages = () => {
    const direct = visibleUnique(
      document.querySelectorAll(
        [
          "[data-slot='assistant-message']",
          "[data-message-author-role='assistant']",
          "article.assistant",
        ].join(", "),
      ),
    );
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
        ].join(", "),
      ),
    );
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
    const latestMessageId = datasetValue(
      latest?.getAttribute("data-message-id"),
    );
    const latestText = latest ? collectText(latest) : "";
    const streamState =
      latest?.getAttribute("data-stream-state") ??
      document.body.dataset.streamState;

    if (messages.length > Math.max(0, baseline.assistantMessageCount ?? 0)) {
      return latest;
    }
    if (
      latest &&
      latestMessageId &&
      latestMessageId !== baseline.messageId
    ) {
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

  const inferAskState = (text, article) => {
    const explicitState =
      article?.getAttribute("data-stream-state") ??
      document.body.dataset.streamState;
    if (explicitState) {
      return explicitState;
    }
    if (stopButtonVisible()) {
      return "running";
    }
    if (text) {
      return "done";
    }
    return "idle";
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

  const inferResearchStatus = (text, statusMessage = readResearchStatusMessage()) => {
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
      datasetValue(article?.getAttribute("data-message-id")) ??
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

  const researchSubmissionStarted = () => readResearchSubmissionState().submitted;

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
    modelClass === "instant" ? "instant answers" : "deep thinking",
    modelClass === "instant" ? "gpt-5.3 instant" : "gpt-5.4 thinking",
  ];

  const modelFallbackTokens = (modelClass) =>
    modelClass === "instant" ? ["instant"] : ["thinking"];

  const effortDefaults = (effort) => [
    effort,
    effort === "standard" ? "standard reasoning" : "extended thinking",
    effort === "standard" ? "default thinking" : "extended reasoning",
  ];

  const effortFallbackTokens = (effort) =>
    effort === "standard" ? ["standard"] : ["extended"];

  const isModelPickerTrigger = (element) =>
    element instanceof HTMLElement &&
    !element.hasAttribute("data-model") &&
    (element.matches("[data-slot='model-picker']") ||
      element.getAttribute("aria-haspopup") !== null) &&
    hasToken(accessibleName(element), [
      "chatgpt",
      "model",
      "selector",
      "engine",
      "gpt",
      "latest",
      "instant",
      "thinking",
      "pro",
    ]);

  const isEffortPickerTrigger = (element) =>
    element instanceof HTMLElement &&
    !element.hasAttribute("data-effort") &&
    (element.matches("[data-slot='effort-toggle']") ||
      (element.getAttribute("aria-haspopup") !== null &&
        isComposerAdjacent(element))) &&
    hasToken(accessibleName(element), [
      "effort",
      "reasoning",
      "thinking",
      "standard",
      "extended",
    ]);

  const isModelOption = (element) =>
    element instanceof HTMLElement &&
    !isModelPickerTrigger(element) &&
    (element.hasAttribute("data-model") ||
      element.closest("[data-slot='model-menu']") !== null) &&
    hasToken(accessibleName(element), [
      "latest",
      "instant",
      "thinking",
      "pro",
      "gpt",
    ]);

  const isEffortOption = (element) =>
    element instanceof HTMLElement &&
    !isEffortPickerTrigger(element) &&
    (element.hasAttribute("data-effort") ||
      element.closest("[data-slot='effort-menu']") !== null) &&
    hasToken(accessibleName(element), [
      "standard",
      "extended",
      "reasoning",
      "thinking",
    ]);

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
      exclude: (element) => isModelPickerTrigger(element) || !isModelOption(element),
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

  const ensurePickerOpen = (trigger) => {
    if (!(trigger instanceof HTMLElement)) {
      return false;
    }
    if (trigger.getAttribute("aria-expanded") === "true") {
      return true;
    }
    return clickControl(trigger);
  };

  const findModelPickerTrigger = (aliases, fallbackTokens) =>
    findByAliases({
      selectors: [
        "[data-slot='model-picker']",
        "[data-slot='model-picker'][aria-haspopup]",
        "[data-slot='model-picker'] [aria-haspopup]",
        "button[aria-haspopup][aria-label*='ChatGPT']",
        "button[aria-haspopup][aria-label*='chatgpt']",
        "button[aria-haspopup][aria-label*='Model']",
        "button[aria-haspopup][aria-label*='model']",
        "button[aria-haspopup][aria-label*='Engine']",
        "button[aria-haspopup][aria-label*='engine']",
        "button[aria-haspopup][aria-label*='GPT']",
        "button[aria-haspopup][aria-label*='gpt']",
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
        "[data-slot='effort-toggle']",
        "[data-slot='effort-toggle'][aria-haspopup]",
        "[data-slot='effort-toggle'] [aria-haspopup]",
        "button[aria-haspopup][aria-label*='Effort']",
        "button[aria-haspopup][aria-label*='effort']",
        "button[aria-haspopup][aria-label*='Reasoning']",
        "button[aria-haspopup][aria-label*='reasoning']",
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
      selectors.flatMap((selector) => Array.from(document.querySelectorAll(selector))),
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
      for (const element of visibleUnique(root.querySelectorAll(actionableSelector))) {
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

  const findConcreteModelControl = (report, modelClass) =>
    findByAliases({
      selectors:
        modelClass === "instant"
          ? [
              "[data-model='instant']",
              "[role='menuitemradio'][data-model='instant']",
              "button[aria-label*='Instant']",
            ]
          : [
              "[data-model='thinking']",
              "[role='menuitemradio'][data-model='thinking']",
              "button[aria-label*='Thinking']",
            ],
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
    findByAliases({
      selectors:
        modelClass === "instant"
          ? [
              "[role='menuitemradio'][data-model='instant']",
              "[role='menuitem'][data-model='instant']",
              "[data-model='instant']",
              "button[aria-label*='Instant']",
            ]
          : [
              "[role='menuitemradio'][data-model='thinking']",
              "[role='menuitem'][data-model='thinking']",
              "[data-model='thinking']",
              "button[aria-label*='Thinking']",
            ],
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

  const findConcreteEffortControl = (report, effort) =>
    findByAliases({
      selectors:
        effort === "standard"
          ? [
              "[data-effort='standard']",
              "[role='menuitemradio'][data-effort='standard']",
              "button[aria-label*='Standard']",
            ]
          : [
              "[data-effort='extended']",
              "[role='menuitemradio'][data-effort='extended']",
              "button[aria-label*='Extended']",
            ],
      aliases: expandAliases(report, "efforts", effort, effortDefaults(effort)),
      fallbackTokens: effortFallbackTokens(effort),
      include: isEffortOption,
      exclude: isEffortPickerTrigger,
    });

  const findOpenEffortMenuControl = (report, effort) =>
    findByAliases({
      selectors:
        effort === "standard"
          ? [
              "[role='menuitemradio'][data-effort='standard']",
              "[role='menuitem'][data-effort='standard']",
              "[data-effort='standard']",
              "button[aria-label*='Standard']",
            ]
          : [
              "[role='menuitemradio'][data-effort='extended']",
              "[role='menuitem'][data-effort='extended']",
              "[data-effort='extended']",
              "button[aria-label*='Extended']",
            ],
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
              "[data-mode='research']",
              "button[aria-label*='Research']",
              "button[aria-label*='Deep research']",
            ],
      aliases,
      fallbackTokens:
        mode === "search" ? ["search", "browse", "web"] : ["research", "deep"],
    });
    if (!control) {
      throwUiDrift(`could not find ${mode} mode control after probing it`);
    }
    if (!isSelected(control) && !clickControl(control)) {
      throwUiDrift(`could not activate ${mode} mode control`);
    }
  };

  const ensureModelClass = (report, modelClass) => {
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
      if (trigger && ensurePickerOpen(trigger)) {
        control =
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
  };

  const ensureEffort = (report, effort) => {
    if (effort === "none") {
      return;
    }
    let control = findConcreteEffortControl(report, effort);
    if (!control) {
      const trigger = findEffortPickerTrigger(
        effortDefaults(effort),
        effortFallbackTokens(effort),
      );
      if (trigger && ensurePickerOpen(trigger)) {
        control =
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
  };

  return {
    composerReady() {
      return Boolean(findComposer());
    },
    fillComposer({ prompt }) {
      setComposerValue(findComposer(), prompt);
      return true;
    },
    clickSubmit() {
      const button = findSubmitButton();
      if (button && clickControl(button)) {
        return true;
      }

      const composer = findComposer();
      if (!(composer instanceof HTMLElement)) {
        return false;
      }
      composer.dispatchEvent(
        new KeyboardEvent("keydown", {
          bubbles: true,
          key: "Enter",
          ctrlKey: true,
        }),
      );
      composer.dispatchEvent(
        new KeyboardEvent("keydown", {
          bubbles: true,
          key: "Enter",
          metaKey: true,
        }),
      );
      return false;
    },
    applyAskConfiguration({ report, request }) {
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
        ensureModelClass(report, request.modelClass);
      }
      if (request.effort) {
        ensureEffort(report, request.effort);
      }
      if (request.mode) {
        ensureMode(report, request.mode);
      }
      return true;
    },
    uploadAcknowledged({ names }) {
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
          candidates.some((candidate) =>
            candidate.includes(normalizeText(name)),
          ),
      );
    },
    readAskBaseline() {
      const article = findLatestAssistantMessage();
      const messages = findAssistantMessages();
      return {
        assistantMessageCount: messages.length,
        messageId: datasetValue(article?.getAttribute("data-message-id")) ?? null,
        text: article ? collectText(article) : "",
      };
    },
    readAskSnapshot({ baseline } = {}) {
      const article = resolveAskArticle(baseline ?? null);
      const text = article ? collectText(article) : "";
      return {
        hasAssistantMessage: Boolean(article),
        text,
        state: inferAskState(text, article),
        conversationId:
          datasetValue(document.body.dataset.conversationId) ??
          parseConversationIdFromUrl(),
        messageId:
          datasetValue(article?.getAttribute("data-message-id")) ??
          (article ? datasetValue(document.body.dataset.lastMessageId) : null) ??
          null,
        currentUrl: window.location.href,
      };
    },
    readResearchSnapshot() {
      const submission = readResearchSubmissionState();
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
      return readResearchSubmissionState();
    },
    researchSubmissionStarted() {
      return researchSubmissionStarted();
    },
    readSubmitState() {
      return readSubmitState();
    },
    submitReady() {
      return submitReady();
    },
    attachmentsStillProcessing() {
      return attachmentsStillProcessing();
    },
    submitStarted(payload) {
      return submitStarted(payload ?? {});
    },
  };
}
