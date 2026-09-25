/**
 * The slice of noVNC's RFB client the Sandbox tab uses. noVNC ships no type
 * declarations; this covers only what `NoVncView` touches.
 */
declare module "@novnc/novnc" {
  export interface RawChannel {
    binaryType: string;
    protocol: string;
    readyState: string;
    send(data: ArrayBuffer | Uint8Array): void;
    close(): void;
    onopen: ((event: Event) => void) | null;
    onmessage: ((event: { data: ArrayBuffer }) => void) | null;
    onclose: ((event: { code: number; reason: string; wasClean: boolean }) => void) | null;
    onerror: ((event: Event) => void) | null;
  }

  export default class RFB extends EventTarget {
    constructor(
      target: HTMLElement,
      urlOrChannel: string | RawChannel,
      options?: { shared?: boolean; credentials?: { password?: string } },
    );
    viewOnly: boolean;
    scaleViewport: boolean;
    resizeSession: boolean;
    clipViewport: boolean;
    showDotCursor: boolean;
    focusOnClick: boolean;
    background: string;
    disconnect(): void;
    focus(): void;
    blur(): void;
  }
}
