function createMessageFilterRuntimeModule({ textUtils, domUtils }) {
  const {
    normalizeText,
    collapseText,
    uniqueCollapsed,
    datasetValue,
    hasToken,
  } = textUtils;
  const { isVisible } = domUtils;

  const isTransientAskStatusText = (value) =>
    /^thought for\b/i.test(collapseText(value));

  const isTransientAskStatusElement = (element) => {
    if (!(element instanceof HTMLElement)) {
      return false;
    }
    const text = collapseText(element.textContent);
    if (!isTransientAskStatusText(text)) {
      return false;
    }
    if (element.tagName.toLowerCase() === "button") {
      return true;
    }
    const button = element.querySelector("button");
    if (!(button instanceof HTMLElement)) {
      return false;
    }
    return (
      isTransientAskStatusText(button.textContent) &&
      Array.from(
        element.querySelectorAll(
          [
            ".markdown",
            "p",
            "pre",
            "code",
            "table",
            "ol",
            "ul",
            "blockquote",
            "h1",
            "h2",
            "h3",
            "h4",
            "h5",
            "h6",
          ].join(", "),
        ),
      ).every((candidate) => candidate === button || button.contains(candidate))
    );
  };

  const isThoughtDividerElement = (element) => {
    if (!(element instanceof HTMLElement)) {
      return false;
    }
    const text = collapseText(element.textContent);
    if (!isTransientAskStatusText(text)) {
      return false;
    }
    return (
      Boolean(element.querySelector("button")) || element.tagName === "BUTTON"
    );
  };

  const explicitAssistantRole = (element) =>
    normalizeText(
      datasetValue(element?.getAttribute?.("data-message-author-role")) ?? "",
    ) === "assistant";

  const isNativeReasoningDurationText = (value) =>
    /^worked for\b/i.test(collapseText(value));

  const visibleElementChildren = (element) =>
    Array.from(element?.children ?? []).filter(
      (child) => child instanceof HTMLElement && isVisible(child),
    );

  const isNativeReasoningDurationControl = (element) =>
    element instanceof HTMLElement &&
    element.tagName.toLowerCase() === "button" &&
    isNativeReasoningDurationText(element.textContent);

  const isNativeReasoningProgressContainer = (element) => {
    if (!(element instanceof HTMLElement)) {
      return false;
    }
    if (explicitAssistantRole(element)) {
      return false;
    }
    if (element.querySelector("[data-message-author-role='assistant']")) {
      return false;
    }
    const firstVisibleChild = visibleElementChildren(element)[0] ?? null;
    return isNativeReasoningDurationControl(firstVisibleChild);
  };

  const closestNativeReasoningProgressContainer = (element) => {
    if (!(element instanceof HTMLElement)) {
      return null;
    }
    let current = element;
    while (current instanceof HTMLElement) {
      if (explicitAssistantRole(current)) {
        return null;
      }
      if (isNativeReasoningProgressContainer(current)) {
        return current;
      }
      current = current.parentElement;
    }
    return null;
  };

  const isInsideNativeReasoningProgress = (element) =>
    Boolean(closestNativeReasoningProgressContainer(element));

  const visibleSibling = (element, direction) => {
    let current =
      direction === "previous"
        ? element?.previousElementSibling
        : element?.nextElementSibling;
    while (current) {
      if (isVisible(current)) {
        return current;
      }
      current =
        direction === "previous"
          ? current.previousElementSibling
          : current.nextElementSibling;
    }
    return null;
  };

  const thoughtDividerBeforeMessage = (element) =>
    isThoughtDividerElement(visibleSibling(element, "previous"));

  const hasFollowingThoughtDivider = (element) => {
    let current = visibleSibling(element, "next");
    while (current) {
      if (isThoughtDividerElement(current)) {
        return true;
      }
      current = visibleSibling(current, "next");
    }
    return false;
  };

  const isReasoningArtifactMessage = (element) => {
    if (!(element instanceof HTMLElement)) {
      return false;
    }
    if (
      isTransientAskStatusElement(element) ||
      isThoughtDividerElement(element) ||
      isNativeReasoningProgressContainer(element)
    ) {
      return true;
    }
    if (!explicitAssistantRole(element)) {
      return false;
    }
    return (
      Boolean(element.querySelector(".markdown, [data-message-content]")) &&
      hasFollowingThoughtDivider(element)
    );
  };

  const isInsideReasoningArtifactMessage = (element) => {
    if (!(element instanceof HTMLElement)) {
      return false;
    }
    if (isInsideNativeReasoningProgress(element)) {
      return true;
    }
    const message = element.closest("[data-message-author-role='assistant']");
    return (
      message instanceof HTMLElement && isReasoningArtifactMessage(message)
    );
  };

  const findMessageContentRoot = (root) => {
    if (!(root instanceof HTMLElement)) {
      return null;
    }
    const candidates = Array.from(
      root.querySelectorAll(".markdown, [data-message-content]"),
    ).filter((candidate) => candidate instanceof HTMLElement);
    for (const candidate of candidates) {
      if (
        !isTransientAskStatusElement(candidate) &&
        !isInsideNativeReasoningProgress(candidate) &&
        !isInsideReasoningArtifactMessage(candidate)
      ) {
        return candidate;
      }
    }
    if (candidates.length > 0) {
      return null;
    }
    if (
      isTransientAskStatusElement(root) ||
      isInsideNativeReasoningProgress(root) ||
      isReasoningArtifactMessage(root)
    ) {
      return null;
    }
    return root;
  };

  const readMessageIdentity = (element) => {
    if (!(element instanceof HTMLElement)) {
      return null;
    }
    return (
      datasetValue(element.getAttribute("data-message-id")) ??
      datasetValue(element.getAttribute("data-testid")) ??
      datasetValue(element.id) ??
      null
    );
  };

  return {
    isTransientAskStatusText,
    isTransientAskStatusElement,
    isThoughtDividerElement,
    isNativeReasoningDurationText,
    isNativeReasoningDurationControl,
    isNativeReasoningProgressContainer,
    isInsideNativeReasoningProgress,
    explicitAssistantRole,
    thoughtDividerBeforeMessage,
    isReasoningArtifactMessage,
    isInsideReasoningArtifactMessage,
    findMessageContentRoot,
    readMessageIdentity,
  };
}
