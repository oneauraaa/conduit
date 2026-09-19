#!/usr/bin/env node
"use strict";

const fs = require("node:fs");
const path = require("node:path");
const { Client } = require("@modelcontextprotocol/sdk/client/index.js");
const { InMemoryTransport } = require("@modelcontextprotocol/sdk/inMemory.js");
const { createConnection } = require("@playwright/mcp");
const { ALLOWED_TOOLS } = require("./sidecar.cjs");

async function main() {
  const server = await createConnection({ capabilities: ["core"], codegen: "none" });
  const pair = InMemoryTransport.createLinkedPair();
  const client = new Client({ name: "conduit-schema-generator", version: "1.0.0" }, { capabilities: {} });
  await server.connect(pair[0]);
  await client.connect(pair[1]);
  const listed = await client.listTools();
  const selected = listed.tools.filter((tool) => ALLOWED_TOOLS.has(tool.name));
  if (selected.length !== ALLOWED_TOOLS.size) {
    throw new Error("the pinned Playwright tool catalog no longer matches Conduit's allowlist");
  }
  selected.sort((left, right) => left.name.localeCompare(right.name));
  const destination = process.argv[2]
    ? path.resolve(process.argv[2])
    : path.join(__dirname, "pinned-tools.json");
  fs.writeFileSync(destination, JSON.stringify(selected, null, 2) + "\n");
  await client.close();
  await server.close();
  process.stdout.write("wrote " + selected.length + " pinned browser schemas to " + destination + "\n");
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
