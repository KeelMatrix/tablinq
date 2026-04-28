import type { BrowserAdapter } from "../../src/browserAdapter";

export async function maybeCloseManagedBrowserOnRunningIdentity(
  adapter: BrowserAdapter,
  update: {
    state: string;
    conversation_id?: string | null;
    message_id?: string | null;
  },
  alreadyClosed: boolean,
): Promise<boolean> {
  if (
    alreadyClosed ||
    update.state !== "running" ||
    (!update.message_id && !update.conversation_id)
  ) {
    return alreadyClosed;
  }
  await adapter.getManagedPageForTest()!.context().browser()!.close();
  return true;
}
