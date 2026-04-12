import fs from "node:fs";
import path from "node:path";

export type ProtocolSchemaName =
  | "artifact-metadata"
  | "capability-report"
  | "commands"
  | "events"
  | "failure-bundle"
  | "health-check"
  | "job-state"
  | "selector-pack"
  | "stream-chunk";

const repoRoot = path.resolve(__dirname, "../../..");

export function schemaPath(name: ProtocolSchemaName): string {
  return path.join(repoRoot, "schemas", "v1", `${name}.schema.json`);
}

export function loadSchema<T = Record<string, unknown>>(
  name: ProtocolSchemaName,
): T {
  return JSON.parse(fs.readFileSync(schemaPath(name), "utf8")) as T;
}
