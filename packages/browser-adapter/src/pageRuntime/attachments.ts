import type { Page } from "playwright-core";

import { callPageAdapter } from "./adapter";

export interface ComposerAttachmentState {
  visibleCount: number;
  visibleNames: string[];
  normalizedNames: string[];
  attachmentsProcessing: boolean;
}

export async function readComposerAttachmentState(
  page: Page,
): Promise<ComposerAttachmentState> {
  const state = await callPageAdapter<ComposerAttachmentState>(
    page,
    "readComposerAttachmentState",
  );
  return normalizeComposerAttachmentState(state);
}

export function normalizeComposerAttachmentState(
  state: Partial<ComposerAttachmentState> | null | undefined,
): ComposerAttachmentState {
  const visibleNames = uniqueCollapsed(state?.visibleNames ?? []);
  const normalizedNames = uniqueCollapsed(
    state?.normalizedNames?.length
      ? state.normalizedNames
      : visibleNames.map((name) => name.toLowerCase()),
  );
  return {
    visibleCount: Math.max(0, Math.floor(Number(state?.visibleCount ?? 0))),
    visibleNames,
    normalizedNames,
    attachmentsProcessing: Boolean(state?.attachmentsProcessing),
  };
}

function uniqueCollapsed(values: string[]): string[] {
  const seen = new Set<string>();
  const output: string[] = [];
  for (const value of values) {
    const collapsed = value.replace(/\s+/g, " ").trim();
    const normalized = collapsed.toLowerCase();
    if (!normalized || seen.has(normalized)) {
      continue;
    }
    seen.add(normalized);
    output.push(collapsed);
  }
  return output;
}
