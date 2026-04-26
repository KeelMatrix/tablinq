import { writeFileSync } from "node:fs";
import { mkdir } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import type { Page } from "playwright-core";

export interface ProfileAccessSnapshot {
  profileAccessible: boolean;
  profileAccessError: string | null;
}

export class DiagnosticsBundleCapture {
  constructor(private readonly profileDir: string) {}

  async checkProfileAccess(): Promise<ProfileAccessSnapshot> {
    try {
      await mkdir(this.profileDir, { recursive: true });
      writeFileSync(
        resolve(this.profileDir, ".tablinq-profile-check"),
        "ok",
        "utf8",
      );
      return { profileAccessible: true, profileAccessError: null };
    } catch (error) {
      return {
        profileAccessible: false,
        profileAccessError: error instanceof Error ? error.message : String(error),
      };
    }
  }

  async captureBundleScreenshot(page: Page, bundleDir: string): Promise<string> {
    const screenshotPath = resolve(bundleDir, "failure.png");
    await mkdir(dirname(screenshotPath), { recursive: true });
    await page.screenshot({ fullPage: true, path: screenshotPath });
    return screenshotPath;
  }
}
