//! Real Wayland clients plus the virtual-pointer protocol exercise normal compositor dispatch.
use std::thread;
use std::time::{Duration, Instant};

use niri_config::{Action, Config};
use smithay::desktop::Window as ServerWindow;
use smithay::reexports::wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1;
use smithay::utils::{Logical, Point};
use wayland_client::protocol::wl_pointer::{Axis, AxisSource, ButtonState};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::Proxy as _;
use wayland_server::Resource as _;

use super::client::{ClientId, PointerEvent};
use super::*;
use crate::utils::get_monotonic_time;

const LEFT: u32 = 0x110;

fn time() -> u32 {
    get_monotonic_time().as_millis() as u32
}

fn create_window(f: &mut Fixture, id: ClientId, app_id: &str) -> WlSurface {
    let window = f.client(id).create_window();
    window.xdg_toplevel.set_app_id(app_id.into());
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);
    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(800, 600);
    // Exercise the difference between the visual geometry and wl_surface buffer origin.
    window.xdg_surface.set_window_geometry(11, 17, 760, 550);
    window.ack_last_and_commit();
    f.double_roundtrip(id);
    f.niri_complete_animations();
    surface
}

fn fixture(
    size: (u16, u16),
    extra_config: &str,
) -> (Fixture, ClientId, WlSurface, ZwlrVirtualPointerV1) {
    let config = Config::parse_mem(&format!(
        r#"
        input {{ focus-follows-mouse; }}
        window-rule {{
            match app-id="^spotify$"
            overview-interactive true
        }}
        {extra_config}
    "#
    ))
    .unwrap();
    let mut f = Fixture::with_config(config);
    f.add_output(1, size);
    let id = f.add_client();
    let spotify = create_window(&mut f, id, "spotify");
    f.niri_state().do_action(Action::FocusWorkspaceDown, false);
    create_window(&mut f, id, "not-spotify");
    f.niri_state().do_action(Action::OpenOverview, false);
    f.niri_complete_animations();
    let output = f.client(id).output("headless-1");
    f.client(id).create_pointer();
    let pointer = f.client(id).create_virtual_pointer(&output);
    f.double_roundtrip(id);
    f.client(id).clear_pointer_events();
    (f, id, spotify, pointer)
}

fn server_window(f: &Fixture, surface: &WlSurface) -> ServerWindow {
    // There is exactly one test client; its client/server object numbers identify its surfaces.
    f.state
        .server
        .state
        .niri
        .layout
        .windows()
        .find(|(_, mapped)| {
            mapped
                .window
                .toplevel()
                .unwrap()
                .wl_surface()
                .id()
                .protocol_id()
                == surface.id().protocol_id()
        })
        .unwrap()
        .1
        .window
        .clone()
}

fn preview_point(
    f: &Fixture,
    surface: &WlSurface,
    local: Point<f64, Logical>,
) -> Point<f64, Logical> {
    let output = f.niri_output(1);
    let niri = &f.state.server.state.niri;
    let window = server_window(f, surface);
    let (origin, zoom) = niri
        .layout
        .overview_window_transform(&output, &window)
        .unwrap();
    niri.global_space
        .output_geometry(&output)
        .unwrap()
        .loc
        .to_f64()
        + origin
        + local.upscale(zoom)
}

fn move_pointer(
    f: &mut Fixture,
    id: ClientId,
    pointer: &ZwlrVirtualPointerV1,
    p: Point<f64, Logical>,
) {
    let output = f.niri_output(1);
    let geometry = f.niri().global_space.output_geometry(&output).unwrap();
    let p = p - geometry.loc.to_f64();
    // Large extents retain subpixel precision through the virtual-pointer fixed-point mapping.
    let extent = 1_000_000.;
    pointer.motion_absolute(
        time(),
        (p.x / f64::from(geometry.size.w) * extent).round() as u32,
        (p.y / f64::from(geometry.size.h) * extent).round() as u32,
        extent as u32,
        extent as u32,
    );
    pointer.frame();
    f.double_roundtrip(id);
}

fn button(
    f: &mut Fixture,
    id: ClientId,
    pointer: &ZwlrVirtualPointerV1,
    code: u32,
    state: ButtonState,
) {
    pointer.button(time(), code, state);
    pointer.frame();
    f.double_roundtrip(id);
}

