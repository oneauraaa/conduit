# conduit

Lets AI agents use your Mac — the way a person does. Looking at the screen,
moving a cursor, clicking, typing.

conduit **is** the MCP server. It exposes computer-use tools on localhost, so
any agent (Claude Code, Codex, Hermes, OpenClaw…) can drive the machine — but
only while conduit is running. Quit it from the tray and the endpoint dies with
it. That's the security model, and it's why the window's `x` hides to the tray
instead of quitting: the app has to stay resident to be useful, without
pretending it isn't.

While an agent is driving, the machine shows it: an aqua glow breathes around
the screen edges, the AI's cursor is a glowing orb with a comet trail, and a
control pill floats above the Dock with a live action readout, a mode switch and
a stop button.

macOS only (14+, developed on Tahoe 26).

---

## Running it

```bash
pnpm install
pnpm tauri dev        # development
pnpm tauri build      # produces src-tauri/target/release/bundle/macos/conduit.app
```

Screen Recording permission is only granted to a real `.app` bundle on Tahoe, so
capture must be tested through the bundle — `cargo run` alone will never appear
in System Settings.

```bash
pnpm icons            # regenerate icons from assets/*.svg (needs Chrome)
cargo test --lib      # config-writer tests, from src-tauri/
```

The UI also runs standalone in a browser against sample data, which is much
faster than a Tauri rebuild when iterating on design:

```bash
pnpm dev
# http://localhost:1420/?tab=tools&theme=dark
# http://localhost:1420/pill.html?demo=approval
# http://localhost:1420/overlay.html?demo=1
```

## Connecting an agent

The Agents tab installs the endpoint with one click, backing up the existing
config first. Manually, it's:

```
http://127.0.0.1:6767/mcp
```

```bash
claude mcp add --transport http conduit http://127.0.0.1:6767/mcp --scope user
```

## Sharing it with a web agent

The Tailscale tab publishes the endpoint at a public HTTPS address, for agents
that live on the web rather than on your Mac — Gemini Spark, for instance, which
only takes a URL.

It needs [Tailscale](https://tailscale.com/download/mac) installed and signed in.
Flip **share on the internet** and conduit runs `tailscale funnel` for you.

The public URL carries a 128-bit secret in its path:

```
https://<your-machine>.<tailnet>.ts.net/<32-hex-key>/mcp
```

**That key is the entire lock.** Anyone holding the full URL can use every tool
you left enabled. "new key" rotates it and instantly kills the old address.

Sharing runs on its own listener (port 6768) rather than exposing 6767. Funnel
proxies from 127.0.0.1, so a funnelled request is indistinguishable from a local
one — there is no way to keep an unauthenticated `/mcp` open to local agents
while denying the public. The second listener has exactly one route, behind the
secret, and disappears when sharing is off.

## Permissions

Two macOS grants, both prompted from the Server tab:

- **Accessibility** — synthesizing input and reading the screen's text.
- **Screen Recording** — screenshots.

## Safety model

Every tool call funnels through one gate (`src-tauri/src/mcp/gate.rs`):

1. panic stop in effect → refuse
2. Tools Access off → refuse
3. that tool switched off → refuse
4. mode says ask → prompt the pill, await the answer (60s → deny)
5. run it, log it

**Modes**, switchable live from the pill:

| mode | behaviour |
|---|---|
| manual | every action waits for approval |
| auto | reads run; `run_shell`, `clipboard_write`, `quit_app` ask |
| full access *(default)* | nothing asks |

Full access is the default because enabling a tool in the Tools tab *is* the
grant — asking again at call time was friction on top of a decision the user had
already made. Tighten a live session from the pill whenever you want.

Three things are deliberately out of an agent's reach:

- **Tools Access** is editable only in the Tools tab. The pill can loosen the
  *mode* for a session, never the tool allowlist.
- **conduit's own windows** reject synthetic clicks. Otherwise an agent could
  screenshot the Tools tab and click its own permissions up.
- **Hold Escape for 800ms** drops control from anywhere. It's a listen-only
  event tap, so Escape still works normally everywhere else.

## Layout

```
src/                     React 19 · Tailwind v4 · Motion · Lucide
  App.tsx                sidebar + tabs
  tabs/                  Server · Tools · Agents · Tailscale
  overlay/               edge glow + AI cursor   (click-through window)
  pill/                  control pill            (interactive window)
  lib/standalone.ts      sample data for browser-only runs

src-tauri/src/
  mcp/gate.rs            the one permission choke point
  mcp/tools.rs           the 22 MCP tools
  mcp/catalog.rs         tool list — the source of truth the UI renders from
  mac/                   AppKit · Quartz · ScreenCaptureKit · AX
  agents.rs              config writers (backup + atomic rename)
  tailscale.rs           funnel control + the sharing key
  mac/appicon.rs         reads agent logos from installed apps
  windows.rs             overlay/pill creation, native NSWindow setup
```

## Notes for the next person

- **ScreenCaptureKit is mandatory.** `CGDisplayCreateImage` still compiles on
  Tahoe but returns desktop wallpaper with no app windows — a silent wrong
  answer, which is worse than an error.
- **The Swift rpath in `build.rs` is load-bearing.** screencapturekit bridges
  through Swift; without `-rpath /usr/lib/swift` the app builds and then dies at
  launch on `@rpath/libswift_Concurrency.dylib`.
- **Capabilities must list every window.** `capabilities/default.json` names
  `main`, `pill` and `overlay-*`. Miss one and its `event.listen` is silently
  rejected — the window renders but never updates.
- **Don't hold a `CFRetained` across an `await`.** It isn't `Send`, and it makes
  the whole enclosing tool future non-`Send`. See the scoped blocks in
  `mac/input.rs`.
- **`macOSPrivateApi` and the `macos-private-api` cargo feature must agree**, or
  transparent windows render solid black.
- **An SVG's declared width/height must match the size `build-icons.mjs` renders
  at.** Chrome draws an SVG at its own declared size and leaves the rest of the
  viewport blank, so a mismatch yields a tiny glyph in the corner — which is
  exactly how the tray icon broke once.
- **Agent logos are never bundled.** `mac/appicon.rs` asks macOS for the icon of
  the vendor app already installed, so conduit redistributes no third-party
  artwork and the marks are always current. No app installed → tinted monogram.
