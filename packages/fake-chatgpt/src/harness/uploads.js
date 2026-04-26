export function createUploadRuntime({ document, variant, config, refs }) {
  const { uploadList, sendButton, staleSendButton, stopButton, artifactLink } =
    refs;
  let uploadProcessingTimer = null;

  function setUploadPopoverOpen(open) {
    document.body.dataset.uploadPopoverOpen = open ? "true" : "false";
  }

  function uploadedFiles() {
    return (document.body.dataset.uploadedFiles || "")
      .split("|")
      .map((value) => value.trim())
      .filter(Boolean);
  }

  function renderUploadedFiles(fileNames) {
    if (!uploadList) {
      return;
    }
    const chips = fileNames
      .map((fileName) =>
        variant === "live"
          ? `<span class="pill" data-testid="upload-chip" aria-label="Attached file ${fileName}">${fileName}</span>`
          : `<span class="pill" data-slot="uploaded-file">${fileName}</span>`,
      )
      .join("");
    const processingIndicator =
      document.body.dataset.uploadProcessing === "true"
        ? `<span class="pill" data-slot="upload-processing" data-upload-processing="true" aria-busy="true">Processing uploads...</span>`
        : "";
    uploadList.innerHTML = `${chips}${processingIndicator}`;
    syncSubmissionControls();
  }

  function syncSubmissionControls() {
    const running = document.body.dataset.streamState === "running";
    const uploadProcessing = document.body.dataset.uploadProcessing === "true";
    const submitBlocked =
      config.hasBlockedSubmitAfterUpload &&
      uploadedFiles().length > 0 &&
      !running;
    if (sendButton instanceof HTMLElement) {
      sendButton.toggleAttribute("hidden", running);
      sendButton.setAttribute(
        "aria-disabled",
        uploadProcessing || submitBlocked ? "true" : "false",
      );
      sendButton.setAttribute("aria-busy", uploadProcessing ? "true" : "false");
      sendButton.dataset.uploadProcessing = uploadProcessing ? "true" : "false";
      sendButton.dataset.busy = uploadProcessing ? "true" : "false";
      if ("disabled" in sendButton) {
        sendButton.disabled = uploadProcessing || submitBlocked;
      }
    }
    if (staleSendButton instanceof HTMLElement) {
      staleSendButton.hidden = uploadedFiles().length === 0 || running;
      staleSendButton.setAttribute("aria-disabled", "false");
      staleSendButton.dataset.busy = "false";
    }
    stopButton?.toggleAttribute("hidden", !running);
    document.body.dataset.keyboardSubmitReady =
      !running && !uploadProcessing && config.keyboardSubmitEnabled
        ? "true"
        : "false";
  }

  function setStreamingControls(running) {
    setUploadPopoverOpen(false);
    syncSubmissionControls();
    stopButton?.toggleAttribute("hidden", !running);
  }

  function stopUploadProcessingTimer() {
    if (uploadProcessingTimer) {
      clearTimeout(uploadProcessingTimer);
      uploadProcessingTimer = null;
    }
  }

  function scheduleUploadProcessingClear(delayMs) {
    stopUploadProcessingTimer();
    uploadProcessingTimer = setTimeout(() => {
      setUploadProcessing(false);
      uploadProcessingTimer = null;
    }, delayMs);
  }

  function setUploadProcessing(processing) {
    document.body.dataset.uploadProcessing = processing ? "true" : "false";
    renderUploadedFiles(uploadedFiles());
    syncSubmissionControls();
  }

  function resetArtifactLink() {
    if (!artifactLink) {
      return;
    }
    if (variant === "live") {
      artifactLink.removeAttribute("href");
      artifactLink.removeAttribute("download");
    } else {
      artifactLink.setAttribute("href", "/artifact.txt");
      artifactLink.setAttribute("download", "artifact.txt");
    }
    artifactLink.setAttribute("data-artifact-ready", "false");
  }

  function setUploadedFiles(fileNames) {
    document.body.dataset.uploadedFiles = fileNames.join("|");
    renderUploadedFiles(fileNames);
    resetArtifactLink();
  }

  function buildArtifactPayload(promptText) {
    const uploads = uploadedFiles();
    return [
      `Prompt: ${promptText || "(empty)"}`,
      `Mode: ${document.body.dataset.selectedMode}`,
      `Model: ${document.body.dataset.selectedModel}`,
      `Effort: ${document.body.dataset.selectedEffort}`,
      `Uploads: ${uploads.length ? uploads.join(", ") : "none"}`,
    ].join("\n");
  }

  function prepareArtifact(promptText) {
    if (!artifactLink) {
      return;
    }
    const uploads = uploadedFiles();
    if (uploads.length === 0) {
      resetArtifactLink();
      return;
    }

    const payload = buildArtifactPayload(promptText);
    const blob = new Blob([payload], { type: "text/plain;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const safeStem = uploads[0]
      .replace(/\.[^.]+$/, "")
      .replace(/[^a-z0-9._-]+/gi, "-")
      .toLowerCase();
    artifactLink.setAttribute("href", url);
    artifactLink.setAttribute(
      "download",
      `${safeStem || "artifact"}-summary.txt`,
    );
    artifactLink.setAttribute("data-artifact-ready", "true");
  }

  return {
    setUploadPopoverOpen,
    uploadedFiles,
    renderUploadedFiles,
    syncSubmissionControls,
    setStreamingControls,
    stopUploadProcessingTimer,
    scheduleUploadProcessingClear,
    setUploadProcessing,
    resetArtifactLink,
    setUploadedFiles,
    buildArtifactPayload,
    prepareArtifact,
  };
}
