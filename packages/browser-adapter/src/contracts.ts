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
  | "observed"
  | "not_observed"
  | "unknown"
  | "blocked";

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

export interface CapabilityReportPayload {
  reported_at: string;
  selector_pack_id: string;
  selector_pack_version: string;
  matrix: CapabilityMatrixSnapshot;
  aliases: RuntimeAliasesSnapshot;
  observed_ui: CapabilityObservedUiSnapshot;
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

export interface CapabilityBridgeMessage {
  type: "capability_report";
  report: CapabilityReportPayload;
  runtime: {
    ui_signature: string;
    strategy_matches: Partial<Record<CapabilityKey, string>>;
    dom_neighborhoods: DomNeighborhoodSnapshot[];
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

export interface BrowserOpenCommand {
  targetUrl: string;
  browserPath?: string | null;
  headless?: boolean;
  forceReload?: boolean;
}

export interface AskExecutionCommand {
  jobId: string;
  prompt: string;
  conversationId?: string | null;
  newConversation?: boolean;
  timeoutMs?: number;
  submitJitterMs?: number;
  mode?: "normal" | "search";
  modelClass?: "instant" | "thinking" | null;
  effort?: "none" | "standard" | "extended" | null;
  files?: string[];
}

export interface ResearchExecutionCommand {
  jobId: string;
  query: string;
  conversationId?: string | null;
  files?: string[];
  outputDir?: string | null;
  timeoutMs?: number;
  submitJitterMs?: number;
}

export interface ArtifactMetadataSnapshot {
  artifact_id: string;
  conversation_id: string | null;
  message_id: string | null;
  file_name: string;
  content_type: string;
  size_bytes: number;
  sha256: string;
  storage_path: string;
}

export interface AskResultSnapshot {
  job_id: string;
  conversation_id: string;
  message_id: string;
  output_text: string;
  artifacts: ArtifactMetadataSnapshot[];
  completed_at: string;
  current_url: string;
}

export interface ResearchProgressSnapshot {
  status: "starting" | "waiting_for_sources" | "running";
  message?: string;
  conversation_id: string | null;
  message_id: string | null;
  current_url: string;
  chunk_kind?: "delta" | "status";
  chunk_text?: string;
}

export interface ResearchResultSnapshot {
  job_id: string;
  conversation_id: string;
  message_id: string;
  report_text: string;
  artifacts: ArtifactMetadataSnapshot[];
  completed_at: string;
  current_url: string;
  output_path?: string | null;
}

export interface StreamChunkSnapshot {
  job_id: string;
  conversation_id: string | null;
  message_id: string | null;
  seq: number;
  kind: "delta" | "done";
  text: string;
}

export interface RequestStateSnapshot {
  state: "running" | "interrupted" | "recovered";
  reason: string;
  at: string;
  conversation_id: string | null;
  message_id: string | null;
  current_url: string | null;
  streaming_safe: boolean;
  baseline_conversation_id?: string | null;
  baseline_message_id?: string | null;
  baseline_assistant_message_count?: number | null;
  baseline_text?: string | null;
}

export interface BrowserAdapterOptions {
  profileDir: string;
  downloadsDir: string;
  defaultTargetUrl: string;
  defaultBrowserPath?: string | null;
  extensionDir?: string | null;
  connectionMode: "attach_first" | "managed_fallback" | "managed_only";
  debugEndpoint?: string | null;
  extensionRequired?: boolean;
  autoOpenTargetTab?: boolean;
}

export interface BrowserStatusSnapshot {
  running: boolean;
  browser_kind: string;
  profile_dir: string;
  downloads_dir: string;
  target_url: string;
  current_url: string | null;
  page_title: string | null;
  page_ready: boolean;
  authenticated: boolean;
  reload_count: number;
  screenshot_path: string | null;
  browser_path: string | null;
  extension_connected: boolean;
  capability_report: CapabilityReportPayload | null;
  attach_mode: "attach_first" | "managed_fallback" | "managed_only";
  browser_owner: "external" | "managed";
  cdp_attached: boolean;
  target_tab_found: boolean;
  target_tab_id: string | null;
  target_workspace_key: string | null;
  debug_endpoint: string | null;
  auth_source: "page" | "extension" | "unknown";
  extension_required: boolean;
  last_capability_report_at: string | null;
  action?: string | null;
  browser_process_closed?: boolean | null;
  reason?: string | null;
}

export interface BrowserLaunchConfig {
  browserKind: string;
  channel?: "msedge" | "chrome";
  executablePath?: string;
}

export interface StrategyAttemptSnapshot {
  capability: string;
  strategy_id: string;
  score: number;
  outcome: string;
  detail: string | null;
}

export interface StrategyTelemetrySnapshot {
  capability: string;
  strategy_id: string;
  success_count: number;
  failure_count: number;
  success_rate: number;
  last_used_at: string | null;
}

export interface SelectorPackSummarySnapshot {
  pack_id: string;
  version: string;
  updated_at: string | null;
  path: string | null;
}

export interface BrowserDiagnosticsSnapshot {
  current_url: string | null;
  capability_matrix: CapabilityMatrixSnapshot;
  strategy_attempts: StrategyAttemptSnapshot[];
  dom_snapshot: string;
  aria_snapshot: string;
  screenshot_path: string | null;
  selector_pack: SelectorPackSummarySnapshot | null;
  strategy_telemetry: StrategyTelemetrySnapshot[];
  browser_detected: boolean;
  detected_browser_path: string | null;
  profile_accessible: boolean;
  profile_access_error: string | null;
}
