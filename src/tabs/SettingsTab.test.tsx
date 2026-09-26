import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { browser as sampleBrowser, settings as sampleSettings } from "@/lib/standalone";

const ipc = vi.hoisted(() => ({
  getSettings: vi.fn(),
  getBrowserState: vi.fn(),
  setStartOnLogin: vi.fn(),
  setStartHidden: vi.fn(),
  setBrowserAutoStart: vi.fn(),
  setOutlineDesktop: vi.fn(),
  setOutlineBrowser: vi.fn(),
  setOutlineBackground: vi.fn(),
  setPillDesktop: vi.fn(),
  setPillBrowser: vi.fn(),
  setPillBackground: vi.fn(),
  setCorsEnabled: vi.fn(),
  setCorsOrigins: vi.fn(),
  uninstallBrowser: vi.fn(),
  subscribe: vi.fn(() => () => {}),
}));

vi.mock("@/lib/ipc", () => ipc);

import { SettingsTab } from "./SettingsTab";

beforeEach(() => {
  ipc.getSettings.mockResolvedValue({ ...sampleSettings });
  ipc.getBrowserState.mockResolvedValue({
    ...sampleBrowser,
    install: { ...sampleBrowser.install, status: "ready", installedRevision: "test" },
  });
  ipc.setOutlineBrowser.mockResolvedValue({ ...sampleSettings, outlineBrowser: true });
  ipc.setOutlineBackground.mockResolvedValue({ ...sampleSettings, outlineBackground: true });
  ipc.setPillDesktop.mockResolvedValue({ ...sampleSettings, pillDesktop: false });
  ipc.setPillBrowser.mockResolvedValue({ ...sampleSettings, pillBrowser: false });
  ipc.setPillBackground.mockResolvedValue({ ...sampleSettings, pillBackground: true });
  ipc.setBrowserAutoStart.mockResolvedValue({ ...sampleSettings, browserAutoStart: false });
  ipc.uninstallBrowser.mockResolvedValue({
    ...sampleBrowser,
    install: { ...sampleBrowser.install, status: "unavailable", installedRevision: null },
  });
});

afterEach(() => cleanup());

it("saves outline and auto start preferences and uninstalls the managed browser", async () => {
  const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
  render(<SettingsTab />);

  fireEvent.click(await screen.findByRole("switch", { name: "show outline for browser actions" }));
  await waitFor(() => expect(ipc.setOutlineBrowser).toHaveBeenCalledWith(true));
  fireEvent.click(screen.getByRole("switch", { name: "start Chromium with Conduit" }));
  await waitFor(() => expect(ipc.setBrowserAutoStart).toHaveBeenCalledWith(false));

  fireEvent.click(await screen.findByRole("button", { name: "uninstall" }));
  expect(confirm).toHaveBeenCalledOnce();
  await waitFor(() => expect(ipc.uninstallBrowser).toHaveBeenCalledOnce());
  await waitFor(() => expect(screen.getByRole("button", { name: "uninstall" })).toBeDisabled());
});

it("saves separate pill visibility preferences", async () => {
  render(<SettingsTab />);

  fireEvent.click(await screen.findByRole("switch", { name: "show pill for desktop actions" }));
  await waitFor(() => expect(ipc.setPillDesktop).toHaveBeenCalledWith(false));
  fireEvent.click(screen.getByRole("switch", { name: "show pill for browser actions" }));
  await waitFor(() => expect(ipc.setPillBrowser).toHaveBeenCalledWith(false));
});

it("defaults background visibility off and saves both preferences", async () => {
  render(<SettingsTab />);
  const outline = await screen.findByRole("switch", { name: "show outline for background actions" });
  const pill = screen.getByRole("switch", { name: "show pill for background actions" });
  expect(outline).not.toBeChecked();
  expect(pill).not.toBeChecked();
  fireEvent.click(outline);
  await waitFor(() => expect(ipc.setOutlineBackground).toHaveBeenCalledWith(true));
  fireEvent.click(pill);
  await waitFor(() => expect(ipc.setPillBackground).toHaveBeenCalledWith(true));
});
