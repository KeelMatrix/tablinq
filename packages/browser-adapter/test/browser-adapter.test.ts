import { mkdtemp, stat } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import {
  BrowserAdapter,
  resolveBrowserLaunchConfig,
} from "../src/browserAdapter";
import { startFakeChatGptServer, stopFakeChatGptServer } from "./helpers";

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

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

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
