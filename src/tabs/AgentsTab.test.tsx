import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentTarget, SandboxesState } from "@/lib/types";

const ipc = vi.hoisted(() => ({
  getSandboxState: vi.fn(),
  installAgent: vi.fn(),
  listAgents: vi.fn(),
  subscribe: vi.fn(() => () => {}),
  uninstallAgent: vi.fn(),
}));

vi.mock("@/lib/ipc", () => ipc);

import { AgentsTab } from "./AgentsTab";

const server = { status: "running", port: 6767, startedAt: 1, lastError: null } as const;

function agent(installedTargets: string[]): AgentTarget {
  return {
    id: "codex",
    name: "codex",
    configPath: "/home/me/.codex/config.toml",
    detected: true,
    installed: installedTargets.includes("host"),
    installedTargets,
    error: null,
    icon: null,
  };
}

const sandboxes: SandboxesState = {
  docker: {
    availability: "ready",
    cliPath: null,
    clientVersion: null,
    serverVersion: null,
    engine: null,
    arch: null,
    cpus: null,
    memoryBytes: null,
    hint: null,
  },
  stopOnQuit: true,
  sandboxes: [
    {
      spec: {
        id: "work",
        name: "work",
        os: "ubuntu-24.04",
        memoryMb: 4096,
        cpus: 2,
        width: 1280,
        height: 800,
        internet: true,
        autoStart: true,
        createdAt: 1,
      },
      status: "running",
      error: null,
      build: null,
      paused: false,
      endpoint: "http://127.0.0.1:6767/sandbox/work/mcp",
      busyCalls: 0,
    },
  ],
};

describe("AgentsTab", () => {
  beforeEach(() => {
    for (const fn of Object.values(ipc)) fn.mockReset();
    ipc.subscribe.mockReturnValue(() => {});
    ipc.getSandboxState.mockResolvedValue(sandboxes);
  });

  afterEach(() => cleanup());

  it("installs for this computer by default", async () => {
    ipc.listAgents.mockResolvedValue([agent([])]);
    ipc.installAgent.mockResolvedValue(agent(["host"]));
    render(<AgentsTab server={server} />);

    fireEvent.click(await screen.findByRole("button", { name: /^install$/i }));
    await waitFor(() => expect(ipc.installAgent).toHaveBeenCalledWith("codex", "host"));
    expect(await screen.findByText("127.0.0.1:6767/mcp")).toBeInTheDocument();
  });

  it("opens on the sandbox it was sent from and installs for it", async () => {
    ipc.listAgents.mockResolvedValue([agent(["host"])]);
    ipc.installAgent.mockResolvedValue(agent(["host", "work"]));
    render(<AgentsTab server={server} initialTarget="work" />);

    expect(await screen.findByRole("button", { name: "install for" })).toHaveTextContent(
      "sandbox · work",
    );
    // Installed for this computer, but not for the chosen sandbox yet.
    expect(await screen.findByText("also set up for this computer")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /^install$/i }));
    await waitFor(() => expect(ipc.installAgent).toHaveBeenCalledWith("codex", "work"));
    expect(await screen.findByText("127.0.0.1:6767/sandbox/work/mcp")).toBeInTheDocument();
  });

  it("switches targets from the picker", async () => {
    ipc.listAgents.mockResolvedValue([agent(["work"])]);
    render(<AgentsTab server={server} />);

    await screen.findByText("also set up for work");
    fireEvent.click(screen.getByRole("button", { name: "install for" }));
    fireEvent.click(screen.getByRole("option", { name: "sandbox · work" }));
    expect(await screen.findByText("127.0.0.1:6767/sandbox/work/mcp")).toBeInTheDocument();
  });
});
