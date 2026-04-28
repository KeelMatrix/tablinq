import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const packageRoot = resolve(scriptDir, "..");
const contentPath = resolve(packageRoot, "src", "probe.content.js");
const runtimeModulePaths = [
  resolve(packageRoot, "src", "probeRuntime", "text.js"),
  resolve(packageRoot, "src", "probeRuntime", "strategies.js"),
  resolve(packageRoot, "src", "probeRuntime", "aliases.js"),
  resolve(packageRoot, "src", "probeRuntime", "documentProbe.js"),
];

const header = `// Generated from src/probe.runtime.js and src/probeRuntime/*.js. Do not edit by hand.
// Run \`pnpm --filter @tablinq/extension run generate:probe-content\` after changing the runtime.

`;

function normalizeLineEndings(source) {
  return source.replace(/\r\n?/g, "\n");
}

function stripRuntimeModuleSyntax(source) {
  return normalizeLineEndings(source)
    .replace(/^import[\s\S]*?;\n/gm, "")
    .trim();
}

function readBundledRuntimeSource() {
  return runtimeModulePaths
    .map((modulePath) => stripRuntimeModuleSyntax(readFileSync(modulePath, "utf8")))
    .join("\n\n");
}

function renderContentProbe(runtimeSource) {
  const normalizedRuntimeSource = normalizeLineEndings(runtimeSource);
  if (!/^export\s+function\s+probeDocument/m.test(runtimeSource)) {
    throw new Error(
      "Expected src/probe.runtime.js to export function probeDocument",
    );
  }
  const runtimeBody = normalizedRuntimeSource
    .replace(/^export\s+/gm, "")
    .trimEnd();

  const indentedRuntime = runtimeBody
    .split("\n")
    .map((line) => (line.length > 0 ? `  ${line}` : line))
    .join("\n");

  return `${header}(() => {
${indentedRuntime}

  globalThis.__TABLINQ_PROBE_DOCUMENT__ = probeDocument;
})();
`;
}

const runtimeSource = readBundledRuntimeSource();
const generatedSource = renderContentProbe(runtimeSource);

if (process.argv.includes("--check")) {
  if (!existsSync(contentPath)) {
    console.error(`${contentPath} does not exist`);
    process.exit(1);
  }
  const currentSource = readFileSync(contentPath, "utf8");
  if (normalizeLineEndings(currentSource) !== generatedSource) {
    console.error(
      "packages/extension/src/probe.content.js is out of sync with probe.runtime.js",
    );
    process.exit(1);
  }
} else {
  writeFileSync(contentPath, generatedSource);
}
