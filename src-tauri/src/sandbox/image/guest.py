#!/usr/bin/env python3
"""conduit's helper inside a sandbox.

conduit copies this file to /opt/conduit/guest.py before every start, so it
always matches the conduit that drives it rather than the image it was built
into. Each call is one `docker exec`: a subcommand, arguments, and a JSON
result on stdout. Failures print a one-line reason to stderr and exit
non-zero; conduit hands that line to the agent.

Only the standard library plus python3-xlib, python3-pil and python3-pyatspi,
all of which the image installs.
"""

import json
import os
import subprocess
import sys
import time

DISPLAY = os.environ.get("DISPLAY", ":1")


def die(message):
    sys.stderr.write(message.strip() + "\n")
    sys.exit(1)


def emit(value):
    sys.stdout.write(json.dumps(value, ensure_ascii=False))
    sys.stdout.flush()


def detached(argv, stdin=None):
    """Starts a process that outlives this one without holding docker exec's
    streams open — otherwise `docker exec` waits for it forever."""
    return subprocess.Popen(
        argv,
        stdin=stdin if stdin is not None else subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
        close_fds=True,
    )


# ── X11 ─────────────────────────────────────────────────────────────


def x():
    from Xlib import display as xdisplay

    try:
        return xdisplay.Display(DISPLAY)
    except Exception as error:  # noqa: BLE001 — any failure means "no display"
        die(f"the sandbox display is not up yet ({error})")


def atom(d, name):
    return d.intern_atom(name)


def prop(d, window, name):
    try:
        value = window.get_full_property(atom(d, name), 0)
    except Exception:  # noqa: BLE001 — windows vanish mid-query all the time
        return None
    return value.value if value is not None else None


def text_prop(d, window, *names):
    for name in names:
        value = prop(d, window, name)
        if value is None:
            continue
        if isinstance(value, bytes):
            value = value.decode("utf-8", "replace")
        if isinstance(value, str) and value.strip():
            return value.strip("\x00").strip()
    return ""


def wm_class(window):
    try:
        cls = window.get_wm_class()
    except Exception:  # noqa: BLE001
        return ""
    if not cls:
        return ""
    return cls[1] or cls[0] or ""


def frame_extents(d, window):
    value = prop(d, window, "_NET_FRAME_EXTENTS")
    if value is None or len(value) != 4:
        return (0, 0, 0, 0)
    left, right, top, bottom = (int(v) for v in value)
    return (left, right, top, bottom)


def client_windows(d):
    """Managed windows, frontmost first, without panels, the desktop or
    anything minimised."""
    root = d.screen().root
    stacking = prop(d, root, "_NET_CLIENT_LIST_STACKING") or prop(d, root, "_NET_CLIENT_LIST") or []
    skip_types = {atom(d, "_NET_WM_WINDOW_TYPE_DOCK"), atom(d, "_NET_WM_WINDOW_TYPE_DESKTOP")}
    hidden = atom(d, "_NET_WM_STATE_HIDDEN")
    out = []
    for wid in reversed(list(stacking)):
        window = d.create_resource_object("window", int(wid))
        types = set(prop(d, window, "_NET_WM_WINDOW_TYPE") or [])
        if types & skip_types:
            continue
        if hidden in set(prop(d, window, "_NET_WM_STATE") or []):
            continue
        out.append(window)
    return out


def window_info(d, window, layer):
    root = d.screen().root
    try:
        geometry = window.get_geometry()
        origin = window.translate_coords(root, 0, 0)
    except Exception:  # noqa: BLE001
        return None
    left, right, top, bottom = frame_extents(d, window)
    pid = prop(d, window, "_NET_WM_PID")
    return {
        "id": int(window.id),
        "title": text_prop(d, window, "_NET_WM_NAME", "WM_NAME"),
        "app": wm_class(window),
        "pid": int(pid[0]) if pid else 0,
        # translate_coords answers "where is root's origin relative to this
        # window", so the window's own position is its negation.
        "x": float(-origin.x - left),
        "y": float(-origin.y - top),
        "width": float(geometry.width + left + right),
        "height": float(geometry.height + top + bottom),
        "layerIndex": layer,
    }


def cmd_ready(_args):
    d = x()
    screen = d.screen()
    check = prop(d, screen.root, "_NET_SUPPORTING_WM_CHECK")
    if not check:
        die("the desktop is still starting")
    emit({"ready": True, "width": screen.width_in_pixels, "height": screen.height_in_pixels})