fn click(f: &mut Fixture, id: ClientId, pointer: &ZwlrVirtualPointerV1) {
    button(f, id, pointer, LEFT, ButtonState::Pressed);
    button(f, id, pointer, LEFT, ButtonState::Released);
}

fn wheel(f: &mut Fixture, id: ClientId, pointer: &ZwlrVirtualPointerV1) {
    // Set metadata after starting the frame: the existing virtual-pointer backend only
    // retains axis_source once a timestamped axis request has created that frame.
    pointer.axis_discrete(time(), Axis::VerticalScroll, 12., 1);
    pointer.axis_source(AxisSource::Wheel);
    pointer.frame();
    f.double_roundtrip(id);
}

fn buttons(f: &Fixture, id: ClientId) -> Vec<(u32, ButtonState)> {
    f.state
        .clients
        .iter()
        .find(|c| c.id == id)
        .unwrap()
        .state
        .pointer_events
        .iter()
        .filter_map(|event| match event {
            PointerEvent::Button { button, state } => Some((*button, *state)),
            _ => None,
        })
        .collect()
}

fn wait_click(f: &mut Fixture, id: ClientId) {
    let deadline = Instant::now() + Duration::from_millis(430);
    while Instant::now() < deadline {
        f.dispatch();
        thread::sleep(Duration::from_millis(2));
    }
    f.double_roundtrip(id);
}

fn point(f: &Fixture, surface: &WlSurface) -> Point<f64, Logical> {
    preview_point(f, surface, Point::from((220., 180.)))
}

#[test]
fn preview_hover_maps_buffer_coordinates_at_two_output_scales() {
    for (size, extra, scale) in [
        ((1920, 1080), "", 1.),
        ((1280, 720), "output \"headless-1\" { scale 1.5; }", 1.5),
    ] {
        let (mut f, id, surface, pointer) = fixture(size, extra);
        assert_eq!(f.niri_output(1).current_scale().fractional_scale(), scale);
        let workspace = f.niri().layout.active_workspace().unwrap().id();
        let p = point(&f, &surface);
        move_pointer(&mut f, id, &pointer, p);
        let events = &f.client(id).state.pointer_events;
        let local = events
            .iter()
            .rev()
            .find_map(|event| match event {
                PointerEvent::Motion { x, y } => Some((*x, *y)),
                PointerEvent::Enter {
                    surface: entered,
                    x,
                    y,
                } if entered == &surface => Some((*x, *y)),
                _ => None,
            })
            .expect("client must receive pointer focus/motion");
        assert!((local.0 - 220.).abs() < 0.1, "{local:?}");
        assert!((local.1 - 180.).abs() < 0.1, "{local:?}");
        assert!(f.niri().layout.is_overview_open());
        assert_eq!(f.niri().layout.active_workspace().unwrap().id(), workspace);
    }
}

#[test]
fn single_click_delivers_once_after_delay_without_activating_background_workspace() {
    let (mut f, id, surface, pointer) = fixture((1920, 1080), "");
    let workspace = f.niri().layout.active_workspace().unwrap().id();
    let focus = f.niri().layout.focus().unwrap().window.clone();
    let p = point(&f, &surface);
    move_pointer(&mut f, id, &pointer, p);
    click(&mut f, id, &pointer);
    assert_eq!(buttons(&f, id), []);
    wait_click(&mut f, id);
    assert_eq!(
        buttons(&f, id),
        [(LEFT, ButtonState::Pressed), (LEFT, ButtonState::Released)]
    );
    assert!(f.niri().layout.is_overview_open());
    assert_eq!(f.niri().layout.active_workspace().unwrap().id(), workspace);
    assert_eq!(f.niri().layout.focus().unwrap().window, focus);
    assert!(f.niri_state().niri.keyboard_focus.is_overview());
}

#[test]
fn double_click_enters_exact_background_window_without_client_clicks() {
    let (mut f, id, surface, pointer) = fixture((1920, 1080), "");
    let target = server_window(&f, &surface);
    let p = point(&f, &surface);
    move_pointer(&mut f, id, &pointer, p);
    click(&mut f, id, &pointer);
    click(&mut f, id, &pointer);
    assert!(!f.niri().layout.is_overview_open());
    assert_eq!(f.niri().layout.focus().unwrap().window, target);
    wait_click(&mut f, id);
    assert_eq!(buttons(&f, id), []);
}

