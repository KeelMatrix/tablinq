import { readFile } from "node:fs/promises";
import { JSDOM } from "jsdom";

import { collectArtifacts } from "../src/pageRuntime/artifacts";
import { createIsolatedTempDir } from "./browser-adapter.integration.helpers";

class FakeElementLocator {
  constructor(private readonly element: Element) {}

  async evaluate<TArg, TResult>(
    pageFunction: (element: Element, arg: TArg) => TResult | Promise<TResult>,
    arg?: TArg,
  ): Promise<TResult> {
    return pageFunction(this.element, arg as TArg);
  }

  async click(): Promise<void> {}
}

class FakeLocator {
  constructor(private readonly elements: Element[]) {}

  first(): FakeLocator {
    return new FakeLocator(this.elements.slice(0, 1));
  }

  async count(): Promise<number> {
    return this.elements.length;
  }

  locator(selector: string): FakeLocator {
    const selectorRules = selector
      .split(",")
      .map((part) => {
        const trimmed = part.trim();
        const textMatch = trimmed.match(/:has-text\((['"])(.*?)\1\)/);
        return {
          base: trimmed.replace(/:has-text\((['"]).*?\1\)/g, "").trim(),
          text: textMatch?.[2] ?? null,
        };
      })
      .filter((rule) => Boolean(rule.base));
    return new FakeLocator(
      this.elements.flatMap((element) =>
        selectorRules.flatMap((rule) =>
          Array.from(element.querySelectorAll(rule.base)).filter((candidate) =>
            rule.text
              ? (candidate.textContent ?? "").includes(rule.text)
              : true,
          ),
        ),
      ),
    );
  }

  async all(): Promise<FakeElementLocator[]> {
    return this.elements.map((element) => new FakeElementLocator(element));
  }
}

class FakePage {
  constructor(
    private readonly document: Document,
    private readonly waitForEventImpl: () => Promise<never> = async () => {
      throw new Error("unexpected download");
    },
  ) {}

  locator(selector: string): FakeLocator {
    return new FakeLocator(
      Array.from(this.document.querySelectorAll(selector)),
    );
  }

  async waitForEvent(): Promise<never> {
    return this.waitForEventImpl();
  }
}

async function withDomGlobals<T>(
  dom: JSDOM,
  run: () => Promise<T>,
): Promise<T> {
  const { window } = dom;
  const previous = {
    HTMLElement: globalThis.HTMLElement,
    Node: globalThis.Node,
    HTMLAnchorElement: globalThis.HTMLAnchorElement,
    HTMLCanvasElement: globalThis.HTMLCanvasElement,
    HTMLImageElement: globalThis.HTMLImageElement,
    DOMParser: globalThis.DOMParser,
    TextDecoder: globalThis.TextDecoder,
    TextEncoder: globalThis.TextEncoder,
    atob: globalThis.atob,
    btoa: globalThis.btoa,
  };

  Object.assign(globalThis, {
    HTMLElement: window.HTMLElement,
    Node: window.Node,
    HTMLAnchorElement: window.HTMLAnchorElement,
    HTMLCanvasElement: window.HTMLCanvasElement,
    HTMLImageElement: window.HTMLImageElement,
    DOMParser: window.DOMParser,
    TextDecoder: globalThis.TextDecoder,
    TextEncoder: globalThis.TextEncoder,
    atob: window.atob.bind(window),
    btoa: window.btoa.bind(window),
  });

  try {
    return await run();
  } finally {
    Object.assign(globalThis, previous);
    dom.window.close();
  }
}

function mermaidPreviewMarkup(svgText: string): string {
  return `
    <div class="markdown">
      <pre class="overflow-visible! px-0!">
        <div class="sticky z-2 select-none top-(--sticky-padding-top)">
          <div class="flex w-full items-center justify-between py-1.5 ps-4 pe-1.5 font-sans md:ps-5">
            <div>Mermaid</div>
            <div class="flex flex-row items-center gap-0.5 justify-self-end">
              <button aria-label="Download PNG" type="button"></button>
              <button aria-label="Code" type="button" aria-pressed="false"></button>
              <button aria-label="Preview" type="button" aria-pressed="true"></button>
            </div>
          </div>
        </div>
        <div class="flex w-full flex-col items-center">
          <img src="data:image/svg+xml;utf8,${encodeURIComponent(svgText)}" />
        </div>
      </pre>
    </div>
  `;
}

describe("browser adapter artifacts", () => {
  it("collects latest-message mermaid previews as inline SVG artifacts", async () => {
    const downloadsDir = await createIsolatedTempDir("artifact-inline-mermaid");
    const dom = new JSDOM(
      `
        <!doctype html>
        <html>
          <body>
            <div data-message-id="msg-older">
              ${mermaidPreviewMarkup(
                '<svg xmlns="http://www.w3.org/2000/svg"><text>Older</text></svg>',
              )}
            </div>
            <div data-message-id="msg-latest">
              ${mermaidPreviewMarkup(
                '<svg xmlns="http://www.w3.org/2000/svg"><text>Exercise</text><text>Recovery Loop</text></svg>',
              )}
            </div>
          </body>
        </html>
      `,
      { url: "https://chatgpt.com/c/conv-inline-mermaid" },
    );

    const page = new FakePage(dom.window.document);
    const artifacts = await withDomGlobals(dom, () =>
      collectArtifacts(
        page as unknown as any,
        downloadsDir,
        "conv-inline-mermaid",
        "msg-latest",
      ),
    );

    expect(artifacts).toHaveLength(1);
    expect(artifacts[0]!.file_name).toMatch(/\.svg$/);
    expect(artifacts[0]!.content_type).toBe("image/svg+xml");

    const saved = await readFile(artifacts[0]!.storage_path, "utf8");
    expect(saved).toContain("<svg");
    expect(saved).toContain("Exercise");
    expect(saved).toContain("Recovery Loop");
  });

  it("skips preview-only mermaid download controls when no captureable payload exists", async () => {
    const downloadsDir = await createIsolatedTempDir("artifact-inline-skip");
    const dom = new JSDOM(
      `
        <!doctype html>
        <html>
          <body>
            <div data-message-id="msg-preview-only">
              <div class="markdown">
                <pre>
                  <div>Mermaid</div>
                  <button aria-label="Download PNG" type="button"></button>
                  <button aria-label="Code" type="button" aria-pressed="false"></button>
                  <button aria-label="Preview" type="button" aria-pressed="true"></button>
                </pre>
              </div>
            </div>
          </body>
        </html>
      `,
      { url: "https://chatgpt.com/c/conv-inline-skip" },
    );

    const page = new FakePage(dom.window.document, async () => {
      throw new Error("download event unavailable");
    });
    const artifacts = await withDomGlobals(dom, () =>
      collectArtifacts(
        page as unknown as any,
        downloadsDir,
        "conv-inline-skip",
        "msg-preview-only",
      ),
    );

    expect(artifacts).toEqual([]);
  });

  it("captures inline text document cards without clicking their download menu", async () => {
    const downloadsDir = await createIsolatedTempDir("artifact-inline-textdoc");
    const dom = new JSDOM(
      `
        <!doctype html>
        <html>
          <body>
            <div data-message-id="msg-textdoc">
              <div id="textdoc-message-1" class="popover">
                <div>
                  <span class="font-semibold">Tablinq Smoke Report Template</span>
                  <button aria-label="Copy">Copy</button>
                  <button>
                    <span>Download</span>
                    <button>Download</button>
                  </button>
                </div>
                <section class="popover">
                  <div class="markdown ProseMirror" contenteditable="false">
                    <h1><span>Tablinq Smoke Report</span></h1>
                    <h2><span>Recovery</span></h2>
                    <ul>
                      <li><p><span>Final State:</span></p></li>
                    </ul>
                  </div>
                </section>
              </div>
            </div>
          </body>
        </html>
      `,
      { url: "https://chatgpt.com/c/conv-textdoc" },
    );

    const page = new FakePage(dom.window.document, async () => {
      throw new Error("download menu should not be clicked");
    });
    const artifacts = await withDomGlobals(dom, () =>
      collectArtifacts(
        page as unknown as any,
        downloadsDir,
        "conv-textdoc",
        "msg-textdoc",
      ),
    );

    expect(artifacts).toHaveLength(1);
    expect(artifacts[0]!.file_name).toMatch(
      /tablinq-smoke-report-template.*\.md$/,
    );
    expect(artifacts[0]!.content_type).toBe("text/markdown; charset=utf-8");
    const saved = await readFile(artifacts[0]!.storage_path, "utf8");
    expect(saved).toContain("# Tablinq Smoke Report");
    expect(saved).toContain("- Final State:");
  });
});
