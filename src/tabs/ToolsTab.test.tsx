import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { settings as sampleSettings } from "@/lib/standalone";
import type { ToolDef } from "@/lib/types";

const ipc = vi.hoisted(() => ({
  getSettings: vi.fn(),
  getToolCatalog: vi.fn(),
  setDefaultAccess: vi.fn(),
  setToolEnabled: vi.fn(),
  setToolsAccess: vi.fn(),
  subscribe: vi.fn(() => () => {}),
}));

vi.mock("@/lib/ipc", () => ipc);

import { ToolsTab } from "./ToolsTab";

afterEach(() => cleanup());

it("shows the Linux shell category and allows a switch while all tools are on", async () => {
  const shell: ToolDef = {
    name: "run_shell",
    group: "linux",
    summary: "run a command in any installed shell and capture its output",
    risky: true,
  };
  ipc.getSettings.mockResolvedValue({ ...sampleSettings, toolsAccess: "all" });
  ipc.getToolCatalog.mockResolvedValue([shell]);
  ipc.setToolEnabled.mockResolvedValue({
    ...sampleSettings,
    toolsAccess: "custom",
    toolToggles: { run_shell: false },
  });

  render(<ToolsTab />);

  expect(await screen.findByText("linux tools")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("switch", { name: "run_shell" }));
  expect(ipc.setToolEnabled).toHaveBeenCalledWith("run_shell", false);
});
