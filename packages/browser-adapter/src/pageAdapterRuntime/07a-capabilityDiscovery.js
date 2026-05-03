function createCapabilityDiscoveryRuntime({
  textUtils,
  domUtils,
  composerRuntime,
  reportHelpers,
}) {
  const { normalizeText, unique, hasToken } = textUtils;
  const {
    isVisible,
    firstVisible,
    accessibleName,
    actionableSelector,
    actionableElements,
    visibleUnique,
    findByAliases,
    clickControl,
  } = domUtils;
  const { isComposerAdjacent, findComposerAdjacentControl } = composerRuntime;
  const {
    expandAliases,
    modelDefaults,
    modelFallbackTokens,
    effortDefaults,
    effortFallbackTokens,
  } = reportHelpers;

  const menuOptionSelector =
    "[role='menuitemradio'], [role='menuitem'], [role='option']";
  const popupRootSelector = "[role='menu'], [role='listbox']";
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
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

  const controlledPopupHasToken = (element, tokens) => {
    const controlledId = element?.getAttribute?.("aria-controls");
    if (!controlledId) {
      return false;
    }
    const controlled = document.getElementById(controlledId);
    if (!(controlled instanceof HTMLElement)) {
      return false;
    }
    return visibleUnique(controlled.querySelectorAll(menuOptionSelector)).some(
      (option) => hasToken(accessibleName(option), tokens),
    );
  };

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
      ]) ||
      controlledPopupHasToken(element, ["instant", "thinking", "pro"]));

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
        "reasoning",
        "depth",
        "standard",
        "extended",
        "effort",
      ]));

  const isComposerModelChip = (element) => {
    if (!(element instanceof HTMLElement) || !isComposerAdjacent(element)) {
      return false;
    }
    const name = accessibleName(element);
    return (
      element.matches(
        "button.__composer-pill, button[aria-haspopup], [role='button']",
      ) &&
      !hasToken(name, ["search", "research", "canvas"]) &&
      hasToken(name, ["thinking", "reasoning", "standard", "extended"])
    );
  };

  const findComposerModelChip = () =>
    findComposerAdjacentControl({
      selectors: [
        "button.__composer-pill",
        "button[aria-haspopup][aria-label*='Thinking']",
        "button[aria-haspopup][aria-label*='thinking']",
      ],
      tokens: ["thinking", "reasoning", "standard", "extended"],
      include: isComposerModelChip,
      exclude: (element) => !isComposerModelChip(element),
    });

  const findComposerModelChipRemove = () =>
    findComposerAdjacentControl({
      selectors: [
        "button[aria-label*='remove' i][aria-label*='think' i]",
        "button[aria-label*='remove' i][aria-label*='reason' i]",
        "button[aria-label*='remove' i][aria-label*='standard' i]",
        "button[aria-label*='remove' i][aria-label*='extended' i]",
      ],
      tokens: ["remove", "thinking", "reasoning", "standard", "extended"],
      include: (element) =>
        isComposerModelChip(element) &&
        accessibleName(element).includes("remove"),
      exclude: (element) =>
        !isComposerModelChip(element) ||
        !accessibleName(element).includes("remove"),
    });

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

  const nameLooksLikeEffortOption = (name) =>
    hasToken(name, ["standard", "extended", "effort", "depth", "balanced"]);

  const nameLooksLikeModelOption = (name) =>
    hasToken(name, [
      "instant",
      "thinking",
      "pro",
      "gpt",
      "model",
      "fast",
      "deep",
      "deeper",
    ]);

  const popupRootLooksLikeEffort = (element) => {
    const root = element.closest(popupRootSelector);
    if (!(root instanceof HTMLElement)) {
      return false;
    }
    const fingerprint = normalizeText(
      [
        root.id,
        root.getAttribute("aria-label") ?? "",
        root.getAttribute("aria-labelledby") ?? "",
        root.getAttribute("data-testid") ?? "",
      ].join(" "),
    );
    return hasToken(fingerprint, ["effort", "reasoning", "depth"]);
  };

  const isModelOption = (element) => {
    if (
      !(element instanceof HTMLElement) ||
      isModelPickerTrigger(element) ||
      !isPopupOption(element) ||
      element.closest(popupRootSelector) === null
    ) {
      return false;
    }
    const testId = element.getAttribute("data-testid") ?? "";
    const name = accessibleName(element);
    if (testId.startsWith("model-switcher-") && !testId.includes("effort")) {
      return true;
    }
    if (element.hasAttribute("data-model") || nameLooksLikeModelOption(name)) {
      return true;
    }
    return (
      !popupRootLooksLikeEffort(element) &&
      !nameLooksLikeEffortOption(name) &&
      !hasToken(name, [
        "search",
        "web search",
        "browse online",
        "browse the web",
        "research",
        "download",
        "upload",
      ])
    );
  };

  const isEffortOption = (element) => {
    if (
      !(element instanceof HTMLElement) ||
      isEffortPickerTrigger(element) ||
      !isPopupOption(element) ||
      element.closest(popupRootSelector) === null
    ) {
      return false;
    }
    const testId = element.getAttribute("data-testid") ?? "";
    const name = accessibleName(element);
    if (testId.startsWith("model-switcher-") && !testId.includes("effort")) {
      return false;
    }
    return (
      element.hasAttribute("data-effort") ||
      nameLooksLikeEffortOption(name) ||
      (popupRootLooksLikeEffort(element) &&
        !hasToken(name, ["instant", "model", "pro"]))
    );
  };

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
    (hasToken(name, ["search"]) ||
      hasToken(name, ["web search"]) ||
      hasToken(name, ["browse online", "browse the web"])) &&
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
        "button.__composer-pill[aria-haspopup]",
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
        "#thinking-effort-trigger",
        "button[aria-controls*='effort']",
        "button[aria-haspopup][aria-label*='Thinking']",
        "button[aria-haspopup][aria-label*='thinking']",
        "button[aria-haspopup][aria-label*='Effort']",
        "button[aria-haspopup][aria-label*='effort']",
      ],
      aliases,
      tokens: unique([
        ...aliases,
        "effort",
        "reasoning",
        "depth",
        ...fallbackTokens,
      ]),
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

  const findComposerToolsTrigger = () =>
    findComposerAdjacentControl({
      selectors: [
        "#composer-plus-btn",
        "[data-testid='composer-plus-btn']",
        "button[aria-label*='Add files'][aria-haspopup]",
        "button[aria-label*='Add photos'][aria-haspopup]",
        "button[aria-label*='tools'][aria-haspopup]",
        "button[aria-label*='more'][aria-haspopup]",
      ],
      tokens: ["add files", "add photos", "tools", "more"],
      include: isComposerToolsTrigger,
      requirePopup: true,
      exclude: (element) => !isComposerToolsTrigger(element),
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

  const findConcreteSearchControl = (report, roots = []) =>
    roots.length > 0
      ? findByAliasesInRoots({
          roots,
          selectors: [
            "button.__composer-pill[aria-label*='Search']",
            "[role='menu'] [role='menuitemradio']",
            "[role='menu'] [role='menuitem']",
          ],
          aliases: expandAliases(report, "modes", "search", [
            "search",
            "web search",
          ]),
          fallbackTokens: ["search", "web search"],
          include: isSearchModeControl,
        })
      : findByAliases({
          selectors: [
            "button.__composer-pill[aria-label*='Search']",
            "[role='menu'] [role='menuitemradio']",
            "[role='menu'] [role='menuitem']",
          ],
          aliases: expandAliases(report, "modes", "search", [
            "search",
            "web search",
          ]),
          fallbackTokens: ["search", "web search"],
          include: isSearchModeControl,
        });

  const findOpenEffortMenuControl = (report, effort) =>
    findByAliasesInRoots({
      roots: visiblePopupRootsFor(isEffortOption),
      aliases: expandAliases(report, "efforts", effort, effortDefaults(effort)),
      fallbackTokens: effortFallbackTokens(effort),
      include: isEffortOption,
      exclude: isEffortPickerTrigger,
    });

  const openNestedThinkingEffortRoots = async () => {
    const trigger = findModelPickerTrigger(
      ["chatgpt", "instant", "thinking"],
      ["instant", "thinking"],
    );
    if (!trigger) {
      return [];
    }
    const modelRoots = await openPopupRootsFor(trigger, isModelOption);
    const effortTrigger = findByAliasesInRoots({
      roots: modelRoots,
      selectors: [
        "[data-testid='model-switcher-gpt-5-4-thinking-thinking-effort']",
        "[data-testid^='model-switcher-gpt-'][data-testid$='-thinking-thinking-effort']",
      ],
      aliases: ["effort"],
      fallbackTokens: ["effort", "standard", "extended"],
      include: isEffortPickerTrigger,
      exclude: (element) => !isEffortPickerTrigger(element),
    });
    return openPopupRootsFor(effortTrigger, isEffortOption);
  };

  return {
    sleep,
    isModelPickerTrigger,
    isEffortPickerTrigger,
    isModelOption,
    isEffortOption,
    isSearchMenuItem,
    isSearchModeControl,
    isResearchModeControl,
    openPopupRootsFor,
    visiblePopupRootsFor,
    findConcreteModelControl,
    findOpenModelMenuControl,
    findConcreteEffortControl,
    findOpenEffortMenuControl,
    openNestedThinkingEffortRoots,
    findConcreteSearchControl,
    findModelPickerTrigger,
    findEffortPickerTrigger,
    findComposerToolsTrigger,
    findComposerModelChip,
    findComposerModelChipRemove,
    discoverVisibleModelAliases,
    discoverVisibleEffortAliases,
  };
}
