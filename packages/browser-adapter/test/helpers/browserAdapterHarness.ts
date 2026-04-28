import { resetProviderPressureForTest } from "../../src/browserAdapter";
import { createBrowserIntegrationHarness } from "../browser-adapter.integration.helpers";

type BrowserIntegrationHarness = ReturnType<
  typeof createBrowserIntegrationHarness
>;

export function installBrowserAdapterHarness(): BrowserIntegrationHarness {
  let harness: BrowserIntegrationHarness | null = null;

  beforeEach(() => {
    harness = createBrowserIntegrationHarness();
    resetProviderPressureForTest();
  });

  afterEach(async () => {
    resetProviderPressureForTest();
    await harness?.cleanup();
    harness = null;
  });

  function currentHarness(): BrowserIntegrationHarness {
    if (!harness) {
      throw new Error("browser adapter test harness is not active");
    }
    return harness;
  }

  return new Proxy({} as BrowserIntegrationHarness, {
    get(_target, property) {
      const value = currentHarness()[property as keyof BrowserIntegrationHarness];
      if (typeof value === "function") {
        return value.bind(currentHarness());
      }
      return value;
    },
  });
}
