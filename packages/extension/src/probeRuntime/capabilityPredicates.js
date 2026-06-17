export function createCapabilityPredicates(context) {
  const {
    documentRoot,
    normalizeText,
    isVisible,
    firstVisible,
    implicitRole,
    accessibleName,
    hasToken,
    controlCandidateSelector,
    firstVisibleBySelector,
    firstVisibleBySelectors,
    findNamedControl,
    findTextbox,
    isComposerAdjacent,
    composerAdjacencyScore,
    findComposerAdjacentNamedControl,
    safeQueryAll,
    visibleGenericControls,
  } = context;

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
      "medium",
      "high",
      "standard",
      "extended",
    ]) ||
    /\b5(?:\.\d+)?\b/.test(name) ||
    (hasToken(name, ["thinking"]) &&
      !hasToken(name, ["medium", "high", "standard", "extended", "effort"]));

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
        "medium",
        "high",
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
    hasToken(accessibleName(element), [
      "instant",
      "thinking",
      "medium",
      "high",
      "pro",
    ]);

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
    hasToken(accessibleName(element), [
      "standard",
      "extended",
      "medium",
      "high",
    ]);

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
    findNamedControl(
      ["instant", "thinking", "medium", "high", "legacy", "gpt", "5.3", "5.4"],
      {
        include: isComposerRightModelPickerTrigger,
      },
    ) ??
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
      [
        "standard",
        "extended",
        "medium",
        "high",
        "effort",
        "reasoning",
        "thinking",
      ],
      {
        include: isEffortPickerTrigger,
      },
    );

  const findSubmitNearComposer = () => {
    const exact = firstVisibleBySelectors([
      "[data-action='submit']",
      "[data-testid='send-button']",
      "button[aria-label='Submit prompt']",
      "button[type='submit']",
    ]);
    if (exact) {
      return exact;
    }
    const composer = findTextbox();
    const containers = composer
      ? [
          composer.closest("form"),
          composer.parentElement,
          composer.closest("section"),
          composer.closest("main"),
        ]
      : [];
    for (const container of containers) {
      if (!container) {
        continue;
      }
      const candidate =
        firstVisible(
          container.querySelectorAll(
            [
              "button[aria-label*='Send']",
              "button[aria-label*='Submit']",
              "button, [role='button'], input[type='submit']",
            ].join(", "),
          ),
        ) ?? null;
      if (candidate) {
        return candidate;
      }
    }
    return firstVisibleBySelector("button[aria-label*='Send']");
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
          "medium",
          "high",
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
          "medium",
          "high",
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
        score += 2 + composerAdjacencyScore(element);
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

  return {
    popupOptionSelector,
    isModelOption,
    isEffortOption,
    isModelPickerTrigger,
    isEffortPickerTrigger,
    isSearchChip,
    isSearchMenuItem,
    isSearchModeControl,
    isResearchModeControl,
    visiblePopupRootsFor,
    composerToolsMenuRootsFor,
    findModelPickerTrigger,
    findEffortPickerTrigger,
    findSubmitNearComposer,
    findUploadLabel,
    genericTextMatch,
    genericAriaRoleMatch,
    genericDomNeighborhoodMatch,
    firstGenericMatch,
  };
}
