export function createTablinqPageAdapterRuntime() {
  const textUtils = createTextUtilsModule();
  const domUtils = createDomUtilsModule({ textUtils });
  const conversationRuntime = createConversationRuntimeModule();
  let messageRuntime;
  const composerRuntime = createComposerRuntimeModule({
    textUtils,
    domUtils,
    getMessageRuntime: () => messageRuntime,
  });
  const messageFilters = createMessageFilterRuntimeModule({
    textUtils,
    domUtils,
  });
  const markdownRuntime = createMarkdownRuntimeModule({
    textUtils,
    domUtils,
    messageFilters,
  });
  messageRuntime = createAssistantMessageRuntimeModule({
    textUtils,
    domUtils,
    messageFilters,
    markdownRuntime,
    composerRuntime,
    conversationRuntime,
  });
  const capabilityRuntime = createCapabilityRuntimeModule({
    textUtils,
    domUtils,
    composerRuntime,
  });
  const researchRuntime = createResearchRuntimeModule({
    textUtils,
    domUtils,
    composerRuntime,
    messageRuntime,
    markdownRuntime,
    conversationRuntime,
  });

  return createCommandRuntime({
    textUtils,
    domUtils,
    composerRuntime,
    messageRuntime,
    capabilityRuntime,
    researchRuntime,
    conversationRuntime,
  });
}
