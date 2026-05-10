import type { Page } from "playwright-core";

import type { AskSnapshot } from "../snapshotState";
import { noteProviderPressure, noteProviderPressureFromText } from "../timing";
import { conversationIdFromUrl } from "../urls";

const PROVIDER_GENERATION_FAILURE_CODE =
  "provider_generation_failed_nonretryable";
export const PROVIDER_STOPPED_THINKING_CODE = "provider_stopped_thinking";
export const PROVIDER_CONVERSATION_LOAD_FAILURE_CODE =
  "provider_conversation_load_failed";
const PROVIDER_NETWORK_PREFLIGHT_TIMEOUT_MS = 1_500;
const PROVIDER_CONVERSATION_LOAD_TIMEOUT_MS = 5_000;
const PROVIDER_CONVERSATION_LOADED_STABLE_MS = 500;
const PROVIDER_CONVERSATION_LOAD_POLL_MS = 150;

type ProviderReachableResult =
  | { ok: true }
  | { ok: false; timedOut: boolean; reason?: string };

class AskProviderFailure extends Error {
  readonly code: string;
  readonly currentUrl: string;
  readonly retryVisible: boolean;

  constructor(
    code: string,
    message: string,
    currentUrl: string,
    retryVisible: boolean,
  ) {
    super(message);
    this.name = "AskProviderFailure";
    this.code = code;
    this.currentUrl = currentUrl;
    this.retryVisible = retryVisible;
  }
}

class ProviderConversationLoadFailure extends Error {
  readonly code = PROVIDER_CONVERSATION_LOAD_FAILURE_CODE;
  readonly currentUrl: string;

  constructor(conversationId: string, currentUrl: string, reason?: string) {
    super(
      `provider_conversation_load_failed: ChatGPT did not load conversation ${conversationId}. No prompt was submitted.${reason ? ` ${reason}` : ""}`,
    );
    this.name = "ProviderConversationLoadFailure";
    this.currentUrl = currentUrl;
  }
}

export function throwIfProviderGenerationFailed(snapshot: AskSnapshot): void {
  if (snapshot.failureCode !== PROVIDER_GENERATION_FAILURE_CODE) {
    return;
  }
  noteProviderPressure("generation_failed");
  if (snapshot.retryVisible) {
    noteProviderPressure("retry_visible");
  }
  noteProviderPressureFromText(snapshot.failureMessage);
  throw new AskProviderFailure(
    PROVIDER_GENERATION_FAILURE_CODE,
    snapshot.failureMessage ??
      "Something went wrong while generating the response. If this issue persists please contact us through our help center.",
    snapshot.currentUrl,
    snapshot.retryVisible,
  );
}

export function throwIfProviderStoppedThinking(snapshot: AskSnapshot): void {
  if (snapshot.failureCode !== PROVIDER_STOPPED_THINKING_CODE) {
    return;
  }
  noteProviderPressure("stopped_thinking");
  throw new AskProviderFailure(
    PROVIDER_STOPPED_THINKING_CODE,
    "provider_stopped_thinking: ChatGPT stopped thinking twice for this request. Start a new conversation and retry the request there.",
    snapshot.currentUrl,
    snapshot.retryVisible,
  );
}

export async function ensureProviderReachable(page: Page): Promise<void> {
  const result = (await page.evaluate(
    `(async () => {
    const timeoutMs = ${PROVIDER_NETWORK_PREFLIGHT_TIMEOUT_MS};
    const controller = new AbortController();
    const timeoutId = window.setTimeout(() => controller.abort(), timeoutMs);
    try {
      const response = await fetch(new URL("/", window.location.origin), {
        method: "GET",
        credentials: "include",
        cache: "no-store",
        signal: controller.signal,
      });
      if (!response.ok) {
        return {
          ok: false,
          timedOut: false,
          reason: "status " + response.status,
        };
      }
      return { ok: true };
    } catch (error) {
      return {
        ok: false,
        timedOut: controller.signal.aborted,
        reason: error instanceof Error ? error.message : String(error),
      };
    } finally {
      window.clearTimeout(timeoutId);
    }
  })()`,
  )) as ProviderReachableResult;
  if (result.ok) {
    return;
  }
  const suffix = result.timedOut ? "timed out" : "failed";
  const reason = result.reason ? `: ${result.reason}` : "";
  throw new Error(
    `provider/network-unavailable: chatgpt preflight request ${suffix}${reason}`,
  );
}

export async function ensureProviderConversationLoaded(
  page: Page,
  conversationId?: string | null,
  options: { failOnUrlMismatch?: boolean } = {},
): Promise<void> {
  if (!conversationId) {
    return;
  }

  const deadline = Date.now() + PROVIDER_CONVERSATION_LOAD_TIMEOUT_MS;
  const failOnUrlMismatch = options.failOnUrlMismatch ?? true;
  let lastState: ProviderConversationLoadState | null = null;
  let matchedSince: number | null = null;

  while (Date.now() < deadline) {
    lastState = await readProviderConversationLoadState(page);
    if (lastState.loadErrorText) {
      throw new ProviderConversationLoadFailure(
        conversationId,
        lastState.currentUrl,
        lastState.loadErrorText,
      );
    }
    if (!failOnUrlMismatch) {
      if (conversationIdFromUrl(lastState.currentUrl) === conversationId) {
        matchedSince ??= Date.now();
        if (
          Date.now() - matchedSince >=
          PROVIDER_CONVERSATION_LOADED_STABLE_MS
        ) {
          return;
        }
      } else {
        matchedSince = null;
      }
    }
    await sleep(PROVIDER_CONVERSATION_LOAD_POLL_MS);
  }

  lastState ??= await readProviderConversationLoadState(page);
  if (
    failOnUrlMismatch &&
    conversationIdFromUrl(lastState.currentUrl) !== conversationId
  ) {
    throw new ProviderConversationLoadFailure(
      conversationId,
      lastState.currentUrl,
      `Current URL is ${lastState.currentUrl}.`,
    );
  }
}

type ProviderConversationLoadState = {
  currentUrl: string;
  loadErrorText: string | null;
};

async function readProviderConversationLoadState(
  page: Page,
): Promise<ProviderConversationLoadState> {
  return page.evaluate(`(() => {
    const candidates = [
      ...Array.from(document.querySelectorAll('[role="alert"]')),
      ...Array.from(document.querySelectorAll("[aria-live]")),
      document.body,
    ];
    const loadErrorText =
      candidates
        .map((element) => (element.textContent ?? "").replace(/\\s+/g, " ").trim())
        .find((text) => /unable to load conversation/i.test(text)) ?? null;

    return {
      currentUrl: window.location.href,
      loadErrorText,
    };
  })()`);
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
