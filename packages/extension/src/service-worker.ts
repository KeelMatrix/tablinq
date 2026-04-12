export type BridgeTransport = "native_messaging" | "websocket_dev";

export interface BridgeStatus {
  transport: BridgeTransport;
  connected: boolean;
}

export function describeBridge(): BridgeStatus {
  return {
    transport: "native_messaging",
    connected: false,
  };
}
