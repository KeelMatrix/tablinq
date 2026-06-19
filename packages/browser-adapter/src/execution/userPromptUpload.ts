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

export const USER_PROMPT_UPLOAD_THRESHOLD_CHARS = 20_000; // practical max is 24_000, more than that cannot safely fit in gpt text composer

const PROMPT_UPLOAD_FILE_NAME_TOKEN = "user-prompt-<timestamp>.txt";

export const UPLOADED_USER_PROMPT_INSTRUCTION_TEMPLATES = [
  "User prompt uploaded as user-prompt-<timestamp>.txt. Follow the uploaded user prompt exactly.",
  "The user prompt was uploaded as user-prompt-<timestamp>.txt. Follow that uploaded prompt exactly.",
  "User prompt file user-prompt-<timestamp>.txt has been uploaded. Follow the uploaded user prompt exactly.",
  "The uploaded user prompt is in user-prompt-<timestamp>.txt. Follow it exactly.",
  "The user prompt is available in user-prompt-<timestamp>.txt. Follow the uploaded user prompt exactly.",
  "A user prompt has been uploaded as user-prompt-<timestamp>.txt. Follow the uploaded prompt exactly.",
  "The uploaded file user-prompt-<timestamp>.txt contains the user prompt. Follow it exactly.",
  "The user prompt has been provided in user-prompt-<timestamp>.txt. Follow the uploaded prompt exactly.",
  "User prompt saved as user-prompt-<timestamp>.txt. Follow the uploaded user prompt exactly.",
  "User prompt file: user-prompt-<timestamp>.txt. Follow the uploaded user prompt exactly.",
  "The file user-prompt-<timestamp>.txt contains the user prompt. Follow the uploaded user prompt exactly.",
  "The prompt from the user is uploaded as user-prompt-<timestamp>.txt. Follow it exactly.",
  "User prompt uploaded in file user-prompt-<timestamp>.txt. Follow the uploaded user prompt exactly.",
  "The uploaded prompt file is user-prompt-<timestamp>.txt. Follow the user prompt in that file exactly.",
  "The user prompt can be found in user-prompt-<timestamp>.txt. Follow the uploaded user prompt exactly.",
  "User prompt included as user-prompt-<timestamp>.txt. Follow the uploaded user prompt exactly.",
  "The user prompt was provided via user-prompt-<timestamp>.txt. Follow the uploaded prompt exactly.",
  "Uploaded user prompt file: user-prompt-<timestamp>.txt. Follow the user prompt exactly.",
  "The file named user-prompt-<timestamp>.txt contains the uploaded user prompt. Follow it exactly.",
  "User prompt is uploaded as user-prompt-<timestamp>.txt. Follow the uploaded user prompt exactly.",
  "The uploaded user prompt is stored in user-prompt-<timestamp>.txt. Follow it exactly.",
  "The user prompt has been uploaded in user-prompt-<timestamp>.txt. Follow that prompt exactly.",
  "Uploaded file user-prompt-<timestamp>.txt contains the user prompt. Follow it exactly.",
  "The user\u2019s prompt is uploaded as user-prompt-<timestamp>.txt. Follow the uploaded prompt exactly.",
  "The uploaded user prompt file is named user-prompt-<timestamp>.txt. Follow it exactly.",
  "User prompt provided as uploaded file user-prompt-<timestamp>.txt. Follow the uploaded user prompt exactly.",
  "The prompt to follow is uploaded as user-prompt-<timestamp>.txt. Follow it exactly.",
  "The user prompt is contained in user-prompt-<timestamp>.txt. Follow the uploaded user prompt exactly.",
  "The uploaded file user-prompt-<timestamp>.txt is the user prompt. Follow it exactly.",
  "User prompt uploaded under the name user-prompt-<timestamp>.txt. Follow the uploaded prompt exactly.",
  "The prompt supplied by the user is in user-prompt-<timestamp>.txt. Follow it exactly.",
  "User prompt available as uploaded file user-prompt-<timestamp>.txt. Follow the uploaded user prompt exactly.",
  "The user prompt has been attached as user-prompt-<timestamp>.txt. Follow the uploaded prompt exactly.",
  "Uploaded user prompt is in user-prompt-<timestamp>.txt. Follow the user prompt exactly.",
  "The file user-prompt-<timestamp>.txt is the uploaded user prompt. Follow it exactly.",
  "The user\u2019s prompt has been uploaded as user-prompt-<timestamp>.txt. Follow that uploaded prompt exactly.",
  "Use the uploaded user prompt in user-prompt-<timestamp>.txt and follow it exactly.",
  "Read the uploaded user prompt from user-prompt-<timestamp>.txt and follow it exactly.",
  "Follow exactly the uploaded user prompt contained in user-prompt-<timestamp>.txt.",
  "Follow the uploaded user prompt exactly as provided in user-prompt-<timestamp>.txt.",
  "The user prompt is provided in the uploaded file user-prompt-<timestamp>.txt. Follow it exactly.",
  "The uploaded file named user-prompt-<timestamp>.txt provides the user prompt. Follow it exactly.",
  "The uploaded user prompt is user-prompt-<timestamp>.txt. Follow the prompt exactly.",
  "User prompt file user-prompt-<timestamp>.txt is available. Follow the uploaded user prompt exactly.",
  "The uploaded prompt user-prompt-<timestamp>.txt contains the user\u2019s instructions. Follow the user prompt exactly.",
  "User instructions are uploaded as user-prompt-<timestamp>.txt. Follow the uploaded user prompt exactly.",
  "The user prompt was attached in user-prompt-<timestamp>.txt. Follow the uploaded prompt exactly.",
  "The uploaded user prompt can be read from user-prompt-<timestamp>.txt. Follow it exactly.",
  "The prompt file user-prompt-<timestamp>.txt contains the user prompt. Follow the uploaded user prompt exactly.",
  "The user prompt is in the uploaded file user-prompt-<timestamp>.txt. Follow the uploaded user prompt exactly.",
] as const;

