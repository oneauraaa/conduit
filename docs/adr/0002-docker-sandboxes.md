# ADR 0002: Docker sandboxes

- Status: accepted
- Date: 2026-09-25

## Context

Every Conduit tool acts on the machine it runs on. An agent working in the
background takes the user's pointer, keyboard, focus and screen: a game loses
input, a video call loses its window. Users want to hand an agent a computer
that is not theirs, pick its operating system and resources, keep several of
them, and watch what the agent does there.

## Decision

A sandbox is a Linux desktop in a Docker container, driven through the
`docker` CLI. The CLI rather than the Engine API, because it already resolves
Docker Desktop, the native engine, OrbStack, Colima and Rancher Desktop through
contexts, and brings BuildKit, credential helpers and proxy settings. Conduit
finds it by absolute path first, since apps launched from Finder or Explorer
do not inherit the shell's PATH, and puts its directory on every spawn's PATH
so credential helpers resolve.

**Image.** One apt-based Dockerfile covers Ubuntu 24.04, Ubuntu 22.04 and
Debian 12. It is embedded in the binary and built locally on first use,
streamed to `docker build -` as an in-memory tar; the tag is a hash of the
Dockerfile, entrypoint and base, so a Conduit update never mistakes an old
image for its own. Base images are referenced by tag, not digest: the build
runs `apt-get upgrade`, so a digest would only freeze security fixes. The
desktop is XFCE on TigerVNC's `Xtigervnc` (an X server with VNC built in,
listening on the container's loopback only). Firefox comes from Mozilla's APT
repository on Ubuntu, with the signing key's fingerprint checked, because
Ubuntu's own package installs a snap. Agents run as uid 1000 `conduit` with
passwordless sudo; `/home/conduit` is a named volume.

**Endpoints.** Each sandbox is served at `/sandbox/<id>/mcp` on the existing
listener, by its own `StreamableHttpService` and its own tool router
(`mcp/sandbox_tools.rs`): the 24 desktop tools minus `list_keybinds`, with the
same names and argument shapes, implemented as argv-only `docker exec` calls
(xdotool, wmctrl, xclip, and a Python helper, `guest.py`, for screenshots,
windows, apps, typing and the AT-SPI tree). The helper is copied in before
every start, so it always matches the Conduit driving it. One service per
sandbox, rather than one reading the id from the path, because rmcp keys
sessions by header alone and a session could otherwise be replayed across
sandboxes. The host's `/mcp` handler is untouched. Agent configs get
`conduit-<id>` beside `conduit`, so an agent can hold both and choose per task.

**Gate.** `gate::run_sandbox` honours panic stop and the Tools tab, adds a
per-sandbox "stop agent" latch, and never raises the host control session: no
glow, pill, AI cursor or approval card appears on the user's screen, which is
the point. Manual/Auto approvals do not apply inside a sandbox. Cancelling a
call — stop agent, panic stop, or the client going away — also kills its shell
command inside the container, found by a marker in its command line.

**Isolation.** The container is created without privileges, published ports,
bind mounts, extra capabilities or the Docker socket, on a dedicated bridge
network with inter-container traffic off, or with no network when internet is
off. Conduit's endpoint is unauthenticated and Docker Desktop forwards
`host.docker.internal` to the host's loopback, so after every start (and
every network change) a throwaway helper sharing the sandbox's network
namespace, holding `NET_ADMIN` the sandbox lacks, rejects traffic to the
gateway and to Docker Desktop's host addresses. A start that cannot be
isolated is stopped. A container found running that Conduit did not start this
run — after a relaunch, or started by hand — is re-isolated before it is
treated as running. Conduit acts only on containers and volumes carrying its
install label.

**Live view.** noVNC in the Sandbox tab, fed by `docker exec -i … socat -
TCP:127.0.0.1:5901` whose stdio is relayed over Tauri IPC: an ordered
`Channel` inbound, serialized raw-body invokes outbound. Nothing is published
on the host, no VNC password exists, the CSP is unchanged, and the view works
with the network off. It streams only while the tab is open.

**State.** Sandbox specs live in their own `sandbox/state.json`, not in
`Settings`: `store::load` resets all settings on any parse error, and one bad
sandbox entry must not reset the user's port, token and tool switches.

## Consequences

Sandboxes need Docker installed and running; the tab says what is missing and
polls until it appears. The first sandbox of each OS spends a few minutes and
about 1.6 GB building its image, and on-demand starts never trigger a build.
Tool calls pay one `docker exec` each (about 100–300 ms); a persistent in-guest
agent can replace `sandbox::guest` later without touching the tools. Changing
the image leaves existing sandboxes on their old image until recreated. Podman
is reported as unsupported for now. On a native Linux engine the container is
a namespace boundary, not a VM; Docker Desktop's VM is the stronger boundary on
macOS and Windows, and the handshake never claims more isolation than exists.

The spike and end-to-end run that settled the details found three things the
design now depends on: `xdotool type` drops the second of two consecutive
characters absent from the keymap in GTK apps, so `guest.py` types such
characters one at a time with a pause; WebKitGTK can settle `import()` of a
module with top-level await (noVNC has one) before it has evaluated, so the
view waits for the export; and a synchronous Tauri command runs on the main
thread, which has no Tokio reactor for `tokio::process`, so the viewer command
is async. The isolation check — a container on the same network reaches a
host service, the sandbox cannot, and root in the sandbox cannot remove the
rule — is a release gate, and should be re-run on Docker Desktop, where
`host.docker.internal` is the path that matters.
