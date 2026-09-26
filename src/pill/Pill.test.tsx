import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { ControlState, PendingApproval } from "@/lib/types";

const ipc = vi.hoisted(() => ({
  getControlState: vi.fn(),
  getPendingApproval: vi.fn(),
  resolveApproval: vi.fn(),
  setSessionMode: vi.fn(),
  stopControl: vi.fn(),
  subscribe: vi.fn(),
}));

vi.mock("@/lib/ipc", () => ipc);

import { Pill } from "./Pill";

const active: ControlState = {
  phase: "active",
  agent: "codex",
  mode: "full",
  action: "checking displays",
  actionSurface: "desktop",
  stopped: false,
};

let onState: (state: ControlState) => void;

beforeEach(() => {
  ipc.getControlState.mockResolvedValue(active);
  ipc.getPendingApproval.mockResolvedValue(null);
  ipc.subscribe.mockImplementation((event, handler) => {
    if (event === "control:state") onState = handler;
    return () => {};
  });
});

afterEach(() => cleanup());

it("replaces the action inside one pill when tools change rapidly", async () => {
  render(<Pill />);
  await screen.findByText("checking displays");

  act(() => onState({ ...active, action: "checking the cursor" }));

  expect(screen.queryByText("checking displays")).not.toBeInTheDocument();
  expect(screen.getByText("checking the cursor")).toBeInTheDocument();
  expect(screen.getAllByRole("button", { name: "Stop — hand control back to me" })).toHaveLength(1);
});

it("does not let a late initial read restore an older action", async () => {
  let resolveInitial!: (state: ControlState) => void;
  ipc.getControlState.mockReturnValue(new Promise<ControlState>((resolve) => {
    resolveInitial = resolve;
  }));
  render(<Pill />);

  act(() => onState({ ...active, action: "checking the cursor" }));
  await act(async () => resolveInitial(active));

  expect(screen.getByText("checking the cursor")).toBeInTheDocument();
  expect(screen.queryByText("checking displays")).not.toBeInTheDocument();
});

it("loads an approval opened while the pill webview was hidden", async () => {
  const pending: PendingApproval = {
    id: "approval-1",
    tool: "run_shell",
    summary: "wants to run a command",
    detail: null,
    agent: "codex",
    category: null,
  };
  ipc.getPendingApproval.mockResolvedValue(pending);

  render(<Pill />);

  expect(await screen.findByText("wants to run a command")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "allow once" })).toBeInTheDocument();
});
