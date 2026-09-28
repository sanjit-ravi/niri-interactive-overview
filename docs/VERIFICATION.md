# Verification

## Extraction checkout — 2026-09-28

Base: Niri v26.04 (`8ed0da44d974c32c6877d2f4630c314da0717ecb`). Final source changes are in `295a3000` and `27aa323e`. Testing used a separate release checkout, never the active desktop source tree.

Environment: Linux x86-64, CachyOS/Arch-family userspace, Rust/Cargo 1.98.1. Default Cargo features and the committed lockfile. These results do not establish compatibility with other distributions, GPUs or mice.

| Check | Observed result |
| --- | --- |
| `cargo build --release --locked` | Passed |
| Built binary validating `resources/interactive-overview.kdl` | Passed |
| `cargo test --release --locked overview -- --test-threads=1` | 37 passed, including 22 Wayland client/virtual-pointer tests |
| Configuration crate | 21 unit tests and 1 wiki parsing test passed |
| Broader workspace tests, excluding GTK visual developer application | 247 compositor tests, 21 config tests, 1 wiki test, 2 IPC tests and 1 IPC doctest passed |
| Upstream randomized layout test, `RUN_SLOW_TESTS=1 PROPTEST_CASES=1000 PROPTEST_RNG_SEED=42` | Passed after fixing the interrupted-gesture cleanup regression |

Normal suite counts include upstream tests that return early unless slow-test variables are enabled. Only the separately listed 1,000-case randomized check was opted in; the full upstream slow/stress campaign was not run. Stable rustfmt was used for the expanded protocol test file; upstream nightly-only formatting settings were unavailable. No claim of a full nightly format or clippy run is made.

### Behavior covered by automation

- Fit camera opening/closing, interrupted transitions, workspace addition/removal, activation interpolation, empty-workspace drop targets and workspace reorder/settling.
- Displayed-camera and buffer-origin coordinate transforms; protocol probes at 1920×1080 scale 1 and 1280×720 scale 1.5.
- Deferred single click at saved coordinates; double-click entering the exact window with zero client clicks; client dragging outside the preview; compositor-modifier preview dragging.
- Wheel delivery and configured-wheel cancellation; trash and badge priority; non-opted-in selection/dragging; scrolling-mode isolation; ordinary desktop click/back-button delivery.
- Target destruction, overview closure, snapshot/non-live geometry, balanced button edges, cancelled secondary-button drainage and grabbed native XDG popup dismissal without replacing preview drag or keyboard focus.
- Canvas button opt-in/remapping, explicit binding priority, navigation, release after config change, cancellation at overview/exit-confirmation boundaries, and fit cursor visibility after cancellation.

### Defects found and corrected

The input review identified a held controller crossing ownership boundaries and a cancelled preview grab being mistaken for a waiting click. Both were reproduced with failing protocol tests before correction. The bounded randomized run found a stranded empty workspace after moving a window, beginning an overview gesture and toggling overview closed; its minimized operation sequence is now a permanent layout regression. A transition assertion also caught cursor hiding during fit-mode canvas cancellation.

## Nested compositor / real renderer

The release binary ran as an isolated nested Wayland compositor with two disposable native clients containing only synthetic text. Captures were visually inspected: both populated workspaces, numbered badges and a hovered close control rendered in fit overview. No DMS, Quickshell or other shell ran inside the nested compositor.

A separate disposable C Wayland virtual-pointer client exercised the running binary: holding the configured canvas button and moving down changed workspace outside overview; opening fit overview while still held stopped that navigation, and the matching release was consumed. This is actual nested-process input/render evidence, not an IPC replacement for pointer movement. IPC was used only to select the mode and inspect workspace state.

All task-owned client/compositor process groups were stopped; their IPC/Wayland listeners were checked and stale task-owned socket files cleaned up. No main-session restart or compositor installation occurred.

## Not established

- Physical mouse side-button behavior on the main desktop.
- Actual Spotify playback, seeking, volume sliders or double-click activation in Spotify itself. Test clients use controlled app IDs; that is not application testing.
- A real-session deployment of the extracted binary, real session-lock client transitions, or nested testing under every shell.
- XWayland preview behavior, all popup/client protocol combinations, multi-monitor hardware, all fractional scales or every GPU.
- The upstream GTK visual-test application, a full slow/property stress campaign, distribution packaging installs, Nix builds or RPM builds.

A main-desktop trial would require an explicitly authorized logout/login or compositor replacement. Configuration reload cannot load a different executable. These unverified areas are not represented as passing tests.