#[test]
fn drag_commits_without_timeout_and_releases_outside_preview() {
    let (mut f, id, surface, pointer) = fixture((1280, 720), "");
    let workspace = f.niri().layout.active_workspace().unwrap().id();
    let p = point(&f, &surface);
    move_pointer(&mut f, id, &pointer, p);
    button(&mut f, id, &pointer, LEFT, ButtonState::Pressed);
    move_pointer(&mut f, id, &pointer, p + Point::from((24., 0.)));
    assert_eq!(buttons(&f, id), [(LEFT, ButtonState::Pressed)]);
    move_pointer(&mut f, id, &pointer, Point::from((1270., 710.)));
    button(&mut f, id, &pointer, LEFT, ButtonState::Released);
    wait_click(&mut f, id);
    assert_eq!(
        buttons(&f, id),
        [(LEFT, ButtonState::Pressed), (LEFT, ButtonState::Released)]
    );
    assert!(f.niri().layout.is_overview_open());
    assert_eq!(f.niri().layout.active_workspace().unwrap().id(), workspace);
}

#[test]
fn wheel_reaches_client_without_navigating_overview() {
    let (mut f, id, surface, pointer) = fixture((1920, 1080), "");
    let workspace = f.niri().layout.active_workspace().unwrap().id();
    let p = point(&f, &surface);
    move_pointer(&mut f, id, &pointer, p);
    wheel(&mut f, id, &pointer);
    assert!(f
        .client(id)
        .state
        .pointer_events
        .iter()
        .any(|event| matches!(event,
        PointerEvent::Axis { axis: Axis::VerticalScroll, value } if *value == 12.)));
    assert!(f.niri().layout.is_overview_open());
    assert_eq!(f.niri().layout.active_workspace().unwrap().id(), workspace);
}

#[test]
fn close_cancels_pending_click_and_balances_delivered_drag() {
    for drag in [false, true] {
        let (mut f, id, surface, pointer) = fixture((1920, 1080), "");
        let p = point(&f, &surface);
        move_pointer(&mut f, id, &pointer, p);
        button(&mut f, id, &pointer, LEFT, ButtonState::Pressed);
        if drag {
            move_pointer(&mut f, id, &pointer, p + Point::from((24., 0.)));
        }
        f.niri_state().do_action(Action::CloseOverview, false);
        button(&mut f, id, &pointer, LEFT, ButtonState::Released);
        wait_click(&mut f, id);
        let expected = if drag {
            vec![(LEFT, ButtonState::Pressed), (LEFT, ButtonState::Released)]
        } else {
            vec![]
        };
        assert_eq!(buttons(&f, id), expected);
        assert!(!f.niri().seat.get_pointer().unwrap().is_grabbed());
    }
}

#[test]
fn destroying_pending_target_does_not_deliver_release_to_remaining_window() {
    let (mut f, id, surface, pointer) = fixture((1920, 1080), "");
    let p = point(&f, &surface);
    move_pointer(&mut f, id, &pointer, p);
    button(&mut f, id, &pointer, LEFT, ButtonState::Pressed);
    let window = f.client(id).window(&surface);
    window.xdg_toplevel.destroy();
    window.xdg_surface.destroy();
    window.surface.destroy();
    f.double_roundtrip(id);
    move_pointer(&mut f, id, &pointer, Point::from((640., 360.)));
    button(&mut f, id, &pointer, LEFT, ButtonState::Released);
    wait_click(&mut f, id);
    assert_eq!(buttons(&f, id), []);
    assert!(!f.niri().seat.get_pointer().unwrap().is_grabbed());
}

#[test]
fn non_live_geometry_cancels_saved_click_instead_of_replaying_after_animation() {
    let (mut f, id, surface, pointer) = fixture((1920, 1080), "");
    let window = server_window(&f, &surface);
    let p = point(&f, &surface);
    move_pointer(&mut f, id, &pointer, p);
    click(&mut f, id, &pointer);
    f.niri().layout.start_open_animation_for_window(&window);
    f.niri_state().refresh_pointer_contents();
    f.niri_complete_animations();
    wait_click(&mut f, id);
    assert_eq!(buttons(&f, id), []);
    assert!(f.niri().layout.is_overview_open());
    assert!(!f.niri().seat.get_pointer().unwrap().is_grabbed());
}

