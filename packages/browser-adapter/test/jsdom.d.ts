declare module "jsdom" {
  export class JSDOM {
    constructor(
      html?: string,
      options?: {
        url?: string;
        runScripts?: "dangerously" | "outside-only";
      },
    );

    window: Window & typeof globalThis;
  }
}
