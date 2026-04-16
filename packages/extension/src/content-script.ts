interface ChromeRuntimeLike {
  sendMessage?: (message: CapabilityBridgeMessage) => void;
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

function probeDocument(options: ProbeDocumentOptions = {}): CapabilityBridgeMessage {
  const probe = (globalThis as TablinqGlobal).__TABLINQ_PROBE_DOCUMENT__;
  if (typeof probe !== "function") {
    throw new Error("Tablinq probe helper is not available");
  }
  return probe(options);
}

function installContentScript(
  options: ProbeDocumentOptions = {},
): ContentScriptHandle {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let observer: MutationObserver | undefined;

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
  publish();

  return {
    publish,
    dispose: () => {
      if (timer) {
        clearTimeout(timer);
      }
      observer?.disconnect();
    },
  };
}

function sendMessage(message: CapabilityBridgeMessage): void {
  const runtime = (globalThis as { chrome?: ChromeLike }).chrome?.runtime;
  if (runtime?.sendMessage) {
    runtime.sendMessage(message);
    return;
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
