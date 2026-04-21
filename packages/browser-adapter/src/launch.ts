import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { basename, resolve } from "node:path";
import type { BrowserLaunchConfig } from "./contracts";
import { resolveTablinqAssetPath } from "./assets";

export function resolveBrowserLaunchConfig(
  platform: NodeJS.Platform,
  browserPath?: string,
): BrowserLaunchConfig {
  if (browserPath) {
    return {
      browserKind: inferBrowserKind(browserPath),
      executablePath: browserPath,
    };
  }

  const detected = detectBrowserExecutable(platform);
  if (detected) {
    return {
      browserKind: inferBrowserKind(detected),
      executablePath: detected,
    };
  }

  throw new Error(
    "no supported browser executable found; set an explicit browser path",
  );
}

export function detectBrowserExecutable(
  platform: NodeJS.Platform,
): string | null {
  const envPath = process.env.TABLINQ_BROWSER_PATH?.trim();
  if (envPath) {
    return envPath;
  }

  if (platform === "win32") {
    return detectWindowsBrowserExecutable();
  }

  const candidates =
    platform === "darwin"
      ? [
          "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
          "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        ]
      : platform === "linux"
        ? ["google-chrome", "chromium", "chromium-browser", "microsoft-edge"]
        : [];

  for (const candidate of candidates) {
    try {
      if (platform === "linux") {
        const resolved = execFileSync("which", [candidate], {
          encoding: "utf8",
        }).trim();
        if (resolved) {
          return resolved;
        }
      } else {
        return candidate;
      }
    } catch {
      continue;
    }
  }

  return null;
}

export function detectBrowserAvailability(
  platform: NodeJS.Platform,
  browserPath?: string,
): { detected: boolean; path: string | null } {
  if (browserPath) {
    return { detected: true, path: browserPath };
  }
  const detected = detectBrowserExecutable(platform);
  return {
    detected: Boolean(detected),
    path: detected,
  };
}

export function resolveExtensionDir(
  extensionDir?: string | null,
): string | null {
  const explicit = extensionDir?.trim();
  if (explicit) {
    const resolved = resolve(explicit);
    return existsSync(resolved) ? resolved : null;
  }

  const candidate = resolve(resolveTablinqAssetPath(["extension"]));
  return existsSync(candidate) ? candidate : null;
}

function detectWindowsBrowserExecutable(): string | null {
  const localAppData = process.env.LOCALAPPDATA?.trim();
  const programFiles = [
    process.env["ProgramFiles(x86)"]?.trim(),
    process.env.ProgramFiles?.trim(),
  ].filter((value): value is string => Boolean(value));

  const chromeCandidates = [
    ...(localAppData
      ? [resolve(localAppData, "Google", "Chrome", "Application", "chrome.exe")]
      : []),
    ...programFiles.map((base) =>
      resolve(base, "Google", "Chrome", "Application", "chrome.exe"),
    ),
  ];
  for (const candidate of chromeCandidates) {
    if (existsSync(candidate)) {
      return candidate;
    }
  }

  const edgeCandidates = [
    ...(localAppData
      ? [
          resolve(
            localAppData,
            "Microsoft",
            "Edge",
            "Application",
            "msedge.exe",
          ),
        ]
      : []),
    ...programFiles.map((base) =>
      resolve(base, "Microsoft", "Edge", "Application", "msedge.exe"),
    ),
  ];
  for (const candidate of edgeCandidates) {
    if (existsSync(candidate)) {
      return candidate;
    }
  }

  for (const commandName of ["chrome.exe", "msedge.exe"]) {
    try {
      const resolved = execFileSync("where", [commandName], {
        encoding: "utf8",
      })
        .split(/\r?\n/)
        .map((value) => value.trim())
        .find(Boolean);
      if (resolved) {
        return resolved;
      }
    } catch {
      continue;
    }
  }

  return null;
}

export function inferBrowserKind(browserPath: string): string {
  const normalizedPath = browserPath.replaceAll("\\", "/");
  const fileName = basename(normalizedPath).toLowerCase();
  if (fileName === "chrome.exe" || fileName === "google-chrome") {
    return "chrome";
  }
  if (fileName === "msedge.exe" || fileName === "microsoft-edge") {
    return "msedge";
  }
  if (fileName.includes("chromium")) {
    return "chromium";
  }
  return "custom";
}
