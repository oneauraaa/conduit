import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { settings } from "@/lib/standalone";
import type { ControlState } from "@/lib/types";

const ipc = vi.hoisted(() => ({
  getControlState: vi.fn(),
  getSettings: vi.fn(),
  subscribe: vi.fn(),
}));

vi.mock("@/lib/ipc", () => ipc);
vi.mock("./AiCursor", () => ({ AiCursor: () => null }));

import { Overlay } from "./Overlay";

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
  ipc.getSettings.mockResolvedValue({ ...settings, outlineDesktop: true, outlineBrowser: false });
  ipc.subscribe.mockImplementation((event, handler) => {
    if (event === "control:state") onState = handler;
    return () => {};
  });
});

afterEach(() => cleanup());

it("removes the desktop outline immediately when browser work starts", async () => {
  const { container } = render(<Overlay />);
  await vi.waitFor(() => expect(container.querySelectorAll(".animate-breathe")).toHaveLength(4));

  act(() => onState({ ...active, actionSurface: "browser" }));

  expect(container.querySelectorAll(".animate-breathe")).toHaveLength(0);
});

it("hides the outline for background work by default", async () => {
  const { container } = render(<Overlay />);
  await vi.waitFor(() => expect(container.querySelectorAll(".animate-breathe")).toHaveLength(4));

  act(() => onState({ ...active, actionSurface: "background" }));

  expect(container.querySelectorAll(".animate-breathe")).toHaveLength(0);
});