export type UploadedUserPrompt = {
  fileName: string;
  instruction: string;
  attachmentState: ComposerAttachmentState;
};

const MAX_SAFE_FILE_NAME_BYTES = 255;

export function shouldUploadUserPromptText({
  prompt,
  preUploadedFileCount,
  promptUploadThresholdChars = USER_PROMPT_UPLOAD_THRESHOLD_CHARS,
  maxUploadFiles = MAX_COMPOSER_UPLOAD_FILES,
}: {
  prompt: string;
  preUploadedFileCount: number;
  promptUploadThresholdChars?: number;
  maxUploadFiles?: number;
}): boolean {
  const threshold = Math.max(1, Math.trunc(promptUploadThresholdChars));
  return (
    promptCharacterLength(prompt) > threshold &&
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

export function buildUploadedUserPromptInstruction(
  fileName: string,
  random: () => number = Math.random,
): string {
  const templateCount = UPLOADED_USER_PROMPT_INSTRUCTION_TEMPLATES.length;
  const rawIndex = Math.floor(random() * templateCount);
  const index = Number.isFinite(rawIndex)
    ? Math.min(Math.max(rawIndex, 0), templateCount - 1)
    : 0;
  return UPLOADED_USER_PROMPT_INSTRUCTION_TEMPLATES[index].replace(
    PROMPT_UPLOAD_FILE_NAME_TOKEN,
    fileName,
  );
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
  promptUploadThresholdChars,
  promptUploadFileName,
  promptUploadComposerText,
  attachmentBaseline,
  report,
}: {
  page: Page;
  prompt: string;
  explicitFiles: string[];
  promptUploadThresholdChars?: number | null;
  promptUploadFileName?: string | null;
  promptUploadComposerText?: string | null;
  attachmentBaseline: ComposerAttachmentState;
  report: CapabilityReportPayload | null;
}): Promise<UploadedUserPrompt | null> {
  const explicitPromptUpload =
    promptUploadFileName != null || promptUploadComposerText != null;
  if (
    (promptUploadFileName == null && promptUploadComposerText != null) ||
    (promptUploadFileName != null && promptUploadComposerText == null)
  ) {
    throw new Error(
      "upload failed: prompt upload file name and composer text must be provided together",
    );
  }
  const threshold =
    promptUploadThresholdChars ?? USER_PROMPT_UPLOAD_THRESHOLD_CHARS;
  if (promptUploadFileName != null && promptUploadComposerText != null) {
    assertExplicitPromptUploadFileName(promptUploadFileName);
    if (promptUploadComposerText.trim().length === 0) {
      throw new Error("upload failed: prompt upload composer text is empty");
    }
    const composerLength = promptCharacterLength(promptUploadComposerText);
    if (composerLength > Math.max(1, Math.trunc(threshold))) {
      throw new Error(
        `upload failed: prompt upload composer text exceeds the resolved prompt upload threshold (${composerLength} > ${Math.max(
          1,
          Math.trunc(threshold),
        )})`,
      );
    }
  }
  const preUploadedFileCount = Math.max(
    explicitFiles.length,
    attachmentBaseline.visibleCount,
  );
  const promptExceedsThreshold =
    promptCharacterLength(prompt) > Math.max(1, Math.trunc(threshold));
  if (!promptExceedsThreshold) {
    return null;
  }
  if (preUploadedFileCount >= MAX_COMPOSER_UPLOAD_FILES) {
    if (explicitPromptUpload) {
      throw new Error(
        [
          "upload failed: explicit prompt upload would exceed the composer attachment limit",
          `limit=${MAX_COMPOSER_UPLOAD_FILES}`,
          `explicit_files=${explicitFiles.length}`,
          `composer_visible_count=${attachmentBaseline.visibleCount}`,
          attachmentBaseline.visibleNames.length > 0
            ? `composer_visible_names=${attachmentBaseline.visibleNames.join(",")}`
            : undefined,
        ]
          .filter(Boolean)
          .join("; "),
      );
    }
    return null;
  }

  const existingNames = [
    ...attachmentBaseline.visibleNames,
    ...explicitFiles.map((file) => basename(file)),
  ];
  const fileName =
    promptUploadFileName ??
    chooseUserPromptUploadFileName({
      existingNames,
    });
  if (
    promptUploadFileName &&
    existingNames.some(
      (existingName) =>
        existingName.trim().toLowerCase() ===
        promptUploadFileName.trim().toLowerCase(),
    )
  ) {
    throw new Error(
      `upload failed: explicit prompt upload file name collides with an existing attachment: ${promptUploadFileName}`,
    );
  }
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
    instruction:
      promptUploadComposerText ?? buildUploadedUserPromptInstruction(fileName),
    attachmentState,
  };
}

function assertExplicitPromptUploadFileName(fileName: string): void {
  if (
    fileName.trim() !== fileName ||
    fileName.length === 0 ||
    Buffer.byteLength(fileName, "utf8") > MAX_SAFE_FILE_NAME_BYTES ||
    !fileName.endsWith(".txt") ||
    /[\/\\]/.test(fileName) ||
    fileName.includes("..") ||
    /[<>:"|?*\x00-\x1f]/.test(fileName) ||
    /[. ]$/.test(fileName) ||
    isWindowsReservedBasename(fileName.slice(0, -".txt".length))
  ) {
    throw new Error(
      `upload failed: explicit prompt upload file name is unsafe: ${fileName}`,
    );
  }
}

function isWindowsReservedBasename(stem: string): boolean {
  const upper = stem.toUpperCase();
  return (
    ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"].includes(upper) ||
    /^COM[1-9]$/.test(upper) ||
    /^LPT[1-9]$/.test(upper)
  );
}

function formatPromptUploadTimestamp(timestamp: Date): string {
  return timestamp
    .toISOString()
    .replace(/[-:]/g, "")
    .replace(".", "")
    .replace("Z", "Z");
}
