import type { Locator, Page } from "playwright-core";

import { INTERACTION_TIMEOUT_MS } from "../../timing";

export async function waitForVisibleLocator(
  locator: Locator,
  timeoutMs: number,
): Promise<Locator | null> {
  const candidate = locator.first();
  try {
    await candidate.waitFor({ state: "visible", timeout: timeoutMs });
    return candidate;
  } catch {
    return null;
  }
}

export async function findVisiblePopupChoicePoint(
  page: Page,
  token: string,
  timeoutMs = INTERACTION_TIMEOUT_MS,
): Promise<{ locator: Locator; checked: string | null } | null> {
  const popupChoices = page.locator(
    "[role='menuitemradio']:visible, [role='menuitem']:visible, [role='option']:visible",
  );
  const deadline = Date.now() + timeoutMs;
  const expectedToken = token.toLowerCase();
  while (Date.now() < deadline) {
    const count = await popupChoices.count();
    for (let index = 0; index < count; index += 1) {
      const locator = popupChoices.nth(index);
      const text = ((await locator.textContent()) ?? "")
        .replace(/\s+/g, " ")
        .trim()
        .toLowerCase();
      if (!text.includes(expectedToken)) {
        continue;
      }
      return {
        locator,
        checked: await locator.getAttribute("aria-checked"),
      };
    }
    await page.waitForTimeout(100);
  }
  return null;
}

export async function readToggleState(locator: Locator): Promise<boolean> {
  const pressed = await locator.getAttribute("aria-pressed").catch(() => null);
  if (pressed !== null) {
    return pressed === "true";
  }
  const checked = await locator.getAttribute("aria-checked").catch(() => null);
  if (checked !== null) {
    return checked === "true";
  }
  return false;
}

export async function selectedChoiceMatches(
  page: Page,
  tokens: readonly string[],
): Promise<boolean> {
  const expectedTokens = tokens
    .map((value) => value.trim().toLowerCase())
    .filter(Boolean);
  if (expectedTokens.length === 0) {
    return false;
  }

  return page
    .evaluate<boolean>(
      `(() => {
      const tokensToMatch = ${JSON.stringify(expectedTokens)};
      const selected = Array.from(
        document.querySelectorAll(
          [
            "[aria-pressed='true']",
            "[aria-checked='true']",
            "[data-selected='true']",
            "[data-state='checked']",
          ].join(", "),
        ),
      );
      for (const element of selected) {
        const text = [
          element.textContent ?? "",
          element.getAttribute("aria-label") ?? "",
          element.getAttribute("data-model") ?? "",
          element.getAttribute("data-effort") ?? "",
          element.getAttribute("data-mode") ?? "",
        ]
          .join(" ")
          .replace(/\\s+/g, " ")
          .trim()
          .toLowerCase();
        if (tokensToMatch.some((token) => text.includes(token))) {
          return true;
        }
      }
      return false;
    })()`,
    )
    .catch(() => false);
}

export async function findVisibleInlineChoicePoint(
  page: Page,
  selectors: string,
  tokens: readonly string[],
  timeoutMs = INTERACTION_TIMEOUT_MS,
): Promise<{ locator: Locator; selected: boolean } | null> {
  const inlineChoices = page.locator(selectors);
  const deadline = Date.now() + timeoutMs;
  const expectedTokens = tokens
    .map((value) => value.trim().toLowerCase())
    .filter(Boolean);
  while (Date.now() < deadline) {
    const count = await inlineChoices.count();
    for (let index = 0; index < count; index += 1) {
      const locator = inlineChoices.nth(index);
      if (!(await locator.isVisible().catch(() => false))) {
        continue;
      }
      const text = [
        (await locator.textContent().catch(() => null)) ?? "",
        (await locator.getAttribute("aria-label").catch(() => null)) ?? "",
      ]
        .join(" ")
        .replace(/\s+/g, " ")
        .trim()
        .toLowerCase();
      if (!expectedTokens.some((token) => text.includes(token))) {
        continue;
      }
      return {
        locator,
        selected: await readToggleState(locator),
      };
    }
    await page.waitForTimeout(100);
  }
  return null;
}

export async function closeOpenComposerMenus(page: Page): Promise<void> {
  const openMenu = page.locator(
    [
      "[role='menu']:visible",
      "[role='listbox']:visible",
      "[data-radix-popper-content-wrapper] [role='menu']:visible",
      "[data-radix-popper-content-wrapper] [role='listbox']:visible",
    ].join(", "),
  );
  if ((await openMenu.count().catch(() => 0)) === 0) {
    return;
  }
  await page.keyboard.press("Escape").catch(() => undefined);
  await page.waitForTimeout(300);
}
