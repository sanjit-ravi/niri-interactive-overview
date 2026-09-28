//! Pointer-only interaction with an explicitly opted-in fit overview preview.
//! The first click is held for DOUBLE_CLICK_TIME; a drag commits immediately.
use std::cell::Cell;
use std::time::Duration;

use calloop::timer::{TimeoutAction, Timer};
use smithay::backend::input::ButtonState;
use smithay::desktop::{Window, WindowSurfaceType};
use smithay::input::pointer::*;
use smithay::input::SeatHandler;
use smithay::output::Output;
use smithay::utils::{IsAlive, Logical, Point, Serial, SERIAL_COUNTER};
use wayland_server::protocol::wl_surface::WlSurface;

use super::DOUBLE_CLICK_TIME;
use crate::layout::LayoutElement;
use crate::niri::{Niri, State};
use crate::utils::get_monotonic_time;

const LEFT: u32 = 0x110;
// Same logical-pixel threshold as MoveGrab.
const SLOP: f64 = 8.;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    FirstDown,
    Waiting,
    SecondDown,
    Drag,
    // Keep the grab until the physical button is released, even after overview closes.
    Cancelled,
}

#[derive(Clone)]
pub struct PreviewTarget {
    pub window: Window,
    output: Output,
    surface: WlSurface,
    // A point in window coordinates used to resolve the SAME subsurface after layout changes.
    anchor: Point<f64, Logical>,
    local: Point<f64, Logical>,
}

impl Niri {
    /// The normal render-order hit test must select this window first. Never hit through chrome,
    /// another window, a layer, a non-input region, or an unapproved application.
    pub fn overview_preview_target(&self, pos: Point<f64, Logical>) -> Option<PreviewTarget> {
        if !self.layout.is_overview_open()
            || self.config.borrow().overview.mode != niri_config::OverviewMode::Fit
            || self.is_locked()
            || self.exit_confirm_dialog.is_open()
            || self.screenshot_ui.is_open()
            || self.window_mru_ui.is_open()
        {
            return None;
        }
        let (output, point) = self.output_under(pos)?;
        if self.is_sticky_obscured_under(output, point) {
            return None;
        }
        let mon = self.layout.monitor_for_output(output)?;
        if mon.overview_workspace_labels().any(|(_, _, origin)| {
            crate::ui::overview_controls::OverviewControls::workspace_label_rect(origin)
                .contains(point)
        }) {
            return None;
        }
        let (mapped, origin, zoom) = self.layout.overview_window_hit(output, point)?;
        if !mapped.rules().overview_interactive {
            return None;
        }
        let rect = self.layout.window_visual_rect(output, &mapped.window)?;
        if crate::ui::overview_controls::OverviewControls::button_rect(rect)
            .is_some_and(|rect| rect.contains(point))
        {
            return None;
        }
        let anchor = (point - origin).downscale(zoom);
        let (surface, offset) = mapped
            .window
            .surface_under(anchor, WindowSurfaceType::ALL)?;
        Some(PreviewTarget {
            window: mapped.window.clone(),
            output: output.clone(),
            surface,
            anchor,
            local: anchor - offset.to_f64(),
        })
    }
}

impl PreviewTarget {
    pub fn focus(&self, pos: Point<f64, Logical>) -> (WlSurface, Point<f64, Logical>) {
        (self.surface.clone(), pos - self.local)
    }

    fn transform(&self, niri: &Niri) -> Option<(Point<f64, Logical>, f64, Point<f64, Logical>)> {
        if !self.window.alive()
            || !self.surface.alive()
            || niri.is_locked()
            || niri.exit_confirm_dialog.is_open()
            || niri.screenshot_ui.is_open()
            || niri.window_mru_ui.is_open()
        {
            return None;
        }
        let mapped = niri
            .layout
            .windows()
            .find(|(_, w)| w.window == self.window)?
            .1;
        if !mapped.rules().overview_interactive {
            return None;
        }
        let (origin, zoom) = niri
            .layout
            .overview_window_transform(&self.output, &self.window)?;
        let output_origin = niri
            .global_space
            .output_geometry(&self.output)?
            .loc
            .to_f64();
        let (surface, offset) = self
            .window
            .surface_under(self.anchor, WindowSurfaceType::ALL)?;
        if surface != self.surface {
            return None;
        }
        Some((origin + output_origin, zoom, offset.to_f64()))
    }

