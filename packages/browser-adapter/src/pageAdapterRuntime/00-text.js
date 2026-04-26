function createTextUtilsModule() {
  const normalizeText = (value) =>
    (value ?? "").replace(/\s+/g, " ").trim().toLowerCase();
  const collapseText = (value) => (value ?? "").replace(/\s+/g, " ").trim();

  const unique = (values) => {
    const seen = new Set();
    const output = [];
    for (const value of values) {
      const normalized = normalizeText(value);
      if (!normalized || seen.has(normalized)) {
        continue;
      }
      seen.add(normalized);
      output.push(normalized);
    }
    return output;
  };

  const uniqueCollapsed = (values) => {
    const seen = new Set();
    const output = [];
    for (const value of values) {
      const collapsed = collapseText(value);
      const normalized = normalizeText(collapsed);
      if (!normalized || seen.has(normalized)) {
        continue;
      }
      seen.add(normalized);
      output.push(collapsed);
    }
    return output;
  };

  const datasetValue = (value) => {
    const trimmed = (value ?? "").trim();
    return trimmed ? trimmed : null;
  };

  const hasToken = (value, tokens) => {
    const normalized = normalizeText(value);
    return tokens.some((token) =>
      new RegExp(
        `(^|[^a-z0-9])${token.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}([^a-z0-9]|$)`,
      ).test(normalized),
    );
  };
  const collapseInlineText = (value) =>
    (value ?? "").replace(/\s+/g, " ").trim();

  const normalizeInlineOutput = (value) =>
    (value ?? "")
      .replace(/[ \t]+\n/g, "\n")
      .replace(/\n[ \t]+/g, "\n")
      .replace(/[ \t]{2,}/g, " ")
      .trim();

  return {
    normalizeText,
    collapseText,
    unique,
    uniqueCollapsed,
    datasetValue,
    hasToken,
    collapseInlineText,
    normalizeInlineOutput,
  };
}
