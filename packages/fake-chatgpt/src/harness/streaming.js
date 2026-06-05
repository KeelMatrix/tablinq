export function createAskStreamState({
  scenario,
  variant,
  promptText,
  conversationId,
  messageId,
  askNumber,
  uploadedFiles,
  uniqueId,
}) {
  const responseBlocks =
    scenario === "followup-rich-text" && askNumber > 1
      ? [
          "I can help with thinking work and doing work.",
          "I can explain concepts, write and edit, summarize, brainstorm, tutor, code, debug, analyze data, compare options, plan projects, draft emails/messages, and turn rough notes into polished documents.",
        ]
      : null;
  const richRecoveryPrompt =
    scenario === "recover-rich-markdown"
      ? "Summarize release notes into a concise update with risks and actions."
      : null;
  const richRecoveryOutline =
    scenario === "recover-rich-markdown"
      ? ["Goal", "Inputs", "Constraints", "Output"]
      : null;
  const reasoningArtifacts =
    scenario === "thinking-artifact-messages" && variant === "live"
      ? [
          "I’m reconstructing the prior context from the recovery note and turning it into a compact comparison that makes the tradeoff explicit rather than just listing features.",
          "The comparison is narrowing to one core distinction: Rust optimizes for correctness under low-level constraints, while TypeScript optimizes for speed and safety of application development. I’m turning that into a decision-oriented visual instead of a feature list.",
        ]
      : null;
  const responsePromptText = compactPromptForResponse(promptText);

  return {
    conversationId,
    messageId,
    promptText,
    responseText:
      scenario === "provider-generation-failure-after-success" &&
      promptText.includes("Recovery note:")
        ? `Recovered response for: ${responsePromptText}\nUploads: ${uploadedFiles().length ? uploadedFiles().join(", ") : "none"}`
        : responseBlocks
          ? responseBlocks.join("\n\n")
          : richRecoveryPrompt
            ? `${richRecoveryPrompt}\n\nOutline\n1. Goal\n2. Inputs\n3. Constraints\n4. Output`
            : promptText
              ? `Fake response for: ${responsePromptText}`
              : "Fake response for an empty prompt.",
    responseBlocks,
    richRecoveryPrompt,
    richRecoveryOutline,
    reasoningArtifacts,
    reasoningArtifactMessageIds: reasoningArtifacts
      ? reasoningArtifacts.map(() => uniqueId("msg"))
      : [],
    index: 0,
    askNumber,
    didReload: false,
    pendingStart: false,
    phase: reasoningArtifacts ? "prelude-1" : "answer",
  };
}

function compactPromptForResponse(promptText) {
  if (promptText.length <= 1_000) {
    return promptText;
  }
  return `${promptText.slice(0, 500)}... [${promptText.length} chars]`;
}
