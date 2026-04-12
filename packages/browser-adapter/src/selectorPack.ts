import fs from "node:fs";

export const selectorCapabilities = [
  "composer_ready",
  "submit_available",
  "model_picker_visible",
  "thinking_effort_visible",
  "search_mode_visible",
  "research_mode_visible",
  "upload_control_visible",
  "downloadable_artifacts_observable",
] as const;

type SelectorCapability = (typeof selectorCapabilities)[number];

export interface SelectorStrategy {
  strategy_id: string;
  kind: "aria_role" | "css" | "text" | "dom_neighborhood" | "hotkey";
  selector: string;
  score: number;
}

export interface SelectorPack {
  format_version: number;
  pack_id: string;
  description: string;
  updated_at: string;
  capabilities: Record<SelectorCapability, SelectorStrategy[]>;
}

export function loadSelectorPack(filePath: string): SelectorPack {
  return parseSelectorPack(JSON.parse(fs.readFileSync(filePath, "utf8")));
}

export function parseSelectorPack(input: unknown): SelectorPack {
  if (!isObject(input)) {
    throw new Error("selector pack must be an object");
  }

  const { format_version, pack_id, description, updated_at, capabilities } =
    input;
  if (format_version !== 1) {
    throw new Error("selector pack format_version must be 1");
  }
  if (!isNonEmptyString(pack_id)) {
    throw new Error("selector pack pack_id must be a non-empty string");
  }
  if (!isNonEmptyString(description)) {
    throw new Error("selector pack description must be a non-empty string");
  }
  if (!isNonEmptyString(updated_at)) {
    throw new Error("selector pack updated_at must be a non-empty string");
  }
  if (!isObject(capabilities)) {
    throw new Error("selector pack capabilities must be an object");
  }

  const normalizedCapabilities = {} as Record<
    SelectorCapability,
    SelectorStrategy[]
  >;
  for (const capability of selectorCapabilities) {
    const strategies = capabilities[capability];
    if (!Array.isArray(strategies) || strategies.length === 0) {
      throw new Error(
        `selector pack capability ${capability} must define strategies`,
      );
    }

    normalizedCapabilities[capability] = strategies.map((strategy) =>
      parseStrategy(capability, strategy),
    );
  }

  return {
    format_version,
    pack_id,
    description,
    updated_at,
    capabilities: normalizedCapabilities,
  };
}

export function normalizeSelectorPack(pack: SelectorPack) {
  return {
    format_version: pack.format_version,
    pack_id: pack.pack_id,
    description: pack.description,
    updated_at: pack.updated_at,
    capabilities: Object.entries(pack.capabilities)
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([capability, strategies]) => ({
        capability,
        strategies: [...strategies].sort((left, right) =>
          left.strategy_id.localeCompare(right.strategy_id),
        ),
      })),
  };
}

function parseStrategy(capability: string, value: unknown): SelectorStrategy {
  if (!isObject(value)) {
    throw new Error(`selector strategy for ${capability} must be an object`);
  }

  const strategy = {
    strategy_id: value.strategy_id,
    kind: value.kind,
    selector: value.selector,
    score: value.score,
  };

  if (!isNonEmptyString(strategy.strategy_id)) {
    throw new Error(`selector strategy for ${capability} needs a strategy_id`);
  }
  if (!isNonEmptyString(strategy.selector)) {
    throw new Error(
      `selector strategy ${strategy.strategy_id} needs a selector`,
    );
  }
  if (!isSelectorKind(strategy.kind)) {
    throw new Error(
      `selector strategy ${strategy.strategy_id} has an unsupported kind`,
    );
  }
  if (typeof strategy.score !== "number") {
    throw new Error(
      `selector strategy ${strategy.strategy_id} needs a numeric score`,
    );
  }

  return strategy;
}

function isSelectorKind(value: unknown): value is SelectorStrategy["kind"] {
  return (
    value === "aria_role" ||
    value === "css" ||
    value === "text" ||
    value === "dom_neighborhood" ||
    value === "hotkey"
  );
}

function isObject(value: unknown): value is Record<string, any> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isNonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0;
}