#[test]
fn secondary_buttons_are_drained_without_unmatched_releases() {
    for drag in [false, true] {
        let (mut f, id, surface, pointer) = fixture((1920, 1080), "");
        let p = point(&f, &surface);
        move_pointer(&mut f, id, &pointer, p);
        button(&mut f, id, &pointer, LEFT, ButtonState::Pressed);
        if drag {
            move_pointer(&mut f, id, &pointer, p + Point::from((24., 0.)));
        }
        button(&mut f, id, &pointer, 0x111, ButtonState::Pressed);
        button(&mut f, id, &pointer, LEFT, ButtonState::Released);
        button(&mut f, id, &pointer, 0x111, ButtonState::Released);
        wait_click(&mut f, id);
        let expected = if drag {
            vec![(LEFT, ButtonState::Pressed), (LEFT, ButtonState::Released)]
        } else {
            vec![]
        };
        assert_eq!(buttons(&f, id), expected);
        assert!(!f.niri().seat.get_pointer().unwrap().is_grabbed());
    }
}

#[test]
fn configured_wheel_action_cancels_deferred_click() {
    let (mut f, id, surface, pointer) = fixture(
        (1920, 1080),
        "binds { WheelScrollDown { focus-workspace-down; }; }",
    );
    let workspace = f.niri().layout.active_workspace().unwrap().id();
    let p = point(&f, &surface);
    move_pointer(&mut f, id, &pointer, p);
    click(&mut f, id, &pointer);
    wheel(&mut f, id, &pointer);
    wait_click(&mut f, id);
    assert_eq!(buttons(&f, id), []);
    assert_ne!(f.niri().layout.active_workspace().unwrap().id(), workspace);
    assert!(f.niri().layout.is_overview_open());
}

#[test]
fn unlisted_application_keeps_single_click_selection() {
    let (mut f, id, _, pointer) = fixture((1920, 1080), "");
    let surface = f.client(id).state.windows.last().unwrap().surface.clone();
    let target = server_window(&f, &surface);
    let p = point(&f, &surface);
    move_pointer(&mut f, id, &pointer, p);
    click(&mut f, id, &pointer);
    assert!(!f.niri().layout.is_overview_open());
    assert_eq!(f.niri().layout.focus().unwrap().window, target);
    assert_eq!(buttons(&f, id), []);
}

#[test]
fn scrolling_mode_does_not_enable_client_preview_input() {
    let (mut f, id, surface, pointer) = fixture((1920, 1080), "overview { mode \"scrolling\"; }");
    let target = server_window(&f, &surface);
    let output = f.niri_output(1);
    assert!(f
        .niri()
        .layout
        .overview_window_transform(&output, &target)
        .is_none());
    click(&mut f, id, &pointer);
    assert!(!f.niri().layout.is_overview_open());
    assert_eq!(buttons(&f, id), []);
}

#[test]
fn normal_desktop_click_has_no_preview_delay() {
    let (mut f, id, _, pointer) = fixture((1920, 1080), "");
    let surface = f.client(id).state.windows.last().unwrap().surface.clone();
    f.niri_state().do_action(Action::CloseOverview, false);
    f.niri_complete_animations();
    let output = f.niri_output(1);
    let window = server_window(&f, &surface);
    let rect = f
        .niri()
        .layout
        .window_visual_rect(&output, &window)
        .unwrap();
    move_pointer(&mut f, id, &pointer, rect.loc + Point::from((100., 100.)));
    click(&mut f, id, &pointer);
    assert_eq!(
        buttons(&f, id),
        [(LEFT, ButtonState::Pressed), (LEFT, ButtonState::Released)]
    );
}

fn window_workspace(f: &Fixture, window: &ServerWindow) -> crate::layout::workspace::WorkspaceId {
    f.state
        .server
        .state
        .niri
        .layout
        .workspaces()
        .find(|(_, _, ws)| ws.windows().any(|w| &w.window == window))
        .unwrap()
        .2
        .id()
}

