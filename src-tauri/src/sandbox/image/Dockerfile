# The desktop every conduit sandbox runs.
#
# Built on the user's machine by conduit (see sandbox/image.rs), never pulled:
# the context is this file plus entrypoint.sh, streamed to `docker build -`.
# The tag is a hash of both, so editing either one produces a new image and
# existing sandboxes keep the one they were created with until recreated.
#
# One file for every apt-based guest. BASE picks the distribution and FIREFOX
# picks where the browser comes from, because Ubuntu's own `firefox` package is
# a stub that installs a snap, and snapd cannot run inside a container.

ARG BASE=ubuntu:24.04
FROM ${BASE}

ARG FIREFOX=mozilla

ENV DEBIAN_FRONTEND=noninteractive \
    LANG=C.UTF-8 \
    LC_ALL=C.UTF-8 \
    TZ=Etc/UTC

# The desktop, and the tools conduit drives it with:
#   Xtigervnc      the X server, with a VNC server built in (the live view)
#   xdotool        pointer and keyboard
#   wmctrl         window focus, move and close
#   xclip          the clipboard
#   python3-*      guest.py: screenshots, window lists, the accessibility tree
#   socat          bridges the live view's VNC stream through `docker exec`
#   iptables/iproute2  used only by the short-lived isolation helper
#
# No screensaver, locker or power manager: nothing should blank or lock a
# screen that no human is looking at.
RUN apt-get update \
 && apt-get upgrade -y \
 && apt-get install -y --no-install-recommends \
      ca-certificates curl gnupg sudo locales tzdata \
      dbus dbus-x11 at-spi2-core \
      tigervnc-standalone-server x11-xserver-utils x11-utils xauth \
      xfce4-session xfwm4 xfce4-panel xfdesktop4 xfce4-settings xfconf \
      xfce4-terminal xfce4-notifyd thunar mousepad libgtk-3-bin \
      adwaita-icon-theme \
      xdotool wmctrl xclip libnotify-bin desktop-file-utils \
      python3 python3-gi gir1.2-atspi-2.0 python3-pyatspi python3-xlib python3-pil \
      socat iptables iproute2 procps less nano git unzip \
      fonts-dejavu-core fonts-liberation fonts-noto-core fonts-noto-color-emoji fonts-noto-cjk \
 && rm -rf /var/lib/apt/lists/*

# Firefox. From Mozilla's own APT repository on Ubuntu, with the signing key's
# fingerprint checked before anything is trusted; Debian's ESR everywhere else.
RUN set -eu; \
    if [ "$FIREFOX" = "mozilla" ]; then \
      install -d -m 0755 /etc/apt/keyrings; \
      curl -fsSL https://packages.mozilla.org/apt/repo-signing-key.gpg \
        -o /etc/apt/keyrings/packages.mozilla.org.asc; \
      fingerprint="$(gpg --show-keys --with-colons /etc/apt/keyrings/packages.mozilla.org.asc \
        | awk -F: '/^fpr/ { print $10; exit }')"; \
      if [ "$fingerprint" != "35BAA0B33E9EB396F59CA838C0BA5CE6DC6315A3" ]; then \
        echo "Mozilla's APT key has an unexpected fingerprint: $fingerprint" >&2; exit 1; \
      fi; \
      echo "deb [signed-by=/etc/apt/keyrings/packages.mozilla.org.asc] https://packages.mozilla.org/apt mozilla main" \
        > /etc/apt/sources.list.d/mozilla.list; \
      printf 'Package: *\nPin: origin packages.mozilla.org\nPin-Priority: 1000\n' \
        > /etc/apt/preferences.d/mozilla; \
      apt-get update; \
      apt-get install -y --no-install-recommends firefox; \
    else \
      apt-get update; \
      apt-get install -y --no-install-recommends firefox-esr; \
    fi; \
    rm -rf /var/lib/apt/lists/*

# The account every agent command runs as. uid 1000 is taken by `ubuntu` on
# recent Ubuntu images, so that account goes first. Passwordless sudo, because
# an agent asked to install something should be able to — the container, not
# the account, is the boundary.
RUN (userdel -r ubuntu 2>/dev/null || true) \
 && useradd --create-home --uid 1000 --shell /bin/bash conduit \
 && echo 'conduit ALL=(ALL) NOPASSWD:ALL' > /etc/sudoers.d/conduit \
 && chmod 0440 /etc/sudoers.d/conduit \
 && install -d -m 0700 -o conduit -g conduit /run/user/1000 \
 && install -d -m 1777 /tmp/.X11-unix \
 && install -d -m 0755 /opt/conduit \
 && update-alternatives --set x-www-browser "$(command -v firefox || command -v firefox-esr)" 2>/dev/null || true

# Every `docker exec` inherits these, which is how xdotool, xclip and pyatspi
# find the display and the session bus without conduit passing anything.
ENV DISPLAY=:1 \
    XDG_RUNTIME_DIR=/run/user/1000 \
    DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus \
    XDG_SESSION_TYPE=x11 \
    XDG_CURRENT_DESKTOP=XFCE \
    NO_AT_BRIDGE=0 \
    GTK_MODULES=gail:atk-bridge \
    ACCESSIBILITY_ENABLED=1 \
    MOZ_ENABLE_WAYLAND=0 \
    HOME=/home/conduit \
    USER=conduit

# Plain COPY rather than --chmod, which the legacy builder rejects; conduit
# packs the file as 0755 and COPY keeps the mode.
COPY entrypoint.sh /usr/local/bin/conduit-entrypoint

USER conduit
WORKDIR /home/conduit
ENTRYPOINT ["/usr/local/bin/conduit-entrypoint"]
