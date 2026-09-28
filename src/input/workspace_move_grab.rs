use smithay::input::pointer::{
    AxisFrame, ButtonEvent, CursorIcon, CursorImageStatus, GestureHoldBeginEvent,
    GestureHoldEndEvent, GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent,
    GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent,
    GrabStartData as PointerGrabStartData, MotionEvent, PointerGrab, PointerInnerHandle,
    RelativeMotionEvent,
};
use smithay::input::SeatHandler;
use smithay::output::Output;
use smithay::utils::{Logical, Point};

use crate::layout::workspace::WorkspaceId;
use crate::niri::State;

pub struct WorkspaceMoveGrab {
    start_data: PointerGrabStartData<State>,
    output: Output,
    workspace: WorkspaceId,
    dragging: bool,
}

impl WorkspaceMoveGrab {
    pub fn new(
        start_data: PointerGrabStartData<State>,
        output: Output,
        workspace: WorkspaceId,
    ) -> Self {
        Self {
            start_data,
            output,
            workspace,
            dragging: false,
        }
    }

    fn motion_to(&mut self, data: &mut State, location: Point<f64, Logical>) {
        let Some((output, pos_within_output)) = data.niri.output_under(location) else {
            return;
        };
        if output != &self.output {
            return;
        }

        if !self.dragging {
            let delta = location - self.start_data.location;
            if delta.x * delta.x + delta.y * delta.y < 8. * 8. {
                return;
            }

            let start_y = data
                .niri
                .output_under(self.start_data.location)
                .filter(|(start_output, _)| *start_output == &self.output)
                .map_or(pos_within_output.y, |(_, pos)| pos.y);
            let started = data
                .niri
                .layout
                .monitor_for_output_mut(&self.output)
                .is_some_and(|monitor| monitor.begin_workspace_drag(self.workspace, start_y));
            if !started {
                return;
            }

            self.dragging = true;
            data.niri
                .cursor_manager
                .set_cursor_image(CursorImageStatus::Named(CursorIcon::Grabbing));
        }

        if let Some(monitor) = data.niri.layout.monitor_for_output_mut(&self.output) {
            monitor.update_workspace_drag(self.workspace, pos_within_output.y);
        }
        data.niri.queue_redraw_all();
    }

    fn on_ungrab(&mut self, data: &mut State) {
        data.niri
            .cursor_manager
            .set_cursor_image(CursorImageStatus::default_named());

        if !self.dragging {
            if let Some((workspace_idx, _)) = data.niri.layout.find_workspace_by_id(self.workspace)
            {
                data.niri.layout.focus_output(&self.output);
                data.niri.layout.toggle_overview_to_workspace(workspace_idx);
            }
        } else if let Some(monitor) = data.niri.layout.monitor_for_output_mut(&self.output) {
            monitor.end_workspace_drag(self.workspace);
        }

        data.niri.queue_redraw_all();
    }
}

impl PointerGrab<State> for WorkspaceMoveGrab {
    fn motion(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        _focus: Option<(<State as SeatHandler>::PointerFocus, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        handle.motion(data, None, event);
        self.motion_to(data, event.location);
    }

    fn relative_motion(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        _focus: Option<(<State as SeatHandler>::PointerFocus, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(data, None, event);
    }

    fn button(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);
        if !handle.current_pressed().contains(&self.start_data.button) {
            handle.unset_grab(self, data, event.serial, event.time, true);
        }
    }

    fn axis(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        details: AxisFrame,
    ) {
        handle.axis(data, details);
    }

    fn frame(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>) {
        handle.frame(data);
    }

    fn gesture_swipe_begin(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureSwipeBeginEvent,
    ) {
        handle.gesture_swipe_begin(data, event);
    }

    fn gesture_swipe_update(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureSwipeUpdateEvent,
    ) {
        handle.gesture_swipe_update(data, event);
    }

    fn gesture_swipe_end(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureSwipeEndEvent,
    ) {
        handle.gesture_swipe_end(data, event);
    }

    fn gesture_pinch_begin(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GesturePinchBeginEvent,
    ) {
        handle.gesture_pinch_begin(data, event);
    }

    fn gesture_pinch_update(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GesturePinchUpdateEvent,
    ) {
        handle.gesture_pinch_update(data, event);
    }

    fn gesture_pinch_end(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GesturePinchEndEvent,
    ) {
        handle.gesture_pinch_end(data, event);
    }

    fn gesture_hold_begin(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureHoldBeginEvent,
    ) {
        handle.gesture_hold_begin(data, event);
    }

    fn gesture_hold_end(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureHoldEndEvent,
    ) {
        handle.gesture_hold_end(data, event);
    }

    fn start_data(&self) -> &PointerGrabStartData<State> {
        &self.start_data
    }

    fn unset(&mut self, data: &mut State) {
        self.on_ungrab(data);
    }
}