#[test]
fn super_drag_moves_interactive_preview_and_plain_drag_moves_unlisted_preview() {
    use smithay::backend::input::{KeyState, Keycode};
    use smithay::input::keyboard::FilterResult;
    use smithay::utils::SERIAL_COUNTER;
    for interactive in [true, false] {
        let (mut f, id, spotify, pointer) = fixture((1920, 1080), "");
        let other = f.client(id).state.windows.last().unwrap().surface.clone();
        let (source, destination) = if interactive {
            (&spotify, &other)
        } else {
            (&other, &spotify)
        };
        let window = server_window(&f, source);
        let target_workspace = window_workspace(&f, &server_window(&f, destination));
        let from = point(&f, source);
        let to = point(&f, destination);
        let keyboard = f.niri().seat.get_keyboard().unwrap();
        if interactive {
            keyboard.input::<(), _>(
                f.niri_state(),
                Keycode::new(133),
                KeyState::Pressed,
                SERIAL_COUNTER.next_serial(),
                time(),
                |_, _, _| FilterResult::Forward,
            );
            assert!(keyboard.modifier_state().logo);
        }
        move_pointer(&mut f, id, &pointer, from);
        button(&mut f, id, &pointer, LEFT, ButtonState::Pressed);
        move_pointer(&mut f, id, &pointer, from + Point::from((24., 0.)));
        move_pointer(&mut f, id, &pointer, to);
        button(&mut f, id, &pointer, LEFT, ButtonState::Released);
        if interactive {
            keyboard.input::<(), _>(
                f.niri_state(),
                Keycode::new(133),
                KeyState::Released,
                SERIAL_COUNTER.next_serial(),
                time(),
                |_, _, _| FilterResult::Forward,
            );
        }
        assert_eq!(buttons(&f, id), []);
        assert!(f.niri().layout.is_overview_open());
        assert_eq!(window_workspace(&f, &window), target_workspace);
    }
}

#[test]
fn trash_cancels_nearby_pending_click_even_when_client_keeps_window_mapped() {
    let (mut f, id, surface, pointer) = fixture((1920, 1080), "");
    let output = f.niri_output(1);
    let window = server_window(&f, &surface);
    let rect = f
        .niri()
        .layout
        .window_visual_rect(&output, &window)
        .unwrap();
    let trash = crate::ui::overview_controls::OverviewControls::button_rect(rect).unwrap();
    let from = trash.loc + Point::from((-3., trash.size.h / 2.));
    move_pointer(&mut f, id, &pointer, from);
    click(&mut f, id, &pointer);
    assert_eq!(buttons(&f, id), []);
    move_pointer(&mut f, id, &pointer, from + Point::from((6., 0.)));
    click(&mut f, id, &pointer);
    wait_click(&mut f, id);
    assert!(f.client(id).window(&surface).close_requested);
    assert_eq!(buttons(&f, id), []);
    assert!(f.niri().layout.is_overview_open());
}

#[test]
fn workspace_badge_keeps_its_explicit_activation_action() {
    let (mut f, id, _, pointer) = fixture((1920, 1080), "");
    let output = f.niri_output(1);
    let (workspace, _, origin) = f
        .niri()
        .layout
        .monitor_for_output(&output)
        .unwrap()
        .overview_workspace_labels()
        .next()
        .unwrap();
    let badge = crate::ui::overview_controls::OverviewControls::workspace_label_rect(origin);
    move_pointer(
        &mut f,
        id,
        &pointer,
        badge.loc + badge.size.to_point().upscale(0.5),
    );
    click(&mut f, id, &pointer);
    assert!(!f.niri().layout.is_overview_open());
    assert_eq!(f.niri().layout.active_workspace().unwrap().id(), workspace);
    assert_eq!(buttons(&f, id), []);
}

#[test]
fn canvas_controller_is_opt_in_and_does_not_steal_desktop_back_clicks() {
    let (mut f, id, _, pointer) = fixture((1920, 1080), "");
    let surface = f.client(id).state.windows.last().unwrap().surface.clone();
    f.niri_state().do_action(Action::CloseOverview, false);
    f.niri_complete_animations();
    let output = f.niri_output(1);
    let window = server_window(&f, &surface);
    let rect = f
        .niri()
        .layout
        .window_visual_rect(&output, &window)
        .unwrap();
    move_pointer(&mut f, id, &pointer, rect.loc + Point::from((100., 100.)));
    button(&mut f, id, &pointer, 275, ButtonState::Pressed);
    button(&mut f, id, &pointer, 275, ButtonState::Released);
    assert_eq!(
        buttons(&f, id),
        [(275, ButtonState::Pressed), (275, ButtonState::Released)]
    );
    assert!(!f.niri().workspace_mouse_camera_active);
}

