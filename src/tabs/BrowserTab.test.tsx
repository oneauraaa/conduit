import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { BrowserState, Settings } from "@/lib/types";

const ipc = vi.hoisted(() => ({
  cancelBrowserInstall: vi.fn(),
  createBrowserProfile: vi.fn(),
  deleteBrowserProfile: vi.fn(),
  getBrowserState: vi.fn(),
  getSettings: vi.fn(),
  installBrowser: vi.fn(),
  openBrowserDownloads: vi.fn(),
  refreshBrowserInstall: vi.fn(),
  renameBrowserProfile: vi.fn(),
  selectBrowserProfile: vi.fn(),
  setBrowserMode: vi.fn(),
  setBrowserPermission: vi.fn(),
  setBrowserTabVisible: vi.fn(),
  startBrowser: vi.fn(),
  stopBrowser: vi.fn(),
  subscribe: vi.fn(() => () => {}),
}));

vi.mock("@/lib/ipc", () => ipc);

import { BrowserTab } from "./BrowserTab";

function browser(status: BrowserState["install"]["status"] = "ready"): BrowserState {
  return {
    install: {
      status,
      expectedRevision: "playwright-mcp-0.0.80-chromium-1243",
      installedRevision: status === "ready" ? "playwright-mcp-0.0.80-chromium-1243" : null,
      downloadedBytes: status === "downloading" ? 25 : 0,
      totalBytes: 100,
      error: status === "error" ? "network interrupted" : null,
    },
    runStatus: "stopped",
    mode: "headless",
    profiles: [
      { id: "default", name: "Default", incognito: false },
      { id: "incognito", name: "Incognito", incognito: true },
    ],
    selectedProfileId: "default",
    tabs: [],
    downloads: [],
    preview: null,
    owner: null,
    stopLatched: false,
  };
}

function settings(): Settings {
  return {
    defaultAccess: "auto",
    toolsAccess: "all",
    toolToggles: {},
    port: 3210,
    theme: "system",
    remoteEnabled: false,
    remoteToken: "fixture-token",
    corsEnabled: false,
    corsOrigins: [],
    startOnLogin: false,
    startHidden: false,
    browserPermissions: {
      openWebsites: "alwaysAllow",
      readHistory: "alwaysAllow",
      downloadFiles: "alwaysAsk",
      uploadFiles: "alwaysAsk",
    },
  };
}

describe("BrowserTab", () => {
  beforeEach(() => {
    ipc.getSettings.mockResolvedValue(settings());
    ipc.refreshBrowserInstall.mockResolvedValue(browser("unavailable"));
    ipc.setBrowserTabVisible.mockResolvedValue(undefined);
    ipc.setBrowserPermission.mockImplementation(async (category, mode) => ({
      ...settings(),
      browserPermissions: { ...settings().browserPermissions, [category]: mode },
    }));
  });

  afterEach(() => cleanup());

  it("shows only the download panel until Chromium is installed", async () => {
    ipc.getBrowserState.mockResolvedValue(browser("unavailable"));
    const view = render(<BrowserTab />);
    expect(await screen.findByText("add Chromium to Conduit")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /download Chromium/i })).toBeInTheDocument();
    expect(screen.queryByText("permissions")).not.toBeInTheDocument();
    expect(screen.queryByText("window mode")).not.toBeInTheDocument();
    view.unmount();
    expect(ipc.setBrowserTabVisible).toHaveBeenNthCalledWith(1, true);
    expect(ipc.setBrowserTabVisible).toHaveBeenLastCalledWith(false);
  });

  it("preflights the platform manifest to show the required bundle size", async () => {
    const initial = browser("unavailable");
    // A mismatched installed revision still has its old bundle size. The UI
    // must replace it with metadata for the required update.
    initial.install.totalBytes = 100;
    const inspected = browser("unavailable");
    inspected.install.totalBytes = 222_487_990;
    ipc.getBrowserState.mockResolvedValue(initial);
    ipc.refreshBrowserInstall.mockResolvedValue(inspected);
    render(<BrowserTab />);
    expect(await screen.findByText("212 MB")).toBeInTheDocument();
    expect(ipc.refreshBrowserInstall).toHaveBeenCalledOnce();
    expect(screen.queryByText("permissions")).not.toBeInTheDocument();
  });

  it("shows byte progress and cancellation without leaking browser controls", async () => {
    ipc.getBrowserState.mockResolvedValue(browser("downloading"));
    render(<BrowserTab />);
    expect(await screen.findByText("25 B / 100 B")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /cancel/i }));
    expect(ipc.cancelBrowserInstall).toHaveBeenCalledOnce();
    expect(screen.queryByText("profile")).not.toBeInTheDocument();
  });

  it("surfaces an install error and retries from the download-only panel", async () => {
    ipc.getBrowserState.mockResolvedValue(browser("error"));
    ipc.installBrowser.mockRejectedValueOnce(new Error("checksum mismatch"));
    render(<BrowserTab />);
    expect(await screen.findByText("network interrupted")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /retry download/i }));
    expect(await screen.findByText(/checksum mismatch/)).toBeInTheDocument();
    expect(ipc.installBrowser).toHaveBeenCalledOnce();
    expect(screen.queryByText("permissions")).not.toBeInTheDocument();
  });

  it("renders the four persisted permission defaults and updates a row", async () => {
    ipc.getBrowserState.mockResolvedValue(browser());
    render(<BrowserTab />);
    expect(await screen.findByText("open websites")).toBeInTheDocument();
    expect(screen.getByText("read navigation history")).toBeInTheDocument();
    expect(screen.getByText("download files")).toBeInTheDocument();
    expect(screen.getByText("upload files")).toBeInTheDocument();
    const rows = screen.getAllByRole("radiogroup");
    fireEvent.click(rows.at(-1)!.querySelector('[role="radio"][aria-checked="false"]')!);
    await waitFor(() => expect(ipc.setBrowserPermission).toHaveBeenCalledWith("uploadFiles", "alwaysAllow"));
  });

  it("warns before a running mode change and passes the chosen restart strategy", async () => {
    const state = browser();
    state.runStatus = "running";
    ipc.getBrowserState.mockResolvedValue(state);
    ipc.setBrowserMode.mockResolvedValue({ ...state, mode: "visible" });
    render(<BrowserTab />);
    await screen.findByText("window mode");
    fireEvent.click(screen.getByRole("radio", { name: "visible" }));
    expect(await screen.findByText("restart Chromium?")).toBeInTheDocument();
    expect(screen.getByText(/Unsaved page state/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "restart fresh" }));
    await waitFor(() => expect(ipc.setBrowserMode).toHaveBeenCalledWith("visible", "fresh"));
  });

  it("keeps Stop available while Chromium is still starting", async () => {
    const state = browser();
    state.runStatus = "starting";
    ipc.getBrowserState.mockResolvedValue(state);
    ipc.stopBrowser.mockResolvedValue({ ...state, runStatus: "stopped", stopLatched: true });
    render(<BrowserTab />);
    const stop = await screen.findByRole("button", { name: "stop" });
    expect(stop).toBeEnabled();
    fireEvent.click(stop);
    await waitFor(() => expect(ipc.stopBrowser).toHaveBeenCalledOnce());
  });

  it("shows a crash reason and protects the last persistent profile", async () => {
    const state = browser();
    state.runStatus = "crashed";
    state.install.error = "Chromium closed unexpectedly";
    ipc.getBrowserState.mockResolvedValue(state);
    render(<BrowserTab />);
    expect(await screen.findByText("Chromium closed unexpectedly")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "delete browser profile" })).toBeDisabled();
  });
});
