#!/bin/bash
# df-snap.sh — Capture the live DragonFruit main window to a PNG.
#
# Intended for an AI agent (Claude) running in dev hot-reload to verify UI
# changes by reading back the rendered window. macOS only.
#
# Usage:
#   scripts/df-snap.sh [output_path]
#   scripts/df-snap.sh --list-windows
#   scripts/df-snap.sh --help
#
# Env:
#   DF_SNAP_PATH       Default output path (overridden by positional arg).
#                      Default: /tmp/df-window.png
#   DF_SNAP_TIMEOUT    Seconds to wait for the main window to appear.
#                      Default: 5
#
# Exit codes:
#   0  capture succeeded
#   1  not running on macOS
#   2  DragonFruit process not running
#   3  main window not found within DF_SNAP_TIMEOUT
#   4  screencapture failed

set -euo pipefail

PROG="df-snap"
log() { printf '[%s] %s\n' "$PROG" "$*" >&2; }

print_help() {
    cat <<'EOF'
df-snap — capture the live DragonFruit main window

Usage:
  scripts/df-snap.sh [output_path]
  scripts/df-snap.sh --list-windows
  scripts/df-snap.sh --help

The captured PNG is written with mode 0600 to avoid leaking on multi-user boxes.
On stdout, only the absolute path of the captured PNG is printed; diagnostics
go to stderr.

Env vars:
  DF_SNAP_PATH      Default output path (default: /tmp/df-window.png)
  DF_SNAP_TIMEOUT   Seconds to wait for main window (default: 5)

Exit codes:
  0  capture succeeded
  1  not running on macOS
  2  DragonFruit process not running
  3  main window not found within DF_SNAP_TIMEOUT
  4  screencapture failed

Example:
  path=$(scripts/df-snap.sh)
  open "$path"
EOF
}

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
    print_help
    exit 0
fi

if [[ "$(uname -s)" != "Darwin" ]]; then
    log "df-snap is macOS-only (uname -s = $(uname -s))"
    exit 1
fi

OUTPUT_PATH="${1:-${DF_SNAP_PATH:-/tmp/df-window.png}}"
TIMEOUT="${DF_SNAP_TIMEOUT:-5}"

# AppleScript: find a window of process "DragonFruit" whose title contains
# "DragonFruit" (excludes future detached tooling) and whose width > 800
# (excludes the 360x280 splashscreen). Returns "x,y,w,h" or empty string.
read_main_window_geometry() {
    osascript <<'APPLESCRIPT' 2>/dev/null || true
tell application "System Events"
    if not (exists (first process whose name is "DragonFruit")) then
        return ""
    end if
    tell (first process whose name is "DragonFruit")
        repeat with w in windows
            try
                set s to size of w
                set t to title of w
                if (item 1 of s) > 800 and t contains "DragonFruit" then
                    set p to position of w
                    return ((item 1 of p as integer) as text) & "," & ((item 2 of p as integer) as text) & "," & ((item 1 of s as integer) as text) & "," & ((item 2 of s as integer) as text)
                end if
            end try
        end repeat
    end tell
    return ""
end tell
APPLESCRIPT
}

list_windows() {
    osascript <<'APPLESCRIPT' 2>/dev/null || true
tell application "System Events"
    if not (exists (first process whose name is "DragonFruit")) then
        return "(DragonFruit not running)"
    end if
    set out to ""
    tell (first process whose name is "DragonFruit")
        repeat with w in windows
            try
                set p to position of w
                set s to size of w
                set t to title of w
                set out to out & "title='" & t & "' pos=" & (item 1 of p as integer) & "," & (item 2 of p as integer) & " size=" & (item 1 of s as integer) & "x" & (item 2 of s as integer) & linefeed
            end try
        end repeat
    end tell
    return out
end tell
APPLESCRIPT
}

if [[ "${1:-}" == "--list-windows" ]]; then
    list_windows
    exit 0
fi

# Confirm DragonFruit process exists at all (so we can return exit 2 distinctly
# from "process up but main window not yet spawned").
if ! pgrep -x DragonFruit >/dev/null 2>&1; then
    log "DragonFruit is not running. Start with: npm run tauri:dev"
    exit 2
fi

# Poll for the main window up to TIMEOUT seconds.
GEOMETRY=""
for ((i = 0; i < TIMEOUT; i++)); do
    GEOMETRY="$(read_main_window_geometry || true)"
    if [[ -n "$GEOMETRY" ]]; then
        break
    fi
    sleep 1
done

if [[ -z "$GEOMETRY" ]]; then
    log "main window not found within ${TIMEOUT}s. If cold-starting, retry with DF_SNAP_TIMEOUT=30."
    log "If osascript itself is failing, grant Screen Recording permission to your terminal in System Settings → Privacy & Security → Screen Recording, then retry."
    exit 3
fi

# Capture with mode 0600 by setting umask just for the screencapture call.
# screencapture -x suppresses the camera-shutter sound.
# screencapture -R uses logical (point) coordinates; the geometry returned by
# System Events is in the same coordinate space.
if ! ( umask 077 && screencapture -x -R "$GEOMETRY" "$OUTPUT_PATH" ); then
    log "screencapture failed for geometry='$GEOMETRY' path='$OUTPUT_PATH'"
    exit 4
fi

if [[ ! -s "$OUTPUT_PATH" ]]; then
    log "screencapture produced no output at $OUTPUT_PATH"
    exit 4
fi

# Print only the absolute path on stdout so callers can `path=$(df-snap.sh)`.
ABS_PATH="$(cd "$(dirname "$OUTPUT_PATH")" && pwd)/$(basename "$OUTPUT_PATH")"
printf '%s\n' "$ABS_PATH"
