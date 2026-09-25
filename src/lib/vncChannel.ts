import { Channel, invoke } from "@tauri-apps/api/core";

type ReadyState = "connecting" | "open" | "closing" | "closed";

/**
 * A WebSocket-shaped pipe to a sandbox's VNC server, for noVNC's
 * "raw channel" mode.
 *
 * There is no socket underneath. Rust runs `socat` inside the container and
 * relays its bytes over Tauri IPC: inbound on a `Channel` (which preserves
 * order), outbound through `sandbox_viewer_send`. Nothing is published on the
 * host and the webview's CSP stays closed.
 *
 * Two details noVNC depends on:
 *  - `send` receives a view into noVNC's reusable send buffer, so the bytes
 *    are copied before anything asynchronous happens.
 *  - IPC calls may complete out of order, and VNC key-down/key-up order
 *    matters, so sends go one at a time, batching whatever queued up while
 *    the previous one was in flight.
 */
export class SandboxVncChannel {
  binaryType = "arraybuffer";
  protocol = "";
  readyState: ReadyState = "connecting";
  onopen: ((event: Event) => void) | null = null;
  onmessage: ((event: { data: ArrayBuffer }) => void) | null = null;
  onclose: ((event: { code: number; reason: string; wasClean: boolean }) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;

  #sandbox: string;
  #handle: string | null = null;
  #early: ArrayBuffer[] = [];
  #outbox: Uint8Array[] = [];
  #sending = false;

  constructor(sandbox: string) {
    this.#sandbox = sandbox;
  }

  /** Connects. Call after handing the channel to noVNC, so its handlers are set. */
  async connect(): Promise<void> {
    const inbound = new Channel<ArrayBuffer>();
    inbound.onmessage = (data) => {
      // Zero bytes is the end-of-stream signal: the sandbox stopped or socat exited.
      if (data.byteLength === 0) {
        this.#finish(false);
        return;
      }
      if (this.readyState === "open") this.onmessage?.({ data });
      else this.#early.push(data);
    };
    try {
      this.#handle = await invoke<string>("open_sandbox_viewer", {
        id: this.#sandbox,
        onData: inbound,
      });
    } catch (error) {
      this.onerror?.(new Event(String(error)));
      this.#finish(false);
      throw error;
    }
    if (this.readyState !== "connecting") return;
    this.readyState = "open";
    this.onopen?.(new Event("open"));
    // The server speaks first (the RFB version banner), often before the
    // handle arrives; replay what was held back, in order.
    for (const data of this.#early.splice(0)) this.onmessage?.({ data });
  }

  send(data: ArrayBuffer | Uint8Array): void {
    if (this.readyState !== "open") return;
    const bytes = data instanceof Uint8Array ? data.slice() : new Uint8Array(data.slice(0));
    this.#outbox.push(bytes);
    void this.#pump();
  }

  close(): void {
    if (this.readyState === "closed" || this.readyState === "closing") return;
    this.readyState = "closing";
    if (this.#handle) void invoke("close_sandbox_viewer", { viewer: this.#handle }).catch(() => {});
    this.#finish(true);
  }

  async #pump(): Promise<void> {
    if (this.#sending) return;
    this.#sending = true;
    try {
      while (this.#outbox.length > 0 && this.#handle && this.readyState === "open") {
        const parts = this.#outbox.splice(0);
        const total = parts.reduce((sum, part) => sum + part.length, 0);
        const batch = new Uint8Array(total);
        let at = 0;
        for (const part of parts) {
          batch.set(part, at);
          at += part.length;
        }
        await invoke("sandbox_viewer_send", batch, { headers: { "x-viewer": this.#handle } });
      }
    } catch {
      this.#finish(false);
    } finally {
      this.#sending = false;
    }
  }

  #finish(clean: boolean): void {
    if (this.readyState === "closed") return;
    this.readyState = "closed";
    this.#outbox = [];
    this.onclose?.({ code: clean ? 1000 : 1006, reason: "", wasClean: clean });
  }
}
