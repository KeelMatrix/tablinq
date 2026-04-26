export function createMenuRuntime({
  document,
  config,
  hasControl,
  isDeepResearchLanding,
  enterDeepResearchLanding,
}) {
  const { variant, usesDeepResearchLanding } = config;
  const modelPickerTrigger = document.querySelector("#model-switcher-trigger");
  const effortPickerTrigger = document.querySelector(
    "#thinking-effort-trigger",
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
      effortVisible &&
      effortToggleIsMenuTrigger &&
      effortToggle.getAttribute("aria-expanded") === "true";
    effortMenu?.toggleAttribute("hidden", !effortMenuOpen);
    effortMenu
      ?.querySelectorAll("[role='menuitemradio']")
      .forEach((element) => {
        element.toggleAttribute("hidden", !effortMenuOpen);
      });
  }

  function registerSelectionEvents() {
    modelPickerTrigger?.addEventListener("click", () => {
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
        document.body.dataset.selectedModel = "instant";
        modelPickerTrigger?.setAttribute("aria-expanded", "false");
        effortPickerTrigger?.setAttribute("aria-expanded", "false");
        syncSelectedState();
      });

    document
      .querySelector("#model-option-thinking")
      ?.addEventListener("click", () => {
        document.body.dataset.selectedModel = "thinking";
        modelPickerTrigger?.setAttribute("aria-expanded", "false");
        syncSelectedState();
      });

    effortPickerTrigger?.addEventListener("click", () => {
      const expanded =
        effortPickerTrigger.getAttribute("aria-expanded") === "true";
      effortPickerTrigger.setAttribute(
        "aria-expanded",
        expanded ? "false" : "true",
      );
      syncSelectedState();
    });

    document
      .querySelector("#effort-option-standard")
      ?.addEventListener("click", () => {
        document.body.dataset.selectedEffort = "standard";
        effortPickerTrigger?.setAttribute("aria-expanded", "false");
        syncSelectedState();
      });

    document
      .querySelector("#effort-option-extended")
      ?.addEventListener("click", () => {
        document.body.dataset.selectedEffort = "extended";
        effortPickerTrigger?.setAttribute("aria-expanded", "false");
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
