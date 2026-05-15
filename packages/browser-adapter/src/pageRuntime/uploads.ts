import { basename, resolve } from "node:path";
import { stat } from "node:fs/promises";
import type { Buffer } from "node:buffer";
import type { Locator, Page } from "playwright-core";

import type { CapabilityReportPayload } from "../contracts";
import {
  adaptivePollIntervalMs,
  INTERACTION_TIMEOUT_MS,
  MAX_UPLOAD_FILE_BYTES,
  uploadAckTimeoutMsForFiles,
} from "../timing";

import { callPageAdapter } from "./adapter";
import {
  clickWithInteractionJitter,
  prepareHighRiskInteraction,
  sleep,
} from "./common";

type MemoryUploadAttachmentInput = {
  name: string;
  mimeType: string;
  buffer: Buffer;
};

export type UploadAttachmentInput = string | MemoryUploadAttachmentInput;
type PlaywrightUploadInputBatch = string[] | MemoryUploadAttachmentInput[];

export async function uploadAttachments(
  page: Page,
  files: UploadAttachmentInput[],
  report: CapabilityReportPayload | null,
): Promise<void> {
  if (files.length === 0) {
    return;
  }

  const resolvedFiles = await Promise.all(
    files.map(async (file) => {
      if (typeof file !== "string") {
        if (file.buffer.length > MAX_UPLOAD_FILE_BYTES) {
          throw new Error(`upload failed: file exceeds 20 MiB: ${file.name}`);
        }
        return {
          input: file,
          name: file.name,
          sizeBytes: file.buffer.length,
        };
      }

      const filePath = file;
      const absolutePath = resolve(filePath);
      const fileStat = await stat(absolutePath).catch(() => null);
      if (!fileStat?.isFile()) {
        throw new Error(`upload failed: file not found: ${absolutePath}`);
      }
      if (fileStat.size > MAX_UPLOAD_FILE_BYTES) {
        throw new Error(`upload failed: file exceeds 20 MiB: ${absolutePath}`);
      }
      return {
        input: absolutePath,
        name: basename(absolutePath),
        sizeBytes: fileStat.size,
      };
    }),
  );

  if (!(await waitForUploadSurface(page, report))) {
    throw new Error(
      "upload failed: no upload control is available in the current browser session",
    );
  }

  const fileInput = await waitForUploadFileInput(page);
  const inputBatch = toPlaywrightUploadInputBatch(
    resolvedFiles.map((item) => item.input),
  );
  if (fileInput) {
    await prepareHighRiskInteraction(page, "upload_entry", fileInput);
    await fileInput.setInputFiles(inputBatch);
  } else {
    const uploadEntry = page
      .locator(
        [
          "[data-slot='upload-entry']",
          "[data-testid='upload-button']",
          "label[for]",
          "button[aria-label*='file']",
          "button[aria-label*='photo']",
        ].join(", "),
      )
      .first();
    if ((await uploadEntry.count()) === 0) {
      throw new Error("upload failed: upload entry point could not be found");
    }
    const chooserPromise = page.waitForEvent("filechooser", {
      timeout: INTERACTION_TIMEOUT_MS,
    });
    await clickWithInteractionJitter(
      uploadEntry,
      "upload_entry",
      undefined,
      page,
    );
    const chooser = await chooserPromise.catch(() => null);
    if (!chooser) {
      throw new Error("upload failed: file chooser did not open");
    }
    await chooser.setFiles(inputBatch);
  }

  const expectedNames = resolvedFiles.map((file) => file.name);
  const acknowledged = await waitForUploadAcknowledged(
    page,
    expectedNames,
    uploadAckTimeoutMsForFiles(resolvedFiles.map((file) => file.sizeBytes)),
  );
  if (!acknowledged) {
    throw new Error(
      `upload failed: the page did not acknowledge uploaded files: ${expectedNames.join(", ")}`,
    );
  }
}

function toPlaywrightUploadInputBatch(
  inputs: UploadAttachmentInput[],
): PlaywrightUploadInputBatch {
  if (inputs.every((input): input is string => typeof input === "string")) {
    return inputs;
  }
  if (
    inputs.every(
      (input): input is MemoryUploadAttachmentInput =>
        typeof input !== "string",
    )
  ) {
    return inputs;
  }
  throw new Error("upload failed: mixed path and in-memory upload batch");
}

function uploadSurfaceLocator(page: Page): Locator {
  return page.locator(
    [
      "input[type='file']",
      "#composer-plus-btn",
      "[data-testid='composer-plus-btn']",
      "[data-slot='upload-entry']",
      "[data-testid='upload-button']",
      "button[aria-label*='Add photos' i]",
      "button[aria-label*='Add files' i]",
      "button[aria-label*='Attach' i]",
      "label[for]",
    ].join(", "),
  );
}

async function waitForUploadSurface(
  page: Page,
  report: CapabilityReportPayload | null,
): Promise<boolean> {
  const startedAt = Date.now();
  const deadline = Date.now() + INTERACTION_TIMEOUT_MS;
  while (Date.now() < deadline) {
    if (
      (await uploadSurfaceLocator(page)
        .count()
        .catch(() => 0)) > 0
    ) {
      return true;
    }
    await sleep(
      Math.min(
        adaptivePollIntervalMs("upload", Date.now() - startedAt),
        Math.max(1, deadline - Date.now()),
      ),
    );
  }
  return (
    (await uploadSurfaceLocator(page)
      .count()
      .catch(() => 0)) > 0 || Boolean(report?.matrix.upload_control_visible)
  );
}

async function findUploadFileInput(page: Page): Promise<Locator | null> {
  const candidates = [
    page.locator("input#upload-files[type='file']").first(),
    page.locator("input[type='file']:not([accept])").first(),
    page.locator("input[type='file']:not([accept*='image' i])").first(),
    page.locator("input[type='file']").first(),
  ];
  for (const candidate of candidates) {
    if ((await candidate.count().catch(() => 0)) > 0) {
      return candidate;
    }
  }
  return null;
}

async function waitForUploadFileInput(page: Page): Promise<Locator | null> {
  const startedAt = Date.now();
  const deadline = Date.now() + INTERACTION_TIMEOUT_MS;
  while (Date.now() < deadline) {
    const input = await findUploadFileInput(page);
    if (input) {
      return input;
    }
    await sleep(
      Math.min(
        adaptivePollIntervalMs("upload", Date.now() - startedAt),
        Math.max(1, deadline - Date.now()),
      ),
    );
  }
  return findUploadFileInput(page);
}

async function waitForUploadAcknowledged(
  page: Page,
  names: string[],
  timeoutMs: number,
): Promise<boolean> {
  const startedAt = Date.now();
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const acknowledged = await callPageAdapter<boolean>(
      page,
      "uploadAcknowledged",
      { names },
    ).catch(() => false);
    if (acknowledged) {
      return true;
    }
    await sleep(
      Math.min(
        adaptivePollIntervalMs("upload", Date.now() - startedAt),
        Math.max(1, deadline - Date.now()),
      ),
    );
  }
  return callPageAdapter<boolean>(page, "uploadAcknowledged", {
    names,
  }).catch(() => false);
}
