export function createCapabilityDefinitions(context, predicates) {
  const {
    documentRoot,
    firstVisible,
    firstVisibleBySelector,
    firstVisibleBySelectors,
    findNamedControl,
    findComposerAdjacentNamedControl,
    safeQueryAll,
  } = context;
  const {
    isModelOption,
    isEffortOption,
    isModelPickerTrigger,
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
  } = predicates;

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
          safeQueryAll(documentRoot, entry.selector).filter(context.isVisible),
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
          find: () => context.findTextbox(),
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
          find: () => findSubmitNearComposer(),
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
            findNamedControl(["instant", "thinking", "medium", "high", "pro"], {
              include: isModelOption,
              roots: visiblePopupRootsFor(isModelOption),
            }),
        },
        {
          id: "inline-model-options",
          score: 0.93,
          find: () =>
            findNamedControl(["instant", "thinking", "medium", "high", "pro"], {
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
            findNamedControl(["standard", "extended", "medium", "high"], {
              include: isEffortOption,
              roots: visiblePopupRootsFor(isEffortOption),
            }),
        },
        {
          id: "inline-effort-options",
          score: 0.9,
          find: () =>
            findComposerAdjacentNamedControl(
              ["standard", "extended", "medium", "high"],
              {
                include: isEffortOption,
              },
            ),
        },
        {
          id: "aria-effort-toggle",
          score: 0.88,
          find: () =>
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
                include: predicates.isEffortPickerTrigger,
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
              include: predicates.isSearchChip,
            }),
        },
        {
          id: "composer-tools-menu-search",
          score: 0.98,
          find: () =>
            findNamedControl(["search", "web search"], {
              include: predicates.isSearchMenuItem,
              includeHidden: true,
              roots: composerToolsMenuRootsFor(predicates.isSearchMenuItem),
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

  return {
    definitions,
    createGenericStrategyFromPackEntry,
  };
}
