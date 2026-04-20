import type { Page } from "playwright-core";

import type { AskBaseline, AskSnapshot } from "../snapshotState";
import {
  normalizeAskBaseline,
  normalizeAskSnapshot,
} from "../snapshotState";

import { callPageAdapter } from "./adapter";

export async function readAskBaseline(page: Page): Promise<AskBaseline> {
  return normalizeAskBaseline(
    await callPageAdapter<AskBaseline>(page, "readAskBaseline"),
  );
}

export async function readAskSnapshot(
  page: Page,
  baseline?: AskBaseline | null,
): Promise<AskSnapshot> {
  return normalizeAskSnapshot(
    await callPageAdapter<AskSnapshot>(page, "readAskSnapshot", {
      baseline: baseline ?? null,
    }),
  );
}
