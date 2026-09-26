import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DockerStatus, SandboxesState, SandboxView } from "@/lib/types";

const ipc = vi.hoisted(() => ({
  cancelSandboxBuild: vi.fn(),
  createSandbox: vi.fn(),
  deleteSandbox: vi.fn(),
  getSandboxState: vi.fn(),
  interruptSandboxAgent: vi.fn(),
  openDockerDownload: vi.fn(),
  refreshDocker: vi.fn(),
  resumeSandboxAgent: vi.fn(),
  setSandboxStopOnQuit: vi.fn(),
  setSandboxTabVisible: vi.fn(),
  startSandbox: vi.fn(),
  stopSandbox: vi.fn(),
  subscribe: vi.fn(() => () => {}),
  updateSandbox: vi.fn(),
}));

vi.mock("@/lib/ipc", () => ipc);
// The live view needs a real VNC stream; here it only has to say what it was asked to show.
vi.mock("@/components/sandbox/NoVncView", () => ({
  NoVncView: (props: { sandboxId: string; running: boolean; interactive: boolean }) => (
    <div data-testid="live-view">
      {props.sandboxId} {props.running ? "running" : "idle"} {props.interactive ? "interactive" : "watching"}
    </div>
  ),
}));

import { SandboxTab } from "./SandboxTab";

function docker(overrides: Partial<DockerStatus> = {}): DockerStatus {
  return {
    availability: "ready",
    cliPath: "/usr/local/bin/docker",
    clientVersion: "27.3.1",
    serverVersion: "27.3.1",
    engine: "Docker Desktop",
    arch: "aarch64",
    cpus: 8,
    memoryBytes: 16 * 1024 ** 3,
    hint: null,
    ...overrides,
  };
}

