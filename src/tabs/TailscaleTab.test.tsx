import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { TailscaleState } from "@/lib/types";

const ipc = vi.hoisted(() => ({
  getTailscaleState: vi.fn(),
  enableRemote: vi.fn(),
  enableRemoteWithPassword: vi.fn(),
  disableRemote: vi.fn(),
  regenerateRemoteToken: vi.fn(),
  subscribe: vi.fn(() => () => {}),
}));
vi.mock("@/lib/ipc", () => ipc);

import { TailscaleTab } from "./TailscaleTab";

afterEach(() => cleanup());

it("asks for the Linux password only after Tailscale requires operator access", async () => {
  const idle: TailscaleState = {
    installed: true,
    connected: true,
    hostname: "desktop.example.ts.net",
    sharing: false,
    publicUrl: null,
    error: null,
  };
  ipc.getTailscaleState.mockResolvedValue(idle);
  ipc.enableRemote.mockRejectedValue("TAILSCALE_OPERATOR_REQUIRED: operator access needed");
  ipc.enableRemoteWithPassword.mockResolvedValue({ ...idle, sharing: true, publicUrl: "https://desktop.example.ts.net/key/mcp" });

  render(<TailscaleTab />);
  fireEvent.click(await screen.findByRole("switch", { name: "Share conduit on the internet" }));
  const dialog = await screen.findByRole("dialog", { name: "Allow Tailscale Funnel" });
  expect(screen.queryByText("TAILSCALE_OPERATOR_REQUIRED: operator access needed")).not.toBeInTheDocument();
  fireEvent.change(screen.getByLabelText("Linux password"), { target: { value: "test-secret" } });
  fireEvent.click(screen.getByRole("button", { name: "enable Funnel" }));
  await waitFor(() => expect(ipc.enableRemoteWithPassword).toHaveBeenCalledWith("test-secret"));
  await waitFor(() => expect(dialog).not.toBeInTheDocument());
  expect(screen.getByText("https://desktop.example.ts.net/key/mcp")).toBeInTheDocument();
});
