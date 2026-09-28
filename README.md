# Niri Interactive Overview

An experimental, independent [Niri](https://github.com/niri-wm/niri) derivative with a mouse-first **fit overview** and optional **interactive application previews**. This is a complete compositor fork, not a Quickshell widget or a configuration bundle.

Based on **Niri v26.04**, commit **`8ed0da44d974c32c6877d2f4630c314da0717ecb`**. It is not an official Niri release and is not endorsed by upstream.

## Demo

A recorded interaction demo is not included yet. The intended demonstration sequence is:

1. Mouse side button → fit overview.
2. Navigate populated workspaces and window columns.
3. Drag a window between workspaces; drag a numbered badge to reorder workspaces.
4. Use an opted-in Spotify preview without leaving overview.
5. Double-click the preview → enter the real window.

Only synthetic or explicitly reviewed content should appear in future recordings. Physical mouse operation and actual Spotify playback/seek/volume are not established by automated protocol tests.

## Features

- **Fit overview:** populated workspaces and their complete column strips fit on the output. The trailing unnamed empty workspace is hidden but remains a drop target below the last visible workspace.
- **Pointer-first navigation:** select a window or workspace badge; drag windows between workspaces, into columns, or into an explicit side split. Drag a badge to reorder its workspace with animated neighbor movement and settling.
- **Close controls:** a hovered preview exposes a trash control that takes priority over client interaction.
- **Animated camera:** fixed-endpoint 133 ms linear fit transitions interpolate position and scale together. Newly populated previews fade in; interrupted transitions continue from displayed geometry. Window activation and horizontal focus scrolling share the closing zoom.
- **Held canvas controller:** an optional, configurable mouse button turns physical mouse travel into inverted desktop navigation outside overview. Right/left reveals the left/right column; down/up reveals the workspace above/below. A generic workspace map and grabbing cursor indicate the held state.
- **Interactive previews, opt-in:** send pointer hover, wheel events and client drags to selected applications while keeping overview open. Ordinary applications retain compositor selection/dragging.
- **Alternative controller:** `overview { mode "scrolling"; }` selects the cursorless centered scrolling overview; interactive preview input is isolated to fit mode.

The feature implementation is native Rust/Smithay compositor code. It does not call DMS, Quickshell, a shell script, or a desktop-specific IPC service. A bar or shell is optional; this project does not provide a complete desktop shell.

## Build

Use current stable Rust, Cargo, Git, a C compiler, Clang/libclang and pkg-config. Keep `Cargo.lock`: builds use `--locked`. Cargo downloads public crates and the pinned public Smithay revision; no private repository or local source checkout is needed.

Arch-family dependencies (package names, not a claim of testing every Arch derivative):

```sh
sudo pacman -S --needed base-devel clang git rust wayland libinput libxkbcommon mesa libglvnd libseat systemd dbus pipewire pango cairo gdk-pixbuf2 libdisplay-info
```

Ubuntu 24.04 / CI dependency recipe:

```sh
sudo apt-get update
sudo apt-get install -y git curl build-essential pkg-config clang libudev-dev libgbm-dev libxkbcommon-dev libegl1-mesa-dev libwayland-dev libinput-dev libdbus-1-dev libsystemd-dev libseat-dev libpipewire-0.3-dev libpango1.0-dev libgdk-pixbuf-2.0-dev libdisplay-info-dev
```

Install current stable Rust using your package manager or [rustup](https://rustup.rs/) if the distribution's Rust is too old. Then:

```sh
git clone https://github.com/sanjit-ravi/niri-interactive-overview.git
cd niri-interactive-overview
cargo build --release --locked
./target/release/niri validate --config resources/interactive-overview.kdl
```

Do **not** use `--all-features`: upstream profiling features can collect unbounded data. The added GdkPixbuf dependency loads desktop icons for the held-controller map.

## Try without replacing stock Niri

From a terminal inside a Wayland desktop:

```sh
./target/release/niri --config resources/interactive-overview.kdl
```

This opens a **nested compositor window**. Do not pass `--session` in a nested run: session mode imports environment into the user's service manager. The example starts no applications or shell. The log prints this instance's Wayland display and IPC socket; launch a disposable native Wayland application with `WAYLAND_DISPLAY` set to that display. Target IPC commands using its `NIRI_SOCKET`, never your parent compositor's socket.

The example uses Super even in nested mode; the parent compositor may intercept it or side buttons. Nested mode is useful for render/development checks, not proof of physical input compatibility. Close only the nested window to return to your desktop.

### Optional side-by-side install

```sh
install -Dm755 target/release/niri "$HOME/.local/bin/niri-interactive-overview"
```

Cargo still builds a binary named `niri`; this command installs it under a distinct name. It does not change `/usr/bin/niri`, system services, session entries, or your current compositor.

For a real-session trial, save your work and log out normally. From an unused TTY, run the distinct binary with a dedicated configuration:

```sh
~/.local/bin/niri-interactive-overview --config /absolute/path/to/interactive-overview.kdl
```

This bare TTY launch does not install portal/session integration. Integrating a display-manager session or user service is an explicit administrator task; review upstream session documentation rather than blindly installing the inherited `resources/niri.service` or distribution packaging metadata, which still target stock binary paths.

## Configuration

[`resources/interactive-overview.kdl`](resources/interactive-overview.kdl) is a standalone minimal example, not a copy of a personal desktop configuration:

```kdl
input {
    mod-key "Super"
    mod-key-nested "Super"
}
overview {
    mode "fit"
    canvas-button 275
}
binds {
    Super+Tab repeat=false { toggle-overview; }
    MouseForward repeat=false { toggle-overview; }
    Super+Shift+E { quit; }
}
```

`canvas-button` is a **Linux evdev button code**, not a device name. `275` is BTN_SIDE / MouseBack, and `276` is BTN_EXTRA / MouseForward. Hardware may label or position them differently. Set `0` or omit it to disable the controller (the default). Other evdev button codes can be used; use a spare button rather than your ordinary left/right click. Change the overview toggle independently using normal Niri bindings. Explicit compositor bindings take priority when they use the same button; do not bind your canvas button to another action if you want its held behavior.

Canvas mode starts only on an unmodified press outside overview and outside an existing pointer grab or compositor modal UI. Opening overview, entering a modal/locked state, or starting another grab cancels navigation; the matching physical release is still consumed, including across configuration changes. Physical travel is unaccelerated and uses a 160-unit directional threshold. Horizontal release retains the cursor when it reaches a newly revealed window, otherwise it centers on the selected target to avoid focus-follows-mouse returning to the origin. Vertical changes retain output-centered cursor behavior. No shell bar is revealed at workspace boundaries.

### Interactive previews

Disabled by default. Add an explicit app-ID rule, for example:

```kdl
window-rule {
    match app-id=r#"^spotify$"#
    overview-interactive true
}
```

This is generic window-rule matching, not a Spotify-only implementation. Inspect your own application's actual app ID. A later matching `overview-interactive false` overrides an earlier true rule. Unlisted applications keep normal overview single-click selection and compositor dragging.

| Gesture in an opted-in fit preview | Result |
| --- | --- |
| Hover / wheel | Delivered to the client through the displayed preview transform |
| Single click | Deferred, then delivered once at the saved surface and client coordinate |
| Ordinary left-drag | Client drag; ownership continues outside the preview |
| Super+left-drag | Move the preview itself (uses your configured compositor modifier) |
| Double-click | Enter the real window on second release; neither click reaches the client |

The double-click interval is **400 ms from the first press**, with **8 logical screen pixels** of movement tolerance. Movement at the threshold while held begins a client drag; a long hold commits at timeout. A released click can resolve early when the pointer moves beyond tolerance. A drag cannot later become an activation double-click.

Preview input does **not grant keyboard focus**, switch workspace or activate the window. Enter the real window for typing. Trash, workspace badges and compositor bindings retain priority. Pending input is cancelled on compositor actions, overview closure, target/surface destruction or invalid geometry; delivered presses are balanced and physical releases drained.

**Popup limits:** grabbed native XDG popup menus are dismissed in fit overview rather than taking keyboard focus. Subsurfaces and non-grabbing popups use normal surface-tree hit testing inside the tile input region; popup regions outside the tile are not interactive.

**Animation limits:** camera/positional animations use currently displayed transforms, including the window's buffer-origin offset. Opening/resize shaders can show snapshots without a uniquely invertible live surface; those intervals reject preview input and cancel pending grabs. XWayland applications and unusual client input protocols require separate testing.

## Tests

Run these from the checkout built above:

```sh
cargo test --release --locked overview --no-run
timeout --kill-after=5s 120s cargo test --release --locked overview -- --test-threads=1
cargo test --release --locked -p niri-config
cargo test --release --locked --workspace --exclude niri-visual-tests --no-run
timeout --kill-after=5s 300s cargo test --release --locked --workspace --exclude niri-visual-tests -- --test-threads=1
RUN_SLOW_TESTS=1 PROPTEST_CASES=1000 PROPTEST_RNG_SEED=42 timeout --kill-after=5s 180s cargo test --release --locked --lib layout::tests::random_operations_dont_panic -- --exact --test-threads=1
```

Compilation is outside the execution timeout. Tests include real in-process Wayland clients and the virtual-pointer protocol, not IPC substitutes for pointer input. `niri-visual-tests` is an upstream GTK developer application, not part of the normal compositor build; it needs additional GTK/libadwaita dependencies. The final command opts into 1,000 seeded randomized layout cases. It is a bounded check, not the full upstream slow/stress campaign.

See [verification notes](docs/VERIFICATION.md) for exact exercised coverage and remaining limits. CI builds the compositor, validates the example, and runs configuration, focused and broader non-visual tests.

## Safety and rollback

Keep stock Niri installed. Never overwrite its executable or restart your live compositor to test this fork. Before a real-session trial, preserve your stock configuration and session selection. Fork-specific `overview` settings and `overview-interactive` rules may fail validation in stock Niri: use your original stock configuration when returning.

To roll back, close the nested instance, or log out of the experimental session and choose your original stock session. The side-by-side install alone needs no service rollback. Restore only service overrides that you explicitly changed yourself. A compositor restart disconnects graphical applications; configuration reload does not replace a running executable.

## Architecture and maintenance

- `src/layout/overview_camera.rs`, `monitor.rs`: fit endpoints, workspace positioning, transitions and reorder geometry.
- `src/layout/{mod,workspace,scrolling,tile}.rs`: preview hit testing, inverse transforms, insertion/drop geometry and activation interpolation.
- `src/ui/overview_controls.rs`: workspace badges and close controls.
- `src/ui/workspace_preview.rs`: generic held-controller map and desktop-icon rendering.
- `src/input/{mod,move_grab,workspace_move_grab,overview_client_grab}.rs`: dispatch priority, drag ownership, pending clicks, balancing and cancellation.
- `src/niri.rs`, `src/handlers/{mod,xdg_shell}.rs`: pointer refresh, modal/grab guards and popup lifecycle.
- `niri-config/src/{misc,window_rule}.rs`, `src/window/mod.rs`: overview settings and opt-in rule resolution.
- `src/tests/overview_input.rs`, `src/layout/tests/overview_*.rs`: protocol and geometry regressions.

This extraction intentionally excludes automatic single-window maximization/border policies, application-specific preview branding, monitor setup, wallpaper, shell widgets, personal launchers, media/RGB integrations and machine-specific services. Upstream source/history and general-purpose defaults remain available for reference; the dedicated example is the supported starting point for these features.

Changes to Smithay pointer-grab semantics, surface coordinates, popup handling, layout insertions or animation geometry require re-running protocol and transform regressions before upgrading the base. This is a maintained-source derivative, not a promise of compatibility with arbitrary Niri versions, distributions, GPUs or mice.

## Upstream relationship, contributions and license

Niri was created by Ivan Molodetskikh and its contributors. Their history and licensing material are retained. Source is **GPL-3.0-or-later** as declared in Cargo metadata; [LICENSE](LICENSE) contains the GPL version 3 text. Preserve notices and provide corresponding source when distributing modified binaries.

Report derivative-specific problems here, not upstream. [Upstream's contribution policy](https://github.com/niri-wm/niri/blob/main/CONTRIBUTING.md) prohibits LLM-created contributions, including issues and comments. This extraction used AI assistance and makes no claim of upstream acceptance. Do not forward generated material upstream. See [CONTRIBUTING.md](CONTRIBUTING.md) for this project's review and testing expectations.
