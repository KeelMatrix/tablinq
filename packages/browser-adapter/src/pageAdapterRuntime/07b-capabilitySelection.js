function createCapabilitySelectionRuntime({
  domUtils,
  discoveryRuntime,
  reportHelpers,
}) {
  const { isSelected, clickControl, findByAliases } = domUtils;
  const {
    expandAliases,
    throwCapabilityMissing,
    throwUiDrift,
    mergeAliasBuckets,
    modelDefaults,
    modelFallbackTokens,
    effortDefaults,
    effortFallbackTokens,
  } = reportHelpers;
  const {
    sleep,
    isModelOption,
    isEffortOption,
    isSearchMenuItem,
    isResearchModeControl,
    openPopupRootsFor,
    findConcreteSearchControl,
    findComposerToolsTrigger,
    findComposerModelChipRemove,
    findComposerModelChip,
    findConcreteModelControl,
    findModelPickerTrigger,
    findOpenModelMenuControl,
    discoverVisibleModelAliases,
    findConcreteEffortControl,
    findEffortPickerTrigger,
    openNestedThinkingEffortRoots,
    findOpenEffortMenuControl,
    discoverVisibleEffortAliases,
  } = discoveryRuntime;

  const ensureMode = async (report, mode) => {
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

    const control =
      mode === "search"
        ? findConcreteSearchControl(report)
        : findByAliases({
            selectors: [
              "a[data-testid='deep-research-sidebar-item']",
              "a[href='/deep-research']",
              "a[href$='/deep-research']",
            ],
            aliases: expandAliases(report, "modes", mode, [
              mode,
              "deep research",
            ]),
            fallbackTokens: ["deep research"],
            include: isResearchModeControl,
          });
    if (!control && mode === "search") {
      const trigger = findComposerToolsTrigger();
      if (trigger) {
        const roots = await openPopupRootsFor(trigger, isSearchMenuItem);
        const openedControl = findConcreteSearchControl(report, roots);
        if (!openedControl) {
          throwUiDrift(`could not find ${mode} mode control after probing it`);
        }
        if (!isSelected(openedControl) && !clickControl(openedControl)) {
          throwUiDrift(`could not activate ${mode} mode control`);
        }
        return;
      }
    }
    if (!control) {
      throwUiDrift(`could not find ${mode} mode control after probing it`);
    }
    if (!isSelected(control) && !clickControl(control)) {
      throwUiDrift(`could not activate ${mode} mode control`);
    }
  };

  const ensureModelClass = async (report, modelClass) => {
    if (modelClass === "instant") {
      const modelChipRemove = findComposerModelChipRemove();
      if (modelChipRemove) {
        if (!clickControl(modelChipRemove)) {
          throwUiDrift("could not clear the active thinking model chip");
        }
        await sleep(150);
        return;
      }
    }
    if (visibleComposerModelMatches(modelClass)) {
      return;
    }
    let control = findConcreteModelControl(report, modelClass);
    if (control) {
      if (!isSelected(control) && !clickControl(control)) {
        throwUiDrift(`could not activate model=${modelClass}`);
      }
      await sleep(150);
      return;
    }
    if (modelClass === "instant" && !findComposerModelChip()) {
      return;
    }
    if (!report.matrix.model_picker_visible) {
      throwCapabilityMissing(
        "model picker",
        modelClass,
        report.aliases.model_classes,
      );
    }
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
    if (visibleComposerEffortMatches(effort)) {
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
      const roots = await openNestedThinkingEffortRoots();
      control =
        findConcreteEffortControl(report, effort, roots) ??
        findOpenEffortMenuControl(report, effort) ??
        findConcreteEffortControl(report, effort);
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
    ensureMode,
    ensureModelClass,
    ensureEffort,
  };
}

function visibleComposerPillTexts() {
  return Array.from(
    document.querySelectorAll(
      "button.__composer-pill[aria-haspopup='menu']:not([aria-label])",
    ),
  )
    .filter((element) => {
      if (!(element instanceof HTMLElement)) {
        return false;
      }
      const style = window.getComputedStyle(element);
      return (
        style.display !== "none" &&
        style.visibility !== "hidden" &&
        style.opacity !== "0" &&
        element.getClientRects().length > 0
      );
    })
    .map((element) =>
      (element.textContent ?? "").replace(/\s+/g, " ").trim().toLowerCase(),
    );
}

function visibleComposerModelMatches(modelClass) {
  return visibleComposerPillTexts().some((text) => {
    if (modelClass === "instant") {
      return text.includes("instant") || text.includes("5.3");
    }
    return (
      text.includes("thinking") ||
      text.includes("standard") ||
      text.includes("extended") ||
      text.includes("5.4") ||
      text.includes("5.5")
    );
  });
}

function visibleComposerEffortMatches(effort) {
  return visibleComposerPillTexts().some((text) => text.includes(effort));
}