function sandbox(id: string, overrides: Partial<SandboxView> = {}): SandboxView {
  return {
    spec: {
      id,
      name: id,
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
    endpoint: `http://127.0.0.1:6767/sandbox/${id}/mcp`,
    busyCalls: 0,
    ...overrides,
  };
}

function state(sandboxes: SandboxView[], dockerStatus = docker()): SandboxesState {
  return { docker: dockerStatus, sandboxes, stopOnQuit: true };
}

describe("SandboxTab", () => {
  beforeEach(() => {
    for (const fn of Object.values(ipc)) fn.mockReset();
    ipc.subscribe.mockReturnValue(() => {});
    ipc.setSandboxTabVisible.mockResolvedValue(undefined);
  });

  afterEach(() => cleanup());

  it("explains what to do when Docker is missing", async () => {
    const missing = state([], docker({ availability: "missing", hint: "install Docker Desktop" }));
    ipc.getSandboxState.mockResolvedValue(missing);
    ipc.refreshDocker.mockResolvedValue(missing);
    render(<SandboxTab onConnectAgent={() => {}} />);

    expect(await screen.findByText("sandboxes need Docker")).toBeInTheDocument();
    expect(screen.getByText("install Docker Desktop")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /get docker/i }));
    expect(ipc.openDockerDownload).toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: /check again/i }));
    expect(ipc.refreshDocker).toHaveBeenCalled();
  });

  it("widens the window while open and narrows it again on leave", async () => {
    ipc.getSandboxState.mockResolvedValue(state([]));
    const { unmount } = render(<SandboxTab onConnectAgent={() => {}} />);
    await screen.findByText("no sandboxes yet");
    expect(ipc.setSandboxTabVisible).toHaveBeenCalledWith(true);
    unmount();
    expect(ipc.setSandboxTabVisible).toHaveBeenLastCalledWith(false);
  });

  it("shows a running sandbox's live view, watch-only until the user takes control", async () => {
    ipc.getSandboxState.mockResolvedValue(state([sandbox("work")]));
    render(<SandboxTab onConnectAgent={() => {}} />);

    const view = await screen.findByTestId("live-view");
    expect(view).toHaveTextContent("work running watching");
    fireEvent.click(screen.getByRole("button", { name: /take control/i }));
    expect(view).toHaveTextContent("interactive");
    expect(screen.getByText("http://127.0.0.1:6767/sandbox/work/mcp")).toBeInTheDocument();
  });

  it("shows build progress instead of a screen while the image builds, and can cancel", async () => {
    const building = sandbox("lab", {
      status: "building",
      build: { os: "debian-12", step: 4, totalSteps: 8, lastLine: "Unpacking xfce4-panel", startedAt: 1 },
      spec: { ...sandbox("lab").spec, os: "debian-12" },
    });
    ipc.getSandboxState.mockResolvedValue(state([building]));
    render(<SandboxTab onConnectAgent={() => {}} />);

    expect(await screen.findByText(/step 4 of 8/)).toBeInTheDocument();
    expect(screen.getByText("Unpacking xfce4-panel")).toBeInTheDocument();
    expect(screen.queryByTestId("live-view")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /cancel/i }));
    expect(ipc.cancelSandboxBuild).toHaveBeenCalledWith("lab");
  });

  it("expands the Docker output beneath a failed image build", async () => {
    ipc.getSandboxState.mockResolvedValue(state([sandbox("lab", {
      status: "needsImage",
      error: "the desktop image did not build: unknown flag: --progress\n\nDocker output (last 2 lines):\nunknown flag: --progress\nRun 'docker build --help' for more information",
    })]));
    render(<SandboxTab onConnectAgent={() => {}} />);

    expect(await screen.findByText("the desktop image did not build: unknown flag: --progress")).toBeInTheDocument();
    const details = screen.getByText("show details").closest("details")!;
    expect(details.open).toBe(false);
    fireEvent.click(screen.getByText("show details"));
    expect(details.open).toBe(true);
    expect(within(details).getByText(/Run 'docker build --help' for more information/)).toBeInTheDocument();
  });

  it("creates a sandbox with sensible defaults and selects it", async () => {
    ipc.getSandboxState.mockResolvedValue(state([]));
    ipc.createSandbox.mockResolvedValue(state([sandbox("work", { status: "starting" })]));
    render(<SandboxTab onConnectAgent={() => {}} />);

    fireEvent.click((await screen.findAllByRole("button", { name: /new sandbox/i }))[0]);
    const dialog = screen.getByRole("dialog", { name: "new sandbox" });
    const create = within(dialog).getByRole("button", { name: /create and start/i });
    expect(create).toBeDisabled();
    fireEvent.change(within(dialog).getByPlaceholderText("e.g. work"), { target: { value: " work " } });
    fireEvent.click(create);

    await waitFor(() =>
      expect(ipc.createSandbox).toHaveBeenCalledWith({
        name: "work",
        os: "ubuntu-24.04",
        memoryMb: 4096,
        cpus: 2,
        width: 1280,
        height: 800,
        internet: true,
        autoStart: true,
      }),
    );
    expect(await screen.findByRole("heading", { name: "work" })).toBeInTheDocument();
  });

  it("offers only what Docker can give", async () => {
    const small = docker({ memoryBytes: 4 * 1024 ** 3, cpus: 2 });
    ipc.getSandboxState.mockResolvedValue(state([], small));
    render(<SandboxTab onConnectAgent={() => {}} />);

    fireEvent.click((await screen.findAllByRole("button", { name: /new sandbox/i }))[0]);
    const dialog = screen.getByRole("dialog", { name: "new sandbox" });
    expect(within(dialog).getByRole("radio", { name: "4 GB" })).toBeInTheDocument();
    expect(within(dialog).queryByRole("radio", { name: "8 GB" })).not.toBeInTheDocument();
    expect(within(dialog).queryByRole("radio", { name: "4" })).not.toBeInTheDocument();
  });

  it("changes settings, stops the agent, and connects agents", async () => {
    const onConnectAgent = vi.fn();
    ipc.getSandboxState.mockResolvedValue(state([sandbox("work")]));
    ipc.updateSandbox.mockResolvedValue(state([sandbox("work")]));
    ipc.interruptSandboxAgent.mockResolvedValue(state([sandbox("work", { paused: true })]));
    render(<SandboxTab onConnectAgent={onConnectAgent} />);

    fireEvent.click(await screen.findByRole("radio", { name: "8 GB" }));
    expect(ipc.updateSandbox).toHaveBeenCalledWith("work", { memoryMb: 8192 });

    // Controls stay disabled while a change is in flight.
    const internet = screen.getByRole("switch", { name: "internet access" });
    await waitFor(() => expect(internet).toBeEnabled());
    fireEvent.click(internet);
    await waitFor(() => expect(ipc.updateSandbox).toHaveBeenCalledWith("work", { internet: false }));

    await waitFor(() => expect(screen.getByRole("button", { name: /stop agent/i })).toBeEnabled());
    fireEvent.click(screen.getByRole("button", { name: /stop agent/i }));
    expect(await screen.findByRole("button", { name: /resume agent/i })).toBeInTheDocument();
    expect(ipc.interruptSandboxAgent).toHaveBeenCalledWith("work");

    fireEvent.click(screen.getByRole("button", { name: /connect an agent/i }));
    expect(onConnectAgent).toHaveBeenCalledWith("work");
  });

  it("asks before deleting", async () => {
    ipc.getSandboxState.mockResolvedValue(state([sandbox("work")]));
    ipc.deleteSandbox.mockResolvedValue(state([]));
    const confirm = vi.spyOn(window, "confirm");
    render(<SandboxTab onConnectAgent={() => {}} />);

    confirm.mockReturnValueOnce(false);
    fireEvent.click(await screen.findByRole("button", { name: /delete sandbox/i }));
    expect(ipc.deleteSandbox).not.toHaveBeenCalled();

    confirm.mockReturnValueOnce(true);
    fireEvent.click(screen.getByRole("button", { name: /delete sandbox/i }));
    await waitFor(() => expect(ipc.deleteSandbox).toHaveBeenCalledWith("work"));
    expect(await screen.findByText("no sandboxes yet")).toBeInTheDocument();
    confirm.mockRestore();
  });

  it("switches between sandboxes from the list", async () => {
    ipc.getSandboxState.mockResolvedValue(
      state([sandbox("work"), sandbox("play", { status: "stopped" })]),
    );
    render(<SandboxTab onConnectAgent={() => {}} />);

    await screen.findByRole("heading", { name: "work" });
    fireEvent.click(screen.getByRole("option", { name: /play/ }));
    expect(await screen.findByRole("heading", { name: "play" })).toBeInTheDocument();
    ipc.startSandbox.mockResolvedValue(state([sandbox("work"), sandbox("play")]));
    fireEvent.click(screen.getByRole("button", { name: /^start$/i }));
    await waitFor(() => expect(ipc.startSandbox).toHaveBeenCalledWith("play"));
  });
});
