import type { CapabilityBridgeMessage } from "./probe.js";

export type BridgeTransport = "native_messaging" | "websocket_dev";

export interface BridgeStatus {
  transport: BridgeTransport;
  connected: boolean;
  last_error: string | null;
  last_reported_at: string | null;
}

interface RuntimeMessageSenderLike {
  tab?: { id?: number; url?: string };
}

interface ChromeRuntimeLike {
  onMessage?: {
    addListener: (
      listener: (
        message: CapabilityBridgeMessage,
        sender: RuntimeMessageSenderLike,
      ) => void,
    ) => void;
  };
  sendNativeMessage?: (
    host: string,
    message: CapabilityBridgeMessage,
    callback?: (response: unknown) => void,
  ) => void;
}

interface ChromeLike {
  runtime?: ChromeRuntimeLike;
}

export interface BridgeController {
  describe: () => BridgeStatus;
  handleMessage: (
    message: CapabilityBridgeMessage,
    sender?: RuntimeMessageSenderLike,
  ) => Promise<void>;
}

export interface BridgeControllerOptions {
  transport?: BridgeTransport;
  nativeHostName?: string;
  webSocketUrl?: string;
  chrome?: ChromeLike;
  createWebSocket?: (url: string) => WebSocket;
}

const defaultStatus: BridgeStatus = {
  transport: "native_messaging",
  connected: false,
  last_error: null,
  last_reported_at: null,
};

let singletonStatus: BridgeStatus = { ...defaultStatus };

export function describeBridge(): BridgeStatus {
  return { ...singletonStatus };
}

export function createBridgeController(
  options: BridgeControllerOptions = {},
): BridgeController {
  const transport = options.transport ?? pickTransport(options.webSocketUrl);
  const nativeHostName = options.nativeHostName ?? "com.tablinq.bridge";
  const webSocketUrl = options.webSocketUrl ?? "ws://127.0.0.1:4319";
  const chromeLike =
    options.chrome ?? (globalThis as { chrome?: ChromeLike }).chrome;
  const createWebSocket =
    options.createWebSocket ?? ((url: string) => new WebSocket(url));

  let socket: WebSocket | undefined;
  const status: BridgeStatus = {
    transport,
    connected: false,
    last_error: null,
    last_reported_at: null,
  };
  singletonStatus = { ...status };

  const syncStatus = () => {
    singletonStatus = { ...status };
  };

  const ensureSocket = () => {
    if (transport !== "websocket_dev") {
      return undefined;
    }
    if (socket && socket.readyState <= WebSocket.OPEN) {
      return socket;
    }
    socket = createWebSocket(webSocketUrl);
    socket.addEventListener("open", () => {
      status.connected = true;
      status.last_error = null;
      syncStatus();
    });
    socket.addEventListener("close", () => {
      status.connected = false;
      syncStatus();
    });
    socket.addEventListener("error", () => {
      status.connected = false;
      status.last_error = "websocket bridge error";
      syncStatus();
    });
    return socket;
  };

  return {
    describe: () => ({ ...status }),
    handleMessage: async (message: CapabilityBridgeMessage, sender) => {
      const annotatedMessage = annotateMessage(message, sender);
      status.last_reported_at = message.report.reported_at;

      if (
        transport === "native_messaging" &&
        chromeLike?.runtime?.sendNativeMessage
      ) {
        chromeLike.runtime.sendNativeMessage(nativeHostName, annotatedMessage);
        status.connected = true;
        status.last_error = null;
        syncStatus();
        return;
      }

      if (transport === "websocket_dev") {
        const connection = ensureSocket();
        if (!connection) {
          status.connected = false;
          status.last_error = "websocket bridge unavailable";
          syncStatus();
          return;
        }

        if (connection.readyState === WebSocket.CONNECTING) {
          await new Promise<void>((resolve) => {
            connection.addEventListener("open", () => resolve(), {
              once: true,
            });
          });
        }

        connection.send(JSON.stringify(annotatedMessage));
        status.connected = true;
        status.last_error = null;
        syncStatus();
        return;
      }

      status.connected = false;
      status.last_error = "native messaging unavailable";
      syncStatus();
    },
  };
}

export function installServiceWorkerBridge(
  options: BridgeControllerOptions = {},
): BridgeController {
  const chromeLike =
    options.chrome ?? (globalThis as { chrome?: ChromeLike }).chrome;
  const controller = createBridgeController(options);
  chromeLike?.runtime?.onMessage?.addListener((message, sender) => {
    void controller.handleMessage(message, sender);
  });
  return controller;
}

function pickTransport(webSocketUrl?: string): BridgeTransport {
  return webSocketUrl ? "websocket_dev" : "native_messaging";
}

function annotateMessage(
  message: CapabilityBridgeMessage,
  sender?: RuntimeMessageSenderLike,
): CapabilityBridgeMessage {
  const url = sender?.tab?.url ?? null;
  return {
    ...message,
    tab: {
      id: sender?.tab?.id ?? null,
      url,
      workspace_key: url ? workspaceKeyFromUrl(url) : null,
    },
  };
}

function pathnameSegments(pathname: string): string[] {
  return pathname.split("/").filter(Boolean);
}

function pathnameFromSegments(segments: string[]): string {
  return segments.length > 0 ? `/${segments.join("/")}` : "/";
}

function conversationWorkspacePathname(pathname: string): string {
  const segments = pathnameSegments(pathname);
  if (
    segments.length >= 2 &&
    segments[segments.length - 2]?.toLowerCase() === "c"
  ) {
    return pathnameFromSegments(segments.slice(0, -2));
  }
  return pathnameFromSegments(segments);
}

function workspaceKeyFromUrl(rawUrl: string): string {
  try {
    const url = new URL(rawUrl);
    const params = new URLSearchParams(url.search);
    params.delete("conversation");
    params.delete("scenario");
    const query = params.toString();
    const pathname = conversationWorkspacePathname(url.pathname);
    return `${url.origin}${pathname}${query ? `?${query}` : ""}`;
  } catch {
    return rawUrl;
  }
}

if ((globalThis as { chrome?: ChromeLike }).chrome?.runtime?.onMessage) {
  installServiceWorkerBridge({
    webSocketUrl: (globalThis as { TABLINQ_DEV_WEBSOCKET_URL?: string })
      .TABLINQ_DEV_WEBSOCKET_URL,
  });
}
