import { basename, resolve } from "node:path";
import { stat } from "node:fs/promises";
import type { Page } from "playwright-core";

import type { CapabilityReportPayload } from "../contracts";
import {
  INTERACTION_TIMEOUT_MS,
  UPLOAD_ACK_TIMEOUT_MS,
} from "../timing";

import { waitForPageAdapterCondition } from "./adapter";

export async function uploadAttachments(
  page: Page,
  files: string[],
  report: CapabilityReportPayload | null,
): Promise<void> {
  if (files.length === 0) {
    return;
  }

  const uploadSurface = page.locator(
    [
      "input[type='file']",
      "[data-slot='upload-entry']",
      "[data-testid='upload-button']",
      "button[aria-label*='Add photos']",
      "button[aria-label*='Add files']",
      "label[for]",
    ].join(", "),
  );
  if (
    !report?.matrix.upload_control_visible &&
    (await uploadSurface.count().catch(() => 0)) === 0
  ) {
    throw new Error(
      "upload failed: no upload control is available in the current browser session",
    );
  }

  const resolvedFiles = await Promise.all(
    files.map(async (filePath) => {
      const absolutePath = resolve(filePath);
      const fileStat = await stat(absolutePath).catch(() => null);
      if (!fileStat?.isFile()) {
        throw new Error(`upload failed: file not found: ${absolutePath}`);
      }
      return absolutePath;
    }),
  );

  const fileInput = page.locator("input[type='file']").first();
  if ((await fileInput.count()) > 0) {
    await fileInput.setInputFiles(resolvedFiles);
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
    await uploadEntry.click();
    const chooser = await chooserPromise.catch(() => null);
    if (!chooser) {
      throw new Error("upload failed: file chooser did not open");
    }
    await chooser.setFiles(resolvedFiles);
  }

  const expectedNames = resolvedFiles.map((filePath) => basename(filePath));
  await waitForPageAdapterCondition(
    page,
    "uploadAcknowledged",
    {
      names: expectedNames,
    },
    UPLOAD_ACK_TIMEOUT_MS,
  ).catch(() => {
    throw new Error(
      `upload failed: the page did not acknowledge uploaded files: ${expectedNames.join(", ")}`,
    );
  });
}
