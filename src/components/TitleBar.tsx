import { X } from "lucide-react";
import { hideToTray } from "@/lib/ipc";
import { trayName } from "@/lib/platform";

/**
 * Replaces the native title bar. The window is `decorations: false`, so the
 * system's own buttons — traffic lights on macOS, the caption buttons on
 * Windows — are gone, and this strip provides both the drag region and the
 * single close affordance.
 *
 * The `x` deliberately does *not* quit — conduit is the MCP server, so quitting
 * would silently revoke every agent's access. It hides to the tray instead;
 * Quit lives in the tray's right-click menu.
 */
export function TitleBar() {
  return (
    <div
      data-tauri-drag-region
      className="absolute inset-x-0 top-0 z-30 flex h-9 items-center justify-end px-2"
    >
      <button
        type="button"
        onClick={() => void hideToTray()}
        aria-label={`Hide conduit to ${trayName}`}
        title={`Hide to ${trayName} — conduit keeps running`}
        className="group grid size-6 place-items-center rounded-md text-[rgb(var(--text-faint))] transition-colors duration-150 hover:bg-[rgb(var(--text-faint)/0.14)] hover:text-[rgb(var(--text))]"
      >
        <X
          size={14}
          strokeWidth={2.5}
          className="transition-transform duration-200 ease-[var(--ease-out-soft)] group-hover:scale-110"
        />
      </button>
    </div>
  );
}
