import { Buffer } from "node:buffer";
import { basename } from "node:path";
import type { Page } from "playwright-core";

import type { CapabilityReportPayload } from "../contracts";
import {
  type ComposerAttachmentState,
  readComposerAttachmentState,
} from "../pageRuntime/attachments";
import { uploadAttachments } from "../pageRuntime/uploads";
import { MAX_COMPOSER_UPLOAD_FILES } from "../timing";

export const USER_PROMPT_UPLOAD_THRESHOLD_CHARS = 4_500;

export type UploadedUserPrompt = {
  fileName: string;
  instruction: string;
  attachmentState: ComposerAttachmentState;
};

export function shouldUploadUserPromptText({
  prompt,
  preUploadedFileCount,
  maxUploadFiles = MAX_COMPOSER_UPLOAD_FILES,
}: {
  prompt: string;
  preUploadedFileCount: number;
  maxUploadFiles?: number;
}): boolean {
  return (
    promptCharacterLength(prompt) > USER_PROMPT_UPLOAD_THRESHOLD_CHARS &&
    preUploadedFileCount < maxUploadFiles
  );
}

export function promptCharacterLength(prompt: string): number {
  return Array.from(prompt).length;
}

export function chooseUserPromptUploadFileName({
  timestamp = new Date(),
  existingNames = [],
  maxAttempts = MAX_COMPOSER_UPLOAD_FILES,
}: {
  timestamp?: Date;
  existingNames?: string[];
  maxAttempts?: number;
} = {}): string {
  const existing = new Set(
    existingNames.map((name) => name.trim().toLowerCase()).filter(Boolean),
  );
  const stem = `user-prompt-${formatPromptUploadTimestamp(timestamp)}`;
  const attempts = Math.max(1, Math.trunc(maxAttempts));
  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    const suffix = attempt === 1 ? "" : `-${attempt}`;
    const fileName = `${stem}${suffix}.txt`;
    if (!existing.has(fileName.toLowerCase())) {
      return fileName;
    }
  }
  throw new Error(
    `upload failed: could not choose a unique prompt upload file name after ${attempts} attempts`,
  );
}

export function buildUploadedUserPromptInstruction(fileName: string): string {
  return `User prompt uploaded as ${fileName}. Follow the uploaded user prompt exactly.`;
}

export function composerAttachmentStateHasExactName(
  state: ComposerAttachmentState,
  fileName: string,
): boolean {
  return state.visibleNames.some((name) => name === fileName);
}

export async function uploadUserPromptTextIfNeeded({
  page,
  prompt,
  explicitFiles,
  attachmentBaseline,
  report,
}: {
  page: Page;
  prompt: string;
  explicitFiles: string[];
  attachmentBaseline: ComposerAttachmentState;
  report: CapabilityReportPayload | null;
}): Promise<UploadedUserPrompt | null> {
  const preUploadedFileCount = Math.max(
    explicitFiles.length,
    attachmentBaseline.visibleCount,
  );
  if (!shouldUploadUserPromptText({ prompt, preUploadedFileCount })) {
    return null;
  }

  const fileName = chooseUserPromptUploadFileName({
    existingNames: [
      ...attachmentBaseline.visibleNames,
      ...explicitFiles.map((file) => basename(file)),
    ],
  });
  await uploadAttachments(
    page,
    [
      {
        name: fileName,
        mimeType: "text/plain",
        buffer: Buffer.from(prompt, "utf8"),
      },
    ],
    report,
  );

  const attachmentState = await readComposerAttachmentState(page);
  if (
    !fileName.endsWith(".txt") ||
    !composerAttachmentStateHasExactName(attachmentState, fileName)
  ) {
    throw new Error(
      `upload failed: prompt upload was not acknowledged with exact .txt file name: ${fileName}`,
    );
  }

  return {
    fileName,
    instruction: buildUploadedUserPromptInstruction(fileName),
    attachmentState,
  };
}

function formatPromptUploadTimestamp(timestamp: Date): string {
  return timestamp
    .toISOString()
    .replace(/[-:]/g, "")
    .replace(".", "")
    .replace("Z", "Z");
}
