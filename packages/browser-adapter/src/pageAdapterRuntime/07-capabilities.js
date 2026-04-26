function createCapabilityRuntimeModule({
  textUtils,
  domUtils,
  composerRuntime,
}) {
  const { normalizeText, unique, hasToken } = textUtils;
  const {
    isVisible,
    firstVisible,
    accessibleName,
    actionableSelector,
    actionableElements,
    visibleUnique,
    isSelected,
    findByAliases,
    clickControl,
  } = domUtils;
  const { isComposerAdjacent, findComposerAdjacentControl } = composerRuntime;

  const expandAliases = (report, bucket, key, defaults) =>
    unique([...(report?.aliases?.[bucket]?.[key] ?? []), ...defaults]);

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

  const isSearchModeControl = (element) =>
    element instanceof HTMLElement &&
    (isSearchChip(element) || isSearchMenuItem(element));

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
      mode === "search" ? "web search" : "deep research",
    ]);
    const control = findByAliases({
      selectors:
        mode === "search"
          ? [
              "button.__composer-pill[aria-label*='Search']",
              "[role='menu'] [role='menuitemradio']",
              "[role='menu'] [role='menuitem']",
            ]
          : [
              "a[data-testid='deep-research-sidebar-item']",
              "a[href='/deep-research']",
              "a[href$='/deep-research']",
            ],
      aliases,
      fallbackTokens:
        mode === "search" ? ["search", "web search"] : ["deep research"],
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

  return {
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
  };
}
