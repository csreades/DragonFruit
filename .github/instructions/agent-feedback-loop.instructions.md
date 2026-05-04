---
description: How to verify DragonFruit UI changes by capturing the live dev window and injecting input.
---

# Agent Feedback Loop

A pair of macOS shell scripts that let an AI agent (e.g. Claude in dev hot-reload) verify UI changes on the running DragonFruit window without asking a human to take screenshots.

- `scripts/df-snap.sh` — captures the DragonFruit main window to a PNG
- `scripts/df-input.sh` — injects clicks, drags, and keystrokes

## When to use

UI changes where the visual outcome is the source of truth and the agent needs to see what was rendered. Examples from the open backlog:

- Pure-visual: tick marks on rotation gizmo (#103), snap intervals (#104), overlapping control panes (#41), support menu raise-model (#118), distorted supports on lys import (#60), multi-object lys import (#56), island detection visualization (#7).
- Interaction-driven: right-click context-menu bug (#55), mirror tool (#72), support selection tools (#14), recalculate supports on grid (#62).

## When NOT to use

- Not a substitute for unit or integration tests for logic changes. The screenshot is opaque to behavioural correctness.
- Not on shared multi-user machines or CI runners. The permission grant required (see Threat surface) is too coarse.
- Not for verifying production behaviour. The dev URL only — `npm run tauri:dev`.

## Threat surface

Granting Accessibility, Screen Recording, and Input Monitoring permissions to your terminal lets ANY process running in that terminal capture screen content and inject keystrokes or clicks, not just these scripts. A compromised shell rcfile or a malicious npm `postinstall` could weaponise these permissions silently.

Recommended hygiene:

- Grant the permissions to a dedicated terminal app or profile used only for agent work (e.g. a separate iTerm profile), not your daily-driver terminal.
- If you grant them to your daily-driver terminal and stop needing the loop, you can revoke later, but in practice many users leave them enabled and accept the standing risk.
- Never install these scripts into a shared environment, CI runner, or another user's account.
- Audit which processes run in your agent-permitted terminal. Keep the surface small.

## Setup once

1. `brew install cliclick`
2. Open System Settings → Privacy & Security and grant your terminal:
   - Accessibility
   - Screen Recording
   - Input Monitoring
3. The first call will trigger the OS prompts. Approve, then re-run.

## Workflow

In one terminal:

```
npm run tauri:dev
```

Wait for the main window to appear (the splashscreen closes automatically).

In another terminal (the agent terminal):

```
path=$(scripts/df-snap.sh)
# read $path with the agent's image-reading capability
```

To drive the UI:

```
scripts/df-input.sh click 240 120
scripts/df-input.sh key cmd+,
scripts/df-input.sh type "search term"
```

Re-run `df-snap.sh` after each input op to verify the outcome.

## Coordinate frame

Both scripts use logical points (the same coordinate space macOS exposes through System Events and Quartz). On Retina displays this matches `screencapture -R` output.

On multi-monitor setups, coordinates are global desktop-space — the values you read off a captured PNG can be passed directly back to `df-input.sh`.

## Limitations

- macOS only.
- Single DragonFruit instance assumed; the scripts target the first window of the `DragonFruit` process whose title contains "DragonFruit" and whose width is greater than 800 (excludes the splashscreen).
- `df-input.sh` activates the DragonFruit window before each op, which steals focus. Acceptable for an agent loop, awkward if a human is using another window in parallel.
- Not a stand-in for production parity testing.

## Examples

### 1. Verify rotation-gizmo tick marks (#103)

```
# Snap before
scripts/df-snap.sh /tmp/before.png

# Open a model and trigger rotate mode (cmd+R or whatever the binding is)
scripts/df-input.sh key cmd+r

# Snap after — read the PNG and confirm the tick marks render
scripts/df-snap.sh /tmp/after.png
```

### 2. Reproduce right-click bug (#55)

```
# Snap to find a coordinate over the 3D viewport
scripts/df-snap.sh /tmp/viewport.png

# Identify viewport-area coords from the PNG (e.g. 800,400)
scripts/df-input.sh rclick 800 400

# Snap and confirm: bug = context menu appears; expected = camera rotates
scripts/df-snap.sh /tmp/after-rclick.png
```

### 3. Check control-pane overlap (#41)

```
# Default window
scripts/df-snap.sh /tmp/default.png

# Resize manually or via key shortcut, then re-snap
scripts/df-snap.sh /tmp/resized.png
```

## Slow cold-start

On a cold `cargo build`, the splashscreen→main transition can exceed the default 5-second poll. Override:

```
DF_SNAP_TIMEOUT=30 scripts/df-snap.sh
```

## Troubleshooting

- `df-snap.sh` exits 2: DragonFruit isn't running. Start with `npm run tauri:dev`.
- `df-snap.sh` exits 3: main window didn't appear in time. Increase `DF_SNAP_TIMEOUT`, or check that osascript has Screen Recording permission.
- `df-input.sh` exits 5: install cliclick — `brew install cliclick`.
- `df-input.sh` exits 6: input validation rejected your args. See `--help` for syntax.
- `df-input.sh` exits 7: cliclick ran but failed. Usually a missing Accessibility or Input Monitoring permission grant.
- Use `scripts/df-snap.sh --list-windows` to see what windows osascript thinks belong to the DragonFruit process.
