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

  return buildCapabilityProbeReport({
    reportedAt,
    selectorPackId,
    selectorPackVersion,
    matrix,
    aliases,
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
    },
    runtime: {
      ui_signature: uiSignature,
      strategy_matches: strategyMatches,
      dom_neighborhoods: domNeighborhoods,
    },
  };
}