def cmd_geometry(args):
    if len(args) != 2:
        die("usage: geometry WIDTH HEIGHT")
    width, height = int(args[0]), int(args[1])
    size = f"{width}x{height}"
    attempts = [
        ["xrandr", "-s", size],
        ["sh", "-c", f"xrandr --newmode {size} 60 {width} {width} {width} {width} "
                     f"{height} {height} {height} {height} 2>/dev/null; "
                     f"xrandr --addmode VNC-0 {size} 2>/dev/null; xrandr --output VNC-0 --mode {size}"],
        ["xrandr", "--fb", size],
    ]
    for argv in attempts:
        subprocess.run(argv, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=10)
        screen = x().screen()
        if (screen.width_in_pixels, screen.height_in_pixels) == (width, height):
            emit({"width": width, "height": height})
            return
    screen = x().screen()
    die(
        f"the screen stayed at {screen.width_in_pixels}x{screen.height_in_pixels}; "
        "the new size applies the next time the sandbox starts"
    )


def cmd_screenshot(args):
    from PIL import Image
    from Xlib import X

    region = None
    scale = 1.0
    it = iter(args)
    for flag in it:
        if flag == "--region":
            region = [int(round(float(v))) for v in next(it).split(",")]
        elif flag == "--scale":
            scale = max(0.1, min(1.0, float(next(it))))
    d = x()
    screen = d.screen()
    sw, sh = screen.width_in_pixels, screen.height_in_pixels
    if region is None:
        rx, ry, rw, rh = 0, 0, sw, sh
    else:
        rx, ry, rw, rh = region
        rx, ry = max(0, rx), max(0, ry)
        rw, rh = min(rw, sw - rx), min(rh, sh - ry)
        if rw <= 0 or rh <= 0:
            die(f"that region is outside the {sw}x{sh} screen")
    raw = screen.root.get_image(rx, ry, rw, rh, X.ZPixmap, 0xFFFFFFFF)
    image = Image.frombytes("RGB", (rw, rh), raw.data, "raw", "BGRX")
    if scale < 1.0:
        size = (max(1, round(rw * scale)), max(1, round(rh * scale)))
        image = image.resize(size, Image.BILINEAR)
    import base64
    import io

    png = io.BytesIO()
    image.save(png, format="PNG", compress_level=6)
    # Base64 because MCP wants it that way anyway: conduit forwards the string
    # as-is instead of decoding and re-encoding a megabyte per screenshot.
    emit({
        "screenWidth": sw,
        "screenHeight": sh,
        "regionWidth": rw,
        "regionHeight": rh,
        "width": image.width,
        "height": image.height,
        "png": base64.b64encode(png.getvalue()).decode("ascii"),
    })


def cmd_windows(_args):
    d = x()
    out = []
    for layer, window in enumerate(client_windows(d)):
        info = window_info(d, window, layer)
        if info is not None:
            out.append(info)
    emit(out)


def active_pid(d):
    root = d.screen().root
    active = prop(d, root, "_NET_ACTIVE_WINDOW")
    if not active or not active[0]:
        return None
    pid = prop(d, d.create_resource_object("window", int(active[0])), "_NET_WM_PID")
    return int(pid[0]) if pid else None


def cmd_apps(_args):
    d = x()
    front = active_pid(d)
    seen = {}
    for window in client_windows(d):
        name = wm_class(window)
        pid = prop(d, window, "_NET_WM_PID")
        pid = int(pid[0]) if pid else 0
        if not name or name in seen:
            continue
        seen[name] = {"name": name, "pid": pid, "bundleId": None, "active": pid == front and pid != 0}
    emit(list(seen.values()))


def cmd_bounds(args):
    if len(args) != 5:
        die("usage: bounds WINDOW_ID X Y WIDTH HEIGHT")
    wid = int(args[0])
    left_x, top_y, width, height = (int(round(float(v))) for v in args[1:])
    d = x()
    window = d.create_resource_object("window", wid)
    left, right, top, bottom = frame_extents(d, window)
    run = lambda argv: subprocess.run(argv, capture_output=True, text=True, timeout=10)  # noqa: E731
    run(["wmctrl", "-i", "-r", hex(wid), "-b", "remove,maximized_vert,maximized_horz"])
    done = run([
        "wmctrl", "-i", "-r", hex(wid), "-e",
        f"0,{left_x},{top_y},{max(1, width - left - right)},{max(1, height - top - bottom)}",
    ])
    if done.returncode != 0:
        die(done.stderr or f"no window {wid}")
    emit({"ok": True})


# ── applications ────────────────────────────────────────────────────


