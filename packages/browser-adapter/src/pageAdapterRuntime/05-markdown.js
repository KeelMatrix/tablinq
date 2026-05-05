function createMarkdownRuntimeModule({ textUtils, domUtils, messageFilters }) {
  const {
    normalizeText,
    uniqueCollapsed,
    hasToken,
    collapseInlineText,
    normalizeInlineOutput,
  } = textUtils;
  const { accessibleName, clickControl, isVisible } = domUtils;
  const {
    isTransientAskStatusText,
    isTransientAskStatusElement,
    isInsideReasoningArtifactMessage,
    findMessageContentRoot,
  } = messageFilters;

  const TEX_ANNOTATION_SELECTOR = "annotation[encoding^='application/x-tex']";
  const TEX_ANNOTATION_CANDIDATE_SELECTOR = "annotation[encoding]";
  const MATHML_TAGS = new Set([
    "annotation",
    "maction",
    "math",
    "menclose",
    "merror",
    "mfenced",
    "mfrac",
    "mi",
    "mmultiscripts",
    "mn",
    "mo",
    "mover",
    "mpadded",
    "mphantom",
    "mprescripts",
    "mroot",
    "mrow",
    "ms",
    "mspace",
    "msqrt",
    "mstyle",
    "msub",
    "msubsup",
    "msup",
    "mtable",
    "mtd",
    "mtext",
    "mtr",
    "munder",
    "munderover",
    "semantics",
  ]);

  const isTexAnnotationElement = (element) => {
    if (!(element instanceof Element)) {
      return false;
    }
    const encoding = (element.getAttribute("encoding") ?? "")
      .trim()
      .toLowerCase();
    return (
      element.tagName.toLowerCase() === "annotation" &&
      encoding.startsWith("application/x-tex")
    );
  };

  const findTexAnnotation = (element) => {
    if (!(element instanceof Element)) {
      return null;
    }
    if (isTexAnnotationElement(element)) {
      return element;
    }
    const direct = element.querySelector(TEX_ANNOTATION_SELECTOR);
    if (isTexAnnotationElement(direct)) {
      return direct;
    }
    return (
      Array.from(
        element.querySelectorAll(TEX_ANNOTATION_CANDIDATE_SELECTOR),
      ).find(isTexAnnotationElement) ?? null
    );
  };

  const hasMathClass = (element) => {
    if (!(element instanceof Element)) {
      return false;
    }
    return Array.from(element.classList).some((token) => {
      const normalized = token.toLowerCase();
      return (
        normalized === "katex" ||
        normalized === "math" ||
        normalized === "mathjax" ||
        normalized === "inline-math" ||
        normalized === "display-math" ||
        normalized.startsWith("katex-") ||
        normalized.startsWith("math-") ||
        normalized.startsWith("mathjax-")
      );
    });
  };

  const isMathTreeElement = (element) => {
    if (!(element instanceof Element)) {
      return false;
    }
    const tag = element.tagName.toLowerCase();
    return (
      hasMathClass(element) ||
      tag === "mjx-container" ||
      tag.startsWith("mjx-") ||
      MATHML_TAGS.has(tag)
    );
  };

  const isTightMathWrapper = (element) => {
    if (!(element instanceof Element) || !findTexAnnotation(element)) {
      return false;
    }
    const children = Array.from(element.childNodes);
    if (children.length === 0) {
      return false;
    }
    return children.every((child) => {
      if (child.nodeType === Node.TEXT_NODE) {
        return !collapseInlineText(child.textContent);
      }
      if (!(child instanceof Element)) {
        return false;
      }
      return isMathTreeElement(child) || isTightMathWrapper(child);
    });
  };

  const isMathElement = (element) => {
    if (!(element instanceof Element)) {
      return false;
    }
    const tag = element.tagName.toLowerCase();
    const canBeClasslessMathWrapper = ![
      "blockquote",
      "li",
      "p",
      "td",
      "th",
    ].includes(tag);
    return (
      hasMathClass(element) ||
      tag === "mjx-container" ||
      tag.startsWith("mjx-") ||
      (canBeClasslessMathWrapper && isTightMathWrapper(element))
    );
  };

  const collectMathTex = (element) => {
    if (!(element instanceof Element)) {
      return "";
    }
    const annotation = findTexAnnotation(element);
    const tex = annotation?.textContent?.trim();
    if (tex) {
      return tex;
    }

    const aria = element.getAttribute("aria-label")?.trim();
    if (aria) {
      return collapseInlineText(aria);
    }

    const alt = element.getAttribute("alt")?.trim();
    if (alt) {
      return collapseInlineText(alt);
    }

    return "";
  };

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

  const collectTextWithRenderedLineBreaks = (element) => {
    if (!(element instanceof HTMLElement)) {
      return "";
    }
    const visit = (node) => {
      if (node.nodeType === Node.TEXT_NODE) {
        return node.textContent ?? "";
      }
      if (!(node instanceof HTMLElement)) {
        return "";
      }
      if (node.tagName.toLowerCase() === "br") {
        return "\n";
      }
      return Array.from(node.childNodes).map(visit).join("");
    };
    return Array.from(element.childNodes).map(visit).join("").trimEnd();
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

  const isCodePreviewBlock = (element) => {
    if (!(element instanceof HTMLElement)) {
      return false;
    }
    return (
      Boolean(element.querySelector("button[aria-label='Code']")) &&
      Boolean(element.querySelector("button[aria-label='Preview']"))
    );
  };

  const isMermaidPreviewBlock = (element) =>
    isCodePreviewBlock(element) &&
    collectCodePreviewLanguage(element) === "mermaid";

  const collectSourceText = (element) => {
    if (!(element instanceof HTMLElement)) {
      return "";
    }
    if (
      element instanceof HTMLTextAreaElement ||
      element instanceof HTMLInputElement
    ) {
      return (element.value || element.textContent || "").trimEnd();
    }
    const renderedLines = collectTextWithRenderedLineBreaks(element);
    if (renderedLines.includes("\n")) {
      return renderedLines;
    }
    return (
      element.innerText ||
      element.textContent ||
      element.getAttribute("value") ||
      ""
    ).trimEnd();
  };

  const collectElementLanguageHint = (element) => {
    if (!(element instanceof HTMLElement)) {
      return "";
    }
    const language = collapseInlineText(
      element.getAttribute("data-language") ??
        element.getAttribute("data-code-language") ??
        element.dataset.language ??
        element.dataset.codeLanguage,
    ).toLowerCase();
    if (language) {
      return language;
    }
    const classText = Array.from(element.classList).join(" ").toLowerCase();
    const classLanguage = classText.match(/(?:^|\s)language-([a-z0-9_-]+)/);
    if (classLanguage?.[1]) {
      return classLanguage[1];
    }
    return "";
  };

  const collectCodePreviewLanguage = (element) => {
    if (!(element instanceof HTMLElement)) {
      return "";
    }
    const direct = collectElementLanguageHint(element);
    if (direct) {
      return direct;
    }

    const textHints = [
      element.getAttribute("aria-label"),
      element.getAttribute("data-testid"),
      element.getAttribute("data-source"),
      element.querySelector("img[alt]")?.getAttribute("alt"),
      ...Array.from(element.children)
        .filter(
          (child) =>
            child instanceof HTMLElement &&
            child.tagName.toLowerCase() !== "button",
        )
        .map((child) => child.textContent),
      element.textContent,
    ];
    const text = collapseInlineText(textHints.filter(Boolean).join(" "));
    if (hasToken(text, ["mermaid"])) {
      return "mermaid";
    }
    if (hasToken(text, ["svg"])) {
      return "svg";
    }
    return "";
  };

  const isWidgetChromeText = (text, language) => {
    const lines = text
      .split("\n")
      .map((line) => collapseInlineText(line))
      .filter(Boolean);
    if (lines.length === 0) {
      return true;
    }
    const chrome = new Set([
      "code",
      "preview",
      "loading diagram...",
      "rendering...",
      language,
      "mermaid",
      "svg",
    ]);
    return lines.every((line) => chrome.has(line.toLowerCase()));
  };

  const isCodePreviewSourceCandidate = (element, language) => {
    if (!(element instanceof HTMLElement) || element.closest("button")) {
      return false;
    }
    const candidateLanguage = collectElementLanguageHint(element);
    if (
      element.tagName.toLowerCase() === "code" ||
      element.classList.contains("cm-content")
    ) {
      return true;
    }
    const labelText = collapseInlineText(
      [
        element.getAttribute("aria-label"),
        element.getAttribute("data-testid"),
        element.getAttribute("data-source"),
        element.getAttribute("name"),
      ]
        .filter(Boolean)
        .join(" "),
    ).toLowerCase();
    return (
      candidateLanguage === language ||
      hasToken(labelText, ["source"]) ||
      hasToken(labelText, ["code"])
    );
  };

  const collectCodePreviewSource = (element, language) => {
    if (!(element instanceof HTMLElement)) {
      return "";
    }
    const candidates = Array.from(
      element.querySelectorAll(
        [
          "code",
          ".cm-content",
          "textarea",
          "input[type='hidden']",
          "[data-language]",
          "[data-code-language]",
          "[data-source]",
          "[aria-label*='source' i]",
          "[aria-label*='code' i]",
          "[data-testid*='source' i]",
          "[data-testid*='code' i]",
        ].join(", "),
      ),
    );
    for (const candidate of candidates) {
      if (!isCodePreviewSourceCandidate(candidate, language)) {
        continue;
      }
      const source = collectSourceText(candidate);
      if (source && !isWidgetChromeText(source, language)) {
        return source;
      }
    }
    return "";
  };

  const collectPreviewSvgLabels = (element) => {
    const previewImage = element.querySelector(
      "img[src^='data:image/svg+xml']",
    );
    const svgText =
      previewImage instanceof HTMLImageElement
        ? decodeDataUrlText(previewImage.currentSrc || previewImage.src)
        : "";
    return collectMermaidSvgLabels(svgText);
  };

  const serializeCodePreviewBlock = (element) => {
    if (!isCodePreviewBlock(element)) {
      return null;
    }
    const language = collectCodePreviewLanguage(element);
    if (!language) {
      return "Preview.";
    }
    const source = collectCodePreviewSource(element, language);
    if (source) {
      return `\`\`\`${language}\n${source}\n\`\`\``;
    }
    const labels = collectPreviewSvgLabels(element);
    if (language === "mermaid") {
      if (labels.length === 0) {
        return "";
      }
      const summary = labels.slice(0, 12).join("; ");
      return `Mermaid diagram preview: ${summary}${labels.length > 12 ? "; ..." : "."}`;
    }
    if (language === "svg") {
      if (labels.length === 0) {
        return "";
      }
      const summary = labels.slice(0, 12).join("; ");
      return `SVG preview: ${summary}${labels.length > 12 ? "; ..." : "."}`;
    }
    if (labels.length === 0) {
      return "";
    }
    const summary = labels.slice(0, 12).join("; ");
    return `Preview: ${summary}${labels.length > 12 ? "; ..." : "."}`;
  };

  const serializeMermaidPreviewBlock = (element) => {
    if (!isMermaidPreviewBlock(element)) {
      return null;
    }
    return serializeCodePreviewBlock(element);
  };

  const findCodePreviewToggle = (element, label) => {
    if (!(element instanceof HTMLElement)) {
      return null;
    }
    const normalizedLabel = label.toLowerCase();
    return (
      Array.from(
        element.querySelectorAll("button, [role='button'], [role='tab']"),
      ).find((candidate) => {
        if (!(candidate instanceof HTMLElement) || !isVisible(candidate)) {
          return false;
        }
        return accessibleName(candidate).toLowerCase() === normalizedLabel;
      }) ?? null
    );
  };

  const waitForWidgetDomUpdate = () =>
    new Promise((resolve) => {
      if (typeof window === "undefined") {
        resolve();
        return;
      }
      window.setTimeout(resolve, 750);
    });

  const storeMaterializedWidgetSource = (element, language, source) => {
    if (
      !(element instanceof HTMLElement) ||
      !language ||
      !source ||
      element.querySelector("[data-tablinq-materialized-source='true']")
    ) {
      return;
    }
    const sourceElement = document.createElement("textarea");
    sourceElement.hidden = true;
    sourceElement.dataset.language = language;
    sourceElement.dataset.tablinqMaterializedSource = "true";
    sourceElement.setAttribute("aria-label", `Tablinq ${language} source`);
    sourceElement.value = source;
    sourceElement.textContent = source;
    element.appendChild(sourceElement);
  };

  const materializeCodePreviewSources = async (root) => {
    if (!(root instanceof HTMLElement)) {
      return 0;
    }
    let materialized = 0;
    const widgets = Array.from(root.querySelectorAll("pre")).filter(
      (element) =>
        element instanceof HTMLElement && isCodePreviewBlock(element),
    );
    for (const widget of widgets) {
      const language = collectCodePreviewLanguage(widget);
      if (!["mermaid", "svg"].includes(language)) {
        continue;
      }
      if (collectCodePreviewSource(widget, language)) {
        continue;
      }
      const codeToggle = findCodePreviewToggle(widget, "Code");
      const previewToggle = findCodePreviewToggle(widget, "Preview");
      if (!codeToggle || !previewToggle) {
        continue;
      }
      if (!clickControl(codeToggle)) {
        continue;
      }
      await waitForWidgetDomUpdate();
      const source = collectCodePreviewSource(widget, language);
      if (!source) {
        continue;
      }
      storeMaterializedWidgetSource(widget, language, source);
      materialized += 1;
      if (isVisible(previewToggle)) {
        clickControl(previewToggle);
        await waitForWidgetDomUpdate();
      }
    }
    return materialized;
  };

  const hasTransientWidgetText = (element) => {
    const text = collapseInlineText(element.textContent);
    return hasToken(text, ["loading", "rendering", "generating"]);
  };

  const hasTransientRenderedWidgets = (root) => {
    if (!(root instanceof HTMLElement)) {
      return false;
    }
    return Array.from(root.querySelectorAll("pre")).some((element) => {
      if (!(element instanceof HTMLElement) || !isCodePreviewBlock(element)) {
        return false;
      }
      const language = collectCodePreviewLanguage(element);
      if (language && collectCodePreviewSource(element, language)) {
        return false;
      }
      if (collectPreviewSvgLabels(element).length > 0) {
        return false;
      }
      return (
        hasTransientWidgetText(element) ||
        Boolean(element.querySelector("canvas, [role='progressbar'], .spinner"))
      );
    });
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
      if (node instanceof Element && isMathElement(node)) {
        const tex = collectMathTex(node);
        return tex ? `$${tex}$` : "";
      }
      return "";
    }
    if (!isVisible(node)) {
      return "";
    }
    if (isIgnoredMessageChrome(node)) {
      return "";
    }
    if (isMathElement(node)) {
      const tex = collectMathTex(node);
      return tex ? `$${tex}$` : "";
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
    if (element instanceof Element && isMathElement(element)) {
      const tex = collectMathTex(element);
      return tex ? `$$\n${tex}\n$$` : "";
    }
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
      const codePreview = serializeCodePreviewBlock(element);
      if (codePreview !== null) {
        return codePreview;
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
    if (
      Array.from(element.children).some(
        (child) =>
          child instanceof HTMLElement &&
          [
            "blockquote",
            "h1",
            "h2",
            "h3",
            "h4",
            "h5",
            "h6",
            "ol",
            "pre",
            "table",
            "ul",
          ].includes(child.tagName.toLowerCase()),
      )
    ) {
      return "";
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
    hasTransientRenderedWidgets,
    isMermaidPreviewBlock,
    materializeCodePreviewSources,
    serializeMermaidPreviewBlock,
  };
}
