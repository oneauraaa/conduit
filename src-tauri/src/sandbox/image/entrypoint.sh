#!/bin/sh
# Brings up a sandbox's display and desktop, then lives exactly as long as the
# X server does. tini (`docker run --init`) is PID 1 and forwards `docker stop`.
#
# The X server is TigerVNC's Xtigervnc: a real X display with a VNC server
# built in, listening on the container's own loopback only. Nothing publishes
# that port — conduit's live view reaches it through `docker exec … socat`.
set -u

geometry="1280x800"
if [ -r /opt/conduit/geometry ]; then
  wanted="$(tr -d '[:space:]' < /opt/conduit/geometry)"
  case "$wanted" in
    [0-9]*x[0-9]*) geometry="$wanted" ;;
  esac
fi

mkdir -p "$XDG_RUNTIME_DIR" 2>/dev/null || true
chmod 700 "$XDG_RUNTIME_DIR" 2>/dev/null || true
# A stopped container keeps its filesystem, including the sockets and lock
# files of the X server and bus it was stopped under.
rm -f /tmp/.X1-lock /tmp/.X11-unix/X1 "$XDG_RUNTIME_DIR/bus"

dbus-daemon --session --address="$DBUS_SESSION_BUS_ADDRESS" --fork --nopidfile --nosyslog

Xtigervnc :1 \
  -localhost -rfbport 5901 -SecurityTypes None -AlwaysShared \
  -geometry "$geometry" -depth 24 -nolisten tcp -ac -desktop conduit &
xvnc=$!

trap 'kill "$xvnc" 2>/dev/null' TERM INT

tries=0
until xdpyinfo >/dev/null 2>&1; do
  tries=$((tries + 1))
  if [ "$tries" -gt 150 ] || ! kill -0 "$xvnc" 2>/dev/null; then
    echo "conduit: the X server did not come up" >&2
    exit 1
  fi
  sleep 0.1
done

# Nothing should blank or lock a screen no human is watching.
xset s off -dpms s noblank 2>/dev/null || true

# XFCE's session is restarted whenever it ends — a logout from the menu, a
# crash — so an agent never finds a bare grey screen with no window manager.
(
  while kill -0 "$xvnc" 2>/dev/null; do
    xfce4-session >/tmp/xfce4-session.log 2>&1
    sleep 1
  done
) &

wait "$xvnc"
