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

  const reportHelpers = {
    expandAliases,
    controlStateDetail,
    throwCapabilityMissing,
    throwUiDrift,
    mergeAliasBuckets,
    modelDefaults,
    modelFallbackTokens,
    effortDefaults,
    effortFallbackTokens,
  };
  const discoveryRuntime = createCapabilityDiscoveryRuntime({
    textUtils,
    domUtils,
    composerRuntime,
    reportHelpers,
  });
  const selectionRuntime = createCapabilitySelectionRuntime({
    domUtils,
    discoveryRuntime,
    reportHelpers,
  });

  return {
    expandAliases,
    controlStateDetail,
    throwCapabilityMissing,
    throwUiDrift,
    mergeAliasBuckets,
    ensureMode: selectionRuntime.ensureMode,
    ensureModelClass: selectionRuntime.ensureModelClass,
    ensureEffort: selectionRuntime.ensureEffort,
    discoverVisibleModelAliases: discoveryRuntime.discoverVisibleModelAliases,
    discoverVisibleEffortAliases: discoveryRuntime.discoverVisibleEffortAliases,
  };
}
