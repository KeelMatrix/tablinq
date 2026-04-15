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

  const actionableElements = () =>
    Array.from(
      document.querySelectorAll(
        [
          "button",
          "[role='button']",
          "[role='tab']",
          "[role='switch']",
          "[role='option']",
          "[aria-haspopup]",
          "label[for]",
          "a",
        ].join(", "),
      ),
    ).filter(isVisible);

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
  }) => {
    const selectorMatch = queryFirstVisible(selectors);
    if (selectorMatch) {
      return selectorMatch;
    }

    const normalizedAliases = unique(aliases);
    for (const element of actionableElements()) {
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
      const fromQuery = url.searchParams.get("conversation");
      if (fromQuery) {
        return fromQuery;
      }
      const match = url.pathname.match(/\/c\/([^/?#]+)/i);
      return match?.[1] ?? null;
    } catch {
      return null;
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

  const findSubmitButton = () => {
    const explicit = queryFirstVisible([
      "[data-action='submit']",
      "[data-testid='send-button']",
      "button[aria-label='Submit prompt']",
      "button[aria-label*='Send']",
      "button[type='submit']",
    ]);
    if (explicit) {
      return explicit;
    }

    const composer = findComposer();
    for (const container of [
      composer?.closest("form"),
      composer?.parentElement,
      composer?.closest("section"),
      composer?.closest("main"),
    ]) {
      if (!container) {
        continue;
      }
      const nearby = firstVisible(
        container.querySelectorAll(
          "button, [role='button'], input[type='submit']",
        ),
      );
      if (nearby) {
        return nearby;
      }
    }

    return findByAliases({
      fallbackTokens: ["send", "submit", "ask"],
    });
  };

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

  const collectText = (root) => {
    if (!(root instanceof HTMLElement)) {
      return "";
    }
    const lines = uniqueCollapsed(
      Array.from(
        root.querySelectorAll(
          [
            "[data-message-content]",
            ".markdown",
            "p",
            "pre",
            "code",
            "li",
            "div",
          ].join(", "),
        ),
      ).map((node) => node.textContent),
    );

    if (lines.length > 0) {
      return lines.join("\n");
    }

    const directText = uniqueCollapsed(
      Array.from(root.childNodes)
        .filter((node) => node.nodeType === Node.TEXT_NODE)
        .map((node) => node.textContent),
    );
    return directText.join("\n");
  };

  const findLatestAssistantMessage = () =>
    latestVisible([
      "[data-slot='assistant-message']",
      "[data-message-author-role='assistant']",
      "article.assistant",
      "main article",
      ".message",
    ]);

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

  const inferResearchStatus = (text) => {
    const explicitState = document.body.dataset.researchState;
    if (explicitState) {
      return explicitState;
    }

    const statusText = normalizeText(
      document.body.dataset.researchStatusMessage ??
        queryFirstVisible([
          "[data-slot='research-status']",
          "[data-testid='research-status']",
          "[aria-live='polite']",
        ])?.textContent,
    );

    if (
      hasToken(statusText, ["failed", "error", "retry"]) &&
      !hasToken(statusText, ["running"])
    ) {
      return "failed";
    }
    if (hasToken(statusText, ["source", "sources", "gathering", "finding"])) {
      return "waiting_for_sources";
    }
    if (stopButtonVisible()) {
      return "running";
    }
    if (text) {
      return "done";
    }
    return "running";
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
    const aliases = expandAliases(report, "model_classes", modelClass, [
      modelClass,
      modelClass === "instant" ? "gpt-5.3 instant" : "gpt-5.4 thinking",
      modelClass === "instant" ? "instant answers" : "deep thinking",
    ]);
    if (aliases.length === 0) {
      throwCapabilityMissing(
        "model class",
        modelClass,
        report.aliases.model_classes,
      );
    }
    const control = findByAliases({
      selectors:
        modelClass === "instant"
          ? ["[data-model='instant']", "button[aria-label*='Instant']"]
          : ["[data-model='thinking']", "button[aria-label*='Thinking']"],
      aliases,
      fallbackTokens:
        modelClass === "instant"
          ? ["instant", "fast", "quick"]
          : ["thinking", "reason", "deep"],
    });
    if (!control) {
      throwUiDrift(
        `could not find a control for model=${modelClass} after alias discovery`,
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
    if (!report.matrix.thinking_effort_visible) {
      throwCapabilityMissing("thinking effort", effort, report.aliases.efforts);
    }
    const aliases = expandAliases(report, "efforts", effort, [
      effort,
      effort === "standard" ? "standard reasoning" : "extended reasoning",
    ]);
    if (aliases.length === 0) {
      throwCapabilityMissing("thinking effort", effort, report.aliases.efforts);
    }
    const control = findByAliases({
      selectors:
        effort === "standard"
          ? ["[data-effort='standard']", "button[aria-label*='Standard']"]
          : ["[data-effort='extended']", "button[aria-label*='Extended']"],
      aliases,
      fallbackTokens:
        effort === "standard"
          ? ["standard", "default"]
          : ["extended", "longer", "heavy"],
    });
    if (!control) {
      throwUiDrift(
        `could not find a control for effort=${effort} after alias discovery`,
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
    readAskSnapshot() {
      const article = findLatestAssistantMessage();
      const text = article ? collectText(article) : "";
      return {
        text,
        state: inferAskState(text, article),
        conversationId:
          datasetValue(document.body.dataset.conversationId) ??
          parseConversationIdFromUrl(),
        messageId:
          datasetValue(article?.getAttribute("data-message-id")) ??
          datasetValue(document.body.dataset.lastMessageId) ??
          null,
        currentUrl: window.location.href,
      };
    },
    readResearchSnapshot() {
      const article =
        latestVisible([
          "[data-slot='research-report']",
          "[data-message-author-role='assistant']",
          "[data-slot='assistant-message']",
          ".message",
        ]) ?? findLatestAssistantMessage();
      const reportText = article ? collectText(article) : "";
      return {
        status: inferResearchStatus(reportText),
        statusMessage:
          document.body.dataset.researchStatusMessage ??
          normalizeText(
            queryFirstVisible([
              "[data-slot='research-status']",
              "[data-testid='research-status']",
              "[aria-live='polite']",
            ])?.textContent,
          ) ??
          null,
        reportText,
        conversationId:
          datasetValue(document.body.dataset.conversationId) ??
          parseConversationIdFromUrl(),
        messageId:
          datasetValue(article?.getAttribute("data-message-id")) ??
          datasetValue(document.body.dataset.lastMessageId) ??
          null,
        currentUrl: window.location.href,
      };
    },
  };
}