#[test]
fn canvas_controller_remaps_navigates_and_drains_release_after_config_change() {
    let (mut f, id, _, pointer) = fixture((1920, 1080), "overview { canvas-button 276; }");
    f.niri_state().do_action(Action::CloseOverview, false);
    f.niri_complete_animations();
    let workspace = f.niri().layout.active_workspace().unwrap().id();
    button(&mut f, id, &pointer, 276, ButtonState::Pressed);
    assert!(f.niri().workspace_mouse_camera_active);
    pointer.motion(time(), 0., 200.);
    pointer.frame();
    f.double_roundtrip(id);
    f.niri_complete_animations();
    assert_ne!(f.niri().layout.active_workspace().unwrap().id(), workspace);
    f.niri().config.borrow_mut().overview.canvas_button = 0;
    button(&mut f, id, &pointer, 276, ButtonState::Released);
    assert!(!f.niri().workspace_mouse_camera_active);
    assert_eq!(buttons(&f, id), []);
}

#[test]
fn configured_button_binding_takes_priority_over_canvas_controller() {
    let (mut f, id, _, pointer) = fixture(
        (1920, 1080),
        "overview { canvas-button 275; }\nbinds { MouseBack { toggle-overview; }; }",
    );
    f.niri_state().do_action(Action::CloseOverview, false);
    f.niri_complete_animations();
    button(&mut f, id, &pointer, 275, ButtonState::Pressed);
    button(&mut f, id, &pointer, 275, ButtonState::Released);
    assert!(f.niri().layout.is_overview_open());
    assert!(!f.niri().workspace_mouse_camera_active);
    assert_eq!(buttons(&f, id), []);
}

// Popup-specific dispatch keeps these protocol probes isolated from normal test windows.
mod popup_probe {
    use super::super::client;
    use smithay::reexports::wayland_protocols::xdg::shell::client::{
        xdg_popup::{self, XdgPopup},
        xdg_positioner::XdgPositioner,
        xdg_surface::{self, XdgSurface},
    };
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use wayland_client::{Connection, Dispatch, QueueHandle};

    pub struct PopupData(pub Arc<AtomicBool>);
    pub struct SurfaceData;
    pub struct PositionerData;

