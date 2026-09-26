import { useEffect, useRef, useState } from "react";
import { LoaderCircle, MonitorOff, RotateCw } from "lucide-react";
import type RFB from "@novnc/novnc";
import { Button } from "@/components/Panel";
import { isStandalone } from "@/lib/standalone";
import { SandboxVncChannel } from "@/lib/vncChannel";

type ViewState = "idle" | "connecting" | "live" | "closed";

/**
 * noVNC's client class, loaded on first use.
 *
 * noVNC has a top-level `await` (a WebCodecs probe), and WebKitGTK can settle
 * `import()` of such a module before the module has finished evaluating — the
 * namespace arrives with `default` still unset and fills in a moment later.
 * Chromium does not do this. The namespace's bindings are live, so waiting
 * briefly for the export is enough.
 */
async function loadClient(): Promise<typeof RFB> {
  const mod = await import("@novnc/novnc");
  for (let tries = 0; tries < 100; tries++) {
    try {
      if (mod.default) return mod.default;
    } catch {
      // A `class` export read before evaluation is in its temporal dead zone.
    }
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  throw new Error("the live view's VNC client did not load");
}

/**
 * A live view of one sandbox's screen.
 *
 * Streams only while mounted and the sandbox is running — leaving the tab
 * closes the connection, so a sandbox nobody is watching costs nothing to
 * render. `interactive` hands the user the mouse and keyboard; otherwise the
 * view is watch-only, so a stray click cannot land in the middle of an
 * agent's work.
 */
export function NoVncView({
  sandboxId,
  running,
  interactive,
}: {
  sandboxId: string;
  running: boolean;
  interactive: boolean;
}) {
  const host = useRef<HTMLDivElement>(null);
  const rfb = useRef<RFB | null>(null);
  const [state, setState] = useState<ViewState>("idle");
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    if (!running || isStandalone || !host.current) {
      setState("idle");
      return;
    }
    let cancelled = false;
    const channel = new SandboxVncChannel(sandboxId);
    setState("connecting");
    void (async () => {
      let Client: typeof RFB;
      try {
        Client = await loadClient();
      } catch {
        if (!cancelled) setState("closed");
        return;
      }
      if (cancelled || !host.current) return;
      const client = new Client(host.current, channel, { shared: true });
      client.scaleViewport = true;
      client.resizeSession = false;
      client.viewOnly = !interactive;
      client.background = "transparent";
      client.addEventListener("connect", () => !cancelled && setState("live"));
      client.addEventListener("disconnect", () => !cancelled && setState("closed"));
      rfb.current = client;
      try {
        await channel.connect();
      } catch {
        if (!cancelled) setState("closed");
      }
    })();
    return () => {
      cancelled = true;
      rfb.current?.disconnect();
      rfb.current = null;
      channel.close();
    };
    // `interactive` is left out on purpose: it is applied live below, where a
    // change does not cost a reconnect.
  }, [sandboxId, running, attempt]);

  useEffect(() => {
    if (rfb.current) rfb.current.viewOnly = !interactive;
  }, [interactive]);

  return (
    <div className="relative size-full">
      <div ref={host} className="size-full [&_canvas]:!cursor-default" />
      {state !== "live" && (
        <div className="absolute inset-0 grid place-items-center">
          {state === "connecting" ? (
            <span className="flex items-center gap-2 text-[11px] text-white/70">
              <LoaderCircle size={13} className="animate-spin" /> connecting to the screen
            </span>
          ) : state === "closed" ? (
            <div className="flex flex-col items-center gap-2 text-[11px] text-white/70">
              <span>the live view disconnected</span>
              <Button onClick={() => setAttempt((n) => n + 1)}>
                <RotateCw size={12} /> reconnect
              </Button>
            </div>
          ) : (
            <span className="flex items-center gap-2 text-[11px] text-white/60">
              <MonitorOff size={14} />
              {isStandalone && running ? "the live view needs the conduit app" : "not running"}
            </span>
          )}
        </div>
      )}
    </div>
  );
}
