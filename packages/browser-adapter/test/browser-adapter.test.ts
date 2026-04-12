import { mkdtemp, stat } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import {
  BrowserAdapter,
  resolveBrowserLaunchConfig,
} from "../src/browserAdapter";
import { startFakeChatGptServer, stopFakeChatGptServer } from "./helpers";

function withHarnessUrl(
  baseUrl: string,
  options: {
    variant?: "classic" | "aria";
    scenario?: "success" | "timeout" | "reload";
  } = {},
): string {
  const url = new URL(baseUrl);
  if (options.variant && options.variant !== "classic") {
    url.searchParams.set("variant", options.variant);
  }
  if (options.scenario && options.scenario !== "success") {
    url.searchParams.set("scenario", options.scenario);
  }
  return url.toString();
}

describe("browser adapter", () => {
  it("launches a managed browser and opens the fake harness", async () => {
    const server = await startFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = new BrowserAdapter({
      profileDir: path.join(root, "profile"),
      downloadsDir: path.join(root, "downloads"),
      defaultTargetUrl: server.url,
    });

    const snapshot = await adapter.open({
      targetUrl: server.url,
      headless: true,
    });

    expect(snapshot.running).toBe(true);
    expect(snapshot.current_url).toContain(server.url);
    expect(snapshot.authenticated).toBe(true);
    expect(snapshot.extension_connected).toBe(true);
    expect(snapshot.capability_report?.matrix.composer_ready).toBe(true);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it.each([
    ["classic", "classic"],
    ["aria fallback", "aria"],
  ] as const)(
    "captures a full capability matrix for the %s harness",
    async (_label, variant) => {
      const server = await startFakeChatGptServer();
      const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
      const adapter = new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, { variant }),
      });

      const snapshot = await adapter.open({
        targetUrl: withHarnessUrl(server.url, { variant }),
        headless: true,
      });

      expect(snapshot.extension_connected).toBe(true);
      expect(snapshot.capability_report?.matrix).toEqual({
        composer_ready: true,
        submit_available: true,
        model_picker_visible: true,
        thinking_effort_visible: true,
        search_mode_visible: true,
        research_mode_visible: true,
        upload_control_visible: true,
        downloadable_artifacts_observable: true,
      });
      expect(adapter.getLastDomNeighborhoodsForTest().length).toBeGreaterThan(
        3,
      );

      await adapter.shutdown();
      await stopFakeChatGptServer(server.process);
    },
  );

  it("reuses the managed page on repeated open", async () => {
    const server = await startFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = new BrowserAdapter({
      profileDir: path.join(root, "profile"),
      downloadsDir: path.join(root, "downloads"),
      defaultTargetUrl: server.url,
    });

    await adapter.open({ targetUrl: server.url, headless: true });
    const firstPage = adapter.getManagedPageForTest();
    await adapter.open({ targetUrl: server.url, headless: true });
    const secondPage = adapter.getManagedPageForTest();

    expect(firstPage).toBeDefined();
    expect(secondPage).toBeDefined();
    expect(firstPage).toBe(secondPage);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("reuses the persistent profile across adapter restarts", async () => {
    const server = await startFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const profileDir = path.join(root, "profile");
    const downloadsDir = path.join(root, "downloads");

    const first = new BrowserAdapter({
      profileDir,
      downloadsDir,
      defaultTargetUrl: server.url,
    });
    await first.open({ targetUrl: server.url, headless: true });
    await first.shutdown();

    const second = new BrowserAdapter({
      profileDir,
      downloadsDir,
      defaultTargetUrl: server.url,
    });
    const snapshot = await second.open({
      targetUrl: server.url,
      headless: true,
    });

    expect(snapshot.running).toBe(true);
    expect(snapshot.profile_dir).toBe(profileDir);

    await second.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("reconnects after the managed tab is closed", async () => {
    const server = await startFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = new BrowserAdapter({
      profileDir: path.join(root, "profile"),
      downloadsDir: path.join(root, "downloads"),
      defaultTargetUrl: server.url,
    });

    await adapter.open({ targetUrl: server.url, headless: true });
    await adapter.getManagedPageForTest()!.close();

    const snapshot = await adapter.health();
    expect(snapshot.running).toBe(true);
    expect(snapshot.current_url).toContain(server.url);
    expect(snapshot.extension_connected).toBe(true);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("captures screenshots on demand", async () => {
    const server = await startFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = new BrowserAdapter({
      profileDir: path.join(root, "profile"),
      downloadsDir: path.join(root, "downloads"),
      defaultTargetUrl: server.url,
    });
    await adapter.open({ targetUrl: server.url, headless: true });

    const screenshotPath = path.join(root, "artifacts", "managed-page.png");
    const snapshot = await adapter.screenshot(screenshotPath);
    const file = await stat(screenshotPath);

    expect(file.size).toBeGreaterThan(0);
    expect(snapshot.screenshot_path).toBe(screenshotPath);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("detects reloads on the managed page", async () => {
    const server = await startFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = new BrowserAdapter({
      profileDir: path.join(root, "profile"),
      downloadsDir: path.join(root, "downloads"),
      defaultTargetUrl: server.url,
    });
    await adapter.open({ targetUrl: server.url, headless: true });

    await adapter
      .getManagedPageForTest()!
      .reload({ waitUntil: "domcontentloaded" });
    const snapshot = await adapter.health();

    expect(snapshot.reload_count).toBeGreaterThan(0);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("streams ask responses to completion", async () => {
    const server = await startFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = new BrowserAdapter({
      profileDir: path.join(root, "profile"),
      downloadsDir: path.join(root, "downloads"),
      defaultTargetUrl: server.url,
    });
    await adapter.open({ targetUrl: server.url, headless: true });

    const chunks: string[] = [];
    const result = await adapter.ask(
      {
        jobId: "job-ask-success",
        prompt: "hello streaming world",
      },
      (chunk) => {
        if (chunk.kind === "delta") {
          chunks.push(chunk.text);
        }
      },
    );

    expect(chunks.join("")).toBe("Fake response for: hello streaming world");
    expect(result.output_text).toBe("Fake response for: hello streaming world");
    expect(result.conversation_id).toMatch(/^conv-/);
    expect(result.message_id).toMatch(/^msg-/);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("fails cleanly when the harness never completes streaming", async () => {
    const server = await startFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = new BrowserAdapter({
      profileDir: path.join(root, "profile"),
      downloadsDir: path.join(root, "downloads"),
      defaultTargetUrl: withHarnessUrl(server.url, { scenario: "timeout" }),
    });
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, { scenario: "timeout" }),
      headless: true,
    });

    await expect(
      adapter.ask(
        {
          jobId: "job-ask-timeout",
          prompt: "never finish",
          timeoutMs: 600,
        },
        () => {},
      ),
    ).rejects.toThrow("timed out");

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("continues ask streaming after a page reload", async () => {
    const server = await startFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = new BrowserAdapter({
      profileDir: path.join(root, "profile"),
      downloadsDir: path.join(root, "downloads"),
      defaultTargetUrl: withHarnessUrl(server.url, { scenario: "reload" }),
    });
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, { scenario: "reload" }),
      headless: true,
    });

    const result = await adapter.ask(
      {
        jobId: "job-ask-reload",
        prompt: "survive reloads",
        timeoutMs: 5000,
      },
      () => {},
    );

    expect(result.output_text).toBe("Fake response for: survive reloads");
    expect((await adapter.health()).reload_count).toBeGreaterThan(0);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });
});

describe("browser launch config", () => {
  it("prefers an explicit browser path", () => {
    const config = resolveBrowserLaunchConfig(
      "win32",
      "C:\\Browsers\\msedge.exe",
    );
    expect(config).toEqual({
      browserKind: "custom",
      executablePath: "C:\\Browsers\\msedge.exe",
    });
  });

  it("defaults to Edge on Windows", () => {
    const config = resolveBrowserLaunchConfig("win32");
    expect(config.browserKind).toBe("msedge");
    expect(config.channel).toBe("msedge");
  });
});