    fn mapped_focus(
        &self,
        niri: &Niri,
        pos: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        let (origin, zoom, offset) = self.transform(niri)?;
        let local = (pos - origin).downscale(zoom) - offset;
        Some((self.surface.clone(), pos - local))
    }

    fn replay_position(&self, niri: &Niri) -> Option<Point<f64, Logical>> {
        let (origin, zoom, offset) = self.transform(niri)?;
        let pos = origin + (self.local + offset).upscale(zoom);
        let target = niri.overview_preview_target(pos)?;
        (target.window == self.window && target.surface == self.surface).then_some(pos)
    }
}

pub struct OverviewClientGrab {
    start: GrabStartData<State>,
    target: PreviewTarget,
    phase: Phase,
    deadline: Duration,
    serial: Serial,
    down: bool,
    delivered: bool,
    cancel_requested: Cell<bool>,
}

impl OverviewClientGrab {
    pub fn start(data: &mut State, target: PreviewTarget, event: &ButtonEvent, defer: bool) {
        let pointer = data.niri.seat.get_pointer().unwrap();
        let location = pointer.current_location();
        let delay = if defer {
            DOUBLE_CLICK_TIME
        } else {
            Duration::ZERO
        };
        let deadline = get_monotonic_time() + delay;
        let grab = Self {
            // Own dead-surface cancellation ourselves. Smithay otherwise replaces the grab
            // with DefaultGrab before we can swallow the physical release.
            start: GrabStartData {
                focus: None,
                button: LEFT,
                location,
            },
            target,
            phase: Phase::FirstDown,
            deadline,
            serial: event.serial,
            down: true,
            delivered: false,
            cancel_requested: Cell::new(false),
        };
        let serial = event.serial;
        pointer.set_grab(data, grab, serial, Focus::Keep);
        // Record the physical button in Smithay without forwarding it to the client yet.
        pointer.button(data, event);
        pointer.frame(data);
        if !defer {
            return;
        }
        // A stale timer cannot affect a later grab, including one on the same window.
        data.niri
            .event_loop
            .insert_source(
                Timer::from_duration(DOUBLE_CLICK_TIME),
                move |_, _, data| {
                    let pointer = data.niri.seat.get_pointer().unwrap();
                    let ours = pointer
                        .with_grab(|_, grab| {
                            grab.as_any()
                                .downcast_ref::<Self>()
                                .is_some_and(|grab| grab.serial == serial)
                        })
                        .unwrap_or(false);
                    if ours {
                        pointer.frame(data);
                    }
                    TimeoutAction::Drop
                },
            )
            .expect("error inserting overview click timer");
    }

    pub fn is_active(data: &State) -> bool {
        data.niri
            .seat
            .get_pointer()
            .unwrap()
            .with_grab(|_, grab| grab.as_any().is::<Self>())
            .unwrap_or(false)
    }

    pub fn is_waiting(data: &State) -> bool {
        data.niri
            .seat
            .get_pointer()
            .unwrap()
            .with_grab(|_, grab| {
                grab.as_any()
                    .downcast_ref::<Self>()
                    .is_some_and(|g| !g.down)
            })
            .unwrap_or(false)
    }

    /// Called before a compositor action takes ownership (including configured wheel binds).
    pub fn cancel_current(data: &mut State) {
        let pointer = data.niri.seat.get_pointer().unwrap();
        let ours = pointer
            .with_grab(|_, grab| {
                if let Some(grab) = grab.as_any().downcast_ref::<Self>() {
                    grab.cancel_requested.set(true);
                    true
                } else {
                    false
                }
            })
            .unwrap_or(false);
        if ours {
            pointer.frame(data);
        }
    }

    fn event(state: ButtonState) -> ButtonEvent {
        ButtonEvent {
            button: LEFT,
            state,
            serial: SERIAL_COUNTER.next_serial(),
            time: get_monotonic_time().as_millis() as u32,
        }
    }

