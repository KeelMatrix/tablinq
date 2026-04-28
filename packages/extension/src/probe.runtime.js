export {
  accessibleProbeName,
  computedStyleForProbeElement,
  createProbeNeighborhood,
  firstVisibleProbeElement,
  implicitProbeRole,
  isVisibleProbeElement,
  normalizeProbeText,
  probeElementPath,
  probeNearbyText,
  probeTextHasToken,
} from "./probeRuntime/text.js";

export {
  controlCandidateSelector,
  orderCapabilityStrategies,
} from "./probeRuntime/strategies.js";

export {
  addProbeAlias,
  captureProbeAliases,
  createAliasBuckets,
} from "./probeRuntime/aliases.js";

export { probeDocument } from "./probeRuntime/documentProbe.js";
