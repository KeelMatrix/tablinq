function createMarkdownRuntimeModule({ textUtils, domUtils, messageFilters }) {
  const {
    normalizeText,
    uniqueCollapsed,
    hasToken,
    collapseInlineText,
    normalizeInlineOutput,
  } = textUtils;
  const { isVisible } = domUtils;
  const {
    isTransientAskStatusText,
    isTransientAskStatusElement,
    isInsideReasoningArtifactMessage,
    findMessageContentRoot,
  } = messageFilters;

  const collectCodeBlockText = (element) => {
    if (!(element instanceof HTMLElement)) {
      return "";
    }
    const explicitCode = element.querySelector("code");
    if (explicitCode instanceof HTMLElement) {
      return (
        explicitCode.innerText ||
        explicitCode.textContent ||
        ""
      ).trimEnd();
    }
    const editorContent = element.querySelector(".cm-content");
    if (editorContent instanceof HTMLElement) {
      return (
        editorContent.innerText ||
        editorContent.textContent ||
        ""
      ).trimEnd();
    }
    return (element.innerText || element.textContent || "").trimEnd();
  };

  const collectCodeBlockLanguage = (element) => {
    if (!(element instanceof HTMLElement)) {
      return "";
    }
    const explicit =
      element.getAttribute("data-language") ??
      element.getAttribute("data-code-language") ??
      element.dataset.language ??
      element.dataset.codeLanguage;
    if (explicit) {
      return collapseInlineText(explicit).toLowerCase();
    }

    const candidates = Array.from(
      element.querySelectorAll(
        [
          "[class*='language']",
          "[class*='font-medium']",
          "[data-language]",
          "[data-code-language]",
        ].join(", "),
      ),
    );
    for (const candidate of candidates) {
      if (!(candidate instanceof HTMLElement) || candidate.closest("button")) {
        continue;
      }
      const label = collapseInlineText(candidate.textContent);
      if (
        label &&
        !hasToken(label, ["copy", "run", "code", "table"]) &&
        label.length <= 32
      ) {
        return label.toLowerCase();
      }
    }
    return "";
  };

  const decodeDataUrlText = (url) => {
    if (typeof url !== "string" || !url.toLowerCase().startsWith("data:")) {
      return "";
    }
    const commaIndex = url.indexOf(",");
    if (commaIndex < 0) {
      return "";
    }
    const metadata = url.slice("data:".length, commaIndex);
    const payload = url.slice(commaIndex + 1);
    const isBase64 = metadata
      .split(";")
      .map((segment) => segment.trim().toLowerCase())
      .includes("base64");
    try {
      if (isBase64) {
        const binary = atob(payload);
        const bytes = Uint8Array.from(binary, (value) => value.charCodeAt(0));
        return new TextDecoder().decode(bytes);
      }
      return decodeURIComponent(payload);
    } catch {
      return "";
    }
  };

  const collectMermaidSvgLabels = (svgText) => {
    if (!svgText || typeof DOMParser !== "function") {
      return [];
    }
    try {
      const parsed = new DOMParser().parseFromString(svgText, "image/svg+xml");
      return uniqueCollapsed(
        Array.from(
          parsed.querySelectorAll(
            "text, tspan, foreignObject p, foreignObject span, .nodeLabel, .edgeLabel",
          ),
        )
          .map((candidate) => candidate.textContent ?? "")
          .filter(Boolean),
      ).filter((label) => normalizeText(label) !== "mermaid");
    } catch {
      return [];
    }
  };

  const isMermaidPreviewBlock = (element) => {
    if (!(element instanceof HTMLElement)) {
      return false;
    }
    if (
      element.querySelector("code") instanceof HTMLElement ||
      element.querySelector(".cm-content") instanceof HTMLElement
    ) {
      return false;
    }
    const hasPreviewSurface = Boolean(
      element.querySelector(
        "img[src^='data:image/'], img[src^='blob:'], canvas",
      ),
    );
    if (!hasPreviewSurface) {
      return false;
    }
    const text = collapseInlineText(element.textContent);
    return (
      hasToken(text, ["mermaid"]) ||
      (Boolean(element.querySelector("button[aria-label='Code']")) &&
        Boolean(element.querySelector("button[aria-label='Preview']")))
    );
  };

  const serializeMermaidPreviewBlock = (element) => {
    if (!isMermaidPreviewBlock(element)) {
      return "";
    }
    const previewImage = element.querySelector(
      "img[src^='data:image/svg+xml']",
    );
    const svgText =
      previewImage instanceof HTMLImageElement
        ? decodeDataUrlText(previewImage.currentSrc || previewImage.src)
        : "";
    const labels = collectMermaidSvgLabels(svgText);
    if (labels.length === 0) {
      return "Mermaid diagram preview.";
    }
    const summary = labels.slice(0, 12).join("; ");
    return `Mermaid diagram preview: ${summary}${labels.length > 12 ? "; ..." : "."}`;
  };

  const isIgnoredMessageChrome = (element) => {
    if (!(element instanceof HTMLElement)) {
      return false;
    }
    if (
      element.matches(
        "[data-testid='artifact-download'], [data-slot='artifact-link']",
      )
    ) {
      return true;
    }
    const ariaLabel = collapseInlineText(element.getAttribute("aria-label"));
    const text = collapseInlineText(element.textContent);
    return (
      element.tagName.toLowerCase() === "a" &&
      ((Boolean(ariaLabel) && hasToken(ariaLabel, ["download", "artifact"])) ||
        (element.hasAttribute("download") &&
          Boolean(text) &&
          hasToken(text, ["artifact", "download"])))
    );
  };

  const serializeInlineNode = (node) => {
    if (node.nodeType === Node.TEXT_NODE) {
      return (node.textContent ?? "").replace(/\s+/g, " ");
    }
    if (!(node instanceof HTMLElement)) {
      return "";
    }
    if (!isVisible(node)) {
      return "";
    }
    if (isIgnoredMessageChrome(node)) {
      return "";
    }
    const tag = node.tagName.toLowerCase();
    if (tag === "button") {
      return "";
    }
    if (tag === "br") {
      return "\n";
    }
    if (tag === "code" && node.closest("pre") === null) {
      return `\`${collapseInlineText(node.textContent)}\``;
    }
    if (tag === "strong") {
      const content = serializeInlineChildren(node);
      return content ? `**${content}**` : "";
    }
    if (tag === "em") {
      const content = serializeInlineChildren(node);
      return content ? `*${content}*` : "";
    }
    if (tag === "a") {
      const text = serializeInlineChildren(node);
      const href = collapseInlineText(node.getAttribute("href"));
      if (text && href) {
        return `[${text}](${href})`;
      }
      return text;
    }
    return serializeInlineChildren(node);
  };

  const serializeInlineChildren = (element) =>
    normalizeInlineOutput(
      Array.from(element.childNodes)
        .map((node) => serializeInlineNode(node))
        .join(""),
    );

  const indentLines = (value, prefix, blankPrefix = prefix) =>
    value
      .split("\n")
      .map((line) => (line ? `${prefix}${line}` : blankPrefix.trimEnd()))
      .join("\n");

  const serializeMarkdownTableCell = (value) =>
    normalizeInlineOutput(value)
      .replace(/[ \t]*\n[ \t]*/g, "<br>")
      .replace(/\|/g, "\\|");

  const serializeTable = (table) => {
    if (!(table instanceof HTMLElement)) {
      return "";
    }
    const rows = Array.from(table.querySelectorAll("tr")).map((row) =>
      Array.from(row.children)
        .filter((cell) => cell instanceof HTMLElement)
        .map((cell) =>
          serializeMarkdownTableCell(serializeInlineChildren(cell)),
        ),
    );
    const header = rows[0] ?? [];
    if (header.length === 0) {
      return "";
    }
    const body = rows.slice(1);
    const separator = header.map(() => "---");
    return [
      `| ${header.join(" | ")} |`,
      `| ${separator.join(" | ")} |`,
      ...body.map((row) => `| ${row.join(" | ")} |`),
    ].join("\n");
  };

  const serializeListItem = (item, prefix, options = {}) => {
    if (!(item instanceof HTMLElement)) {
      return "";
    }
    const childElements = Array.from(item.children).filter(
      (child) =>
        child instanceof HTMLElement &&
        child.tagName.toLowerCase() !== "button" &&
        !isIgnoredMessageChrome(child),
    );
    const nestedLists = childElements.filter((child) =>
      ["ul", "ol"].includes(child.tagName.toLowerCase()),
    );
    const contentElements = childElements.filter(
      (child) => !["ul", "ol"].includes(child.tagName.toLowerCase()),
    );
    const content =
      contentElements.length > 0
        ? contentElements
            .map((child, index) =>
              serializeBlock(child, {
                ...options,
                isLastBlock:
                  nestedLists.length === 0 &&
                  index === contentElements.length - 1 &&
                  options.isLastBlock,
              }),
            )
            .filter(Boolean)
            .join("\n\n")
        : serializeInlineChildren(item);
    const normalized = normalizeInlineOutput(content);
    if (!normalized) {
      return "";
    }
    const lines = normalized.split("\n");
    const firstLine = `${prefix}${lines[0]}`;
    const continuationPrefix = " ".repeat(prefix.length);
    const rest = lines
      .slice(1)
      .map((line) => `${continuationPrefix}${line}`)
      .join("\n");
    const nested = nestedLists
      .map((list, index) =>
        serializeBlock(list, {
          ...options,
          isLastBlock: index === nestedLists.length - 1 && options.isLastBlock,
        }),
      )
      .filter(Boolean)
      .map((value) =>
        indentLines(value, continuationPrefix, continuationPrefix),
      )
      .join("\n");
    return [firstLine, rest, nested].filter(Boolean).join("\n");
  };

  const serializeBlock = (element, options = {}) => {
    if (!(element instanceof HTMLElement)) {
      return "";
    }
    if (isIgnoredMessageChrome(element)) {
      return "";
    }
    const tag = element.tagName.toLowerCase();
    if (tag === "button") {
      return "";
    }
    if (
      tag === "h1" ||
      tag === "h2" ||
      tag === "h3" ||
      tag === "h4" ||
      tag === "h5" ||
      tag === "h6"
    ) {
      const level = Number(tag.slice(1));
      return `${"#".repeat(level)} ${serializeInlineChildren(element)}`.trim();
    }
    if (tag === "p") {
      return serializeInlineChildren(element);
    }
    if (tag === "blockquote") {
      const content = serializeBlockChildren(element);
      return content ? indentLines(content, "> ", ">") : "";
    }
    if (tag === "pre") {
      const mermaidPreview = serializeMermaidPreviewBlock(element);
      if (mermaidPreview) {
        return mermaidPreview;
      }
      const code = collectCodeBlockText(element);
      if (!code) {
        return "";
      }
      const language = collectCodeBlockLanguage(element);
      if (options.streamRunning && options.isLastBlock) {
        return `\`\`\`${language}\n${code}`;
      }
      return `\`\`\`${language}\n${code}\n\`\`\``;
    }
    if (tag === "ul") {
      return Array.from(element.children)
        .map((child, index, items) =>
          serializeListItem(child, "- ", {
            ...options,
            isLastBlock: index === items.length - 1 && options.isLastBlock,
          }),
        )
        .filter(Boolean)
        .join("\n");
    }
    if (tag === "ol") {
      return Array.from(element.children)
        .map((child, index, items) =>
          serializeListItem(child, `${index + 1}. `, {
            ...options,
            isLastBlock: index === items.length - 1 && options.isLastBlock,
          }),
        )
        .filter(Boolean)
        .join("\n");
    }
    if (tag === "table") {
      return serializeTable(element);
    }
    return serializeBlockChildren(element, options);
  };

  const serializeBlockChildren = (element, options = {}) => {
    if (!(element instanceof HTMLElement)) {
      return "";
    }
    const blocks = Array.from(element.children)
      .map((child, index, items) => ({
        tag: child instanceof HTMLElement ? child.tagName.toLowerCase() : "",
        text: serializeBlock(child, {
          ...options,
          isLastBlock: index === items.length - 1 && options.isLastBlock,
        }),
      }))
      .filter((entry) => Boolean(entry.text));
    if (blocks.length > 0) {
      return blocks.reduce((output, entry, index) => {
        if (index === 0) {
          return entry.text;
        }
        const previousTag = blocks[index - 1]?.tag ?? "";
        const joiner =
          /^h[1-6]$/.test(previousTag) && /^h[1-6]$/.test(entry.tag)
            ? "\n"
            : "\n\n";
        return `${output}${joiner}${entry.text}`;
      }, "");
    }
    return serializeInlineChildren(element);
  };

  const collectText = (root) => {
    const contentRoot = findMessageContentRoot(root);
    if (!(contentRoot instanceof HTMLElement)) {
      return "";
    }
    if (isTransientAskStatusElement(contentRoot)) {
      return "";
    }
    const streamRunning =
      (root?.getAttribute?.("data-stream-state") ??
        document.body.dataset.streamState ??
        "idle") === "running";
    const serialized = serializeBlockChildren(contentRoot, {
      isLastBlock: true,
      streamRunning,
    });
    if (serialized && !isTransientAskStatusText(serialized)) {
      return serialized;
    }

    const directText = uniqueCollapsed(
      Array.from(contentRoot.childNodes)
        .filter((node) => node.nodeType === Node.TEXT_NODE)
        .map((node) => node.textContent),
    );
    return directText
      .filter((value) => !isTransientAskStatusText(value))
      .join("\n");
  };

  return {
    collectText,
    collectMermaidSvgLabels,
    isMermaidPreviewBlock,
    serializeMermaidPreviewBlock,
  };
}
