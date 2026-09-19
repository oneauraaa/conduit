#!/usr/bin/env node
"use strict";

const fs = require("node:fs");
const path = require("node:path");

const runtimeRoot = __dirname;
const relayCatalog = path.resolve(runtimeRoot, "../relay-v1.0/conduit_tools.json");
const pinned = JSON.parse(fs.readFileSync(path.join(runtimeRoot, "pinned-tools.json"), "utf8"));
const existing = JSON.parse(fs.readFileSync(relayCatalog, "utf8"));

const history = {
  name: "browser_history",
  description: "Search the selected Conduit browser profile's full navigation history, newest first.",
  inputSchema: {
    type: "object",
    properties: {
      query: { type: "string", description: "Optional case-insensitive text to match against the URL or page title." },
      before: { type: "string", description: "Opaque continuation cursor returned by an earlier browser_history call." },
      limit: { type: "integer", minimum: 1, maximum: 200, default: 50, description: "Maximum entries to return." },
    },
    additionalProperties: false,
  },
};

const browserTools = [...pinned, history]
  .sort((a, b) => a.name.localeCompare(b.name))
  .map((tool) => ({
    name: tool.name,
    group: "browser",
    risky: tool.name === "browser_close",
    description: tool.description,
    parameters: tool.inputSchema,
  }));

const desktopTools = existing.filter((tool) => !tool.name.startsWith("browser_"));
if (desktopTools.length !== 24 || browserTools.length !== 20) {
  throw new Error(`expected 24 desktop and 20 browser tools, got ${desktopTools.length} and ${browserTools.length}`);
}
fs.writeFileSync(relayCatalog, JSON.stringify([...desktopTools, ...browserTools], null, 2) + "\n");
