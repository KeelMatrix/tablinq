export type CapabilityKey =
  | "composer_ready"
  | "submit_available"
  | "model_picker_visible"
  | "thinking_effort_visible"
  | "search_mode_visible"
  | "research_mode_visible"
  | "upload_control_visible"
  | "downloadable_artifacts_observable";

export interface CapabilityMatrixSnapshot {
  composer_ready: boolean;
  submit_available: boolean;
  model_picker_visible: boolean;
  thinking_effort_visible: boolean;
  search_mode_visible: boolean;
  research_mode_visible: boolean;
  upload_control_visible: boolean;
  downloadable_artifacts_observable: boolean;
}

export interface RuntimeAliasesSnapshot {
  modes: Record<string, string[]>;
  model_classes: Record<string, string[]>;
  efforts: Record<string, string[]>;
}

export type CapabilityObservationStatus =
  "observed" | "not_observed" | "unknown" | "blocked";

export interface BinaryCapabilityObservationSnapshot {
  status: CapabilityObservationStatus;
  observed: boolean;
}

export interface ModelSelectionObservationSnapshot {
  status: CapabilityObservationStatus;
  picker_visible: boolean;
  selected_model_class: string | null;
  selected_label: string | null;
  labels: string[];
  unsupported_pro_selected: boolean;
  ambiguous_selection: boolean;
}

export interface ThinkingEffortObservationSnapshot {
  status: CapabilityObservationStatus;
  controls_visible: boolean;
  selected_effort: string | null;
  labels: string[];
}

export interface SearchModeObservationSnapshot {
  status: CapabilityObservationStatus;
  visible: boolean;
  selected: boolean;
  labels: string[];
}

export interface ResearchFlowObservationSnapshot {
  status: CapabilityObservationStatus;
  entrypoint_visible: boolean;
  on_research_url: boolean;
  labels: string[];
}

export interface CapabilityObservedUiSnapshot {
  composer_readiness: BinaryCapabilityObservationSnapshot;
  submit_availability: BinaryCapabilityObservationSnapshot;
  selected_model: ModelSelectionObservationSnapshot;
  thinking_effort: ThinkingEffortObservationSnapshot;
  search_mode: SearchModeObservationSnapshot;
  research_flow: ResearchFlowObservationSnapshot;
  upload_availability: BinaryCapabilityObservationSnapshot;
}

export interface DomNeighborhoodSnapshot {
  capability: CapabilityKey;
  strategy_id: string;
  anchor_tag: string;
  anchor_role: string | null;
  anchor_name: string;
  anchor_path: string;
  parent_name: string | null;
  sibling_names: string[];
  nearby_text: string[];
}

export interface CapabilityReportPayload {
  reported_at: string;
  selector_pack_id: string;
  selector_pack_version: string;
  matrix: CapabilityMatrixSnapshot;
  aliases: RuntimeAliasesSnapshot;
  observed_ui: CapabilityObservedUiSnapshot;
}

export interface ProbeRuntimeSnapshot {
  ui_signature: string;
  strategy_matches: Partial<Record<CapabilityKey, string>>;
  dom_neighborhoods: DomNeighborhoodSnapshot[];
}

export interface CapabilityBridgeMessage {
  type: "capability_report";
  report: CapabilityReportPayload;
  runtime: ProbeRuntimeSnapshot;
  tab?: {
    id: number | null;
    url: string | null;
    workspace_key: string | null;
  };
}

export interface SelectorStrategySnapshot {
  strategy_id: string;
  kind: "aria_role" | "css" | "text" | "dom_neighborhood" | "hotkey";
  selector: string;
  score: number;
}

export interface SelectorPackSnapshot {
  format_version: number;
  pack_id: string;
  description: string;
  updated_at: string;
  capabilities: Partial<Record<CapabilityKey, SelectorStrategySnapshot[]>>;
}

export interface ProbeDocumentOptions {
  selectorPackId?: string;
  selectorPackVersion?: string;
  reportedAt?: string;
  preferredStrategies?: Partial<Record<CapabilityKey, string>>;
  selectorPack?: SelectorPackSnapshot;
}

import { probeDocument as runtimeProbeDocument } from "./probe.runtime.js";

export const probeDocument: (
  options?: ProbeDocumentOptions,
) => CapabilityBridgeMessage = runtimeProbeDocument as (
  options?: ProbeDocumentOptions,
) => CapabilityBridgeMessage;
