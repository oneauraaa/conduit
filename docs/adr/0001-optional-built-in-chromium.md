# ADR 0001: Optional built-in Chromium

- Status: accepted
- Date: 2026-09-09

## Context

Conduit needs browser automation without making Chromium or Node part of the
base application, exposing a debugging port, or letting webpage content weaken
the existing tool and Panic Stop gates. Users also need a visible-browser mode,
a headless preview, persistent profiles, and explicit policy at the four places
where browser activity crosses an important data boundary.

Desktop screen capture is a separate concern, but its former continuously
consumed PipeWire stream caused avoidable GPU/compositor work between agent
calls on Linux.

## Decision

Conduit downloads a platform-specific release bundle only from the Browser tab.
The manifest and archive URLs are tied to the same immutable GitHub release as
the Conduit build; the manifest pins the expected revision, archive size,
SHA-256, target, and executable paths. Installation resumes partial transfers,
extracts into an owned staging directory, rejects traversal, links, and
oversized archives, runs a headless self-test, and atomically promotes the
verified runtime while retaining the previous version.

The bundle contains Chromium and a self-contained Node sidecar built from exact
pins of `@playwright/mcp@0.0.80` and `@yao-pkg/pkg@6.22.0`. Rust communicates
with it using JSON lines over private stdin/stdout. No localhost CDP or remote
debugging port is opened. Both processes enforce the 19-tool upstream
allowlist; Conduit adds `browser_history` as the twentieth tool.

The Browser tab owns four persisted global policies: open websites, read
history, download files, and upload files. `alwaysAllow` and `alwaysAsk`
override Manual/Auto/Full only for their category. Tool switches, Panic Stop,
runtime availability, URL restrictions, output containment, and sandboxing
remain higher-priority hard gates. Session approvals are keyed by category and
expire on idle, ownership change, disconnect, or Panic Stop.

Browser tools are dynamically advertised only while the expected runtime is
ready. Chromium launches lazily, stays warm, and has one MCP owner until the
existing 12-second control session expires. Manual Stop latches browser restart
until the user presses Start. Crashes fail the active request and the next call
starts a fresh process.

Headless preview uses an in-process CDP screencast capped at 10 FPS and runs
only while the Browser tab is visible. Visible mode opens Chromium's own
window. Page content, snapshots, dialogs, and download names are always treated
as untrusted data.

Desktop screenshots are now one-shot on every platform. On Linux the portal
session may stay authorized, but PipeWire is connected only during an agent's
`screenshot` tool call and is closed after the first frame.

## Consequences

The base Conduit package remains small, while release automation must publish
four browser ZIPs and their manifests. A required revision mismatch temporarily
hides browser controls and tools until the update verifies. Browser behavior is
split across Rust policy/lifecycle code and a pinned Node runtime, so schema
generation and packaged cross-platform smoke tests are release gates.
