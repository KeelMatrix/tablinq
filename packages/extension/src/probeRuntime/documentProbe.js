import { captureProbeAliases, createAliasBuckets } from "./aliases.js";
import { createCapabilityDefinitions } from "./capabilityDefinitions.js";
import { createCapabilityPredicates } from "./capabilityPredicates.js";
import { createProbeContext } from "./probeContext.js";
import { orderCapabilityStrategies } from "./strategies.js";

export function probeDocument(options = {}) {
  const selectorPackId = options.selectorPackId ?? "chatgpt-default";
  const selectorPackVersion = options.selectorPackVersion ?? "v1";
  const reportedAt = options.reportedAt ?? new Date().toISOString();
  const preferredStrategies = options.preferredStrategies ?? {};
  const selectorPack = options.selectorPack ?? null;

  const context = createProbeContext(document);
  const predicates = createCapabilityPredicates(context);
  const { definitions, createGenericStrategyFromPackEntry } =
    createCapabilityDefinitions(context, predicates);
  const aliases = createAliasBuckets();

  const captureAliases = (capability, element) =>
    captureProbeAliases({
      aliases,
      capability,
      element,
      accessibleName: context.accessibleName,
      hasToken: context.hasToken,
      visiblePopupRootsFor: predicates.visiblePopupRootsFor,
      isVisible: context.isVisible,
      isModelOption: predicates.isModelOption,
      isEffortOption: predicates.isEffortOption,
      isModelPickerTrigger: predicates.isModelPickerTrigger,
      isSearchModeControl: predicates.isSearchModeControl,
      isResearchModeControl: predicates.isResearchModeControl,
      popupOptionSelector: predicates.popupOptionSelector,
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

  const uiSignature = [
    window.location.hostname || "unknown-host",
    context.body?.dataset.uiVariant || "unlabelled",
    context.body?.querySelector("#prompt-textarea")
      ? "prompt-textarea"
      : "no-prompt-id",
    context.body?.querySelector("[data-slot]") ? "slot-ui" : "aria-ui",
    context.body?.querySelector("[role='textbox']")
      ? "role-textbox"
      : "native-textbox",
  ].join(":");
  const strategyMatches = {};
  const domNeighborhoods = [];

  const orderStrategies = (capability, strategies) =>
    orderCapabilityStrategies({
      capability,
      strategies,
      selectorPack,
      preferredStrategies,
      createGenericStrategy: (entry) =>
        createGenericStrategyFromPackEntry(capability, entry),
    });

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

    strategyMatches[definition.key] = strategyId;
    domNeighborhoods.push(
      context.createNeighborhood(definition.key, strategyId, match),
    );
    captureAliases(definition.key, match);
  }

  const observedUi = createObservedUiState({
    context,
    predicates,
    matrix,
  });

  return buildCapabilityProbeReport({
    reportedAt,
    selectorPackId,
    selectorPackVersion,
    matrix,
    aliases,
    observedUi,
    uiSignature,
    strategyMatches,
    domNeighborhoods,
  });
}

function buildCapabilityProbeReport({
  reportedAt,
  selectorPackId,
  selectorPackVersion,
  matrix,
  aliases,
  observedUi,
  uiSignature,
  strategyMatches,
  domNeighborhoods,
}) {
  return {
    type: "capability_report",
    report: {
      reported_at: reportedAt,
      selector_pack_id: selectorPackId,
      selector_pack_version: selectorPackVersion,
      matrix,
      aliases,
      observed_ui: observedUi,
    },
    runtime: {
      ui_signature: uiSignature,
      strategy_matches: strategyMatches,
      dom_neighborhoods: domNeighborhoods,
    },
  };
}

function createObservedUiState({ context, predicates, matrix }) {
  const model = observeSelectedModel({ context, predicates, matrix });
  return {
    composer_readiness: observeBinary(matrix.composer_ready),
    submit_availability: observeBinary(matrix.submit_available),
    selected_model: model,
    thinking_effort: observeThinkingEffort({
      context,
      predicates,
      matrix,
      model,
    }),
    search_mode: observeSearchMode({ context, predicates, matrix }),
    research_flow: observeResearchFlow({ context, predicates, matrix }),
    upload_availability: observeBinary(matrix.upload_control_visible),
  };
}

function observeBinary(observed) {
  return {
    status: observed ? "observed" : "not_observed",
    observed,
  };
}

function observeSelectedModel({ context, predicates, matrix }) {
  const labels = collectSelectedModelLabels(context, predicates);
  const hasUnsupportedPro = labels.some((label) => label.includes("pro"));
  const hasInstant = labels.some(
    (label) => label.includes("instant") || label.includes("5.3"),
  );
  const hasThinking = labels.some(
    (label) =>
      !label.includes("pro") &&
      (label.includes("thinking") ||
        label.includes("standard") ||
        label.includes("extended") ||
        label.includes("5.4")),
  );
  const hasAmbiguousFiveFive = labels.some(
    (label) =>
      label.includes("5.5") &&
      !label.includes("pro") &&
      !label.includes("instant") &&
      !label.includes("thinking") &&
      !label.includes("standard") &&
      !label.includes("extended") &&
      !label.includes("5.3") &&
      !label.includes("5.4"),
  );
  const ambiguous =
    hasAmbiguousFiveFive ||
    (hasInstant && hasThinking) ||
    (!hasUnsupportedPro && labels.length > 0 && !hasInstant && !hasThinking);

  let selectedModelClass = null;
  if (!hasUnsupportedPro && !ambiguous) {
    if (hasInstant) {
      selectedModelClass = "instant";
    } else if (hasThinking) {
      selectedModelClass = "thinking";
    }
  }

  return {
    status: hasUnsupportedPro
      ? "blocked"
      : ambiguous
        ? "unknown"
        : selectedModelClass
          ? "observed"
          : matrix.model_picker_visible
            ? "not_observed"
            : "not_observed",
    picker_visible: matrix.model_picker_visible,
    selected_model_class: selectedModelClass,
    selected_label: selectedModelClass
      ? (labels.find((label) =>
          selectedModelClass === "instant"
            ? label.includes("instant") || label.includes("5.3")
            : label.includes("thinking") ||
              label.includes("standard") ||
              label.includes("extended") ||
              label.includes("5.4"),
        ) ?? null)
      : null,
    labels,
    unsupported_pro_selected: hasUnsupportedPro,
    ambiguous_selection: ambiguous,
  };
}

function observeThinkingEffort({ context, predicates, matrix, model }) {
  const labels = collectSelectedEffortLabels(context, predicates);
  const selectedEffort = labels.some((label) => label.includes("extended"))
    ? "extended"
    : labels.some((label) => label.includes("standard"))
      ? "standard"
      : null;
  const status =
    model.status === "blocked"
      ? "blocked"
      : selectedEffort
        ? "observed"
        : matrix.thinking_effort_visible || labels.length > 0
          ? "unknown"
          : "not_observed";
  return {
    status,
    controls_visible: matrix.thinking_effort_visible,
    selected_effort: selectedEffort,
    labels,
  };
}

function observeSearchMode({ context, predicates, matrix }) {
  const labels = collectVisibleLabels(
    context,
    Array.from(
      context.documentRoot.querySelectorAll("button, [role='button']"),
    ).filter((element) => predicates.isSearchModeControl(element)),
  );
  const selected =
    Array.from(
      context.documentRoot.querySelectorAll("button, [role='button']"),
    ).some(
      (element) =>
        predicates.isSearchChip(element) || isSelectedControl(element),
    ) &&
    Array.from(
      context.documentRoot.querySelectorAll("button, [role='button']"),
    ).some((element) => predicates.isSearchChip(element));
  return {
    status: matrix.search_mode_visible ? "observed" : "not_observed",
    visible: matrix.search_mode_visible,
    selected,
    labels,
  };
}

function observeResearchFlow({ context, predicates, matrix }) {
  const labels = collectVisibleLabels(
    context,
    Array.from(
      context.documentRoot.querySelectorAll("a, button, [role='button']"),
    ).filter((element) => predicates.isResearchModeControl(element)),
  );
  const onResearchUrl = window.location.pathname
    .toLowerCase()
    .split("/")
    .filter(Boolean)
    .includes("deep-research");
  return {
    status:
      matrix.research_mode_visible || onResearchUrl
        ? "observed"
        : "not_observed",
    entrypoint_visible: matrix.research_mode_visible,
    on_research_url: onResearchUrl,
    labels,
  };
}

function collectSelectedModelLabels(context, predicates) {
  const explicitSelections = Array.from(
    context.documentRoot.querySelectorAll(
      "[data-model], [role='menuitemradio'], [role='option']",
    ),
  ).filter(
    (element) =>
      predicates.isModelOption(element) && isSelectedControl(element),
  );
  const trigger = predicates.findModelPickerTrigger();
  const composerPills = Array.from(
    context.documentRoot.querySelectorAll(
      "#thinking-effort-trigger, button.__composer-pill[aria-haspopup='menu']",
    ),
  ).filter(
    (element) =>
      context.isVisible(element) && context.isComposerAdjacent(element),
  );
  return collectVisibleLabels(context, [
    ...explicitSelections,
    ...(trigger ? [trigger] : []),
    ...composerPills,
  ]).filter((label) =>
    context.hasToken(label, [
      "instant",
      "thinking",
      "pro",
      "standard",
      "extended",
      "5.3",
      "5.4",
      "5.5",
      "latest",
    ]),
  );
}

function collectSelectedEffortLabels(context, predicates) {
  const explicitSelections = Array.from(
    context.documentRoot.querySelectorAll(
      "[data-effort], [role='menuitemradio'], [role='option']",
    ),
  ).filter(
    (element) =>
      predicates.isEffortOption(element) && isSelectedControl(element),
  );
  const trigger = predicates.findEffortPickerTrigger();
  const composerPills = Array.from(
    context.documentRoot.querySelectorAll(
      "#thinking-effort-trigger, button.__composer-pill[aria-haspopup='menu']",
    ),
  ).filter(
    (element) =>
      context.isVisible(element) && context.isComposerAdjacent(element),
  );
  return collectVisibleLabels(context, [
    ...explicitSelections,
    ...(trigger ? [trigger] : []),
    ...composerPills,
  ]).filter((label) =>
    context.hasToken(label, ["standard", "extended", "thinking"]),
  );
}

function collectVisibleLabels(context, elements) {
  const labels = [];
  for (const element of elements) {
    if (!(element instanceof HTMLElement) || !context.isVisible(element)) {
      continue;
    }
    const label = context.normalizeText(context.accessibleName(element));
    if (label && !labels.includes(label)) {
      labels.push(label);
    }
  }
  return labels;
}

function isSelectedControl(element) {
  return (
    element instanceof HTMLElement &&
    (element.getAttribute("aria-pressed") === "true" ||
      element.getAttribute("aria-selected") === "true" ||
      element.getAttribute("data-selected") === "true" ||
      element.dataset.selected === "true")
  );
}
