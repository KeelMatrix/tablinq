import { createHash } from "node:crypto";
import { mkdir, readFile, stat, writeFile } from "node:fs/promises";
import { basename, dirname } from "node:path";
import type { Download, Locator, Page } from "playwright-core";

import { chooseUniqueArtifactPath, contentTypeForFile } from "../artifacts";
import type { ArtifactMetadataSnapshot } from "../contracts";
import { DOWNLOAD_TIMEOUT_MS } from "../timing";

const ARTIFACT_HANDLE_SELECTOR = [
  "[data-slot='artifact-link']",
  "[data-testid='artifact-download']",
  "a[download]",
  "a[aria-label*='Download']",
  "button[aria-label*='Download']",
  "a:has-text('Download')",
  "button:has-text('Download')",
  "a:has-text('Export')",
  "button:has-text('Export')",
].join(", ");

interface InlineArtifactCapture {
  bodyBase64?: string;
  bodyText?: string;
  contentType: string;
  suggestedFileName: string;
}

const INLINE_ARTIFACT_CAPTURE_SOURCE = String.raw`
  const normalizeText = (value) => (value ?? "").replace(/\s+/g, " ").trim();
  const unique = (values) => {
    const seen = new Set();
    const output = [];
    for (const value of values) {
      const normalized = normalizeText(value);
      if (!normalized) {
        continue;
      }
      const key = normalized.toLowerCase();
      if (seen.has(key)) {
        continue;
      }
      seen.add(key);
      output.push(normalized);
    }
    return output;
  };
  const arrayBufferToBase64 = (buffer) => {
    const bytes = new Uint8Array(buffer);
    const chunkSize = 0x8000;
    let binary = "";
    for (let offset = 0; offset < bytes.length; offset += chunkSize) {
      const chunk = bytes.subarray(offset, offset + chunkSize);
      let chunkBinary = "";
      for (const byte of chunk) {
        chunkBinary += String.fromCharCode(byte);
      }
      binary += chunkBinary;
    }
    return btoa(binary);
  };
  const decodeDataUrl = (url) => {
    const commaIndex = url.indexOf(",");
    if (!url.startsWith("data:") || commaIndex < 0) {
      return null;
    }
    const metadata = url.slice("data:".length, commaIndex);
    const payload = url.slice(commaIndex + 1);
    const segments = metadata
      .split(";")
      .map((segment) => segment.trim())
      .filter(Boolean);
    const contentType = segments[0] || "application/octet-stream";
    const isBase64 = segments.some((segment) => segment.toLowerCase() === "base64");
    if (isBase64) {
      return {
        contentType,
        bodyBase64: payload,
      };
    }
    try {
      return {
        contentType,
        bodyText: decodeURIComponent(payload),
      };
    } catch {
      return {
        contentType,
        bodyText: payload,
      };
    }
  };
  const extensionForContentType = (contentType) => {
    const normalized = normalizeText(contentType).toLowerCase();
    if (normalized.includes("image/svg+xml")) {
      return ".svg";
    }
    if (normalized.includes("image/png")) {
      return ".png";
    }
    if (normalized.includes("image/jpeg")) {
      return ".jpg";
    }
    if (normalized.includes("image/webp")) {
      return ".webp";
    }
    if (normalized.includes("image/gif")) {
      return ".gif";
    }
    if (normalized.includes("text/plain")) {
      return ".txt";
    }
    return ".bin";
  };
  const safeStem = (value) => {
    const normalized = normalizeText(value)
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "");
    return normalized || "artifact-" + (artifactIndex + 1);
  };
  const collectSvgLabels = (svgText) => {
    if (!svgText || typeof DOMParser !== "function") {
      return [];
    }
    try {
      const parsed = new DOMParser().parseFromString(svgText, "image/svg+xml");
      return unique(
        Array.from(
          parsed.querySelectorAll(
            "text, tspan, foreignObject p, foreignObject span, .nodeLabel, .edgeLabel",
          ),
        )
          .map((candidate) => candidate.textContent ?? "")
          .filter(Boolean),
      ).filter((label) => label.toLowerCase() !== "mermaid");
    } catch {
      return [];
    }
  };
  const readResource = async (url) => {
    if (url.startsWith("data:")) {
      return decodeDataUrl(url);
    }
    const response = await fetch(url);
    if (!response.ok) {
      throw new Error("resource fetch failed with status " + response.status);
    }
    return {
      contentType:
        response.headers.get("content-type") || "application/octet-stream",
      bodyBase64: arrayBufferToBase64(await response.arrayBuffer()),
    };
  };

  let current = element;
  for (let depth = 0; current && depth < 8; depth += 1) {
    const image = current.querySelector(
      "img[src^='data:image/'], img[src^='blob:'], img[src^='http']",
    );
    if (image instanceof HTMLImageElement) {
      const source = image.currentSrc || image.src;
      const resource = await readResource(source);
      if (!resource) {
        return null;
      }
      const contentType = resource.contentType;
      let labels = [];
      if (contentType.toLowerCase().includes("image/svg+xml")) {
        const svgText =
          resource.bodyText ??
          (() => {
            try {
              return new TextDecoder().decode(
                Uint8Array.from(atob(resource.bodyBase64 ?? ""), (value) =>
                  value.charCodeAt(0),
                ),
              );
            } catch {
              return "";
            }
          })();
        labels = collectSvgLabels(svgText);
      }
      const stem = labels[0] ? safeStem(labels[0]) : "mermaid-diagram";
      return {
        bodyBase64: resource.bodyBase64,
        bodyText: resource.bodyText,
        contentType,
        suggestedFileName:
          stem +
          "-" +
          (artifactIndex + 1) +
          extensionForContentType(contentType),
      };
    }

    const canvas = current.querySelector("canvas");
    if (canvas instanceof HTMLCanvasElement && canvas.width > 0 && canvas.height > 0) {
      const decoded = decodeDataUrl(canvas.toDataURL("image/png"));
      if (!decoded) {
        return null;
      }
      return {
        bodyBase64: decoded.bodyBase64,
        bodyText: decoded.bodyText,
        contentType: decoded.contentType,
        suggestedFileName: "inline-artifact-" + (artifactIndex + 1) + ".png",
      };
    }

    current = current.parentElement;
  }

  return null;
`;

