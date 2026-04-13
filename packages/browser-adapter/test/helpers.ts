import net from "node:net";
import path from "node:path";
import { spawn, type ChildProcess } from "node:child_process";

export async function allocatePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      if (!address || typeof address === "string") {
        reject(new Error("could not allocate port"));
        return;
      }
      const port = address.port;
      server.close((error) => {
        if (error) {
          reject(error);
          return;
        }
        resolve(port);
      });
    });
    server.on("error", reject);
  });
}

export async function startFakeChatGptServer(): Promise<{
  process: ChildProcess;
  url: string;
}> {
  const port = await allocatePort();
  const repoRoot = path.resolve(__dirname, "../../..");
  const serverPath = path.join(
    repoRoot,
    "packages",
    "fake-chatgpt",
    "server.mjs",
  );
  const child = spawn(process.execPath, [serverPath], {
    cwd: repoRoot,
    env: { ...process.env, PORT: String(port) },
    stdio: ["ignore", "ignore", "ignore"],
  });

  const url = `http://127.0.0.1:${port}`;
  await waitForServer(url);
  return { process: child, url };
}

export async function stopFakeChatGptServer(
  child: ChildProcess,
): Promise<void> {
  if (child.killed || child.exitCode !== null) {
    return;
  }

  child.kill();
  const exited = await new Promise<boolean>((resolve) => {
    child.once("exit", () => resolve(true));
    setTimeout(() => resolve(false), 2000);
  });
  if (exited) {
    return;
  }

  if (process.platform === "win32" && child.pid) {
    const killer = spawn("taskkill", ["/F", "/T", "/PID", String(child.pid)], {
      stdio: "ignore",
    });
    await new Promise<void>((resolve) => {
      killer.once("exit", () => resolve());
      killer.once("error", () => resolve());
    });
    return;
  }

  child.kill("SIGKILL");
  await new Promise<void>((resolve) => {
    child.once("exit", () => resolve());
    setTimeout(resolve, 2000);
  });
}

async function waitForServer(url: string): Promise<void> {
  const deadline = Date.now() + 10000;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(url);
      if (response.ok) {
        return;
      }
    } catch {}
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`fake-chatgpt server did not become ready at ${url}`);
}
