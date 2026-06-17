export function createAliasBuckets() {
  return {
    modes: {},
    model_classes: {},
    efforts: {},
  };
}

export function addProbeAlias(aliases, bucket, key, label) {
  if (!label) {
    return;
  }
  const current = aliases[bucket][key] ?? [];
  if (!current.includes(label)) {
    aliases[bucket][key] = [...current, label];
  }
}

export function captureProbeAliases({
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
    for (const name of ["standard", "extended", "medium", "high"]) {
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
      if (hasToken(name, ["thinking", "medium", "high"])) {
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
      if (hasToken(name, ["extended", "high"])) {
        addProbeAlias(aliases, "efforts", "extended", name);
      }
      if (hasToken(name, ["standard", "medium"])) {
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