const INLINE_ARTIFACT_CAPTURE_EVALUATOR = Function(
  `return (element, payload) => new Function("element", "artifactIndex", payload.source)(element, payload.index);`,
)() as (
  element: Element,
  payload: { index: number; source: string },
) => Promise<InlineArtifactCapture | null>;

const INLINE_CAPTURE_HINT_EVALUATOR = Function(
  `return (element) => {
    if (!(element instanceof HTMLElement)) {
      return false;
    }

    const normalize = (value) =>
      (value ?? "").replace(/\\s+/g, " ").trim().toLowerCase();
    const text = normalize(
      (element.getAttribute("aria-label") ?? "") + " " + (element.textContent ?? ""),
    );
    if (text.includes("download png")) {
      return true;
    }

    if (
      element instanceof HTMLAnchorElement &&
      (element.hasAttribute("download") || Boolean(element.getAttribute("href")))
    ) {
      return false;
    }

    let current = element;
    for (let depth = 0; current && depth < 8; depth += 1) {
      if (
        current.querySelector("img[src^='data:image/'], img[src^='blob:'], canvas")
      ) {
        return true;
      }
      const containerText = normalize(current.textContent);
      if (
        containerText.includes("mermaid") ||
        (Boolean(current.querySelector("button[aria-label='Code']")) &&
          Boolean(current.querySelector("button[aria-label='Preview']")))
      ) {
        return true;
      }
      current = current.parentElement;
    }

    return false;
  };`,
)() as (element: Element) => boolean;

const ARTIFACT_READY_EVALUATOR = Function(
  `return (element) => {
    if (!(element instanceof HTMLElement)) {
      return false;
    }
    const explicit =
      element.dataset.artifactReady ??
      element.getAttribute("data-artifact-ready");
    if (explicit === "false") {
      return false;
    }
    if (explicit === "true") {
      return true;
    }
    if (element instanceof HTMLAnchorElement) {
      return Boolean(
        element.hasAttribute("download") || element.getAttribute("href"),
      );
    }
    return !(
      element.getAttribute("aria-disabled") === "true" ||
      element.hasAttribute("disabled")
    );
  };`,
)() as (element: Element) => boolean;

export async function collectArtifacts(
  page: Page,
  downloadsDir: string,
  conversationId: string | null,
  messageId: string | null,
): Promise<ArtifactMetadataSnapshot[]> {
  const artifactRoot = await resolveArtifactRoot(page, messageId);
  const handles = await artifactRoot.locator(ARTIFACT_HANDLE_SELECTOR).all();
  const artifacts: ArtifactMetadataSnapshot[] = [];

  for (let index = 0; index < handles.length; index += 1) {
    const handle = handles[index]!;
    const artifactReady = await handle.evaluate(ARTIFACT_READY_EVALUATOR);
    if (!artifactReady) {
      continue;
    }

    const prefersInlineCapture = await handlePrefersInlineCapture(handle);
    if (prefersInlineCapture) {
      const inlineArtifact = await extractInlineArtifactCapture(
        handle,
        index,
      ).catch(() => null);
      if (inlineArtifact) {
        artifacts.push(
          await persistInlineArtifact(
            inlineArtifact,
            downloadsDir,
            conversationId,
            messageId,
            index,
          ),
        );
        continue;
      }
    }

    try {
      const download = await clickAndCaptureDownload(page, handle);
      artifacts.push(
        await persistDownload(
          download,
          downloadsDir,
          conversationId,
          messageId,
          index,
        ),
      );
    } catch (error) {
      if (prefersInlineCapture) {
        continue;
      }
      throw new Error(
        `download failed: ${error instanceof Error ? error.message : String(error)}`,
      );
    }
  }

  return artifacts;
}

