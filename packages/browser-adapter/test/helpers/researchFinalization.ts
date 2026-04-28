export function fakeArtifactPage(): any {
  const locator = {
    first: () => locator,
    count: async () => 0,
    all: async () => [],
  };
  return {
    locator: () => locator,
    evaluate: async () => false,
  };
}

export function fakeResearchHost(): any {
  return {
    getDownloadsDir: () => process.cwd(),
    ensureManagedPage: async () => fakeArtifactPage(),
    resolveConversationTarget: (conversationId: string | null) =>
      `https://chatgpt.com/c/${conversationId ?? "conv-research-final"}`,
  };
}

export function fakeResearchSnapshot(reportText: string): any {
  return {
    status: "done",
    statusMessage: "research completed",
    reportText,
    conversationId: "conv-research-final",
    messageId: "msg-research-final",
    currentUrl: "https://chatgpt.com/c/conv-research-final",
  };
}
