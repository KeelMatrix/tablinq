export function probeDocument(options = {}) {
  const selectorPackId = options.selectorPackId ?? "chatgpt-default";
  const selectorPackVersion = options.selectorPackVersion ?? "v1";
  const reportedAt = options.reportedAt ?? new Date().toISOString();
  const preferredStrategies = options.preferredStrategies ?? {};
  const selectorPack = options.selectorPack ?? null;

  const normalizeText = (value) =>
    (value ?? "").replace(/\s+/g, " ").trim().toLowerCase();

  const documentRoot = document;
  const body = documentRoot.body;

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
    if (element.hidden) {
      return false;
    }
    if (element.getAttribute("aria-hidden") === "true") {
      return false;
    }
    const computed = getComputed(element);
    if (
      computed &&
      (computed.display === "none" ||
        computed.visibility === "hidden" ||
        computed.opacity === "0")
    ) {
      return false;
    }
    return true;
  };

  const firstVisible = (elements) => {
    for (const element of Array.from(elements)) {
      if (isVisible(element)) {
        return element;
      }
    }
    return null;
  };

  const implicitRole = (element) => {
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
  };

  const accessibleName = (element) => {
    if (!element) {
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
        .map((id) => documentRoot.getElementById(id))
        .filter(Boolean)
        .map((value) => normalizeText(value.textContent));
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
          .map((label) => normalizeText(label.textContent))
          .join(" ");
      }
      if (element.placeholder) {
        return normalizeText(element.placeholder);
      }
      if (element.value) {
        return normalizeText(element.value);
      }
    }

    return normalizeText(element.textContent);
  };

  const hasToken = (value, tokens) =>
    tokens.some((token) =>
      new RegExp(
        `(^|[^a-z0-9])${token.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}([^a-z0-9]|$)`,
      ).test(value),
    );

  const firstVisibleBySelector = (selector) =>
    firstVisible(documentRoot.querySelectorAll(selector));

  const findNamedControl = (tokens) => {
    const candidates = documentRoot.querySelectorAll(
      [
        "button",
        "[role='button']",
        "[role='switch']",
        "[role='tab']",
        "[role='option']",
        "[aria-haspopup]",
        "label",
        "a",
      ].join(", "),
    );
    for (const candidate of Array.from(candidates)) {
      if (!isVisible(candidate)) {
        continue;
      }
      const name = accessibleName(candidate);
      if (hasToken(name, tokens)) {
        return candidate;
      }
    }
    return null;
  };

  const findTextbox = () => {
    const candidates = documentRoot.querySelectorAll(
      [
        "[data-slot='composer']",
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

  const findSubmitNearComposer = () => {
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
      if (!hasToken(name, ["attach", "upload", "file", "add files"])) {
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

  const elementPath = (element) => {
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
  };

  const nearbyText = (element) => {
    const container = element.parentElement ?? element;
    return Array.from(container.querySelectorAll("*"))
      .slice(0, 8)
      .map((node) => normalizeText(node.textContent))
      .filter((value) => value.length > 0)
      .slice(0, 4);
  };

  const createNeighborhood = (capability, strategyId, element) => ({
    capability,
    strategy_id: strategyId,
    anchor_tag: element.tagName.toLowerCase(),
    anchor_role: element.getAttribute("role") ?? implicitRole(element),
    anchor_name: accessibleName(element),
    anchor_path: elementPath(element),
    parent_name:
      normalizeText(element.parentElement?.textContent ?? null) || null,
    sibling_names: Array.from(element.parentElement?.children ?? [])
      .filter((candidate) => candidate !== element)
      .map((candidate) => accessibleName(candidate))
      .filter((value) => value.length > 0)
      .slice(0, 4),
    nearby_text: nearbyText(element),
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
          id: "aria-model-trigger",
          score: 0.9,
          find: () =>
            findNamedControl(["gpt", "model", "engine", "instant", "thinking"]),
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
          id: "aria-effort-toggle",
          score: 0.88,
          find: () =>
            findNamedControl(["standard", "extended", "effort", "reasoning"]),
        },
      ],
    },
    {
      key: "search_mode_visible",
      strategies: [
        {
          id: "data-mode-search",
          score: 1,
          find: () => firstVisibleBySelector("[data-mode='search']"),
        },
        {
          id: "aria-search-toggle",
          score: 0.88,
          find: () => findNamedControl(["search", "browse", "web"]),
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
          id: "aria-research-toggle",
          score: 0.88,
          find: () => findNamedControl(["research", "deep research"]),
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

  const orderStrategies = (capability, strategies) => {
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
  };

  const aliases = {
    modes: {},
    model_classes: {},
    efforts: {},
  };

  const addAlias = (bucket, key, label) => {
    if (!label) {
      return;
    }
    const current = aliases[bucket][key] ?? [];
    if (!current.includes(label)) {
      aliases[bucket][key] = [...current, label];
    }
  };

  const collectAliasCandidates = (element) => {
    const names = [];
    const pushName = (candidate) => {
      const name = accessibleName(candidate);
      if (name && !names.includes(name)) {
        names.push(name);
      }
    };

    pushName(element);
    const scoped = element.matches(
      "[data-slot='model-picker'], [data-slot='effort-toggle']",
    )
      ? element
      : element.parentElement;
    for (const candidate of Array.from(
      scoped?.querySelectorAll(
        [
          "button",
          "[role='button']",
          "[role='tab']",
          "[role='option']",
          "[aria-haspopup]",
          "label",
        ].join(", "),
      ) ?? [],
    )) {
      if (isVisible(candidate)) {
        pushName(candidate);
      }
    }
    return names;
  };

  const captureAliases = (capability, element) => {
    if (!element) {
      return;
    }
    const names = collectAliasCandidates(element);
    if (names.length === 0) {
      return;
    }
    if (capability === "model_picker_visible") {
      for (const name of names) {
        if (hasToken(name, ["instant"])) {
          addAlias("model_classes", "instant", name);
        }
        if (hasToken(name, ["thinking"])) {
          addAlias("model_classes", "thinking", name);
        }
      }
      if (Object.keys(aliases.model_classes).length === 0) {
        addAlias("model_classes", "thinking", names[0]);
      }
    }
    if (capability === "thinking_effort_visible") {
      for (const name of names) {
        if (hasToken(name, ["extended"])) {
          addAlias("efforts", "extended", name);
        }
        if (hasToken(name, ["standard"])) {
          addAlias("efforts", "standard", name);
        }
        if (!hasToken(name, ["standard", "extended"])) {
          addAlias("efforts", "none", name);
        }
      }
    }
    if (capability === "search_mode_visible") {
      for (const name of names) {
        addAlias("modes", "search", name);
      }
    }
    if (capability === "research_mode_visible") {
      for (const name of names) {
        addAlias("modes", "research", name);
      }
    }
  };

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
      body?.dataset.uiVariant || "unlabelled",
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
