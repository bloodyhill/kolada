# Benchmarking launch time

Notsy's core claim is "the fastest-launching notes app". Every number we publish comes from `notsy-bench`. The tool measures any Windows app the same way, from the frames Windows actually puts on screen.

## What is measured

The clock starts just before the launch call, which is `ShellExecuteEx` with `--shell`, the same path as a Start-menu click. It stops at a frame's **present time**: the QPC timestamp DWM gives the frame when it is composed for the display.

| Metric | Meaning |
|---|---|
| `window_shown` | The app's main window first became visible (polled every ~2 ms). |
| `first_pixel` | First presented frame in which the window area differs from the stage. |
| **`complete_98`** | **Headline.** First frame after which the window area stays within 2% of its final look. |
| `visually_complete` | First frame after which the window area no longer changes (a blinking caret is ignored). |

`complete_98` is the headline because apps often make small cosmetic changes long after they are usable. Windows 11 scrollbars, for example, collapse about 3 s after launch, and `visually_complete` counts that. The 2% tolerance is measured against the total change the launch produced. That is far smaller than a note body, a sidebar or a loading screen, so none of those can hide in it.

## How

- **Capture.** DXGI Desktop Duplication on the GPU that drives the screen. Each frame is box-filtered on the GPU to 1/8 size (`GenerateMips`), so a single text stroke still changes a thumbnail pixel, and converted to luminance.
- **Change threshold.** A thumbnail pixel counts as changed when its luminance moves by more than 24 of 255. A frame counts as different when at least 16 pixels changed, which is enough to ignore a blinking caret.
- **Stage.** A plain mid-gray window covers the monitor's work area during a benchmark. Other apps animating behind the target can't disturb the measurement, and mid-gray differs clearly from both light and dark themes.
- **Taskbar.** Analysis is clipped to the work area, so taskbar launch animations and the clock never count.
- **Foreground.** Before each launch the bench takes the foreground and calls `AllowSetForegroundWindow(ASFW_ANY)`, as Explorer does, so the new window opens in front as it would for a user. If a window still opens behind the stage, the bench raises it without activating it and marks the run `raised`.
- **Baseline.** Only a truly presented frame is used. The first image Desktop Duplication hands back can be black.
- **End of run.** A run ends 2.5 s after the last change in the work area, or at a 15 s timeout.

## Scenarios

| Scenario | How | Notes |
|---|---|---|
| Warm | `--between quit` (default) | App fully quit between runs; OS file cache warm. The everyday "I click the icon" case. |
| Cold | reboot, then `--runs 1 --warmup 0` | First launch after boot. Repeat across several reboots. |
| Resident | `--between hide` | Only the window is closed, for apps that stay running in the tray (e.g. Obsidian's tray plugin). This times re-showing a running app, not a launch. |

Always report the power source (AC or battery) and the refresh rate. Both are recorded in `results.json`. At 60 Hz every timing is quantized to about 16.7 ms frames.

## Commands

```bash
notsy-bench monitors
notsy-bench launch --label "notepad" --exe notepad.exe --shell
notsy-bench launch --label "obsidian closed" --exe "%LOCALAPPDATA%\Programs\Obsidian\Obsidian.exe" --shell
notsy-bench launch --label "obsidian resident" --exe "%LOCALAPPDATA%\Programs\Obsidian\Obsidian.exe" --shell --between hide
```

Add `--save-frames` to keep the key frames and a per-frame trace (`runNN-trace.csv`) for auditing a run. Add `--save-all-frames` for everything.

## Privacy

`bench-results/` contains screenshots of whatever the app showed, including real notes. It is gitignored. Never commit it, and share only `results.json` files.

## Known limits

- Frame timing is quantized to the display refresh: ±1 frame.
- Changes smaller than about 16 thumbnail pixels (roughly 1,000 screen pixels) are below the threshold by design.
- Desktop Duplication adds a small, constant GPU load that is the same for every app measured.
