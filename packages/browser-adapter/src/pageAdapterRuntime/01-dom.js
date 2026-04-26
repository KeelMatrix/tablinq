function createDomUtilsModule({ textUtils }) {
  const {
    normalizeText,
    unique,
    hasToken,
  } = textUtils;

  const getComputed = (element) => {
    if (
      typeof window === "undefined" ||
      typeof window.getComputedStyle !== "function"
    ) {
      return null;
    }
    try {
      return window.getComputedStyle(element);
    } catch {
      return null;
    }
  };

  const isVisible = (element) => {
    if (!(element instanceof HTMLElement)) {
      return false;
    }
    if (element.hidden || element.getAttribute("aria-hidden") === "true") {
      return false;
    }
    if (
      element instanceof HTMLInputElement &&
      element.type === "hidden" &&
      !element.isConnected
    ) {
      return false;
    }
    const computed = getComputed(element);
    if (!computed) {
      return true;
    }
    return (
      computed.display !== "none" &&
      computed.visibility !== "hidden" &&
      computed.opacity !== "0"
    );
  };

  const firstVisible = (elements) => {
    for (const element of Array.from(elements)) {
      if (isVisible(element)) {
        return element;
      }
    }
    return null;
  };

  const accessibleName = (element) => {
    if (!(element instanceof HTMLElement)) {
      return "";
    }
    const ariaLabel = element.getAttribute("aria-label");
    if (ariaLabel) {
      return normalizeText(ariaLabel);
    }

    const labelledBy = element.getAttribute("aria-labelledby");
    if (labelledBy) {
      const labels = labelledBy
        .split(/\s+/)
        .map((id) => document.getElementById(id))
        .filter(Boolean)
        .map((value) => normalizeText(value.textContent))
        .filter(Boolean);
      if (labels.length > 0) {
        return labels.join(" ");
      }
    }

    if (
      element instanceof HTMLInputElement ||
      element instanceof HTMLTextAreaElement
    ) {
      if (element.labels?.length) {
        const text = Array.from(element.labels)
          .map((label) => normalizeText(label.textContent))
          .filter(Boolean)
          .join(" ");
        if (text) {
          return text;
        }
      }
      if (element.placeholder) {
        return normalizeText(element.placeholder);
      }
      if (element.value) {
        return normalizeText(element.value);
      }
    }

    const title = element.getAttribute("title");
    if (title) {
      return normalizeText(title);
    }

    return normalizeText(element.textContent);
  };

  const queryFirstVisible = (selectors) => {
    for (const selector of selectors) {
      const match = firstVisible(document.querySelectorAll(selector));
      if (match) {
        return match;
      }
    }
    return null;
  };

  const queryVisible = (selectors) =>
    visibleUnique(
      selectors.flatMap((selector) =>
        Array.from(document.querySelectorAll(selector)),
      ),
    );

  const actionableSelector = [
    "button",
    "[role='button']",
    "[role='tab']",
    "[role='switch']",
    "[role='option']",
    "[role='menuitem']",
    "[role='menuitemradio']",
    "[aria-haspopup]",
    "label[for]",
    "a",
  ].join(", ");

  const actionableElements = () =>
    Array.from(document.querySelectorAll(actionableSelector)).filter(isVisible);

  const latestVisible = (selectors) => {
    const matches = [];
    for (const selector of selectors) {
      matches.push(...Array.from(document.querySelectorAll(selector)));
    }
    const visible = matches.filter(isVisible);
    if (visible.length === 0) {
      return null;
    }
    return visible[visible.length - 1] ?? null;
  };

  const visibleUnique = (elements) => {
    const seen = new Set();
    const output = [];
    for (const element of Array.from(elements)) {
      if (!isVisible(element) || seen.has(element)) {
        continue;
      }
      seen.add(element);
      output.push(element);
    }
    return output;
  };

  const isSelected = (element) =>
    element.getAttribute("aria-pressed") === "true" ||
    element.getAttribute("aria-checked") === "true" ||
    element.getAttribute("aria-selected") === "true" ||
    element.dataset.selected === "true" ||
    element.dataset.active === "true";

  const isDisabled = (element) =>
    element.getAttribute("aria-disabled") === "true" ||
    element.hasAttribute("disabled") ||
    element.dataset.disabled === "true";

  const findByAliases = ({
    selectors = [],
    aliases = [],
    fallbackTokens = [],
    include,
    exclude,
  }) => {
    const selectorMatch = queryFirstVisible(selectors);
    if (
      selectorMatch &&
      !exclude?.(selectorMatch) &&
      (!include || include(selectorMatch))
    ) {
      return selectorMatch;
    }

    const normalizedAliases = unique(aliases);
    for (const element of actionableElements()) {
      if (exclude?.(element) || (include && !include(element))) {
        continue;
      }
      const name = accessibleName(element);
      if (!name) {
        continue;
      }
      if (
        normalizedAliases.some(
          (alias) =>
            name === alias || name.includes(alias) || alias.includes(name),
        )
      ) {
        return element;
      }
    }

    for (const element of actionableElements()) {
      if (exclude?.(element) || (include && !include(element))) {
        continue;
      }
      if (hasToken(accessibleName(element), fallbackTokens)) {
        return element;
      }
    }

    return null;
  };

  const clickControl = (element) => {
    if (!(element instanceof HTMLElement)) {
      return false;
    }
    if (isDisabled(element)) {
      return false;
    }
    if (element.tagName.toLowerCase() === "label") {
      const inputId = element.getAttribute("for");
      if (inputId) {
        const input = document.getElementById(inputId);
        if (input instanceof HTMLElement) {
          input.click();
          return true;
        }
      }
    }
    element.focus?.();
    try {
      element.click();
      return true;
    } catch {}
    const rect = element.getBoundingClientRect();
    const eventInit = {
      bubbles: true,
      cancelable: true,
      composed: true,
      button: 0,
      buttons: 1,
      clientX:
        rect.left + Math.max(1, Math.min(rect.width / 2, rect.width - 1)),
      clientY:
        rect.top + Math.max(1, Math.min(rect.height / 2, rect.height - 1)),
    };
    if (typeof PointerEvent === "function") {
      element.dispatchEvent(
        new PointerEvent("pointerdown", {
          ...eventInit,
          pointerId: 1,
          pointerType: "mouse",
          isPrimary: true,
        }),
      );
    }
    element.dispatchEvent(new MouseEvent("mousedown", eventInit));
    if (typeof PointerEvent === "function") {
      element.dispatchEvent(
        new PointerEvent("pointerup", {
          ...eventInit,
          pointerId: 1,
          pointerType: "mouse",
          isPrimary: true,
        }),
      );
    }
    element.dispatchEvent(new MouseEvent("mouseup", eventInit));
    element.dispatchEvent(new MouseEvent("click", eventInit));
    return true;
  };

  return {
    getComputed,
    isVisible,
    firstVisible,
    accessibleName,
    queryFirstVisible,
    queryVisible,
    actionableSelector,
    actionableElements,
    latestVisible,
    visibleUnique,
    isSelected,
    isDisabled,
    findByAliases,
    clickControl,
  };
}
