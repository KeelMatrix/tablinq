import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";

let probeSource: string | undefined;
let pageAdapterSource: string | undefined;

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
    pageAdapterSource = readFileSync(
      resolveTablinqAssetPath(
        ["browser-adapter", "src", "pageAdapter.runtime.js"],
        ["packages", "browser-adapter", "src", "pageAdapter.runtime.js"],
      ),
      "utf8",
    ).replace(
      /^export\s+function\s+createTablinqPageAdapterRuntime/m,
      "function createTablinqPageAdapterRuntime",
    );
  }

  return pageAdapterSource;
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
