import { createServer } from "node:http";
import { readFileSync } from "node:fs";
import { extname, join } from "node:path";
import { fileURLToPath } from "node:url";

const port = Number(process.env.PORT || 4177);
const root = fileURLToPath(new URL("./src/", import.meta.url));

const mimeTypes = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
};

createServer((request, response) => {
  const url = new URL(request.url || "/", `http://127.0.0.1:${port}`);
  const pathname = url.pathname === "/" ? "/index.html" : url.pathname;
  let filePath = join(root, pathname);

  try {
    const body = readFileSync(filePath);
    response.writeHead(200, {
      "content-type": mimeTypes[extname(filePath)] || "application/octet-stream",
    });
    response.end(body);
  } catch {
    if (!extname(pathname)) {
      filePath = join(root, "index.html");
      const body = readFileSync(filePath);
      response.writeHead(200, {
        "content-type": mimeTypes[".html"],
      });
      response.end(body);
      return;
    }

    response.writeHead(404, { "content-type": "text/plain; charset=utf-8" });
    response.end("not found");
  }
}).listen(port, () => {
  console.log(`fake-chatgpt listening on http://127.0.0.1:${port}`);
});