    fn finish(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>) {
        self.phase = Phase::Cancelled;
        // Secondary buttons were swallowed too. Drain every physical release before returning
        // to DefaultGrab, or a release could reach a client which never received its press.
        if !handle.current_pressed().is_empty() {
            return;
        }
        let event = Self::event(ButtonState::Released);
        handle.unset_grab(self, data, event.serial, event.time, true);
    }

    fn release(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>) {
        if self.delivered {
            handle.button(data, &Self::event(ButtonState::Released));
            handle.frame(data);
            self.delivered = false;
        }
    }

    fn cancel(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>) {
        self.release(data, handle);
        self.phase = Phase::Cancelled;
        if !self.down {
            self.finish(data, handle);
        }
    }

    fn commit(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>) -> bool {
        if self.target.replay_position(&data.niri).is_none() {
            self.cancel(data, handle);
            return false;
        }
        let current = handle.current_location();
        let event = Self::event(ButtonState::Pressed);
        // Deliver at the saved client coordinate, not at the later pointer position.
        handle.motion(
            data,
            Some(self.target.focus(current)),
            &MotionEvent {
                location: current,
                serial: event.serial,
                time: event.time,
            },
        );
        handle.button(data, &event);
        handle.frame(data);
        self.delivered = true;
        self.phase = Phase::Drag;
        if !self.down {
            self.release(data, handle);
            self.finish(data, handle);
            return false;
        }
        true
    }

    fn activate(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>) {
        // Same activation/closing path as MoveGrab::on_ungrab, with no client buttons delivered.
        let layout = &mut data.niri.layout;
        let destination = layout.workspaces().find_map(|(mon, idx, ws)| {
            ws.windows()
                .any(|w| w.window == self.target.window)
                .then(|| (mon.map(|m| m.output().clone()), idx))
        });
        if let Some((Some(output), idx)) = destination {
            layout.activate_window(&self.target.window);
            layout.focus_output(&output);
            layout.toggle_overview_to_workspace(idx);
            data.niri.queue_redraw_all();
        }
        self.finish(data, handle);
    }
}

fn moved(a: Point<f64, Logical>, b: Point<f64, Logical>) -> bool {
    let d = a - b;
    d.x * d.x + d.y * d.y >= SLOP * SLOP
}

