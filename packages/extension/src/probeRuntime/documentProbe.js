import {
  accessibleProbeName,
  createProbeNeighborhood,
  firstVisibleProbeElement,
  implicitProbeRole,
  isVisibleProbeElement,
  normalizeProbeText,
  probeTextHasToken,
} from "./text.js";
import {
  controlCandidateSelector,
  orderCapabilityStrategies,
} from "./strategies.js";
import { captureProbeAliases, createAliasBuckets } from "./aliases.js";

export function probeDocument(options = {}) {
  const selectorPackId = options.selectorPackId ?? "chatgpt-default";
  const selectorPackVersion = options.selectorPackVersion ?? "v1";
  const reportedAt = options.reportedAt ?? new Date().toISOString();
  const preferredStrategies = options.preferredStrategies ?? {};
  const selectorPack = options.selectorPack ?? null;

  const documentRoot = document;
  const body = documentRoot.body;
  const normalizeText = normalizeProbeText;
  const isVisible = isVisibleProbeElement;
  const firstVisible = (elements) =>
    firstVisibleProbeElement(elements, isVisible);
  const implicitRole = implicitProbeRole;
  const accessibleName = (element) =>
    accessibleProbeName(element, documentRoot);
  const hasToken = probeTextHasToken;

  const firstVisibleBySelector = (selector) =>
    firstVisible(documentRoot.querySelectorAll(selector));

  const firstVisibleBySelectors = (selectors) => {
    for (const selector of selectors) {
      const match = firstVisibleBySelector(selector);
      if (match) {
        return match;
      }
    }
    return null;
  };

  const findNamedControl = (tokens, options = {}) => {
    const { include, includeHidden = false, roots = [documentRoot] } = options;
    for (const root of roots) {
      const candidates =
        root instanceof Document || root instanceof Element
          ? root.querySelectorAll(controlCandidateSelector)
          : [];
      for (const candidate of Array.from(candidates)) {
        if (!includeHidden && !isVisible(candidate)) {
          continue;
        }
        if (include && !include(candidate)) {
          continue;
        }
        const name = accessibleName(candidate);
        if (hasToken(name, tokens)) {
          return candidate;
        }
      }
    }
    return null;
  };

  const findTextbox = () => {
    const candidates = documentRoot.querySelectorAll(
      [
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
      ].join(", "),
    );
    for (const candidate of Array.from(candidates)) {
      if (!isVisible(candidate)) {
        continue;
      }
      const name = accessibleName(candidate);
      if (
        candidate.getAttribute("data-slot") === "composer" ||
        hasToken(name, ["prompt", "message", "composer", "ask", "chat"])
      ) {
        return candidate;
      }
      if (
        candidate.tagName.toLowerCase() === "textarea" ||
        candidate.getAttribute("role") === "textbox" ||
        candidate.isContentEditable
      ) {
        return candidate;
      }
    }
    return null;
  };

  const composerContainers = () => {
    const composer = findTextbox();
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

  const findComposerAdjacentNamedControl = (tokens, options = {}) =>
    findNamedControl(tokens, {
      ...options,
      roots: composerContainers(),
    });

  const popupRootSelector = "[role='menu'], [role='listbox']";
  const popupOptionSelector =
    "[role='menuitemradio'], [role='menuitem'], [role='option']";
  const elementRole = (element) => element?.getAttribute?.("role") ?? "";
  const isPopupOption = (element) =>
    element instanceof HTMLElement &&
    ["menuitemradio", "menuitem", "option"].includes(elementRole(element));

  const nameLooksLikeComposerModelTrigger = (name) =>
    hasToken(name, [
      "instant",
      "legacy",
      "gpt",
      "pro",
      "standard",
      "extended",
    ]) ||
    /\b5(?:\.\d+)?\b/.test(name) ||
    (hasToken(name, ["thinking"]) &&
      !hasToken(name, ["standard", "extended", "effort"]));

  const isComposerRightModelPickerTrigger = (element) =>
    element instanceof HTMLElement &&
    element.matches("button.__composer-pill[aria-haspopup]") &&
    !element.hasAttribute("aria-label") &&
    isComposerAdjacent(element) &&
    nameLooksLikeComposerModelTrigger(accessibleName(element));

  const isModelPickerTrigger = (element) =>
    element instanceof HTMLElement &&
    element.getAttribute("aria-haspopup") !== null &&
    (element.matches("button[aria-label='Model selector']") ||
      element.matches("button[aria-label='Switch model']") ||
      element.matches("button[data-testid='model-switcher-dropdown-button']") ||
      isComposerRightModelPickerTrigger(element) ||
      hasToken(accessibleName(element), [
        "chatgpt",
        "model",
        "selector",
        "engine",
      ]));

  const isEffortPickerTrigger = (element) =>
    element instanceof HTMLElement &&
    element.getAttribute("aria-haspopup") !== null &&
    !isComposerRightModelPickerTrigger(element) &&
    (element.matches(
      "[data-testid='model-switcher-gpt-5-4-thinking-thinking-effort']",
    ) ||
      element.matches(
        "[data-testid^='model-switcher-gpt-'][data-testid$='-thinking-thinking-effort']",
      ) ||
      (isComposerAdjacent(element) &&
        element.matches(
          "#thinking-effort-trigger, button[aria-controls*='effort'], button[aria-label*='Thinking'], button[aria-label*='thinking'], button[aria-label*='Effort'], button[aria-label*='effort']",
        )) ||
      hasToken(accessibleName(element), [
        "thinking",
        "standard",
        "extended",
        "effort",
      ]));

  const isComposerToolsTrigger = (element) =>
    element instanceof HTMLElement &&
    element.getAttribute("aria-haspopup") !== null &&
    isComposerAdjacent(element) &&
    (element.matches("#composer-plus-btn") ||
      element.matches("[data-testid='composer-plus-btn']") ||
      hasToken(accessibleName(element), [
        "add files",
        "add photos",
        "tools",
        "more",
      ]));

  const isModelOption = (element) =>
    element instanceof HTMLElement &&
    !isModelPickerTrigger(element) &&
    ((element.hasAttribute("data-model") &&
      (element.getAttribute("aria-pressed") !== null ||
        element.tagName.toLowerCase() === "button")) ||
      (isPopupOption(element) &&
        element.closest(popupRootSelector) !== null)) &&
    hasToken(accessibleName(element), ["instant", "thinking", "pro"]);

  const isEffortOption = (element) =>
    element instanceof HTMLElement &&
    !isEffortPickerTrigger(element) &&
    !(element.getAttribute("data-testid") ?? "").startsWith(
      "model-switcher-",
    ) &&
    ((element.hasAttribute("data-effort") &&
      (element.getAttribute("aria-pressed") !== null ||
        element.tagName.toLowerCase() === "button")) ||
      (isPopupOption(element) &&
        element.closest(popupRootSelector) !== null)) &&
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

  const isSearchModeLabel = (name) =>
    (hasToken(name, ["search"]) || hasToken(name, ["web search"])) &&
    !name.includes("search chats");

  const isSearchChip = (element) =>
    element instanceof HTMLElement &&
    isComposerAdjacent(element) &&
    element.matches("button, [role='button']") &&
    isSearchModeLabel(accessibleName(element)) &&
    (accessibleName(element).includes("click to remove") ||
      element.classList.contains("__composer-pill") ||
      element.getAttribute("aria-pressed") === "true" ||
      element.dataset.selected === "true");

  const isSearchMenuItem = (element) =>
    element instanceof HTMLElement &&
    isPopupOption(element) &&
    element.closest(popupRootSelector) !== null &&
    isSearchModeLabel(accessibleName(element));

  const isLegacySearchModeControl = (element) =>
    element instanceof HTMLElement &&
    element.getAttribute("data-mode") === "search" &&
    isSearchModeLabel(accessibleName(element));

  const isSearchModeControl = (element) =>
    element instanceof HTMLElement &&
    (isSearchChip(element) ||
      isSearchMenuItem(element) ||
      isLegacySearchModeControl(element));

  const isResearchModeControl = (element) => {
    if (!(element instanceof HTMLElement)) {
      return false;
    }
    const pathname = elementPathname(element);
    return (
      element.matches("[data-mode='research']") ||
      element.getAttribute("data-testid") === "deep-research-sidebar-item" ||
      ((element.tagName.toLowerCase() === "button" ||
        element.getAttribute("role") === "button") &&
        hasToken(accessibleName(element), ["research", "deep research"])) ||
      (element.tagName.toLowerCase() === "a" &&
        pathnameHasSegment(pathname, "deep-research"))
    );
  };

  const visiblePopupRootsFor = (include, trigger = null) => {
    const roots = [];
    if (trigger instanceof HTMLElement) {
      const controlledId = trigger.getAttribute("aria-controls");
      if (controlledId) {
        const controlled = documentRoot.getElementById(controlledId);
        if (controlled instanceof HTMLElement && isVisible(controlled)) {
          roots.push(controlled);
        }
      }
    }
    for (const root of Array.from(
      documentRoot.querySelectorAll(popupRootSelector),
    )) {
      if (
        isVisible(root) &&
        !roots.includes(root) &&
        Array.from(root.querySelectorAll(popupOptionSelector)).some(
          (element) => isVisible(element) && include(element),
        )
      ) {
        roots.push(root);
      }
    }
    return roots;
  };

  const composerToolsMenuRootsFor = (include) => {
    const roots = [];
    for (const trigger of Array.from(
      documentRoot.querySelectorAll(controlCandidateSelector),
    )) {
      if (!isVisible(trigger) || !isComposerToolsTrigger(trigger)) {
        continue;
      }
      const controlledId = trigger.getAttribute("aria-controls");
      if (!controlledId) {
        continue;
      }
      const controlled = documentRoot.getElementById(controlledId);
      if (
        controlled instanceof HTMLElement &&
        !roots.includes(controlled) &&
        Array.from(controlled.querySelectorAll(popupOptionSelector)).some(
          (element) => include(element),
        )
      ) {
        roots.push(controlled);
      }
    }
    for (const root of visiblePopupRootsFor(include)) {
      if (!roots.includes(root)) {
        roots.push(root);
      }
    }
    return roots;
  };

  const findModelPickerTrigger = () =>
    firstVisibleBySelectors([
      "button[aria-label='Model selector']",
      "button[aria-label='Switch model']",
      "button[data-testid='model-switcher-dropdown-button']",
    ]) ??
    findNamedControl(["instant", "thinking", "legacy", "gpt", "5.3", "5.4"], {
      include: isComposerRightModelPickerTrigger,
    }) ??
    findNamedControl(["chatgpt", "model", "selector", "engine"], {
      include: isModelPickerTrigger,
    });

  const findEffortPickerTrigger = () =>
    firstVisibleBySelectors([
      "#thinking-effort-trigger",
      "button[aria-controls*='effort']",
      "[data-testid='model-switcher-gpt-5-4-thinking-thinking-effort']",
      "[data-testid^='model-switcher-gpt-'][data-testid$='-thinking-thinking-effort']",
      "button[aria-haspopup][aria-label*='Thinking']",
      "button[aria-haspopup][aria-label*='thinking']",
      "button[aria-haspopup][aria-label*='Effort']",
      "button[aria-haspopup][aria-label*='effort']",
    ]) ??
    findComposerAdjacentNamedControl(
      ["standard", "extended", "effort", "reasoning", "thinking"],
      {
        include: isEffortPickerTrigger,
      },
    );

  const discoverNamedControls = (tokens, options = {}) => {
    const { includeHidden = false, include } = options;
    const names = [];
    for (const candidate of Array.from(
      documentRoot.querySelectorAll(controlCandidateSelector),
    )) {
      if (!includeHidden && !isVisible(candidate)) {
        continue;
      }
      if (include && !include(candidate)) {
        continue;
      }
      const name = accessibleName(candidate);
      if (!name || !hasToken(name, tokens) || names.includes(name)) {
        continue;
      }
      names.push(name);
    }
    return names;
  };

  const findSubmitNearComposer = () => {
    const explicit = firstVisibleBySelectors([
      "[data-action='submit']",
      "[data-testid='send-button']",
      "button[aria-label='Submit prompt']",
      "button[aria-label*='Send']",
      "button[type='submit']",
    ]);
    if (explicit) {
      return explicit;
    }
    const composer = findTextbox();
    const containers = composer
      ? [
          composer.parentElement,
          composer.closest("form"),
          composer.closest("section"),
        ]
      : [];
    for (const container of containers) {
      if (!container) {
        continue;
      }
      const candidate = firstVisible(
        container.querySelectorAll(
          "button, [role='button'], input[type='submit']",
        ),
      );
      if (candidate) {
        return candidate;
      }
    }
    return null;
  };

  const findUploadLabel = () => {
    const labels = documentRoot.querySelectorAll(
      "label[for], button, [role='button']",
    );
    for (const label of Array.from(labels)) {
      if (!isVisible(label)) {
        continue;
      }
      const name = accessibleName(label);
      if (!hasToken(name, ["attach", "upload", "file", "files", "photos"])) {
        continue;
      }
      if (
        label.tagName.toLowerCase() === "label" &&
        label.getAttribute("for") &&
        documentRoot.getElementById(label.getAttribute("for")) instanceof
          HTMLInputElement
      ) {
        return label;
      }
      return label;
    }
    return null;
  };

  const createNeighborhood = (capability, strategyId, element) =>
    createProbeNeighborhood(capability, strategyId, element);

  const safeQueryAll = (root, selector) => {
    try {
      return Array.from(root.querySelectorAll(selector));
    } catch {
      return [];
    }
  };

  const normalizedSelectorText = (selector) => normalizeText(selector);

  const elementFingerprint = (element) =>
    normalizeText(
      [
        accessibleName(element),
        element.textContent ?? "",
        element.getAttribute?.("title") ?? "",
        element.getAttribute?.("placeholder") ?? "",
        element.getAttribute?.("aria-label") ?? "",
        element.getAttribute?.("data-testid") ?? "",
        element.getAttribute?.("data-slot") ?? "",
        element.getAttribute?.("data-action") ?? "",
        element.getAttribute?.("data-mode") ?? "",
        element.getAttribute?.("data-model") ?? "",
        element.getAttribute?.("data-effort") ?? "",
      ].join(" "),
    );

  const selectorTextMatches = (element, selector) => {
    const expected = normalizedSelectorText(selector);
    if (!expected) {
      return false;
    }
    const fingerprint = elementFingerprint(element);
    return (
      fingerprint === expected ||
      fingerprint.includes(expected) ||
      hasToken(fingerprint, [expected])
    );
  };

  const genericControlSelector = [
    controlCandidateSelector,
    "textarea",
    "[role='textbox']",
    "[contenteditable='true']",
    "input[type='file']",
    "input[type='submit']",
    "a[download]",
  ].join(", ");

  const visibleGenericControls = (roots = [documentRoot]) => {
    const seen = new Set();
    const controls = [];
    for (const root of roots) {
      for (const element of safeQueryAll(root, genericControlSelector)) {
        if (!isVisible(element) || seen.has(element)) {
          continue;
        }
        seen.add(element);
        controls.push(element);
      }
    }
    return controls;
  };

  const isUploadLikeControl = (element) =>
    element instanceof HTMLElement &&
    (element.matches(
      [
        "[data-slot='upload-entry']",
        "[data-testid='upload-button']",
        "label[for]",
        "input[type='file']",
      ].join(", "),
    ) ||
      hasToken(elementFingerprint(element), [
        "add files",
        "add photos",
        "attach",
        "upload",
        "file",
        "files",
        "photo",
        "photos",
      ]));

  const isSubmitLikeControl = (element) =>
    element instanceof HTMLElement &&
    !isUploadLikeControl(element) &&
    !hasToken(elementFingerprint(element), ["stop", "cancel"]) &&
    (element.matches(
      [
        "[data-action='submit']",
        "[data-testid='send-button']",
        "button",
        "[role='button']",
        "input[type='submit']",
      ].join(", "),
    ) ||
      hasToken(elementFingerprint(element), ["send", "submit", "ask"]));

  const isTextboxLikeComposer = (element) =>
    element instanceof HTMLElement &&
    (element.matches(
      [
        "[data-slot='composer']",
        "#prompt-textarea",
        "textarea",
        "[role='textbox']",
        "[contenteditable='true']",
      ].join(", "),
    ) ||
      hasToken(elementFingerprint(element), [
        "prompt",
        "message",
        "composer",
        "ask",
        "chat",
      ]));

  const isDownloadLikeControl = (element) =>
    element instanceof HTMLElement &&
    (element.matches("a[download], [data-slot='artifact-link']") ||
      hasToken(elementFingerprint(element), [
        "download",
        "artifact",
        "export",
        "save",
      ]));

  const genericCapabilityInclude = (capability, element, selector = "") => {
    if (!(element instanceof HTMLElement)) {
      return false;
    }
    if (capability === "composer_ready") {
      return isTextboxLikeComposer(element);
    }
    if (capability === "submit_available") {
      return isSubmitLikeControl(element);
    }
    if (capability === "model_picker_visible") {
      return (
        isModelPickerTrigger(element) ||
        isModelOption(element) ||
        selectorTextMatches(element, selector) ||
        hasToken(elementFingerprint(element), [
          "model",
          "engine",
          "chatgpt",
          "instant",
          "thinking",
          "pro",
        ])
      );
    }
    if (capability === "thinking_effort_visible") {
      return (
        isEffortPickerTrigger(element) ||
        isEffortOption(element) ||
        selectorTextMatches(element, selector) ||
        hasToken(elementFingerprint(element), [
          "effort",
          "reasoning",
          "thinking",
          "standard",
          "extended",
        ])
      );
    }
    if (capability === "search_mode_visible") {
      return (
        isSearchModeControl(element) ||
        (selectorTextMatches(element, selector) &&
          isSearchModeLabel(elementFingerprint(element)))
      );
    }
    if (capability === "research_mode_visible") {
      return (
        isResearchModeControl(element) ||
        (selectorTextMatches(element, selector) &&
          hasToken(elementFingerprint(element), ["research", "deep research"]))
      );
    }
    if (capability === "upload_control_visible") {
      return isUploadLikeControl(element);
    }
    if (capability === "downloadable_artifacts_observable") {
      return isDownloadLikeControl(element);
    }
    return false;
  };

  const firstGenericMatch = (capability, elements, selector) => {
    for (const element of elements) {
      if (genericCapabilityInclude(capability, element, selector)) {
        return element;
      }
    }
    return null;
  };

  const genericTextMatch = (capability, selector) =>
    firstGenericMatch(
      capability,
      visibleGenericControls().filter((element) =>
        selectorTextMatches(element, selector),
      ),
      selector,
    );

  const genericAriaRoleMatch = (capability, selector) => {
    const cssMatches = firstGenericMatch(
      capability,
      visibleGenericControls().filter((element) =>
        safeQueryAll(documentRoot, selector).includes(element),
      ),
      selector,
    );
    if (cssMatches) {
      return cssMatches;
    }

    const expected = normalizedSelectorText(selector).replace(/^role=/, "");
    const roleMatches = visibleGenericControls().filter((element) => {
      const role = normalizeText(
        element.getAttribute("role") ?? implicitRole(element) ?? "",
      );
      return role === expected || selectorTextMatches(element, selector);
    });
    return firstGenericMatch(capability, roleMatches, selector);
  };

  const genericDomNeighborhoodMatch = (capability, selector) => {
    const selectorMatches = safeQueryAll(documentRoot, selector).filter(
      (element) => element instanceof HTMLElement && isVisible(element),
    );
    const candidates =
      selectorMatches.length > 0 ? selectorMatches : visibleGenericControls();
    let best = null;
    let bestScore = 0;
    for (const element of candidates) {
      if (!genericCapabilityInclude(capability, element, selector)) {
        continue;
      }
      let score = 1;
      if (isComposerAdjacent(element)) {
        score += 5;
      }
      if (selectorTextMatches(element, selector)) {
        score += 3;
      }
      if (element.closest(popupRootSelector)) {
        score += 2;
      }
      if (score > bestScore) {
        best = element;
        bestScore = score;
      }
    }
    return best;
  };

  const createGenericStrategyFromPackEntry = (capability, entry) => ({
    id: entry.strategy_id,
    score: entry.score,
    find: () => {
      if (entry.kind === "hotkey") {
        return null;
      }
      if (entry.kind === "css") {
        return firstGenericMatch(
          capability,
          safeQueryAll(documentRoot, entry.selector).filter(isVisible),
          entry.selector,
        );
      }
      if (entry.kind === "text") {
        return genericTextMatch(capability, entry.selector);
      }
      if (entry.kind === "aria_role") {
        return genericAriaRoleMatch(capability, entry.selector);
      }
      if (entry.kind === "dom_neighborhood") {
        return genericDomNeighborhoodMatch(capability, entry.selector);
      }
      return null;
    },
  });

  const definitions = [
    {
      key: "composer_ready",
      strategies: [
        {
          id: "data-slot-composer",
          score: 1,
          find: () => firstVisibleBySelector("[data-slot='composer']"),
        },
        {
          id: "chatgpt-prompt-textarea",
          score: 0.99,
          find: () => firstVisibleBySelector("#prompt-textarea"),
        },
        {
          id: "chatgpt-composer-root",
          score: 0.97,
          find: () =>
            firstVisibleBySelectors([
              "[data-testid='composer-root'] textarea",
              "[data-testid='composer-root'] [contenteditable='true']",
            ]),
        },
        {
          id: "aria-textbox",
          score: 0.94,
          find: () => findTextbox(),
        },
        {
          id: "submit-neighborhood",
          score: 0.76,
          find: () => {
            const submit = findNamedControl(["send", "submit"]);
            const container =
              submit?.parentElement ?? submit?.closest("section");
            return container
              ? firstVisible(
                  container.querySelectorAll(
                    "textarea, [role='textbox'], [contenteditable='true']",
                  ),
                )
              : null;
          },
        },
      ],
    },
    {
      key: "submit_available",
      strategies: [
        {
          id: "data-action-submit",
          score: 1,
          find: () => firstVisibleBySelector("[data-action='submit']"),
        },
        {
          id: "chatgpt-send-button",
          score: 0.97,
          find: () =>
            firstVisibleBySelectors([
              "[data-testid='send-button']",
              "button[aria-label='Send prompt']",
              "button[aria-label*='Send']",
            ]),
        },
        {
          id: "button-send",
          score: 0.92,
          find: () => findNamedControl(["send", "submit"]),
        },
        {
          id: "composer-neighborhood",
          score: 0.8,
          find: () => findSubmitNearComposer(),
        },
      ],
    },
    {
      key: "model_picker_visible",
      strategies: [
        {
          id: "data-slot-model-picker",
          score: 1,
          find: () => firstVisibleBySelector("[data-slot='model-picker']"),
        },
        {
          id: "chatgpt-model-trigger",
          score: 0.99,
          find: () => findModelPickerTrigger(),
        },
        {
          id: "visible-model-options",
          score: 0.95,
          find: () =>
            findNamedControl(["instant", "thinking", "pro"], {
              include: isModelOption,
              roots: visiblePopupRootsFor(isModelOption),
            }),
        },
        {
          id: "inline-model-options",
          score: 0.93,
          find: () =>
            findNamedControl(["instant", "thinking", "pro"], {
              include: isModelOption,
            }),
        },
        {
          id: "aria-model-trigger",
          score: 0.9,
          find: () =>
            findNamedControl(["chatgpt", "model", "selector"], {
              include: isModelPickerTrigger,
            }),
        },
      ],
    },
    {
      key: "thinking_effort_visible",
      strategies: [
        {
          id: "data-slot-effort-toggle",
          score: 1,
          find: () => firstVisibleBySelector("[data-slot='effort-toggle']"),
        },
        {
          id: "composer-effort-trigger",
          score: 0.96,
          find: () => findEffortPickerTrigger(),
        },
        {
          id: "visible-effort-options",
          score: 0.91,
          find: () =>
            findNamedControl(["standard", "extended"], {
              include: isEffortOption,
              roots: visiblePopupRootsFor(isEffortOption),
            }),
        },
        {
          id: "inline-effort-options",
          score: 0.9,
          find: () =>
            findComposerAdjacentNamedControl(["standard", "extended"], {
              include: isEffortOption,
            }),
        },
        {
          id: "aria-effort-toggle",
          score: 0.88,
          find: () =>
            findComposerAdjacentNamedControl(
              ["standard", "extended", "effort", "reasoning", "thinking"],
              {
                include: isEffortPickerTrigger,
              },
            ),
        },
      ],
    },
    {
      key: "search_mode_visible",
      strategies: [
        {
          id: "composer-search-chip",
          score: 1,
          find: () =>
            findComposerAdjacentNamedControl(["search"], {
              include: isSearchChip,
            }),
        },
        {
          id: "composer-tools-menu-search",
          score: 0.98,
          find: () =>
            findNamedControl(["search", "web search"], {
              include: isSearchMenuItem,
              includeHidden: true,
              roots: composerToolsMenuRootsFor(isSearchMenuItem),
            }),
        },
        {
          id: "aria-search-toggle",
          score: 0.88,
          find: () =>
            findNamedControl(["search", "web search"], {
              include: isSearchModeControl,
            }),
        },
      ],
    },
    {
      key: "research_mode_visible",
      strategies: [
        {
          id: "data-mode-research",
          score: 1,
          find: () => firstVisibleBySelector("[data-mode='research']"),
        },
        {
          id: "deep-research-anchor",
          score: 0.97,
          find: () =>
            firstVisibleBySelectors([
              "a[data-testid='deep-research-sidebar-item']",
              "a[href='/deep-research']",
              "a[href$='/deep-research']",
            ]),
        },
        {
          id: "chatgpt-research-toggle",
          score: 0.92,
          find: () =>
            firstVisibleBySelectors([
              "button[aria-label*='Research']",
              "button[aria-label*='Deep research']",
            ]) ??
            findNamedControl(["research", "deep research"], {
              include: isResearchModeControl,
            }),
        },
        {
          id: "aria-research-toggle",
          score: 0.88,
          find: () =>
            findNamedControl(["research", "deep research"], {
              include: isResearchModeControl,
            }),
        },
      ],
    },
    {
      key: "upload_control_visible",
      strategies: [
        {
          id: "data-slot-upload-entry",
          score: 1,
          find: () => firstVisibleBySelector("[data-slot='upload-entry']"),
        },
        {
          id: "chatgpt-upload-entry",
          score: 0.94,
          find: () =>
            firstVisibleBySelectors([
              "[data-testid='upload-button']",
              "button[aria-label*='Add photos']",
              "button[aria-label*='Add files']",
            ]) ?? findUploadLabel(),
        },
        {
          id: "native-file-input",
          score: 0.88,
          find: () => {
            const input = firstVisible(
              documentRoot.querySelectorAll("input[type='file']"),
            );
            if (input) {
              return input;
            }
            return findUploadLabel();
          },
        },
      ],
    },
    {
      key: "downloadable_artifacts_observable",
      strategies: [
        {
          id: "data-slot-artifact-link",
          score: 1,
          find: () => firstVisibleBySelector("[data-slot='artifact-link']"),
        },
        {
          id: "chatgpt-download-link",
          score: 0.94,
          find: () =>
            firstVisibleBySelectors([
              "[data-testid='artifact-download']",
              "a[aria-label*='Download']",
              "button[aria-label*='Download']",
            ]) ?? findNamedControl(["download", "artifact", "export", "save"]),
        },
        {
          id: "download-link",
          score: 0.9,
          find: () =>
            firstVisible(documentRoot.querySelectorAll("a[download]")),
        },
        {
          id: "aria-download-link",
          score: 0.72,
          find: () =>
            findNamedControl(["download", "artifact", "export", "save"]),
        },
      ],
    },
  ];

  const orderStrategies = (capability, strategies) =>
    orderCapabilityStrategies({
      capability,
      strategies,
      selectorPack,
      preferredStrategies,
      createGenericStrategy: (entry) =>
        createGenericStrategyFromPackEntry(capability, entry),
    });

  const aliases = createAliasBuckets();

  const captureAliases = (capability, element) =>
    captureProbeAliases({
      aliases,
      capability,
      element,
      accessibleName,
      hasToken,
      visiblePopupRootsFor,
      isVisible,
      isModelOption,
      isEffortOption,
      isModelPickerTrigger,
      isSearchModeControl,
      isResearchModeControl,
      popupOptionSelector,
    });

  const matrix = {
    composer_ready: false,
    submit_available: false,
    model_picker_visible: false,
    thinking_effort_visible: false,
    search_mode_visible: false,
    research_mode_visible: false,
    upload_control_visible: false,
    downloadable_artifacts_observable: false,
  };

  const uiSignature = [
    window.location.hostname || "unknown-host",
    body?.dataset.uiVariant || "unlabelled",
    body?.querySelector("#prompt-textarea")
      ? "prompt-textarea"
      : "no-prompt-id",
    body?.querySelector("[data-slot]") ? "slot-ui" : "aria-ui",
    body?.querySelector("[role='textbox']") ? "role-textbox" : "native-textbox",
  ].join(":");
  const strategyMatches = {};
  const domNeighborhoods = [];

  for (const definition of definitions) {
    const ordered = orderStrategies(definition.key, definition.strategies);
    let match = null;
    let strategyId;

    for (const strategy of ordered) {
      const candidate = strategy.find();
      if (candidate) {
        match = candidate;
        strategyId = strategy.id;
        break;
      }
    }

    matrix[definition.key] = Boolean(match);
    if (!match || !strategyId) {
      continue;
    }

    strategyMatches[definition.key] = strategyId;
    domNeighborhoods.push(
      createNeighborhood(definition.key, strategyId, match),
    );
    captureAliases(definition.key, match);
  }

  return buildCapabilityProbeReport({
    reportedAt,
    selectorPackId,
    selectorPackVersion,
    matrix,
    aliases,
    uiSignature,
    strategyMatches,
    domNeighborhoods,
  });
}

function buildCapabilityProbeReport({
  reportedAt,
  selectorPackId,
  selectorPackVersion,
  matrix,
  aliases,
  uiSignature,
  strategyMatches,
  domNeighborhoods,
}) {
  return {
    type: "capability_report",
    report: {
      reported_at: reportedAt,
      selector_pack_id: selectorPackId,
      selector_pack_version: selectorPackVersion,
      matrix,
      aliases,
    },
    runtime: {
      ui_signature: uiSignature,
      strategy_matches: strategyMatches,
      dom_neighborhoods: domNeighborhoods,
    },
  };
}
