"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const path = require("node:path");
const {
  ALLOWED_TOOLS,
  REVISION,
  isAllowedUrl,
  safeFilename,
  uniqueDestination,
  outputPath,
  previewFrameDue,
} = require("./sidecar.cjs");

test("pins the expected upstream browser revision", () => {
  assert.equal(REVISION, "playwright-mcp-0.0.80-chromium-1243");
  assert.equal(require("@playwright/mcp/package.json").version, "0.0.80");
});

test("exports exactly the approved nineteen upstream tools", () => {
  assert.equal(ALLOWED_TOOLS.size, 19);
  assert.equal(ALLOWED_TOOLS.has("browser_evaluate"), false);
  assert.equal(ALLOWED_TOOLS.has("browser_console_messages"), false);
  assert.equal(ALLOWED_TOOLS.has("browser_network_requests"), false);
  assert.equal(ALLOWED_TOOLS.has("browser_pdf_save"), false);
});

test("blocks privileged and local URL schemes", () => {
  assert.equal(isAllowedUrl("https://example.com/path"), true);
  assert.equal(isAllowedUrl("http://127.0.0.1:8000"), true);
  assert.equal(isAllowedUrl("about:blank"), true);
  assert.equal(isAllowedUrl("file:///etc/passwd"), false);
  assert.equal(isAllowedUrl("chrome://settings"), false);
  assert.equal(isAllowedUrl("devtools://devtools"), false);
  assert.equal(isAllowedUrl("data:text/html,hello"), false);
});

test("download names cannot traverse out of the profile folder", () => {
  assert.equal(safeFilename("../../secret.txt"), "secret.txt");
  assert.equal(safeFilename("bad<name>.txt"), "bad_name_.txt");
});

test("simultaneous downloads reserve distinct destinations", () => {
  const root = path.resolve("download-fixture");
  const reserved = new Set();
  const first = uniqueDestination("report.txt", root, reserved);
  const second = uniqueDestination("report.txt", root, reserved);
  assert.equal(path.basename(first), "report.txt");
  assert.equal(path.basename(second), "report-1.txt");
});

test("headless preview frames are capped at ten per second", () => {
  assert.equal(previewFrameDue(1_000, 1_099), false);
  assert.equal(previewFrameDue(1_000, 1_100), true);
});

test("explicit output paths cannot escape the configured directory", () => {
  const root = path.resolve("output-fixture");
  assert.throws(() => outputPath("../outside.png", root), /escaped/);
  assert.throws(() => outputPath(path.resolve("outside.png"), root), /relative/);
});
