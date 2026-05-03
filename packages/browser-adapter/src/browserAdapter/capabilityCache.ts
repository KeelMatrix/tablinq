import { readFileSync, writeFileSync } from "node:fs";
import type {
  CapabilityBridgeMessage,
  CapabilityKey,
  CapabilityReportPayload,
  DomNeighborhoodSnapshot,
  SelectorPackSnapshot,
  StrategyAttemptSnapshot,
  StrategyTelemetrySnapshot,
} from "../contracts";
import { telemetryKey } from "../capabilityProbe";
import { workspaceKeyFromUrl } from "../urls";
import type { TargetPageState } from "./pageRegistry";

interface CapabilityCacheEntry {
  workspace_key: string;
  saved_at: string;
  ui_signature: string | null;
  preferred_strategies: Partial<Record<CapabilityKey, string>>;
  report: CapabilityReportPayload;
}

interface StrategyTelemetryStat {
  capability: CapabilityKey;
  strategy_id: string;
  success_count: number;
  failure_count: number;
  last_used_at: string | null;
}

interface CapabilityCacheFile {
  version: 2;
  workspaces: Record<string, CapabilityCacheEntry>;
  telemetry: StrategyTelemetryStat[];
}

const probedCapabilities = [
  "composer_ready",
  "submit_available",
  "model_picker_visible",
  "thinking_effort_visible",
  "search_mode_visible",
  "research_mode_visible",
  "upload_control_visible",
  "downloadable_artifacts_observable",
] satisfies CapabilityKey[];

export class CapabilityStrategyCache {
  private loaded = false;
  private cache: CapabilityCacheFile = {
    version: 2,
    workspaces: {},
    telemetry: [],
  };
  private selectorPack: SelectorPackSnapshot | null = null;
  private readonly strategyTelemetry = new Map<
    string,
    Partial<Record<CapabilityKey, string>>
  >();
  private readonly strategyTelemetryStats = new Map<
    string,
    StrategyTelemetryStat
  >();

  constructor(
    private readonly cachePath: string,
    readonly selectorPackPath: string,
  ) {}

  hydrate(): void {
    if (this.loaded) {
      return;
    }
    this.loaded = true;

    try {
      const parsed = JSON.parse(
        readFileSync(this.cachePath, "utf8"),
      ) as CapabilityCacheFile & {
        version?: number;
        telemetry?: StrategyTelemetryStat[];
      };
      if (!parsed?.workspaces) {
        this.resetCache();
        return;
      }
      this.cache = {
        version: 2,
        workspaces: parsed.workspaces,
        telemetry: parsed.telemetry ?? [],
      };
      for (const entry of Object.values(this.cache.workspaces)) {
        if (entry.ui_signature) {
          this.strategyTelemetry.set(
            entry.ui_signature,
            entry.preferred_strategies ?? {},
          );
        }
      }
      for (const stat of this.cache.telemetry) {
        this.strategyTelemetryStats.set(
          telemetryKey(stat.capability, stat.strategy_id),
          stat,
        );
      }
    } catch {
      this.resetCache();
    }
  }

  preferredStrategiesFor(
    pageState: TargetPageState,
    currentUrl: string,
  ): Partial<Record<CapabilityKey, string>> {
    this.hydrate();
    return pageState.ui_signature
      ? (this.strategyTelemetry.get(pageState.ui_signature) ?? {})
      : (this.cache.workspaces[workspaceKeyFromUrl(currentUrl)]
          ?.preferred_strategies ?? {});
  }

  recordCapabilitySuccess(
    currentUrl: string,
    message: CapabilityBridgeMessage,
    previousUiSignature: string | null,
  ): void {
    this.hydrate();
    this.strategyTelemetry.set(
      message.runtime.ui_signature,
      message.runtime.strategy_matches,
    );
    this.recordStrategyStats(
      message.runtime.strategy_matches,
      previousUiSignature,
    );
    this.persistCapabilitySnapshot(currentUrl, message);
  }

  ensureSelectorPackLoaded(): SelectorPackSnapshot | null {
    if (this.selectorPack) {
      return this.selectorPack;
    }

    try {
      const parsed = JSON.parse(
        readFileSync(this.selectorPackPath, "utf8"),
      ) as SelectorPackSnapshot;
      if (!parsed?.pack_id || parsed.format_version !== 1) {
        return null;
      }
      this.selectorPack = parsed;
      return this.selectorPack;
    } catch {
      return null;
    }
  }

