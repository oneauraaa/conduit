import { useEffect, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { Sidebar, type Tab } from "@/components/Sidebar";
import { TitleBar } from "@/components/TitleBar";
import { ServerTab } from "@/tabs/ServerTab";
import { ToolsTab } from "@/tabs/ToolsTab";
import { AgentsTab } from "@/tabs/AgentsTab";
import { TailscaleTab } from "@/tabs/TailscaleTab";
import { getServerState, subscribe } from "@/lib/ipc";
import { isStandalone } from "@/lib/standalone";
import type { ServerState } from "@/lib/types";

const INITIAL_SERVER: ServerState = {
  status: "stopped",
  port: 6767,
  startedAt: null,
  lastError: null,
};

/**
 * Outside the Tauri shell, `?tab=tools` opens straight to a tab. Only useful
 * for reviewing the UI in a browser — the packaged app never has a query
 * string, so this always returns the default there.
 */
function initialTab(): Tab {
  if (!isStandalone) return "server";
  const t = new URLSearchParams(window.location.search).get("tab");
  return t === "tools" || t === "agents" || t === "tailscale" ? t : "server";
}

export default function App() {
  const [tab, setTab] = useState<Tab>(initialTab);
  const [server, setServer] = useState<ServerState>(INITIAL_SERVER);

  useEffect(() => {
    void getServerState().then(setServer).catch(() => {});
    return subscribe("server:state", setServer);
  }, []);

  return (
    <div className="relative flex h-full overflow-hidden rounded-[14px] border hairline bg-[rgb(var(--surface))] app-shadow">
      <TitleBar />
      <Sidebar tab={tab} onTab={setTab} serverStatus={server.status} />

      <main className="relative flex-1 overflow-hidden">
        <AnimatePresence mode="wait" initial={false}>
          <motion.div
            key={tab}
            initial={{ opacity: 0, x: 8 }}
            animate={{ opacity: 1, x: 0 }}
            exit={{ opacity: 0, x: -8 }}
            transition={{ duration: 0.18, ease: [0.22, 1, 0.36, 1] }}
            className="h-full overflow-y-auto"
          >
            {tab === "server" && <ServerTab server={server} />}
            {tab === "tools" && <ToolsTab />}
            {tab === "agents" && <AgentsTab server={server} />}
            {tab === "tailscale" && <TailscaleTab />}
          </motion.div>
        </AnimatePresence>
      </main>
    </div>
  );
}
