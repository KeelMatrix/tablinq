import { mkdir, mkdtemp, readFile, stat, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import type { ChildProcess } from "node:child_process";
import {
  BrowserAdapter,
  chooseUniqueArtifactPath,
  resolveBrowserLaunchConfig,
} from "../src/browserAdapter";
import { startFakeChatGptServer, stopFakeChatGptServer } from "./helpers";

function withHarnessUrl(
  baseUrl: string,
  options: {
    variant?: "classic" | "aria" | "live";
    scenario?:
      | "success"
      | "timeout"
      | "reload"
      | "research-success"
      | "research-failure";
    missing?: string[];
  } = {},
): string {
  const url = new URL(baseUrl);
  if (options.variant && options.variant !== "classic") {
    url.searchParams.set("variant", options.variant);
  }
  if (options.scenario && options.scenario !== "success") {
    url.searchParams.set("scenario", options.scenario);
  }
  if (options.missing && options.missing.length > 0) {
    url.searchParams.set("missing", options.missing.join(","));
  }
  return url.toString();
}

let trackedAdapters: BrowserAdapter[] = [];
let trackedServers: ChildProcess[] = [];

function trackAdapter(adapter: BrowserAdapter): BrowserAdapter {
  trackedAdapters.push(adapter);
  return adapter;
}

async function startTrackedFakeChatGptServer() {
  const server = await startFakeChatGptServer();
  trackedServers.push(server.process);
  return server;
}

beforeEach(() => {
  trackedAdapters = [];
  trackedServers = [];
});

afterEach(async () => {
  await Promise.allSettled(
    trackedAdapters
      .slice()
      .reverse()
      .map((adapter) => adapter.shutdown()),
  );
  await Promise.allSettled(
    trackedServers
      .slice()
      .reverse()
      .map((server) => stopFakeChatGptServer(server)),
  );
});

describe("browser adapter", () => {
  it("launches a managed browser and opens the fake harness", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(new BrowserAdapter({
      profileDir: path.join(root, "profile"),
      downloadsDir: path.join(root, "downloads"),
      defaultTargetUrl: server.url,
    }));

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
    ["live-like", "live"],
  ] as const)(
    "captures a full capability matrix for the %s harness",
    async (_label, variant) => {
      const server = await startTrackedFakeChatGptServer();
      const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
      const adapter = trackAdapter(new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, { variant }),
      }));

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
      expect(
        snapshot.capability_report?.aliases.model_classes.instant,
      ).toBeTruthy();
      expect(
        snapshot.capability_report?.aliases.model_classes.thinking,
      ).toBeTruthy();
      const diagnostics = await adapter.diagnostics();
      expect(diagnostics.selector_pack?.pack_id).toBe("chatgpt-default");
      expect(diagnostics.strategy_attempts.length).toBeGreaterThan(3);
      expect(diagnostics.strategy_telemetry.length).toBeGreaterThan(0);
      expect(adapter.getLastDomNeighborhoodsForTest().length).toBeGreaterThan(
        3,
      );

      await adapter.shutdown();
      await stopFakeChatGptServer(server.process);
    },
  );

  it("reuses the managed page on repeated open", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(new BrowserAdapter({
      profileDir: path.join(root, "profile"),
      downloadsDir: path.join(root, "downloads"),
      defaultTargetUrl: server.url,
    }));

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
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const profileDir = path.join(root, "profile");
    const downloadsDir = path.join(root, "downloads");

    const first = trackAdapter(new BrowserAdapter({
      profileDir,
      downloadsDir,
      defaultTargetUrl: server.url,
    }));
    await first.open({ targetUrl: server.url, headless: true });
    await first.shutdown();

    const second = trackAdapter(new BrowserAdapter({
      profileDir,
      downloadsDir,
      defaultTargetUrl: server.url,
    }));
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
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(new BrowserAdapter({
      profileDir: path.join(root, "profile"),
      downloadsDir: path.join(root, "downloads"),
      defaultTargetUrl: server.url,
    }));

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
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(new BrowserAdapter({
      profileDir: path.join(root, "profile"),
      downloadsDir: path.join(root, "downloads"),
      defaultTargetUrl: server.url,
    }));
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
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(new BrowserAdapter({
      profileDir: path.join(root, "profile"),
      downloadsDir: path.join(root, "downloads"),
      defaultTargetUrl: server.url,
    }));
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
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(new BrowserAdapter({
      profileDir: path.join(root, "profile"),
      downloadsDir: path.join(root, "downloads"),
      defaultTargetUrl: server.url,
    }));
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

  it("refuses asks when the managed page is a signed-out local login screen", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(new BrowserAdapter({
      profileDir: path.join(root, "profile"),
      downloadsDir: path.join(root, "downloads"),
      defaultTargetUrl: `${server.url}/login-expired.html`,
    }));
    await adapter.open({
      targetUrl: `${server.url}/login-expired.html`,
      headless: true,
    });

    await expect(
      adapter.ask(
        {
          jobId: "job-ask-auth-expired",
          prompt: "should refuse while signed out",
        },
        () => {},
      ),
    ).rejects.toThrow("auth required");

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("selects model, effort, and search mode on the classic harness", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(new BrowserAdapter({
      profileDir: path.join(root, "profile"),
      downloadsDir: path.join(root, "downloads"),
      defaultTargetUrl: server.url,
    }));
    await adapter.open({ targetUrl: server.url, headless: true });

    await adapter.ask(
      {
        jobId: "job-ask-configured-classic",
        prompt: "configure classic",
        mode: "search",
        modelClass: "instant",
        effort: "extended",
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      mode: document.body.dataset.selectedMode,
      model: document.body.dataset.selectedModel,
      effort: document.body.dataset.selectedEffort,
    }));

    expect(state).toEqual({
      mode: "search",
      model: "instant",
      effort: "extended",
    });

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("selects renamed controls on the aria harness", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(new BrowserAdapter({
      profileDir: path.join(root, "profile"),
      downloadsDir: path.join(root, "downloads"),
      defaultTargetUrl: withHarnessUrl(server.url, { variant: "aria" }),
    }));
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, { variant: "aria" }),
      headless: true,
    });

    await adapter.ask(
      {
        jobId: "job-ask-configured-aria",
        prompt: "configure aria",
        mode: "research",
        modelClass: "thinking",
        effort: "extended",
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      mode: document.body.dataset.selectedMode,
      model: document.body.dataset.selectedModel,
      effort: document.body.dataset.selectedEffort,
    }));

    expect(state).toEqual({
      mode: "research",
      model: "thinking",
      effort: "extended",
    });

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("drives the live-like harness through ask, download recovery, and ready-state checks", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const fixture = path.join(root, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    const adapter = trackAdapter(new BrowserAdapter({
      profileDir: path.join(root, "profile"),
      downloadsDir: path.join(root, "downloads"),
      defaultTargetUrl: withHarnessUrl(server.url, { variant: "live" }),
    }));
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, { variant: "live" }),
      headless: true,
    });

    const result = await adapter.ask(
      {
        jobId: "job-ask-live-variant",
        prompt: "configure live",
        mode: "research",
        modelClass: "instant",
        effort: "extended",
        files: [fixture],
      },
      () => {},
    );
    const artifactBody = await readFile(
      result.artifacts[0]!.storage_path,
      "utf8",
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      currentUrl: window.location.href,
      composerReady: Boolean(document.querySelector("#prompt-textarea")),
      sendHidden:
        document
          .querySelector("[data-testid='send-button']")
          ?.hasAttribute("hidden") ?? false,
      stopHidden:
        document
          .querySelector("[data-testid='stop-button']")
          ?.hasAttribute("hidden") ?? true,
      uploadChipCount: document.querySelectorAll("[data-testid='upload-chip']")
        .length,
    }));

    expect(state).toEqual({
      currentUrl: withHarnessUrl(server.url, {
        variant: "live",
      }).concat(`&conversation=${result.conversation_id}`),
      composerReady: true,
      sendHidden: false,
      stopHidden: true,
      uploadChipCount: 0,
    });
    expect(result.output_text).toBe("Fake response for: configure live");
    expect(result.artifacts).toHaveLength(1);
    expect(artifactBody).toContain("Mode: research");
    expect(artifactBody).toContain("Model: instant");
    expect(artifactBody).toContain("Effort: extended");
    expect(artifactBody).toContain("Uploads: brief.txt");

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("fails with a capability-missing error when search mode is unavailable", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(new BrowserAdapter({
      profileDir: path.join(root, "profile"),
      downloadsDir: path.join(root, "downloads"),
      defaultTargetUrl: withHarnessUrl(server.url, { missing: ["search"] }),
    }));
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, { missing: ["search"] }),
      headless: true,
    });

    await expect(
      adapter.ask(
        {
          jobId: "job-ask-missing-search",
          prompt: "search for docs",
          mode: "search",
        },
        () => {},
      ),
    ).rejects.toThrow(/capability missing: search mode/i);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("uploads files and persists generated artifacts", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const fixtureDir = path.join(root, "fixtures");
    const firstFile = path.join(fixtureDir, "brief.txt");
    const secondFile = path.join(fixtureDir, "notes.csv");
    await mkdir(fixtureDir, { recursive: true });
    await writeFile(firstFile, "brief body", "utf8");
    await writeFile(secondFile, "col\nvalue", "utf8");

    const adapter = trackAdapter(new BrowserAdapter({
      profileDir: path.join(root, "profile"),
      downloadsDir: path.join(root, "downloads"),
      defaultTargetUrl: server.url,
    }));
    await adapter.open({ targetUrl: server.url, headless: true });

    const result = await adapter.ask(
      {
        jobId: "job-ask-upload-artifact",
        prompt: "summarize uploads",
        files: [firstFile, secondFile],
      },
      () => {},
    );

    expect(result.artifacts).toHaveLength(1);
    expect(result.artifacts[0]!.file_name).toContain("brief-summary");
    expect(result.artifacts[0]!.content_type).toBe("text/plain; charset=utf-8");
    expect(result.artifacts[0]!.size_bytes).toBeGreaterThan(0);
    const downloaded = await stat(result.artifacts[0]!.storage_path);
    expect(downloaded.size).toBeGreaterThan(0);
    const artifactBody = await readFile(
      result.artifacts[0]!.storage_path,
      "utf8",
    );
    expect(artifactBody).toContain("Uploads: brief.txt, notes.csv");

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("runs research jobs with progress updates and persisted artifacts", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const fixture = path.join(root, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    const adapter = trackAdapter(new BrowserAdapter({
      profileDir: path.join(root, "profile"),
      downloadsDir: path.join(root, "downloads"),
      defaultTargetUrl: withHarnessUrl(server.url, {
        scenario: "research-success",
      }),
    }));
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, { scenario: "research-success" }),
      headless: true,
    });

    const updates: string[] = [];
    const result = await adapter.research(
      {
        jobId: "job-research-success",
        query: "investigate launch risks",
        files: [fixture],
        timeoutMs: 5_000,
      },
      (update) => {
        updates.push(update.status);
      },
    );

    expect(updates).toContain("starting");
    expect(updates).toContain("waiting_for_sources");
    expect(updates).toContain("running");
    expect(result.report_text).toContain(
      "Research report for: investigate launch risks",
    );
    expect(result.artifacts).toHaveLength(1);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("fails research jobs with a research-specific error", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(new BrowserAdapter({
      profileDir: path.join(root, "profile"),
      downloadsDir: path.join(root, "downloads"),
      defaultTargetUrl: withHarnessUrl(server.url, {
        scenario: "research-failure",
      }),
    }));
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, { scenario: "research-failure" }),
      headless: true,
    });

    await expect(
      adapter.research(
        {
          jobId: "job-research-failure",
          query: "investigate flaky providers",
          timeoutMs: 5_000,
        },
        () => {},
      ),
    ).rejects.toThrow(/research job failed/i);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("fails with an upload error when files are requested without an upload control", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const fixture = path.join(root, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    const adapter = trackAdapter(new BrowserAdapter({
      profileDir: path.join(root, "profile"),
      downloadsDir: path.join(root, "downloads"),
      defaultTargetUrl: withHarnessUrl(server.url, { missing: ["upload"] }),
    }));
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, { missing: ["upload"] }),
      headless: true,
    });

    await expect(
      adapter.ask(
        {
          jobId: "job-ask-missing-upload",
          prompt: "summarize uploads",
          files: [fixture],
        },
        () => {},
      ),
    ).rejects.toThrow(/upload failed/i);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("fails cleanly when the harness never completes streaming", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(new BrowserAdapter({
      profileDir: path.join(root, "profile"),
      downloadsDir: path.join(root, "downloads"),
      defaultTargetUrl: withHarnessUrl(server.url, { scenario: "timeout" }),
    }));
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

    const diagnostics = await adapter.diagnostics(path.join(root, "bundle"));
    expect(diagnostics.screenshot_path).toContain("failure.png");
    expect(diagnostics.dom_snapshot).toContain("body_excerpt");
    expect(diagnostics.aria_snapshot).toContain("role");

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("continues ask streaming after a page reload", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(new BrowserAdapter({
      profileDir: path.join(root, "profile"),
      downloadsDir: path.join(root, "downloads"),
      defaultTargetUrl: withHarnessUrl(server.url, { scenario: "reload" }),
    }));
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

  it("allocates unique artifact paths without overwriting existing files", async () => {
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-artifacts-"));
    const first = await chooseUniqueArtifactPath(root, "report.txt");
    await writeFile(first, "first", "utf8");
    const second = await chooseUniqueArtifactPath(root, "report.txt");

    expect(path.basename(first)).toBe("report.txt");
    expect(path.basename(second)).toBe("report-2.txt");
  });
});
