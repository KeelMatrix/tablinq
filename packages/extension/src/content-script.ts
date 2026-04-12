export interface CapabilityProbeSnapshot {
  composerReady: boolean;
  submitAvailable: boolean;
  modelPickerVisible: boolean;
}

export function probeStub(
  documentRoot: Document = document,
): CapabilityProbeSnapshot {
  return {
    composerReady: Boolean(
      documentRoot.querySelector("[data-slot='composer']"),
    ),
    submitAvailable: Boolean(
      documentRoot.querySelector("[data-action='submit']"),
    ),
    modelPickerVisible: Boolean(
      documentRoot.querySelector("[data-slot='model-picker']"),
    ),
  };
}
