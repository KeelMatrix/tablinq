

export function normalizeProbeText(value) {
  return (value ?? "").replace(/\s+/g, " ").trim().toLowerCase();
}

export function computedStyleForProbeElement(element) {
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
}

export function isVisibleProbeElement(element) {
  if (!(element instanceof HTMLElement)) {
    return false;
  }
  if (element.hidden) {
    return false;
  }
  if (element.getAttribute("aria-hidden") === "true") {
    return false;
  }
  const computed = computedStyleForProbeElement(element);
  if (
    computed &&
    (computed.display === "none" ||
      computed.visibility === "hidden" ||
      computed.opacity === "0")
  ) {
    return false;
  }
  return true;
}

export function firstVisibleProbeElement(elements, isVisible) {
  for (const element of Array.from(elements)) {
    if (isVisible(element)) {
      return element;
    }
  }
  return null;
}

export function implicitProbeRole(element) {
  const tagName = element.tagName.toLowerCase();
  if (tagName === "button") {
    return "button";
  }
  if (tagName === "a" && element.href) {
    return "link";
  }
  if (tagName === "textarea") {
    return "textbox";
  }
  if (tagName === "input") {
    const type = element.type.toLowerCase();
    if (type === "file") {
      return "button";
    }
    if (["text", "search", "email", "url"].includes(type)) {
      return "textbox";
    }
  }
  if (element.hasAttribute("contenteditable")) {
    return "textbox";
  }
  return null;
}

export function accessibleProbeName(element, documentRoot = document) {
  if (!element) {
    return "";
  }
  const ariaLabel = element.getAttribute("aria-label");
  if (ariaLabel) {
    return normalizeProbeText(ariaLabel);
  }

  const labelledBy = element.getAttribute("aria-labelledby");
  if (labelledBy) {
    const labels = labelledBy
      .split(/\s+/)
      .map((id) => documentRoot.getElementById(id))
      .filter(Boolean)
      .map((value) => normalizeProbeText(value.textContent));
    if (labels.length > 0) {
      return labels.join(" ");
    }
  }

  if (
    element instanceof HTMLInputElement ||
    element instanceof HTMLTextAreaElement
  ) {
    if (element.labels?.length) {
      return Array.from(element.labels)
        .map((label) => normalizeProbeText(label.textContent))
        .join(" ");
    }
    if (element.placeholder) {
      return normalizeProbeText(element.placeholder);
    }
    if (element.value) {
      return normalizeProbeText(element.value);
    }
  }

  return normalizeProbeText(element.textContent);
}

export function probeTextHasToken(value, tokens) {
  return tokens.some((token) =>
    new RegExp(
      `(^|[^a-z0-9])${token.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}([^a-z0-9]|$)`,
    ).test(value),
  );
}

export function probeElementPath(element) {
  const segments = [];
  let current = element;
  while (current && segments.length < 5) {
    const tag = current.tagName.toLowerCase();
    const id = current.id ? `#${current.id}` : "";
    const role = current.getAttribute("role");
    const slot = current.getAttribute("data-slot");
    const descriptor = [
      tag,
      id,
      role ? `[role=${role}]` : "",
      slot ? `[data-slot=${slot}]` : "",
    ].join("");
    segments.unshift(descriptor);
    current = current.parentElement;
  }
  return segments.join(" > ");
}

export function probeNearbyText(element) {
  const container = element.parentElement ?? element;
  return Array.from(container.querySelectorAll("*"))
    .slice(0, 8)
    .map((node) => normalizeProbeText(node.textContent))
    .filter((value) => value.length > 0)
    .slice(0, 4);
}

export function createProbeNeighborhood(capability, strategyId, element) {
  return {
    capability,
    strategy_id: strategyId,
    anchor_tag: element.tagName.toLowerCase(),
    anchor_role: element.getAttribute("role") ?? implicitProbeRole(element),
    anchor_name: accessibleProbeName(element),
    anchor_path: probeElementPath(element),
    parent_name:
      normalizeProbeText(element.parentElement?.textContent ?? null) || null,
    sibling_names: Array.from(element.parentElement?.children ?? [])
      .filter((candidate) => candidate !== element)
      .map((candidate) => accessibleProbeName(candidate))
      .filter((value) => value.length > 0)
      .slice(0, 4),
    nearby_text: probeNearbyText(element),
  };
}