  buildStrategyAttempts({
    lastCapabilityReport,
    lastUiSignature,
    lastDomNeighborhoods,
  }: {
    lastCapabilityReport: CapabilityReportPayload | null;
    lastUiSignature: string | null;
    lastDomNeighborhoods: DomNeighborhoodSnapshot[];
  }): StrategyAttemptSnapshot[] {
    const selectorPack = this.ensureSelectorPackLoaded();
    const matrix = lastCapabilityReport?.matrix;
    const matched = lastUiSignature
      ? (this.strategyTelemetry.get(lastUiSignature) ?? {})
      : {};
    const attempts: StrategyAttemptSnapshot[] = [];
    const capabilities = Object.keys(
      selectorPack?.capabilities ?? {},
    ) as CapabilityKey[];

    for (const capability of capabilities) {
      const strategies = selectorPack?.capabilities?.[capability] ?? [];
      const matchedStrategy = matched[capability];
      if (matchedStrategy) {
        const strategy = strategies.find(
          (entry) => entry.strategy_id === matchedStrategy,
        );
        attempts.push({
          capability,
          strategy_id: matchedStrategy,
          score: strategy?.score ?? 1,
          outcome: "matched",
          detail:
            lastDomNeighborhoods.find(
              (entry) =>
                entry.capability === capability &&
                entry.strategy_id === matchedStrategy,
            )?.anchor_path ?? null,
        });
        continue;
      }

      const first = strategies[0];
      if (!first) {
        continue;
      }

      if (!lastCapabilityReport) {
        attempts.push({
          capability,
          strategy_id: first.strategy_id,
          score: first.score,
          outcome: "missing",
          detail: "no capability report available after probe attempt",
        });
        continue;
      }

      if (matrix && !matrix[capability]) {
        attempts.push({
          capability,
          strategy_id: first.strategy_id,
          score: first.score,
          outcome: "missing",
          detail: "probe did not find a matching visible control",
        });
        continue;
      }

      if (matrix?.[capability]) {
        attempts.push({
          capability,
          strategy_id: first.strategy_id,
          score: first.score,
          outcome: "detected",
          detail: "capability visible but no strategy match recorded",
        });
      }
    }

    return attempts;
  }

  getStrategyTelemetrySnapshots(): StrategyTelemetrySnapshot[] {
    return Array.from(this.strategyTelemetryStats.values())
      .sort((left, right) => {
        if (left.capability === right.capability) {
          return left.strategy_id.localeCompare(right.strategy_id);
        }
        return left.capability.localeCompare(right.capability);
      })
      .map((entry) => ({
        capability: entry.capability,
        strategy_id: entry.strategy_id,
        success_count: entry.success_count,
        failure_count: entry.failure_count,
        success_rate:
          entry.success_count + entry.failure_count === 0
            ? 0
            : entry.success_count / (entry.success_count + entry.failure_count),
        last_used_at: entry.last_used_at,
      }));
  }

  private resetCache(): void {
    this.cache = {
      version: 2,
      workspaces: {},
      telemetry: [],
    };
  }

  private persistCapabilitySnapshot(
    currentUrl: string,
    message: CapabilityBridgeMessage,
  ): void {
    const workspaceKey = workspaceKeyFromUrl(currentUrl);
    this.cache.workspaces[workspaceKey] = {
      workspace_key: workspaceKey,
      saved_at: new Date().toISOString(),
      ui_signature: message.runtime.ui_signature,
      preferred_strategies: message.runtime.strategy_matches,
      report: message.report,
    };
    this.cache.telemetry = Array.from(this.strategyTelemetryStats.values());

    try {
      writeFileSync(
        this.cachePath,
        JSON.stringify(this.cache, null, 2),
        "utf8",
      );
    } catch {
      // Cache persistence is opportunistic and should not fail browser control.
    }
  }

  private recordStrategyStats(
    strategyMatches: Partial<Record<CapabilityKey, string>>,
    previousUiSignature: string | null,
  ): void {
    for (const capability of probedCapabilities) {
      const matched = strategyMatches[capability];
      if (matched) {
        const key = telemetryKey(capability, matched);
        const current = this.strategyTelemetryStats.get(key) ?? {
          capability,
          strategy_id: matched,
          success_count: 0,
          failure_count: 0,
          last_used_at: null,
        };
        current.success_count += 1;
        current.last_used_at = new Date().toISOString();
        this.strategyTelemetryStats.set(key, current);
        continue;
      }

      const preferred = previousUiSignature
        ? this.strategyTelemetry.get(previousUiSignature)?.[capability]
        : undefined;
      const strategyId =
        preferred ??
        this.ensureSelectorPackLoaded()?.capabilities?.[capability]?.[0]
          ?.strategy_id;
      if (!strategyId) {
        continue;
      }
      const key = telemetryKey(capability, strategyId);
      const current = this.strategyTelemetryStats.get(key) ?? {
        capability,
        strategy_id: strategyId,
        success_count: 0,
        failure_count: 0,
        last_used_at: null,
      };
      current.failure_count += 1;
      this.strategyTelemetryStats.set(key, current);
    }
  }
}
