// Generated from src/probe.runtime.js and src/probeRuntime/*.js. Do not edit by hand.
// Run `pnpm --filter @tablinq/extension run generate:probe-content` after changing the runtime.

(() => {
  function normalizeProbeText(value) {
    return (value ?? "").replace(/\s+/g, " ").trim().toLowerCase();
  }

  function computedStyleForProbeElement(element) {
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
  }

  function isVisibleProbeElement(element) {
    if (!(element instanceof HTMLElement)) {
      return false;
    }
    if (element.hidden) {
      return false;
    }
    if (element.getAttribute("aria-hidden") === "true") {
      return false;
    }
    const computed = computedStyleForProbeElement(element);
    if (
      computed &&
      (computed.display === "none" ||
        computed.visibility === "hidden" ||
        computed.opacity === "0")
    ) {
      return false;
    }
    return true;
  }

  function firstVisibleProbeElement(elements, isVisible) {
    for (const element of Array.from(elements)) {
      if (isVisible(element)) {
        return element;
      }
    }
    return null;
  }

  function implicitProbeRole(element) {
    const tagName = element.tagName.toLowerCase();
    if (tagName === "button") {
      return "button";
    }
    if (tagName === "a" && element.href) {
      return "link";
    }
    if (tagName === "textarea") {
      return "textbox";
    }
    if (tagName === "input") {
      const type = element.type.toLowerCase();
      if (type === "file") {
        return "button";
      }
      if (["text", "search", "email", "url"].includes(type)) {
        return "textbox";
      }
    }
    if (element.hasAttribute("contenteditable")) {
      return "textbox";
    }
    return null;
  }

  function accessibleProbeName(element, documentRoot = document) {
    if (!element) {
      return "";
    }
    const ariaLabel = element.getAttribute("aria-label");
    if (ariaLabel) {
      return normalizeProbeText(ariaLabel);
    }

    const labelledBy = element.getAttribute("aria-labelledby");
    if (labelledBy) {
      const labels = labelledBy
        .split(/\s+/)
        .map((id) => documentRoot.getElementById(id))
        .filter(Boolean)
        .map((value) => normalizeProbeText(value.textContent));
      if (labels.length > 0) {
        return labels.join(" ");
      }
    }

    if (
      element instanceof HTMLInputElement ||
      element instanceof HTMLTextAreaElement
    ) {
      if (element.labels?.length) {
        return Array.from(element.labels)
          .map((label) => normalizeProbeText(label.textContent))
          .join(" ");
      }
      if (element.placeholder) {
        return normalizeProbeText(element.placeholder);
      }
      if (element.value) {
        return normalizeProbeText(element.value);
      }
    }

    return normalizeProbeText(element.textContent);
  }

  function probeTextHasToken(value, tokens) {
    return tokens.some((token) =>
      new RegExp(
        `(^|[^a-z0-9])${token.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}([^a-z0-9]|$)`,
      ).test(value),
    );
  }

  function probeElementPath(element) {
    const segments = [];
    let current = element;
    while (current && segments.length < 5) {
      const tag = current.tagName.toLowerCase();
      const id = current.id ? `#${current.id}` : "";
      const role = current.getAttribute("role");
      const slot = current.getAttribute("data-slot");
      const descriptor = [
        tag,
        id,
        role ? `[role=${role}]` : "",
        slot ? `[data-slot=${slot}]` : "",
      ].join("");
      segments.unshift(descriptor);
      current = current.parentElement;
    }
    return segments.join(" > ");
  }

  function probeNearbyText(element) {
    const container = element.parentElement ?? element;
    return Array.from(container.querySelectorAll("*"))
      .slice(0, 8)
      .map((node) => normalizeProbeText(node.textContent))
      .filter((value) => value.length > 0)
      .slice(0, 4);
  }

  function createProbeNeighborhood(capability, strategyId, element) {
    return {
      capability,
      strategy_id: strategyId,
      anchor_tag: element.tagName.toLowerCase(),
      anchor_role: element.getAttribute("role") ?? implicitProbeRole(element),
      anchor_name: accessibleProbeName(element),
      anchor_path: probeElementPath(element),
      parent_name:
        normalizeProbeText(element.parentElement?.textContent ?? null) || null,
      sibling_names: Array.from(element.parentElement?.children ?? [])
        .filter((candidate) => candidate !== element)
        .map((candidate) => accessibleProbeName(candidate))
        .filter((value) => value.length > 0)
        .slice(0, 4),
      nearby_text: probeNearbyText(element),
    };
  }

  const controlCandidateSelector = [
    "button",
    "[role='button']",
    "[role='switch']",
    "[role='tab']",
    "[role='option']",
    "[role='menuitem']",
    "[role='menuitemradio']",
    "[aria-haspopup]",
    "label",
    "a",
  ].join(", ");

  function orderCapabilityStrategies({
    capability,
    strategies,
    selectorPack,
    preferredStrategies,
  }) {
    const fromPack =
      selectorPack?.capabilities?.[capability]
        ?.map((entry) => {
          const match = strategies.find(
            (strategy) => strategy.id === entry.strategy_id,
          );
          return match ? { ...match, score: entry.score } : null;
        })
        .filter(Boolean) ?? [];
    const orderedStrategies =
      fromPack.length > 0
        ? [
            ...fromPack,
            ...strategies.filter(
              (strategy) => !fromPack.some((entry) => entry.id === strategy.id),
            ),
          ]
        : [...strategies];
    const preferred = preferredStrategies[capability];
    if (!preferred) {
      return orderedStrategies.sort((left, right) => right.score - left.score);
    }
    return orderedStrategies.sort((left, right) => {
      if (left.id === preferred) {
        return -1;
      }
      if (right.id === preferred) {
        return 1;
      }
      return right.score - left.score;
    });
  }

  function createAliasBuckets() {
    return {
      modes: {},
      model_classes: {},
      efforts: {},
    };
  }

  function addProbeAlias(aliases, bucket, key, label) {
    if (!label) {
      return;
    }
    const current = aliases[bucket][key] ?? [];
    if (!current.includes(label)) {
      aliases[bucket][key] = [...current, label];
    }
  }

  function captureProbeAliases({
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
  }) {
    if (!element) {
      return;
    }
    const names = [];
    const pushName = (candidate) => {
      const name =
        typeof candidate === "string" ? candidate : accessibleName(candidate);
      if (name && !names.includes(name)) {
        names.push(name);
      }
    };

    pushName(element);
    if (capability === "model_picker_visible") {
      for (const root of visiblePopupRootsFor(isModelOption, element)) {
        for (const candidate of Array.from(
          root.querySelectorAll(popupOptionSelector),
        )) {
          if (isVisible(candidate) && isModelOption(candidate)) {
            pushName(candidate);
          }
        }
      }
      for (const name of ["instant", "thinking"]) {
        pushName(name);
      }
    }
    if (capability === "thinking_effort_visible") {
      for (const root of visiblePopupRootsFor(isEffortOption, element)) {
        for (const candidate of Array.from(
          root.querySelectorAll(popupOptionSelector),
        )) {
          if (isVisible(candidate) && isEffortOption(candidate)) {
            pushName(candidate);
          }
        }
      }
      for (const name of ["standard", "extended"]) {
        pushName(name);
      }
    }
    if (capability === "search_mode_visible" && isSearchModeControl(element)) {
      pushName("search");
    }
    if (
      capability === "research_mode_visible" &&
      isResearchModeControl(element)
    ) {
      pushName("deep research");
    }
    if (names.length === 0) {
      return;
    }
    if (capability === "model_picker_visible") {
      for (const name of names) {
        if (hasToken(name, ["instant"])) {
          addProbeAlias(aliases, "model_classes", "instant", name);
        }
        if (hasToken(name, ["thinking"])) {
          addProbeAlias(aliases, "model_classes", "thinking", name);
        }
      }
      if (
        Object.keys(aliases.model_classes).length === 0 &&
        isModelPickerTrigger(element)
      ) {
        addProbeAlias(aliases, "model_classes", "instant", "instant");
        addProbeAlias(aliases, "model_classes", "thinking", "thinking");
      }
    }
    if (capability === "thinking_effort_visible") {
      for (const name of names) {
        if (hasToken(name, ["extended"])) {
          addProbeAlias(aliases, "efforts", "extended", name);
        }
        if (hasToken(name, ["standard"])) {
          addProbeAlias(aliases, "efforts", "standard", name);
        }
        if (hasToken(name, ["none", "auto"])) {
          addProbeAlias(aliases, "efforts", "none", name);
        }
      }
    }
    if (capability === "search_mode_visible") {
      for (const name of names) {
        if (hasToken(name, ["search", "browse the web"])) {
          addProbeAlias(aliases, "modes", "search", name);
        }
      }
    }
    if (capability === "research_mode_visible") {
      for (const name of names) {
        if (hasToken(name, ["deep research"])) {
          addProbeAlias(aliases, "modes", "research", name);
        }
      }
    }
  }

  function probeDocument(options = {}) {
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
      findNamedControl(["chatgpt", "model", "selector"], {
        include: isModelPickerTrigger,
      });

    const findEffortPickerTrigger = () =>
      firstVisibleBySelectors([
        "button.__composer-pill[aria-haspopup]",
        "button[aria-haspopup][aria-label*='Thinking']",
        "button[aria-haspopup][aria-label*='thinking']",
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

    const runtime = {
      ui_signature: [
        window.location.hostname || "unknown-host",
        body?.dataset.uiVariant || "unlabelled",
        body?.querySelector("#prompt-textarea")
          ? "prompt-textarea"
          : "no-prompt-id",
        body?.querySelector("[data-slot]") ? "slot-ui" : "aria-ui",
        body?.querySelector("[role='textbox']")
          ? "role-textbox"
          : "native-textbox",
      ].join(":"),
      strategy_matches: {},
      dom_neighborhoods: [],
    };

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

      runtime.strategy_matches[definition.key] = strategyId;
      runtime.dom_neighborhoods.push(
        createNeighborhood(definition.key, strategyId, match),
      );
      captureAliases(definition.key, match);
    }

    return {
      type: "capability_report",
      report: {
        reported_at: reportedAt,
        selector_pack_id: selectorPackId,
        selector_pack_version: selectorPackVersion,
        matrix,
        aliases,
      },
      runtime,
    };
  }

  globalThis.__TABLINQ_PROBE_DOCUMENT__ = probeDocument;
})();
