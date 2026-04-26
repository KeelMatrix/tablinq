function createComposerRuntimeModule({ textUtils, domUtils, getMessageRuntime }) {
  const {
    normalizeText,
    collapseText,
    unique,
    hasToken,
  } = textUtils;
  const {
    isVisible,
    firstVisible,
    accessibleName,
    queryFirstVisible,
    queryVisible,
    actionableSelector,
    visibleUnique,
    isDisabled,
    findByAliases,
  } = domUtils;
  const findAssistantMessages = () =>
    getMessageRuntime()?.findAssistantMessages?.() ?? [];
  const findLatestAssistantMessage = () =>
    getMessageRuntime()?.findLatestAssistantMessage?.() ?? null;

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

  return {
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
  };
}