export async function readArtifactStabilitySignature(
  page: Page,
  messageId: string | null,
): Promise<string> {
  const artifactRoot = await resolveArtifactRoot(page, messageId);
  const handles = await artifactRoot.locator(ARTIFACT_HANDLE_SELECTOR).all();
  const entries = [];
  for (let index = 0; index < handles.length; index += 1) {
    const handle = handles[index]!;
    const ready = await handle.evaluate(ARTIFACT_READY_EVALUATOR);
    const descriptor = await handle.evaluate((element, position) => {
      const htmlElement = element instanceof HTMLElement ? element : null;
      return {
        position,
        tagName: element.tagName,
        text: element.textContent?.trim() ?? "",
        href: element.getAttribute("href"),
        download: element.getAttribute("download"),
        ariaLabel: element.getAttribute("aria-label"),
        title: element.getAttribute("title"),
        disabled:
          htmlElement?.getAttribute("aria-disabled") ??
          (htmlElement?.hasAttribute("disabled") ? "true" : null),
      };
    }, index);
    entries.push({ ready, descriptor });
  }
  return JSON.stringify(entries);
}

async function resolveArtifactRoot(
  page: Page,
  messageId: string | null,
): Promise<Page | Locator> {
  if (!messageId) {
    return page;
  }

  const selector = [
    `[data-message-id="${escapeAttributeValue(messageId)}"]`,
    `[data-testid="${escapeAttributeValue(messageId)}"]`,
    `[id="${escapeAttributeValue(messageId)}"]`,
  ].join(", ");
  const locator = page.locator(selector).first();
  if ((await locator.count()) > 0) {
    return locator;
  }
  return page;
}

function escapeAttributeValue(value: string): string {
  return value.replaceAll("\\", "\\\\").replaceAll('"', '\\"');
}

export async function clickAndCaptureDownload(
  page: Page,
  handle: Locator,
): Promise<Download> {
  const downloadPromise = page.waitForEvent("download", {
    timeout: DOWNLOAD_TIMEOUT_MS,
  });
  await handle.click({ noWaitAfter: true, force: true });
  return downloadPromise;
}

async function handlePrefersInlineCapture(handle: Locator): Promise<boolean> {
  return handle.evaluate(INLINE_CAPTURE_HINT_EVALUATOR);
}

export async function extractInlineArtifactCapture(
  handle: Locator,
  index: number,
): Promise<InlineArtifactCapture | null> {
  return handle.evaluate(INLINE_ARTIFACT_CAPTURE_EVALUATOR, {
    index,
    source: `return (async () => {${INLINE_ARTIFACT_CAPTURE_SOURCE}})();`,
  });
}

export async function persistDownload(
  download: Download,
  downloadsDir: string,
  conversationId: string | null,
  messageId: string | null,
  index: number,
): Promise<ArtifactMetadataSnapshot> {
  const suggestedName =
    download.suggestedFilename() || `artifact-${index + 1}.bin`;
  const storagePath = await chooseUniqueArtifactPath(
    downloadsDir,
    suggestedName,
  );
  await mkdir(dirname(storagePath), { recursive: true });
  await download.saveAs(storagePath);

  return persistStoredArtifact(storagePath, conversationId, messageId);
}

async function persistInlineArtifact(
  artifact: InlineArtifactCapture,
  downloadsDir: string,
  conversationId: string | null,
  messageId: string | null,
  index: number,
): Promise<ArtifactMetadataSnapshot> {
  const suggestedName =
    artifact.suggestedFileName || `artifact-${index + 1}.bin`;
  const storagePath = await chooseUniqueArtifactPath(
    downloadsDir,
    suggestedName,
  );
  await mkdir(dirname(storagePath), { recursive: true });
  if (typeof artifact.bodyText === "string") {
    await writeFile(storagePath, artifact.bodyText, "utf8");
  } else {
    await writeFile(
      storagePath,
      Buffer.from(artifact.bodyBase64 ?? "", "base64"),
    );
  }

  return persistStoredArtifact(storagePath, conversationId, messageId);
}

async function persistStoredArtifact(
  storagePath: string,
  conversationId: string | null,
  messageId: string | null,
): Promise<ArtifactMetadataSnapshot> {
  const fileBuffer = await readFile(storagePath);
  const fileStat = await stat(storagePath);
  const sha256 = createHash("sha256").update(fileBuffer).digest("hex");
  const artifactId = createHash("sha1")
    .update(
      `${conversationId ?? "no-conversation"}:${messageId ?? "no-message"}:${storagePath}`,
    )
    .digest("hex")
    .slice(0, 16);

  return {
    artifact_id: `artifact-${artifactId}`,
    conversation_id: conversationId,
    message_id: messageId,
    file_name: basename(storagePath),
    content_type: contentTypeForFile(storagePath),
    size_bytes: fileStat.size,
    sha256,
    storage_path: storagePath,
  };
}
