import {
  probeDocument,
  type CapabilityBridgeMessage,
  type ProbeDocumentOptions,
} from "./probe.js";

interface ChromeRuntimeLike {
  sendMessage?: (message: CapabilityBridgeMessage) => void;
  id?: string;
}

interface ChromeLike {
  runtime?: ChromeRuntimeLike;
}

declare global {
  interface Window {
    __TABLINQ_DISABLE_CONTENT_SCRIPT_AUTOSTART__?: boolean;
  }
}

export { probeDocument };

export interface ContentScriptHandle {
  publish: () => CapabilityBridgeMessage;
  dispose: () => void;
}

export function installContentScript(
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

export function sendMessage(message: CapabilityBridgeMessage): void {
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
  !window.__TABLINQ_DISABLE_CONTENT_SCRIPT_AUTOSTART__
) {
  installContentScript();
}
