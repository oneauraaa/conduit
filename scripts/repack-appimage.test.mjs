import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { GTK_HOOK } from "./verify-appimage.mjs";
import { patchGtkLauncher } from "./repack-appimage.mjs";

test("AppImage launcher selects Wayland for the layer-shell UI", () => {
  const appDir = mkdtempSync(join(tmpdir(), "conduit-launcher-test-"));
  const hookPath = join(appDir, GTK_HOOK);
  mkdirSync(join(appDir, "apprun-hooks"));
  writeFileSync(
    hookPath,
    "export GDK_BACKEND=x11 # LinuxDeploy default\nexport GTK_THEME=Adwaita\n",
  );

  try {
    assert.equal(patchGtkLauncher(appDir), true);
    assert.equal(
      readFileSync(hookPath, "utf8"),
      "export GDK_BACKEND=wayland # Conduit's GTK layer-shell UI requires Wayland\nexport GTK_THEME=Adwaita\n",
    );
    assert.equal(patchGtkLauncher(appDir), false);
  } finally {
    rmSync(appDir, { recursive: true, force: true });
  }
});
