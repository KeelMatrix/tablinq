import { createServer } from "node:http";
import { readFileSync } from "node:fs";
import { extname, join } from "node:path";

const port = Number(process.env.PORT || 4177);
const root = new URL("./src/", import.meta.url);

const mimeTypes = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
};

createServer((request, response) => {
  const pathname = request.url === "/" ? "/index.html" : request.url;
  const filePath = join(root.pathname, pathname);

  try {
    const body = readFileSync(filePath);
    response.writeHead(200, {
      "content-type": mimeTypes[extname(filePath)] || "application/octet-stream",
    });
    response.end(body);
  } catch {
    response.writeHead(404, { "content-type": "text/plain; charset=utf-8" });
    response.end("not found");
  }
}).listen(port, () => {
  console.log(`fake-chatgpt listening on http://127.0.0.1:${port}`);
});
