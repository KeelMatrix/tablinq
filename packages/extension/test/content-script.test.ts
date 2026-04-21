// @vitest-environment jsdom

import { afterEach, beforeEach, expect, it, vi } from "vitest";

declare global {
  interface Window {
    __TABLINQ_PROBE_DOCUMENT__?: (options?: unknown) => {
      type: "capability_report";
      report: { reported_at: string };
      runtime: Record<string, unknown>;
    };
    __TABLINQ_DISABLE_CONTENT_SCRIPT_AUTOSTART__?: boolean;
  }

  // eslint-disable-next-line no-var
  var chrome:
    | {
        runtime?: {
          sendMessage?: (message: unknown) => void | Promise<unknown>;
        };
      }
    | undefined;
}

beforeEach(() => {
  vi.resetModules();
  document.body.innerHTML = "";
  document.documentElement.innerHTML = "<body></body>";
  window.__TABLINQ_DISABLE_CONTENT_SCRIPT_AUTOSTART__ = false;
  window.__TABLINQ_PROBE_DOCUMENT__ = () => ({
    type: "capability_report",
    report: { reported_at: "2026-04-19T00:00:00Z" },
    runtime: {},
  });
});

afterEach(() => {
  delete window.__TABLINQ_PROBE_DOCUMENT__;
  delete window.__TABLINQ_DISABLE_CONTENT_SCRIPT_AUTOSTART__;
  globalThis.chrome = undefined;
  vi.restoreAllMocks();
});

it("falls back to page messaging when runtime.sendMessage rejects asynchronously", async () => {
  const postMessageSpy = vi
    .spyOn(window, "postMessage")
    .mockImplementation(() => undefined);
  globalThis.chrome = {
    runtime: {
      sendMessage: () =>
        Promise.reject(new Error("Extension context invalidated")),
    },
  };

  await import("../src/content-script");
  await Promise.resolve();
  await Promise.resolve();

  expect(postMessageSpy).toHaveBeenCalledWith(
    expect.objectContaining({
      source: "tablinq-content-script",
      type: "capability_report",
    }),
    "*",
  );
});

it("responds to dom snapshot requests with cleaned text", async () => {
  document.body.innerHTML =
    "<main><h1>Alpha</h1><script>secret-token</script><style>.hidden{display:none}</style><p>Beta text</p></main>";
  const postMessageSpy = vi
    .spyOn(window, "postMessage")
    .mockImplementation(() => undefined);

  await import("../src/content-script");

  window.dispatchEvent(
    new MessageEvent("message", {
      source: window,
      data: {
        source: "tablinq-page",
        type: "GET_DOM_SNAPSHOT",
        request_id: "req-dom-1",
      },
    }),
  );

  const domResponse = postMessageSpy.mock.calls.find(
    ([message]) =>
      (message as { type?: string }).type === "GET_DOM_SNAPSHOT_RESULT",
  )?.[0] as { dom_snapshot?: string; request_id?: string } | undefined;

  expect(domResponse?.request_id).toBe("req-dom-1");
  expect(domResponse?.dom_snapshot).toContain("Alpha");
  expect(domResponse?.dom_snapshot).toContain("Beta text");
  expect(domResponse?.dom_snapshot).not.toContain("secret-token");
});