impl PointerGrab<State> for OverviewClientGrab {
    fn motion(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        _focus: Option<(<State as SeatHandler>::PointerFocus, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        if self.phase == Phase::Cancelled {
            handle.motion(data, None, event);
            return;
        }
        let Some(focus) = self.target.mapped_focus(&data.niri, event.location) else {
            self.cancel(data, handle);
            handle.motion(data, None, event);
            return;
        };
        // Update the physical pointer even if committing a released click ends this grab.
        handle.motion(data, Some(focus.clone()), event);
        if self.phase != Phase::Drag && moved(event.location, self.start.location) {
            if !self.commit(data, handle) {
                return;
            }
        }
        handle.motion(data, Some(focus), event);
    }

    fn relative_motion(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        _focus: Option<(<State as SeatHandler>::PointerFocus, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        if self.phase == Phase::Cancelled {
            return;
        }
        if let Some((_, zoom, _)) = self.target.transform(&data.niri) {
            handle.relative_motion(
                data,
                None,
                &RelativeMotionEvent {
                    delta: event.delta.downscale(zoom),
                    delta_unaccel: event.delta_unaccel.downscale(zoom),
                    utime: event.utime,
                },
            );
        }
    }

    fn button(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &ButtonEvent,
    ) {
        if event.button != LEFT {
            if matches!(
                self.phase,
                Phase::FirstDown | Phase::Waiting | Phase::SecondDown
            ) {
                self.cancel(data, handle);
            } else if self.phase == Phase::Cancelled {
                self.finish(data, handle);
            }
            return;
        }
        self.down = event.state == ButtonState::Pressed;
        if self.target.transform(&data.niri).is_none() {
            self.cancel(data, handle);
            return;
        }
        match (self.phase, event.state) {
            (Phase::FirstDown, ButtonState::Released) => self.phase = Phase::Waiting,
            (Phase::Waiting, ButtonState::Pressed) => {
                let pos = handle.current_location();
                let unmodified = super::modifiers_from_state(
                    data.niri.seat.get_keyboard().unwrap().modifier_state(),
                )
                .is_empty();
                let target = data
                    .niri
                    .overview_preview_target(pos)
                    .filter(|t| t.window == self.target.window);
                if get_monotonic_time() < self.deadline
                    && unmodified
                    && target.is_some()
                    && !moved(pos, self.start.location)
                {
                    self.target = target.unwrap();
                    self.start.location = pos;
                    self.phase = Phase::SecondDown;
                } else {
                    self.cancel(data, handle);
                }
            }
            (Phase::SecondDown, ButtonState::Released) => self.activate(data, handle),
            (Phase::Drag, ButtonState::Released) => {
                self.release(data, handle);
                self.finish(data, handle);
            }
            (Phase::Cancelled, ButtonState::Released) => self.finish(data, handle),
            _ => (),
        }
    }

    fn axis(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        details: AxisFrame,
    ) {
        // A wheel gesture takes ownership: discard any ambiguous click rather than replay it
        // into content which scrolling is about to change.
        if self.phase != Phase::Drag {
            self.cancel(data, handle);
        }
        if self.target.transform(&data.niri).is_some() {
            handle.axis(data, details);
        }
    }

    fn frame(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>) {
        if self.cancel_requested.replace(false)
            || (self.phase != Phase::Cancelled && self.target.transform(&data.niri).is_none())
        {
            self.cancel(data, handle);
        } else if matches!(self.phase, Phase::FirstDown | Phase::Waiting)
            && get_monotonic_time() >= self.deadline
        {
            self.commit(data, handle);
        }
        if self.phase == Phase::Drag {
            let location = handle.current_location();
            if let Some(focus) = self.target.mapped_focus(&data.niri, location) {
                if handle.current_focus().as_ref() != Some(&focus) {
                    handle.motion(
                        data,
                        Some(focus),
                        &MotionEvent {
                            location,
                            serial: SERIAL_COUNTER.next_serial(),
                            time: get_monotonic_time().as_millis() as u32,
                        },
                    );
                }
            }
        }
        handle.frame(data);
    }

    fn gesture_swipe_begin(
        &mut self,
        _: &mut State,
        _: &mut PointerInnerHandle<'_, State>,
        _: &GestureSwipeBeginEvent,
    ) {
    }
    fn gesture_swipe_update(
        &mut self,
        _: &mut State,
        _: &mut PointerInnerHandle<'_, State>,
        _: &GestureSwipeUpdateEvent,
    ) {
    }
    fn gesture_swipe_end(
        &mut self,
        _: &mut State,
        _: &mut PointerInnerHandle<'_, State>,
        _: &GestureSwipeEndEvent,
    ) {
    }
    fn gesture_pinch_begin(
        &mut self,
        _: &mut State,
        _: &mut PointerInnerHandle<'_, State>,
        _: &GesturePinchBeginEvent,
    ) {
    }
    fn gesture_pinch_update(
        &mut self,
        _: &mut State,
        _: &mut PointerInnerHandle<'_, State>,
        _: &GesturePinchUpdateEvent,
    ) {
    }
    fn gesture_pinch_end(
        &mut self,
        _: &mut State,
        _: &mut PointerInnerHandle<'_, State>,
        _: &GesturePinchEndEvent,
    ) {
    }
    fn gesture_hold_begin(
        &mut self,
        _: &mut State,
        _: &mut PointerInnerHandle<'_, State>,
        _: &GestureHoldBeginEvent,
    ) {
    }
    fn gesture_hold_end(
        &mut self,
        _: &mut State,
        _: &mut PointerInnerHandle<'_, State>,
        _: &GestureHoldEndEvent,
    ) {
    }
    fn start_data(&self) -> &GrabStartData<State> {
        &self.start
    }
    fn unset(&mut self, data: &mut State) {
        // Replacement by another compositor/client grab must balance the client press. Calling
        // PointerHandle here would deadlock: Smithay already holds its pointer mutex.
        if self.delivered {
            let seat = data.niri.seat.clone();
            self.target
                .surface
                .button(&seat, data, &Self::event(ButtonState::Released));
            self.target.surface.frame(&seat, data);
            self.delivered = false;
        }
    }
}
