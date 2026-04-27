import type { BrowserAdapter } from "../browserAdapter";
import type { BrowserStatusSnapshot } from "../contracts";
import {
  normalizeAskCommand,
  normalizeAskRecoveryState,
  normalizeResearchCommand,
  normalizeResearchRecoveryState,
} from "./normalization";
import type { AdapterFrame, AdapterRequest, ControlRequest } from "./protocol";
import {
  type FrameWriter,
  requestStateFrame,
  researchUpdateFrame,
  streamChunkFrame,
} from "./frames";

export type DispatchOutcome = {
  shutdown: boolean;
};

export async function dispatchAdapterRequest(
  adapter: BrowserAdapter,
  request: AdapterRequest,
  write: FrameWriter,
): Promise<DispatchOutcome> {
  switch (request.type) {
    case "ask":
    case "recover_ask":
      await dispatchAskRequest(adapter, request, write);
      return { shutdown: false };
    case "research":
    case "recover_research":
      await dispatchResearchRequest(adapter, request, write);
      return { shutdown: false };
    case "diagnostics":
      await dispatchDiagnosticsRequest(adapter, request, write);
      return { shutdown: false };
    case "open":
    case "health":
    case "close":
    case "shutdown": {
      const state = await handleControlRequest(adapter, request);
      write({
        type: "response",
        ok: true,
        state,
      });
      return { shutdown: request.type === "shutdown" };
    }
  }
}

export async function handleControlRequest(
  adapter: BrowserAdapter,
  request: ControlRequest,
): Promise<BrowserStatusSnapshot> {
  switch (request.type) {
    case "open":
      return adapter.open({
        targetUrl: request.target_url,
        browserPath: request.browser_path,
        headless: request.headless,
        forceReload: request.force_reload,
      });
    case "health":
      return adapter.health();
    case "close":
      return adapter.close();
    case "shutdown":
      return adapter.shutdown();
  }
}

async function dispatchAskRequest(
  adapter: BrowserAdapter,
  request: Extract<AdapterRequest, { type: "ask" | "recover_ask" }>,
  write: FrameWriter,
): Promise<void> {
  const command = normalizeAskCommand(request);
  const result =
    request.type === "recover_ask"
      ? await adapter.recoverAsk(
          command,
          normalizeAskRecoveryState(request),
          (chunk) => {
            write(streamChunkFrame(chunk));
          },
          (update) => {
            write(requestStateFrame(update));
          },
        )
      : await adapter.ask(
          command,
          (chunk) => {
            write(streamChunkFrame(chunk));
          },
          (update) => {
            write(requestStateFrame(update));
          },
        );

  write({
    type: "ask_result",
    ok: true,
    state: await adapter.snapshot(),
    result,
  });
}

async function dispatchResearchRequest(
  adapter: BrowserAdapter,
  request: Extract<AdapterRequest, { type: "research" | "recover_research" }>,
  write: FrameWriter,
): Promise<void> {
  const command = normalizeResearchCommand(request);
  const result =
    request.type === "recover_research"
      ? await adapter.recoverResearch(
          command,
          normalizeResearchRecoveryState(request),
          (update) => {
            write(researchUpdateFrame(update));
          },
          (update) => {
            write(requestStateFrame(update));
          },
        )
      : await adapter.research(
          command,
          (update) => {
            write(researchUpdateFrame(update));
          },
          (update) => {
            write(requestStateFrame(update));
          },
        );

  write({
    type: "research_result",
    ok: true,
    state: await adapter.snapshot(),
    result,
  });
}

async function dispatchDiagnosticsRequest(
  adapter: BrowserAdapter,
  request: Extract<AdapterRequest, { type: "diagnostics" }>,
  write: FrameWriter,
): Promise<void> {
  const state = await adapter.snapshot();
  const diagnostics = await adapter.diagnostics(
    request.bundle_dir ?? undefined,
  );
  const frame: AdapterFrame = {
    type: "diagnostics",
    ok: true,
    state,
    diagnostics,
  };
  write(frame);
}
