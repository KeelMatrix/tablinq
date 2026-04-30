export function createMenuRuntime({
  document,
  config,
  hasControl,
  isDeepResearchLanding,
  enterDeepResearchLanding,
}) {
  const { variant, usesDeepResearchLanding } = config;
  const modelPickerTrigger =
    document.querySelector("#model-switcher-trigger") ??
    document.querySelector("[data-harness-model-trigger='composer-right']");
  const effortPickerTrigger = document.querySelector(
    "#thinking-effort-trigger",
  );
  const composerToolsTrigger = document.querySelector("#composer-plus-btn");
  const nestedEffortTrigger = document.querySelector(
    "[data-testid='model-switcher-gpt-5-4-thinking-thinking-effort']",
  );

  function applyPressedState(selector, selectedValue) {
    document.querySelectorAll(selector).forEach((element) => {
      const key = element.getAttribute("data-mode");
      const selected = key === selectedValue;
      element.setAttribute("aria-pressed", selected ? "true" : "false");
      if (element.getAttribute("role") === "menuitemradio") {
        element.setAttribute("aria-checked", selected ? "true" : "false");
      }
      element.dataset.selected = selected ? "true" : "false";
    });
  }

  function syncModernMenuOption(selector, selected) {
    const element = document.querySelector(selector);
    if (!(element instanceof HTMLElement)) {
      return;
    }
    element.setAttribute("aria-checked", selected ? "true" : "false");
    element.dataset.selected = selected ? "true" : "false";
  }

  function syncSelectedState() {
    const selectedMode = document.body.dataset.selectedMode;
    applyPressedState("[data-mode]", selectedMode);

    document.querySelectorAll("[data-model]").forEach((element) => {
      const selected =
        element.getAttribute("data-model") ===
        document.body.dataset.selectedModel;
      element.setAttribute("aria-pressed", selected ? "true" : "false");
      element.dataset.selected = selected ? "true" : "false";
    });
    document.querySelectorAll("[data-effort]").forEach((element) => {
      const selected =
        element.getAttribute("data-effort") ===
        document.body.dataset.selectedEffort;
      element.setAttribute("aria-pressed", selected ? "true" : "false");
      element.dataset.selected = selected ? "true" : "false";
    });
    const deepResearchNav = document.querySelector("#deep-research-nav");
    if (deepResearchNav instanceof HTMLElement) {
      const selected = selectedMode === "research" || isDeepResearchLanding();
      deepResearchNav.dataset.selected = selected ? "true" : "false";
      if (selected) {
        deepResearchNav.setAttribute("aria-current", "page");
      } else {
        deepResearchNav.removeAttribute("aria-current");
      }
    }

    syncModernMenuOption(
      "#model-option-instant",
      document.body.dataset.selectedModel === "instant",
    );
    syncModernMenuOption(
      "#model-option-thinking",
      document.body.dataset.selectedModel === "thinking",
    );
    syncModernMenuOption("#model-option-pro", false);
    syncModernMenuOption(
      "#effort-option-standard",
      document.body.dataset.selectedEffort === "standard",
    );
    syncModernMenuOption(
      "#effort-option-extended",
      document.body.dataset.selectedEffort === "extended",
    );

    const modelMenu = document.querySelector("#tablinq-model-menu");
    const modelMenuOpen =
      modelPickerTrigger?.getAttribute("aria-expanded") === "true";
    if (
      variant === "composer-right" &&
      modelPickerTrigger instanceof HTMLElement
    ) {
      modelPickerTrigger.textContent =
        document.body.dataset.selectedModel === "thinking"
          ? `5.4 ${document.body.dataset.selectedEffort === "extended" ? "Extended" : "Standard"}`
          : "Instant";
    }
    modelMenu?.toggleAttribute("hidden", !modelMenuOpen);
    modelMenu
      ?.querySelectorAll("[role='menuitemradio'], [role='menuitem']")
      .forEach((element) => {
        element.toggleAttribute("hidden", !modelMenuOpen);
      });

    const currentUiVariant = variant === "live" || variant === "menu";
    const effortVisible =
      hasControl("effort") &&
      (!currentUiVariant || document.body.dataset.selectedModel === "thinking");
    const effortToggle = document.querySelector("#thinking-effort-trigger");
    const effortToggleIsMenuTrigger =
      effortToggle instanceof HTMLElement &&
      effortToggle.getAttribute("aria-haspopup") !== null;
    if (effortToggleIsMenuTrigger) {
      effortToggle.toggleAttribute("hidden", !effortVisible);
      effortToggle.textContent =
        document.body.dataset.selectedEffort === "extended"
          ? "Extended thinking"
          : "Thinking";
      if (!effortVisible) {
        effortToggle.setAttribute("aria-expanded", "false");
      }
    }
    const effortMenu = document.querySelector("#tablinq-effort-menu");
    const effortMenuOpen =
      (effortVisible &&
        effortToggleIsMenuTrigger &&
        effortToggle.getAttribute("aria-expanded") === "true") ||
      (nestedEffortTrigger instanceof HTMLElement &&
        nestedEffortTrigger.getAttribute("aria-expanded") === "true");
    effortMenu?.toggleAttribute("hidden", !effortMenuOpen);
    effortMenu
      ?.querySelectorAll("[role='menuitemradio']")
      .forEach((element) => {
        element.toggleAttribute("hidden", !effortMenuOpen);
      });

    const searchSelected = selectedMode === "search";
    const searchChip = document.querySelector("#search-chip");
    if (searchChip instanceof HTMLElement) {
      searchChip.toggleAttribute("hidden", !searchSelected);
      searchChip.setAttribute(
        "aria-label",
        searchSelected ? "Search, click to remove" : "Search",
      );
      searchChip.dataset.selected = searchSelected ? "true" : "false";
    }
    const toolsMenu = document.querySelector("#tablinq-tools-menu");
    const toolsMenuOpen =
      composerToolsTrigger?.getAttribute("aria-expanded") === "true";
    toolsMenu?.toggleAttribute("hidden", !toolsMenuOpen);
    toolsMenu
      ?.querySelectorAll("[role='menuitemradio'], [role='menuitem']")
      .forEach((element) => {
        element.toggleAttribute("hidden", !toolsMenuOpen);
      });
    syncModernMenuOption("#tool-option-web-search", searchSelected);
  }

  function registerSelectionEvents() {
    modelPickerTrigger?.addEventListener("click", () => {
      document.body.dataset.modelMenuClicks = String(
        Number.parseInt(document.body.dataset.modelMenuClicks || "0", 10) + 1,
      );
      const expanded =
        modelPickerTrigger.getAttribute("aria-expanded") === "true";
      modelPickerTrigger.setAttribute(
        "aria-expanded",
        expanded ? "false" : "true",
      );
      syncSelectedState();
    });

    document
      .querySelector("#model-option-instant")
      ?.addEventListener("click", () => {
        document.body.dataset.modelOptionClicks = String(
          Number.parseInt(document.body.dataset.modelOptionClicks || "0", 10) +
            1,
        );
        document.body.dataset.selectedModel = "instant";
        modelPickerTrigger?.setAttribute("aria-expanded", "false");
        effortPickerTrigger?.setAttribute("aria-expanded", "false");
        syncSelectedState();
      });

    document
      .querySelector("#model-option-thinking")
      ?.addEventListener("click", () => {
        document.body.dataset.modelOptionClicks = String(
          Number.parseInt(document.body.dataset.modelOptionClicks || "0", 10) +
            1,
        );
        document.body.dataset.selectedModel = "thinking";
        modelPickerTrigger?.setAttribute("aria-expanded", "false");
        syncSelectedState();
      });

    effortPickerTrigger?.addEventListener("click", () => {
      document.body.dataset.effortMenuClicks = String(
        Number.parseInt(document.body.dataset.effortMenuClicks || "0", 10) + 1,
      );
      const expanded =
        effortPickerTrigger.getAttribute("aria-expanded") === "true";
      effortPickerTrigger.setAttribute(
        "aria-expanded",
        expanded ? "false" : "true",
      );
      syncSelectedState();
    });

    nestedEffortTrigger?.addEventListener("click", (event) => {
      event.stopPropagation();
      document.body.dataset.effortMenuClicks = String(
        Number.parseInt(document.body.dataset.effortMenuClicks || "0", 10) + 1,
      );
      const expanded =
        nestedEffortTrigger.getAttribute("aria-expanded") === "true";
      nestedEffortTrigger.setAttribute(
        "aria-expanded",
        expanded ? "false" : "true",
      );
      syncSelectedState();
    });

    composerToolsTrigger?.addEventListener("click", () => {
      document.body.dataset.toolsMenuClicks = String(
        Number.parseInt(document.body.dataset.toolsMenuClicks || "0", 10) + 1,
      );
      const expanded =
        composerToolsTrigger.getAttribute("aria-expanded") === "true";
      composerToolsTrigger.setAttribute(
        "aria-expanded",
        expanded ? "false" : "true",
      );
      syncSelectedState();
    });

    document
      .querySelector("#tool-option-web-search")
      ?.addEventListener("click", () => {
        const searchToolClicks =
          Number.parseInt(document.body.dataset.searchToolClicks || "0", 10) +
          1;
        document.body.dataset.searchToolClicks = String(searchToolClicks);
        const ignoredClicks = Number.parseInt(
          config.params.get("search_clicks_before_activate") || "0",
          10,
        );
        if (searchToolClicks <= ignoredClicks) {
          composerToolsTrigger?.setAttribute("aria-expanded", "false");
          syncSelectedState();
          return;
        }
        document.body.dataset.selectedMode = "search";
        if (config.searchResetsModelOnActivate) {
          document.body.dataset.selectedModel = "instant";
        }
        composerToolsTrigger?.setAttribute("aria-expanded", "false");
        syncSelectedState();
      });

    document.querySelector("#search-chip")?.addEventListener("click", () => {
      document.body.dataset.selectedMode = "normal";
      syncSelectedState();
    });

    document
      .querySelector("#effort-option-standard")
      ?.addEventListener("click", () => {
        document.body.dataset.effortOptionClicks = String(
          Number.parseInt(document.body.dataset.effortOptionClicks || "0", 10) +
            1,
        );
        document.body.dataset.selectedEffort = "standard";
        if (nestedEffortTrigger?.getAttribute("aria-expanded") === "true") {
          document.body.dataset.selectedModel = "thinking";
        }
        effortPickerTrigger?.setAttribute("aria-expanded", "false");
        nestedEffortTrigger?.setAttribute("aria-expanded", "false");
        syncSelectedState();
      });

    document
      .querySelector("#effort-option-extended")
      ?.addEventListener("click", () => {
        document.body.dataset.effortOptionClicks = String(
          Number.parseInt(document.body.dataset.effortOptionClicks || "0", 10) +
            1,
        );
        document.body.dataset.selectedEffort = "extended";
        if (nestedEffortTrigger?.getAttribute("aria-expanded") === "true") {
          document.body.dataset.selectedModel = "thinking";
        }
        effortPickerTrigger?.setAttribute("aria-expanded", "false");
        nestedEffortTrigger?.setAttribute("aria-expanded", "false");
        syncSelectedState();
      });

    document.querySelectorAll("[data-model]").forEach((element) => {
      element.addEventListener("click", () => {
        const selectedModel = element.getAttribute("data-model");
        if (!selectedModel) {
          return;
        }
        document.body.dataset.selectedModel = selectedModel;
        syncSelectedState();
      });
    });

    document.querySelectorAll("[data-effort]").forEach((element) => {
      element.addEventListener("click", () => {
        const selectedEffort = element.getAttribute("data-effort");
        if (!selectedEffort) {
          return;
        }
        document.body.dataset.selectedEffort = selectedEffort;
        syncSelectedState();
      });
    });

    document.querySelectorAll("[data-mode]").forEach((element) => {
      element.addEventListener("click", () => {
        document.body.dataset.selectedMode = element.getAttribute("data-mode");
        if (
          usesDeepResearchLanding &&
          document.body.dataset.selectedMode === "research"
        ) {
          enterDeepResearchLanding();
        }
        syncSelectedState();
      });
    });

    document
      .querySelector("#deep-research-nav")
      ?.addEventListener("click", (event) => {
        event.preventDefault();
        document.body.dataset.selectedMode = "research";
        enterDeepResearchLanding();
        syncSelectedState();
      });
  }

  return {
    registerSelectionEvents,
    syncSelectedState,
  };
}
