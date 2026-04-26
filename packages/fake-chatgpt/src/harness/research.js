export function createResearchState({
  promptText,
  conversationId,
  messageId,
  uploadedFiles,
  usesDeepResearchFrame,
}) {
  return {
    conversationId,
    messageId,
    promptText,
    reportText: promptText
      ? `Research report for: ${promptText}\n\nSources were reviewed and summarized.\nUploads: ${uploadedFiles().length ? uploadedFiles().join(", ") : "none"}`
      : "Research report for an empty query.",
    index: 0,
    phase: usesDeepResearchFrame ? "planning" : "waiting",
    waitTicks: 0,
  };
}