def desktop_entries():
    import configparser

    dirs = [
        os.path.expanduser("~/.local/share/applications"),
        "/usr/local/share/applications",
        "/usr/share/applications",
    ]
    seen = set()
    for base in dirs:
        if not os.path.isdir(base):
            continue
        for name in sorted(os.listdir(base)):
            if not name.endswith(".desktop") or name in seen:
                continue
            seen.add(name)
            parser = configparser.ConfigParser(interpolation=None, strict=False)
            try:
                parser.read(os.path.join(base, name), encoding="utf-8")
                entry = parser["Desktop Entry"]
            except Exception:  # noqa: BLE001 — malformed entries are common
                continue
            if entry.get("NoDisplay", "false").lower() == "true" or entry.get("Type") != "Application":
                continue
            exec_line = entry.get("Exec", "").split()
            yield {
                "id": name[: -len(".desktop")],
                "name": entry.get("Name", ""),
                "generic": entry.get("GenericName", ""),
                "keywords": entry.get("Keywords", ""),
                "exec": os.path.basename(exec_line[0]) if exec_line else "",
                # `mousepad` and `mousepad --preferences` share a binary; the
                # entry without arguments is the app itself.
                "exec_args": len(exec_line) - 1,
                # A terminal program opens inside a terminal window; given the
                # choice, the agent asked for the graphical app.
                "terminal": entry.get("Terminal", "false").lower() == "true",
                "wmclass": entry.get("StartupWMClass", ""),
            }


def entry_score(entry, q):
    """How well a desktop entry answers to `q`; lower is better, None is no
    match. Names beat ids beat descriptions beat binaries, so "Mousepad"
    opens the editor rather than its settings dialog, which runs the same
    binary with a flag."""
    name = entry["name"].lower()
    ident = entry["id"].lower()
    short_id = ident.rsplit(".", 1)[-1]
    generic = entry["generic"].lower()
    exe = entry["exec"].lower()
    if q == name:
        return 0
    if q in (ident, short_id):
        return 1
    if q == generic:
        return 2
    if q == exe:
        return 3
    if name.startswith(q):
        return 4
    if any(k.startswith(q) for k in (generic, short_id, exe) if k):
        return 5
    if any(q in k for k in (name, generic, ident, exe, entry["keywords"].lower()) if k):
        return 6
    return None


def match_entry(query):
    q = query.strip().lower()
    scored = []
    for entry in desktop_entries():
        score = entry_score(entry, q)
        if score is not None:
            scored.append(
                (score, entry["terminal"], entry["exec_args"], len(entry["name"]), entry["id"], entry)
            )
    if not scored:
        return None
    scored.sort(key=lambda item: item[:5])
    return scored[0][5]


def windows_matching(d, names):
    wanted = [n.lower() for n in names if n]
    out = []
    for window in client_windows(d):
        cls = wm_class(window).lower()
        if any(w == cls or w in cls or cls in w for w in wanted if cls):
            out.append(window)
    return out


def cmd_open(args):
    if not args:
        die("usage: open NAME")
    query = " ".join(args)
    entry = match_entry(query)
    d = x()
    names = [query]
    if entry:
        names += [entry["wmclass"], entry["exec"], entry["name"]]
    running = windows_matching(d, names)
    if running:
        subprocess.run(["wmctrl", "-i", "-a", hex(int(running[0].id))], timeout=10)
        emit({"opened": wm_class(running[0]) or query, "how": "focused the running app"})
        return
    if entry:
        detached(["gtk-launch", entry["id"]])
        emit({"opened": entry["name"], "how": "launched"})
        return
    from shutil import which

    binary = which(query.strip().lower())
    if binary:
        detached([binary])
        emit({"opened": query, "how": "launched"})
        return
    die(f"no application called {query!r} is installed in this sandbox")


def cmd_quit(args):
    if not args:
        die("usage: quit NAME")
    query = " ".join(args)
    entry = match_entry(query)
    d = x()
    names = [query]
    if entry:
        names += [entry["wmclass"], entry["exec"], entry["name"]]
    windows = windows_matching(d, names)
    if not windows:
        die(f"{query} is not running")
    for window in windows:
        subprocess.run(["wmctrl", "-i", "-c", hex(int(window.id))], timeout=10)
    emit({"closed": len(windows)})


# ── accessibility ───────────────────────────────────────────────────

MAX_NODES = 5000
MAX_ELEMENTS = 1500
BUDGET_SECONDS = 4.0


