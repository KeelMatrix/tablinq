interface ChromeRuntimeLike {
  sendMessage?: (message: CapabilityBridgeMessage) => void | Promise<unknown>;
  id?: string;
}

interface ChromeLike {
  runtime?: ChromeRuntimeLike;
}

interface ProbeDocumentOptions {
  selectorPackId?: string;
  selectorPackVersion?: string;
  reportedAt?: string;
  preferredStrategies?: Record<string, string>;
  selectorPack?: unknown;
}

interface CapabilityBridgeMessage {
  type: "capability_report";
  report: {
    reported_at: string;
  };
  runtime: unknown;
  tab?: {
    id: number | null;
    url: string | null;
    workspace_key: string | null;
  };
}

interface DomSnapshotRequest {
  source: "tablinq-page";
  type: "GET_DOM_SNAPSHOT";
  request_id: string;
}

interface ContentScriptHandle {
  publish: () => CapabilityBridgeMessage;
  dispose: () => void;
}

interface TablinqGlobal {
  __TABLINQ_PROBE_DOCUMENT__?: (
    options?: ProbeDocumentOptions,
  ) => CapabilityBridgeMessage;
}

interface TablinqWindow extends Window {
  __TABLINQ_DISABLE_CONTENT_SCRIPT_AUTOSTART__?: boolean;
}

const DOM_SNAPSHOT_MAX_CHARS = 16_000;

function probeDocument(
  options: ProbeDocumentOptions = {},
): CapabilityBridgeMessage {
  const probe = (globalThis as TablinqGlobal).__TABLINQ_PROBE_DOCUMENT__;
  if (typeof probe !== "function") {
    throw new Error("Tablinq probe helper is not available");
  }
  return probe(options);
}

function cleanDomText(maxChars = DOM_SNAPSHOT_MAX_CHARS): string {
  const root = document.body ?? document.documentElement;
  if (!root || typeof document.createTreeWalker !== "function") {
    return "";
  }

  const lines: string[] = [];
  let total = 0;
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode: (node) => {
      const parent = node.parentElement;
      if (!parent) {
        return NodeFilter.FILTER_REJECT;
      }
      const tag = parent.tagName.toLowerCase();
      if (
        tag === "script" ||
        tag === "style" ||
        tag === "noscript" ||
        tag === "template"
      ) {
        return NodeFilter.FILTER_REJECT;
      }
      const text = node.textContent?.replace(/\s+/g, " ").trim() ?? "";
      return text ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_REJECT;
    },
  });

  while (walker.nextNode()) {
    const text = walker.currentNode.textContent?.replace(/\s+/g, " ").trim();
    if (!text) {
      continue;
    }
    const remaining = maxChars - total;
    if (remaining <= 0) {
      break;
    }
    const excerpt = text.slice(0, remaining);
    lines.push(excerpt);
    total += excerpt.length + 1;
  }

  return lines.join("\n").slice(0, maxChars);
}

function installContentScript(
  options: ProbeDocumentOptions = {},
): ContentScriptHandle {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let observer: MutationObserver | undefined;
  const handleWindowMessage = (event: MessageEvent<unknown>) => {
    const message = event.data;
    if (
      event.source !== window ||
      !message ||
      typeof message !== "object" ||
      (message as DomSnapshotRequest).source !== "tablinq-page" ||
      (message as DomSnapshotRequest).type !== "GET_DOM_SNAPSHOT"
    ) {
      return;
    }
    const request = message as DomSnapshotRequest;
    window.postMessage(
      {
        source: "tablinq-content-script",
        type: "GET_DOM_SNAPSHOT_RESULT",
        request_id: request.request_id,
        dom_snapshot: cleanDomText(),
      },
      "*",
    );
  };

  const publish = (): CapabilityBridgeMessage => {
    const message = probeDocument(options);
    sendMessage(message);
    return message;
  };

  const schedulePublish = () => {
    if (timer) {
      clearTimeout(timer);
    }
    timer = setTimeout(() => {
      publish();
    }, 50);
  };

  if (typeof MutationObserver !== "undefined" && document.documentElement) {
    observer = new MutationObserver(() => {
      schedulePublish();
    });
    observer.observe(document.documentElement, {
      subtree: true,
      childList: true,
      attributes: true,
      characterData: true,
    });
  }

  window.addEventListener("load", schedulePublish, { once: true });
  window.addEventListener("message", handleWindowMessage);
  publish();

  return {
    publish,
    dispose: () => {
      if (timer) {
        clearTimeout(timer);
      }
      observer?.disconnect();
      window.removeEventListener("message", handleWindowMessage);
    },
  };
}

function sendMessage(message: CapabilityBridgeMessage): void {
  const runtime = (globalThis as { chrome?: ChromeLike }).chrome?.runtime;
  if (runtime?.sendMessage) {
    try {
      const pending = runtime.sendMessage(message);
      if (
        pending &&
        typeof (pending as Promise<unknown>).catch === "function"
      ) {
        void (pending as Promise<unknown>).catch(() => {
          window.postMessage(
            {
              source: "tablinq-content-script",
              ...message,
            },
            "*",
          );
        });
        return;
      }
      return;
    } catch {
      // The extension may be unloading while Chrome keeps the old content
      // script alive briefly. Fall through to the page bridge instead of
      // surfacing an uncaught exception in chrome://extensions.
    }
  }

  window.postMessage(
    {
      source: "tablinq-content-script",
      ...message,
    },
    "*",
  );
}

if (
  typeof window !== "undefined" &&
  typeof document !== "undefined" &&
  !(window as TablinqWindow).__TABLINQ_DISABLE_CONTENT_SCRIPT_AUTOSTART__
) {
  installContentScript();
}
