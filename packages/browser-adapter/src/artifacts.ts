import { stat } from "node:fs/promises";
import { basename, extname, resolve } from "node:path";

export async function chooseUniqueArtifactPath(
  downloadsDir: string,
  fileName: string,
): Promise<string> {
  const absoluteDownloadsDir = resolve(downloadsDir);
  const baseName = basename(fileName);
  let candidate = resolve(absoluteDownloadsDir, baseName);
  if (!(await pathExists(candidate))) {
    return candidate;
  }

  const extension = extname(baseName);
  const stem = extension ? baseName.slice(0, -extension.length) : baseName;
  for (let index = 2; index < 10_000; index += 1) {
    candidate = resolve(absoluteDownloadsDir, `${stem}-${index}${extension}`);
    if (!(await pathExists(candidate))) {
      return candidate;
    }
  }

  throw new Error(
    `download failed: could not allocate a unique path for ${fileName}`,
  );
}

export function contentTypeForFile(path: string): string {
  switch (extname(path).toLowerCase()) {
    case ".txt":
      return "text/plain; charset=utf-8";
    case ".svg":
      return "image/svg+xml";
    case ".png":
      return "image/png";
    case ".jpg":
    case ".jpeg":
      return "image/jpeg";
    case ".webp":
      return "image/webp";
    case ".gif":
      return "image/gif";
    case ".json":
      return "application/json";
    case ".csv":
      return "text/csv; charset=utf-8";
    case ".html":
      return "text/html; charset=utf-8";
    case ".pdf":
      return "application/pdf";
    default:
      return "application/octet-stream";
  }
}

async function pathExists(path: string): Promise<boolean> {
  try {
    await stat(path);
    return true;
  } catch {
    return false;
  }
}
