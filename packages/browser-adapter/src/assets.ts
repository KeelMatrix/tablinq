import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";

let probeSource: string | undefined;
let pageAdapterSource: string | undefined;

const pageAdapterRuntimeModuleParts = [
  "pageAdapterRuntime/00-text.js",
  "pageAdapterRuntime/01-dom.js",
  "pageAdapterRuntime/02-conversation.js",
  "pageAdapterRuntime/03-composer.js",
  "pageAdapterRuntime/04-messageFilters.js",
  "pageAdapterRuntime/05-markdown.js",
  "pageAdapterRuntime/06-assistantMessages.js",
  "pageAdapterRuntime/07-capabilities.js",
  "pageAdapterRuntime/08-research.js",
  "pageAdapterRuntime/09-command.js",
];
const pageAdapterRuntimeEntryPart = "pageAdapter.runtime.js";

export function loadProbeSource(): string {
  if (!probeSource) {
    probeSource = readFileSync(
      resolveTablinqAssetPath(
        ["extension", "src", "probe.runtime.js"],
        ["packages", "extension", "src", "probe.runtime.js"],
      ),
      "utf8",
    ).replace(/^export\s+function\s+probeDocument/m, "function probeDocument");
  }

  return probeSource;
}

export function loadPageAdapterSource(): string {
  if (!pageAdapterSource) {
    const moduleSource = pageAdapterRuntimeModuleParts
      .map((part) => readPageAdapterRuntimePart(part))
      .join("\n\n");
    const entrySource = readPageAdapterRuntimePart(pageAdapterRuntimeEntryPart);
    pageAdapterSource = `${moduleSource}\n${entrySource}`.replace(
      /^export\s+function\s+createTablinqPageAdapterRuntime/m,
      "function createTablinqPageAdapterRuntime",
    );
  }

  return pageAdapterSource;
}

function readPageAdapterRuntimePart(part: string): string {
  const segments = part.split("/");
  return readFileSync(
    resolveTablinqAssetPath(
      ["browser-adapter", "src", ...segments],
      ["packages", "browser-adapter", "src", ...segments],
    ),
    "utf8",
  ).trimEnd();
}

export function resolveTablinqAssetPath(
  installedRelativeSegments: string[],
  repoRelativeSegments: string[] = installedRelativeSegments,
): string {
  const candidates: string[] = [];
  const assetRoot = process.env.TABLINQ_ASSET_ROOT;
  if (assetRoot) {
    candidates.push(resolve(assetRoot, ...installedRelativeSegments));
    candidates.push(resolve(assetRoot, ...repoRelativeSegments));
  }

  for (const upward of [
    "..",
    "../..",
    "../../..",
    "../../../..",
    "../../../../..",
  ]) {
    candidates.push(resolve(__dirname, upward, ...installedRelativeSegments));
    candidates.push(resolve(__dirname, upward, ...repoRelativeSegments));
  }

  for (const candidate of candidates) {
    if (existsSync(candidate)) {
      return candidate;
    }
  }

  return resolve(
    assetRoot ?? __dirname,
    ...(assetRoot ? installedRelativeSegments : repoRelativeSegments),
  );
}