def cmd_a11y(args):
    app_name = None
    if len(args) >= 2 and args[0] == "--app":
        app_name = " ".join(args[1:]).strip().lower()
    try:
        import pyatspi
    except Exception as error:  # noqa: BLE001
        die(f"the accessibility bridge is unavailable ({error})")

    desktop = pyatspi.Registry.getDesktop(0)
    apps = [desktop.getChildAtIndex(i) for i in range(desktop.childCount)]
    apps = [a for a in apps if a is not None]

    target = None
    if app_name:
        for app in apps:
            if (app.name or "").lower() == app_name:
                target = app
                break
        if target is None:
            for app in apps:
                if app_name in (app.name or "").lower():
                    target = app
                    break
        if target is None:
            die(f"no running app called {app_name!r} publishes an accessibility tree")
    else:
        pid = active_pid(x())
        for app in apps:
            try:
                if pid and app.get_process_id() == pid:
                    target = app
                    break
            except Exception:  # noqa: BLE001
                continue
    roots = [target] if target is not None else apps

    started = time.monotonic()
    visited = 0
    out = []
    stack = list(reversed(roots))
    while stack and visited < MAX_NODES and len(out) < MAX_ELEMENTS:
        if time.monotonic() - started > BUDGET_SECONDS:
            break
        node = stack.pop()
        visited += 1
        try:
            states = node.getState()
            if node.getRoleName() not in ("application",) and not states.contains(pyatspi.STATE_SHOWING):
                continue
            role = node.getRoleName()
            text = (node.name or "").strip()
            if not text and role in ("text", "entry", "paragraph", "label", "heading", "link"):
                try:
                    t = node.queryText()
                    text = t.getText(0, min(t.characterCount, 200)).strip()
                except Exception:  # noqa: BLE001
                    text = ""
            if text:
                try:
                    ext = node.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
                    if ext.width > 0 and ext.height > 0:
                        out.append({
                            "role": role,
                            "text": text[:200],
                            "x": float(ext.x),
                            "y": float(ext.y),
                            "width": float(ext.width),
                            "height": float(ext.height),
                            "centerX": float(ext.x + ext.width / 2),
                            "centerY": float(ext.y + ext.height / 2),
                        })
                except Exception:  # noqa: BLE001 — not every node is a component
                    pass
            for i in reversed(range(min(node.childCount, 500))):
                child = node.getChildAtIndex(i)
                if child is not None:
                    stack.append(child)
        except Exception:  # noqa: BLE001 — nodes die mid-walk; skip them
            continue
    emit(out)


# ── keyboard ────────────────────────────────────────────────────────


def type_segments(text):
    """Splits text into what xdotool can type in one go (ASCII, which the
    keymap already has) and characters it has to remap a spare keycode for.

    Those go one at a time: xdotool reuses a single scratch keycode, and when
    two such characters arrive back to back a GTK app can read the keymap
    after it was already remapped for the next one — `世界` arrives as `世`."""
    run = []
    for ch in text:
        if 0x20 <= ord(ch) < 0x7F or ch in "\n\t":
            run.append(ch)
            continue
        if run:
            yield "".join(run), False
            run = []
        yield ch, True
    if run:
        yield "".join(run), False


def cmd_type(_args):
    text = sys.stdin.buffer.read().decode("utf-8")
    for segment, remapped in type_segments(text):
        if remapped:
            # Let the previous keystrokes land before the keymap changes.
            time.sleep(0.03)
        done = subprocess.run(
            # xdotool holds a remapped keycode for the length of its delay
            # before restoring it; a longer one gives the app time to read it.
            ["xdotool", "type", "--clearmodifiers", "--delay", "60" if remapped else "12", "--", segment],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            text=True,
        )
        if done.returncode != 0:
            die(done.stderr or "xdotool could not type")
        if remapped:
            time.sleep(0.05)
    emit({"typed": len(text)})


# ── clipboard ───────────────────────────────────────────────────────


def cmd_clip_set(_args):
    data = sys.stdin.buffer.read()
    proc = subprocess.Popen(
        ["xclip", "-selection", "clipboard", "-i"],
        stdin=subprocess.PIPE,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
        close_fds=True,
    )
    proc.stdin.write(data)
    proc.stdin.close()
    # xclip forks a child that owns the selection and exits the parent once
    # it has read everything; waiting for the parent is waiting for "copied".
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        pass
    emit({"ok": True})


COMMANDS = {
    "ready": cmd_ready,
    "geometry": cmd_geometry,
    "screenshot": cmd_screenshot,
    "windows": cmd_windows,
    "apps": cmd_apps,
    "bounds": cmd_bounds,
    "open": cmd_open,
    "quit": cmd_quit,
    "a11y": cmd_a11y,
    "type": cmd_type,
    "clip-set": cmd_clip_set,
}


def main():
    if len(sys.argv) < 2 or sys.argv[1] not in COMMANDS:
        die(f"usage: guest.py {{{'|'.join(COMMANDS)}}} …")
    try:
        COMMANDS[sys.argv[1]](sys.argv[2:])
    except SystemExit:
        raise
    except Exception as error:  # noqa: BLE001 — one line for the agent, not a traceback
        die(f"{sys.argv[1]} failed: {error}")


if __name__ == "__main__":
    main()
