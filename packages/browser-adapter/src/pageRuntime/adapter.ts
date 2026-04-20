import type { Page } from "playwright-core";

import { loadPageAdapterSource } from "../assets";

export async function callPageAdapter<T>(
  page: Page,
  methodName:
    | "composerReady"
    | "fillComposer"
    | "clickSubmit"
    | "applyAskConfiguration"
    | "uploadAcknowledged"
    | "submitReady"
    | "attachmentsStillProcessing"
    | "readSubmitState"
    | "readAskBaseline"
    | "readAskSnapshot"
    | "readResearchSubmissionState"
    | "readResearchSnapshot"
    | "submitStarted"
    | "researchSubmissionStarted",
  payload?: unknown,
): Promise<T> {
  return page.evaluate(
    ({
      source,
      methodName,
      payload,
    }: {
      source: string;
      methodName: string;
      payload?: unknown;
    }) => {
      const runtime = new Function(
        `${source}\nreturn createTablinqPageAdapterRuntime();`,
      )() as Record<string, (value?: unknown) => unknown>;
      const method = runtime[methodName];
      if (typeof method !== "function") {
        throw new Error(`page adapter method not found: ${methodName}`);
      }
      return method(payload);
    },
    {
      source: loadPageAdapterSource(),
      methodName,
      payload,
    },
  ) as Promise<T>;
}

export async function waitForPageAdapterCondition(
  page: Page,
  methodName:
    | "composerReady"
    | "uploadAcknowledged"
    | "submitReady"
    | "attachmentsStillProcessing"
    | "submitStarted"
    | "researchSubmissionStarted",
  payload: unknown,
  timeoutMs: number,
): Promise<void> {
  await page.waitForFunction(
    ({
      source,
      methodName,
      payload,
    }: {
      source: string;
      methodName: string;
      payload?: unknown;
    }) => {
      const runtime = new Function(
        `${source}\nreturn createTablinqPageAdapterRuntime();`,
      )() as Record<string, (value?: unknown) => unknown>;
      const method = runtime[methodName];
      return typeof method === "function" ? Boolean(method(payload)) : false;
    },
    {
      source: loadPageAdapterSource(),
      methodName,
      payload,
    },
    {
      timeout: timeoutMs,
    },
  );
}
