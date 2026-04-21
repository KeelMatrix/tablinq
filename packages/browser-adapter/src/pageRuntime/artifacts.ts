import { createHash } from "node:crypto";
import { mkdir, readFile, stat } from "node:fs/promises";
import { basename, dirname } from "node:path";
import type { Download, Locator, Page } from "playwright-core";

import { chooseUniqueArtifactPath, contentTypeForFile } from "../artifacts";
import type { ArtifactMetadataSnapshot } from "../contracts";
import { DOWNLOAD_TIMEOUT_MS } from "../timing";

export async function collectArtifacts(
  page: Page,
  downloadsDir: string,
  conversationId: string | null,
  messageId: string | null,
): Promise<ArtifactMetadataSnapshot[]> {
  const handles = await page
    .locator(
      [
        "[data-slot='artifact-link']",
        "[data-testid='artifact-download']",
        "a[download]",
        "a[aria-label*='Download']",
        "button[aria-label*='Download']",
        "a:has-text('Download')",
        "button:has-text('Download')",
        "a:has-text('Export')",
        "button:has-text('Export')",
      ].join(", "),
    )
    .all();
  const artifacts: ArtifactMetadataSnapshot[] = [];

  for (let index = 0; index < handles.length; index += 1) {
    const handle = handles[index]!;
    const artifactReady = await handle.evaluate((element) => {
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
        return Boolean(element.hasAttribute("download") || element.href);
      }
      return !(
        element.getAttribute("aria-disabled") === "true" ||
        element.hasAttribute("disabled")
      );
    });
    if (!artifactReady) {
      continue;
    }

    const download = await clickAndCaptureDownload(page, handle).catch(
      (error) => {
        throw new Error(
          `download failed: ${error instanceof Error ? error.message : String(error)}`,
        );
      },
    );
    artifacts.push(
      await persistDownload(
        download,
        downloadsDir,
        conversationId,
        messageId,
        index,
      ),
    );
  }

  return artifacts;
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
