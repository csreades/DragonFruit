#!/bin/bash
# df-input.sh — Inject clicks and keystrokes into the live DragonFruit window.
#
# Intended for an AI agent (Claude) running in dev hot-reload. macOS only.
# Requires cliclick: brew install cliclick.
#
# Usage:
#   scripts/df-input.sh click X Y
#   scripts/df-input.sh dblclick X Y
#   scripts/df-input.sh rclick X Y
#   scripts/df-input.sh move X Y
#   scripts/df-input.sh drag X1 Y1 X2 Y2
#   scripts/df-input.sh key COMBO         (e.g. cmd+s, esc, shift+r, a,b,c)
#   scripts/df-input.sh type TEXT
#   scripts/df-input.sh --help
#
# Exit codes:
#   0  succeeded
#   2  DragonFruit not running
#   5  cliclick not installed
#   6  input validation failed
#   7  cliclick runtime error
#
# Coordinates are logical points; same coordinate space as scripts/df-snap.sh.

set -euo pipefail

PROG="df-input"
log() { printf '[%s] %s\n' "$PROG" "$*" >&2; }

print_help() {
    cat <<'EOF'
df-input — inject clicks and keystrokes into the DragonFruit window

Usage:
  scripts/df-input.sh click X Y                Single left click
  scripts/df-input.sh dblclick X Y             Double left click
  scripts/df-input.sh rclick X Y               Right click (reproduces #55)
  scripts/df-input.sh move X Y                 Move cursor without clicking
  scripts/df-input.sh drag X1 Y1 X2 Y2         Left-button drag
  scripts/df-input.sh key COMBO                Keystroke or chord
  scripts/df-input.sh type TEXT                Type a literal string

Key combo syntax (user-facing, translated to cliclick before invocation):
  cmd+s, shift+esc, ctrl+alt+a, fn+f1
  Sequences: a,b,c (press a, then b, then c)
  Allowed chars: [a-zA-Z0-9+,_-] — translated + → - for cliclick chord syntax.

Type constraints:
  Max 256 bytes; control characters (newlines, CR, tabs, etc.) rejected.

Coordinates are logical points; same coordinate space as df-snap.sh.

Example:
  scripts/df-input.sh click 240 120
  scripts/df-input.sh key cmd+,           # open settings
  scripts/df-input.sh type "hello world"

Exit codes:
  0  succeeded
  2  DragonFruit not running
  5  cliclick not installed
  6  input validation failed
  7  cliclick runtime error
EOF
}

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
    print_help
    exit 0
fi

if [[ "$(uname -s)" != "Darwin" ]]; then
    log "df-input is macOS-only (uname -s = $(uname -s))"
    exit 1
fi

if [[ $# -eq 0 ]]; then
    print_help >&2
    exit 6
fi

require_int() {
    local name="$1" value="$2"
    if [[ ! "$value" =~ ^-?[0-9]+$ ]]; then
        log "$name must be an integer, got: '$value'"
        exit 6
    fi
}

# run_cliclick is called only after subcommand args are fully validated. It
# checks cliclick presence, activates DragonFruit, and runs cliclick — failing
# at the right step with the right exit code.
run_cliclick() {
    if ! command -v cliclick >/dev/null 2>&1; then
        log "cliclick not found. Install with: brew install cliclick"
        exit 5
    fi
    if ! osascript -e 'tell application "DragonFruit" to activate' >/dev/null 2>&1; then
        log "DragonFruit is not running. Start with: npm run tauri:dev"
        exit 2
    fi
    if ! cliclick "$@"; then
        log "cliclick failed. If the app does not respond, grant Accessibility AND Input Monitoring permission to your terminal in System Settings → Privacy & Security, then retry."
        exit 7
    fi
}

SUBCOMMAND="$1"
shift

case "$SUBCOMMAND" in
    click)
        [[ $# -eq 2 ]] || { log "click requires X Y"; exit 6; }
        require_int X "$1"
        require_int Y "$2"
        run_cliclick "c:$1,$2"
        ;;
    dblclick)
        [[ $# -eq 2 ]] || { log "dblclick requires X Y"; exit 6; }
        require_int X "$1"
        require_int Y "$2"
        run_cliclick "dc:$1,$2"
        ;;
    rclick)
        [[ $# -eq 2 ]] || { log "rclick requires X Y"; exit 6; }
        require_int X "$1"
        require_int Y "$2"
        run_cliclick "rc:$1,$2"
        ;;
    move)
        [[ $# -eq 2 ]] || { log "move requires X Y"; exit 6; }
        require_int X "$1"
        require_int Y "$2"
        run_cliclick "m:$1,$2"
        ;;
    drag)
        [[ $# -eq 4 ]] || { log "drag requires X1 Y1 X2 Y2"; exit 6; }
        require_int X1 "$1"
        require_int Y1 "$2"
        require_int X2 "$3"
        require_int Y2 "$4"
        run_cliclick "dd:$1,$2" "du:$3,$4"
        ;;
    key)
        [[ $# -eq 1 ]] || { log "key requires COMBO"; exit 6; }
        COMBO="$1"
        if [[ ! "$COMBO" =~ ^[a-zA-Z0-9+,_-]+$ ]]; then
            log "key combo must match [a-zA-Z0-9+,_-]+, got: '$COMBO'"
            exit 6
        fi
        # Translate user-facing chord syntax (cmd+s) to cliclick's (cmd-s).
        # Sequences (a,b,c) stay comma-separated.
        CC_COMBO="${COMBO//+/-}"
        run_cliclick "kp:$CC_COMBO"
        ;;
    type)
        [[ $# -eq 1 ]] || { log "type requires TEXT"; exit 6; }
        TEXT="$1"
        if [[ ${#TEXT} -gt 256 ]]; then
            log "type text exceeds 256 bytes (got ${#TEXT})"
            exit 6
        fi
        if [[ "$TEXT" =~ [[:cntrl:]] ]]; then
            log "type text contains control characters; only printable input is allowed"
            exit 6
        fi
        # TEXT is a single argv element to cliclick — bash double-quoting around
        # "$TEXT" preserves its literal contents without further expansion.
        run_cliclick "t:$TEXT"
        ;;
    *)
        log "unknown subcommand: '$SUBCOMMAND'. See --help."
        exit 6
        ;;
esac