    impl Dispatch<XdgPopup, PopupData> for client::State {
        fn event(
            _: &mut Self,
            _: &XdgPopup,
            event: xdg_popup::Event,
            data: &PopupData,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let xdg_popup::Event::PopupDone = event {
                data.0.store(true, Ordering::Relaxed);
            }
        }
    }
    impl Dispatch<XdgSurface, SurfaceData> for client::State {
        fn event(
            _: &mut Self,
            surface: &XdgSurface,
            event: xdg_surface::Event,
            _: &SurfaceData,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let xdg_surface::Event::Configure { serial } = event {
                surface.ack_configure(serial);
            }
        }
    }
    impl Dispatch<XdgPositioner, PositionerData> for client::State {
        fn event(
            _: &mut Self,
            _: &XdgPositioner,
            _: <XdgPositioner as wayland_client::Proxy>::Event,
            _: &PositionerData,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }
}

#[test]
fn grabbed_popup_is_dismissed_without_replacing_preview_drag_or_keyboard_focus() {
    use popup_probe::*;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let (mut f, id, surface, pointer) = fixture((1920, 1080), "");
    let p = point(&f, &surface);
    move_pointer(&mut f, id, &pointer, p);
    button(&mut f, id, &pointer, LEFT, ButtonState::Pressed);
    move_pointer(&mut f, id, &pointer, p + Point::from((24., 0.)));
    let keyboard_focus = f.niri().seat.get_keyboard().unwrap().current_focus();
    let done = Arc::new(AtomicBool::new(false));
    let client = f.client(id);
    let qh = client.qh.clone();
    let wm = client.state.xdg_wm_base.as_ref().unwrap();
    let popup_surface = client
        .state
        .compositor
        .as_ref()
        .unwrap()
        .create_surface(&qh, ());
    let xdg_surface = wm.get_xdg_surface(&popup_surface, &qh, SurfaceData);
    let positioner = wm.create_positioner(&qh, PositionerData);
    positioner.set_size(100, 100);
    positioner.set_anchor_rect(10, 10, 20, 20);
    let parent = client
        .state
        .windows
        .iter()
        .find(|w| w.surface == surface)
        .unwrap();
    let popup = xdg_surface.get_popup(
        Some(&parent.xdg_surface),
        &positioner,
        &qh,
        PopupData(done.clone()),
    );
    let serial = client.state.pointer_button_serial;
    assert_ne!(serial, 0);
    popup.grab(client.state.seat.as_ref().unwrap(), serial);
    popup_surface.commit();
    f.double_roundtrip(id);
    assert!(done.load(Ordering::Relaxed));
    assert!(f.niri().popup_grab.is_none());
    assert!(crate::input::overview_client_grab::OverviewClientGrab::is_active(f.niri_state()));
    assert_eq!(
        f.niri().seat.get_keyboard().unwrap().current_focus(),
        keyboard_focus
    );
    button(&mut f, id, &pointer, LEFT, ButtonState::Released);
    assert_eq!(
        buttons(&f, id),
        [(LEFT, ButtonState::Pressed), (LEFT, ButtonState::Released)]
    );
    popup.destroy();
    xdg_surface.destroy();
    popup_surface.destroy();
    positioner.destroy();
    f.double_roundtrip(id);
}

#[test]
fn canvas_controller_stops_at_overview_and_modal_ownership_boundaries() {
    for action in [Action::OpenOverview, Action::Quit(false)] {
        let (mut f, id, _, pointer) = fixture((1920, 1080), "overview { canvas-button 275; }");
        f.niri_state().do_action(Action::CloseOverview, false);
        f.niri_complete_animations();
        let workspace = f.niri().layout.active_workspace().unwrap().id();
        button(&mut f, id, &pointer, 275, ButtonState::Pressed);
        assert!(f.niri().workspace_mouse_camera_active);
        f.niri_state().do_action(action, false);
        f.niri_state().sync_workspace_mouse_camera();
        if f.niri().layout.is_overview_open() {
            assert!(f.niri().pointer_visibility.is_visible());
        }
        pointer.motion(time(), 0., 200.);
        pointer.frame();
        f.double_roundtrip(id);
        assert_eq!(f.niri().layout.active_workspace().unwrap().id(), workspace);
        assert!(!f.niri().workspace_mouse_camera_active);
        button(&mut f, id, &pointer, 275, ButtonState::Released);
        assert_eq!(buttons(&f, id), []);
    }
}

#[test]
fn cancelled_preview_grab_drains_secondary_release_after_modified_press() {
    use smithay::backend::input::{KeyState, Keycode};
    use smithay::input::keyboard::FilterResult;
    use smithay::utils::SERIAL_COUNTER;
    let (mut f, id, surface, pointer) = fixture((1920, 1080), "");
    let p = point(&f, &surface);
    move_pointer(&mut f, id, &pointer, p);
    click(&mut f, id, &pointer);
    button(&mut f, id, &pointer, 0x111, ButtonState::Pressed);
    let keyboard = f.niri().seat.get_keyboard().unwrap();
    keyboard.input::<(), _>(
        f.niri_state(),
        Keycode::new(133),
        KeyState::Pressed,
        SERIAL_COUNTER.next_serial(),
        time(),
        |_, _, _| FilterResult::Forward,
    );
    button(&mut f, id, &pointer, 275, ButtonState::Pressed);
    move_pointer(&mut f, id, &pointer, p + Point::from((1., 0.)));
    button(&mut f, id, &pointer, 0x111, ButtonState::Released);
    button(&mut f, id, &pointer, 275, ButtonState::Released);
    keyboard.input::<(), _>(
        f.niri_state(),
        Keycode::new(133),
        KeyState::Released,
        SERIAL_COUNTER.next_serial(),
        time(),
        |_, _, _| FilterResult::Forward,
    );
    assert_eq!(buttons(&f, id), []);
    assert!(!f.niri().seat.get_pointer().unwrap().is_grabbed());
}
