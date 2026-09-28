use smithay::output::Output;
use smithay::utils::{Logical, Point, Rectangle};

use super::super::workspace::WorkspaceId;
use super::*;

const OVERVIEW_ANIMATION_MS: i32 = 133;
const HALF_PROGRESS: f64 = (OVERVIEW_ANIMATION_MS / 2) as f64 / OVERVIEW_ANIMATION_MS as f64;
const QUARTER_PROGRESS: f64 = (OVERVIEW_ANIMATION_MS / 4) as f64 / OVERVIEW_ANIMATION_MS as f64;
const POSITION_EPSILON: f64 = 2.;
const ZOOM_EPSILON: f64 = 1e-6;

fn layout_with_three_populated_workspaces() -> Layout<TestWindow> {
    check_ops_with_options(
        Options::default(),
        [
            Op::AddOutput(1),
            Op::AddWindow {
                params: TestWindowParams::new(1),
            },
            Op::FocusWorkspaceDown,
            Op::AddWindow {
                params: TestWindowParams::new(2),
            },
            Op::FocusWorkspaceDown,
            Op::AddWindow {
                params: TestWindowParams::new(3),
            },
            Op::CompleteAnimations,
        ],
    )
}

fn workspace_geometry(
    layout: &Layout<TestWindow>,
    output: &Output,
    workspace_id: WorkspaceId,
) -> Rectangle<f64, Logical> {
    let monitor = layout.monitor_for_output(output).unwrap();
    monitor
        .workspaces
        .iter()
        .zip(monitor.workspaces_render_geo())
        .find(|(workspace, _)| workspace.id() == workspace_id)
        .map(|(_, geometry)| geometry)
        .unwrap_or_else(|| panic!("workspace {workspace_id:?} is missing"))
}

fn workspace_center(geometry: Rectangle<f64, Logical>) -> Point<f64, Logical> {
    geometry.loc + geometry.size.to_point().upscale(0.5)
}

fn populated_workspace_ids(layout: &Layout<TestWindow>, output: &Output) -> Vec<WorkspaceId> {
    layout
        .monitor_for_output(output)
        .unwrap()
        .workspaces
        .iter()
        .filter(|workspace| workspace.has_windows())
        .map(|workspace| workspace.id())
        .collect()
}

fn advance_and_sync(layout: &mut Layout<TestWindow>, output: &Output, msec_delta: i32) {
    Op::AdvanceAnimations { msec_delta }.apply(layout);
    layout.update_render_elements(Some(output));
}

fn assert_close(actual: f64, expected: f64, epsilon: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= epsilon,
        "{what}: expected {expected}, got {actual}"
    );
}

fn assert_geometry_close(
    actual: Rectangle<f64, Logical>,
    expected: Rectangle<f64, Logical>,
    what: &str,
) {
    assert_close(
        actual.loc.x,
        expected.loc.x,
        POSITION_EPSILON,
        &format!("{what}.x"),
    );
    assert_close(
        actual.loc.y,
        expected.loc.y,
        POSITION_EPSILON,
        &format!("{what}.y"),
    );
    assert_close(
        actual.size.w,
        expected.size.w,
        POSITION_EPSILON,
        &format!("{what}.width"),
    );
    assert_close(
        actual.size.h,
        expected.size.h,
        POSITION_EPSILON,
        &format!("{what}.height"),
    );
}

fn assert_linear_sample(
    start: Rectangle<f64, Logical>,
    sample: Rectangle<f64, Logical>,
    end: Rectangle<f64, Logical>,
    fraction: f64,
    what: &str,
) {
    let expected = Rectangle::new(
        start.loc + (end.loc - start.loc).upscale(fraction),
        Size::from((
            start.size.w + (end.size.w - start.size.w) * fraction,
            start.size.h + (end.size.h - start.size.h) * fraction,
        )),
    );
    assert_geometry_close(sample, expected, what);
}

#[test]
fn fit_overview_zoom_in_to_another_column_moves_window_linearly() {
    for across_workspaces in [false, true] {
        let mut layout = check_ops([
            Op::AddOutput(1),
            Op::AddWindow {
                params: TestWindowParams::new(1),
            },
            Op::AddWindow {
                params: TestWindowParams::new(2),
            },
            Op::SetWindowWidth {
                id: Some(1),
                change: SizeChange::SetProportion(100.),
            },
            Op::SetWindowWidth {
                id: Some(2),
                change: SizeChange::SetProportion(100.),
            },
            Op::Communicate(1),
            Op::Communicate(2),
        ]);
        let output = layout.outputs().next().unwrap().clone();
        Op::CompleteAnimations.apply(&mut layout);
        let normal_target = layout.window_visual_rect(&output, &2).unwrap();
        layout.activate_window(&1);
        Op::CompleteAnimations.apply(&mut layout);
        if across_workspaces {
            Op::FocusWorkspaceDown.apply(&mut layout);
            Op::AddWindow {
                params: TestWindowParams::new(3),
            }
            .apply(&mut layout);
            Op::Communicate(3).apply(&mut layout);
            Op::CompleteAnimations.apply(&mut layout);
        }
        layout.open_overview();
        Op::CompleteAnimations.apply(&mut layout);
        let start = layout.window_visual_rect(&output, &2).unwrap();
        layout.activate_window(&2);
        layout.close_overview();
        layout.update_render_elements(Some(&output));
        // The activation configure commits while the zoom is running.
        layout.update_window(&2, None);
        assert_geometry_close(
            layout.window_visual_rect(&output, &2).unwrap(),
            start,
            "selection starts at shown window",
        );
        let sample_ms = OVERVIEW_ANIMATION_MS / 2;
        advance_and_sync(&mut layout, &output, sample_ms);
        let middle = layout.window_visual_rect(&output, &2).unwrap();
        advance_and_sync(&mut layout, &output, OVERVIEW_ANIMATION_MS - sample_ms);
        let end = layout.window_visual_rect(&output, &2).unwrap();
        assert_linear_sample(
            start,
            middle,
            end,
            f64::from(sample_ms) / f64::from(OVERVIEW_ANIMATION_MS),
            "selected window zoom",
        );
        advance_and_sync(&mut layout, &output, 500);
        assert_geometry_close(
            layout.window_visual_rect(&output, &2).unwrap(),
            end,
            "no focus-scroll tail after zoom",
        );
        assert_geometry_close(end, normal_target, "selected column aligned");
    }
}

#[test]
fn fit_overview_open_close_camera_is_linear_for_top_and_bottom_focus() {
    for focused_idx in [0, 2] {
        let mut layout = layout_with_three_populated_workspaces();
        Op::FocusWorkspace(focused_idx).apply(&mut layout);
        Op::CompleteAnimations.apply(&mut layout);

        let output = layout.outputs().next().unwrap().clone();
        let focused_workspace =
            layout.monitor_for_output(&output).unwrap().workspaces[focused_idx].id();
        let normal_geometry = workspace_geometry(&layout, &output, focused_workspace);
        let normal_zoom = layout.monitor_for_output(&output).unwrap().overview_zoom();

        Op::ToggleOverview.apply(&mut layout);
        layout.update_render_elements(Some(&output));
        let open_start = workspace_geometry(&layout, &output, focused_workspace);
        let open_start_zoom = layout.monitor_for_output(&output).unwrap().overview_zoom();
        advance_and_sync(&mut layout, &output, OVERVIEW_ANIMATION_MS / 2);
        let open_mid = workspace_geometry(&layout, &output, focused_workspace);
        let open_mid_zoom = layout.monitor_for_output(&output).unwrap().overview_zoom();
        advance_and_sync(
            &mut layout,
            &output,
            OVERVIEW_ANIMATION_MS - OVERVIEW_ANIMATION_MS / 2,
        );
        let open_end = workspace_geometry(&layout, &output, focused_workspace);
        let open_end_zoom = layout.monitor_for_output(&output).unwrap().overview_zoom();

        assert_close(
            open_start_zoom,
            normal_zoom,
            ZOOM_EPSILON,
            "open start zoom",
        );
        assert_close(open_start_zoom, 1., ZOOM_EPSILON, "open start zoom");
        assert_close(
            open_mid_zoom,
            open_start_zoom + (open_end_zoom - open_start_zoom) * HALF_PROGRESS,
            ZOOM_EPSILON,
            "open midpoint zoom",
        );
        assert!(open_end_zoom < open_mid_zoom && open_mid_zoom < open_start_zoom);
        assert_linear_sample(
            open_start,
            open_mid,
            open_end,
            HALF_PROGRESS,
            "open midpoint geometry",
        );

        let open_start_center = workspace_center(open_start);
        let open_mid_center = workspace_center(open_mid);
        let open_end_center = workspace_center(open_end);
        assert_close(
            open_start_center.x,
            640.,
            POSITION_EPSILON,
            "open start center x",
        );
        assert_close(
            open_start_center.y,
            360.,
            POSITION_EPSILON,
            "open start center y",
        );
        assert_close(
            open_mid_center.y,
            open_start_center.y + (open_end_center.y - open_start_center.y) * HALF_PROGRESS,
            POSITION_EPSILON,
            "open midpoint center y",
        );
        assert!(
            open_end_center.y < open_mid_center.y && open_mid_center.y < open_start_center.y
                || open_start_center.y < open_mid_center.y && open_mid_center.y < open_end_center.y,
            "open camera reversed for focused workspace {focused_idx}: start={}, mid={}, end={}",
            open_start_center.y,
            open_mid_center.y,
            open_end_center.y
        );

        Op::ToggleOverview.apply(&mut layout);
        layout.update_render_elements(Some(&output));
        let close_start = workspace_geometry(&layout, &output, focused_workspace);
        let close_start_zoom = layout.monitor_for_output(&output).unwrap().overview_zoom();
        assert_geometry_close(close_start, open_end, "close starts at open endpoint");
        assert_close(
            close_start_zoom,
            open_end_zoom,
            ZOOM_EPSILON,
            "close start zoom",
        );
        advance_and_sync(&mut layout, &output, OVERVIEW_ANIMATION_MS / 2);
        let close_mid = workspace_geometry(&layout, &output, focused_workspace);
        let close_mid_zoom = layout.monitor_for_output(&output).unwrap().overview_zoom();
        advance_and_sync(
            &mut layout,
            &output,
            OVERVIEW_ANIMATION_MS - OVERVIEW_ANIMATION_MS / 2,
        );
        let close_end = workspace_geometry(&layout, &output, focused_workspace);
        let close_end_zoom = layout.monitor_for_output(&output).unwrap().overview_zoom();

        assert_geometry_close(close_end, normal_geometry, "close endpoint");
        assert_close(
            close_end_zoom,
            normal_zoom,
            ZOOM_EPSILON,
            "close endpoint zoom",
        );
        assert_close(
            close_mid_zoom,
            close_start_zoom + (close_end_zoom - close_start_zoom) * HALF_PROGRESS,
            ZOOM_EPSILON,
            "close midpoint zoom",
        );
        assert_linear_sample(
            close_start,
            close_mid,
            close_end,
            HALF_PROGRESS,
            "close midpoint geometry",
        );
    }
}

#[test]
fn fit_overview_workspace_addition_and_removal_interpolate_by_workspace_id() {
    let mut layout = layout_with_three_populated_workspaces();
    Op::FocusWorkspace(0).apply(&mut layout);
    Op::CompleteAnimations.apply(&mut layout);
    Op::ToggleOverview.apply(&mut layout);
    Op::CompleteAnimations.apply(&mut layout);

    let output = layout.outputs().next().unwrap().clone();
    let existing_ids = populated_workspace_ids(&layout, &output);
    assert_eq!(existing_ids.len(), 3);
    // Focus the trailing empty workspace before populating it. In Fit mode this does not move the
    // displayed stack, and completing the switch keeps its animation out of the mutation sample.
    Op::FocusWorkspace(3).apply(&mut layout);
    Op::CompleteAnimations.apply(&mut layout);
    layout.update_render_elements(Some(&output));
    let old_geometry: Vec<_> = existing_ids
        .iter()
        .map(|id| (*id, workspace_geometry(&layout, &output, *id)))
        .collect();
    let old_zoom = layout.monitor_for_output(&output).unwrap().overview_zoom();

    Op::AddWindow {
        params: TestWindowParams::new(4),
    }
    .apply(&mut layout);
    layout.verify_invariants();
    layout.update_render_elements(Some(&output));

    let added_workspace = layout
        .monitor_for_output(&output)
        .unwrap()
        .workspaces
        .iter()
        .find(|workspace| workspace.has_window(&4))
        .unwrap()
        .id();
    let add_start: Vec<_> = existing_ids
        .iter()
        .map(|id| (*id, workspace_geometry(&layout, &output, *id)))
        .collect();
    for (id, geometry) in &old_geometry {
        assert_geometry_close(
            *geometry,
            workspace_geometry(&layout, &output, *id),
            "add start",
        );
    }
    assert_close(
        layout.monitor_for_output(&output).unwrap().overview_zoom(),
        old_zoom,
        ZOOM_EPSILON,
        "add start zoom",
    );

    advance_and_sync(&mut layout, &output, OVERVIEW_ANIMATION_MS / 4);
    let add_quarter: Vec<_> = existing_ids
        .iter()
        .map(|id| (*id, workspace_geometry(&layout, &output, *id)))
        .collect();
    advance_and_sync(&mut layout, &output, OVERVIEW_ANIMATION_MS / 4);
    let add_half: Vec<_> = existing_ids
        .iter()
        .map(|id| (*id, workspace_geometry(&layout, &output, *id)))
        .collect();
    advance_and_sync(
        &mut layout,
        &output,
        OVERVIEW_ANIMATION_MS - 2 * (OVERVIEW_ANIMATION_MS / 4),
    );
    let add_end: Vec<_> = existing_ids
        .iter()
        .map(|id| (*id, workspace_geometry(&layout, &output, *id)))
        .collect();

    for ((id, start), (_, quarter)) in add_start.iter().zip(&add_quarter) {
        let end = add_end.iter().find(|(end_id, _)| end_id == id).unwrap().1;
        assert_linear_sample(*start, *quarter, end, QUARTER_PROGRESS, "add quarter");
    }
    for ((id, start), (_, half)) in add_start.iter().zip(&add_half) {
        let end = add_end.iter().find(|(end_id, _)| end_id == id).unwrap().1;
        assert_linear_sample(*start, *half, end, 2. * QUARTER_PROGRESS, "add midpoint");
    }
    assert!(layout
        .monitor_for_output(&output)
        .unwrap()
        .workspaces_with_render_geo()
        .any(|(workspace, geometry)| workspace.id() == added_workspace && geometry.size.h > 0.));
    assert_eq!(
        layout
            .monitor_for_output(&output)
            .unwrap()
            .workspaces_with_render_geo()
            .count(),
        4
    );

    // Move focus away from the now-populated trailing workspace so closing its only window removes
    // that workspace rather than retaining it as the active empty drop target.
    Op::FocusWorkspace(0).apply(&mut layout);
    Op::CompleteAnimations.apply(&mut layout);
    layout.update_render_elements(Some(&output));
    let remove_start: Vec<_> = existing_ids
        .iter()
        .map(|id| (*id, workspace_geometry(&layout, &output, *id)))
        .collect();
    Op::CloseWindow(4).apply(&mut layout);
    layout.verify_invariants();
    layout.update_render_elements(Some(&output));

    for (id, geometry) in &remove_start {
        assert_geometry_close(
            *geometry,
            workspace_geometry(&layout, &output, *id),
            "remove start",
        );
    }
    advance_and_sync(&mut layout, &output, OVERVIEW_ANIMATION_MS / 4);
    let remove_quarter: Vec<_> = existing_ids
        .iter()
        .map(|id| (*id, workspace_geometry(&layout, &output, *id)))
        .collect();
    advance_and_sync(&mut layout, &output, OVERVIEW_ANIMATION_MS / 4);
    let remove_half: Vec<_> = existing_ids
        .iter()
        .map(|id| (*id, workspace_geometry(&layout, &output, *id)))
        .collect();
    advance_and_sync(
        &mut layout,
        &output,
        OVERVIEW_ANIMATION_MS - 2 * (OVERVIEW_ANIMATION_MS / 4),
    );
    let remove_end: Vec<_> = existing_ids
        .iter()
        .map(|id| (*id, workspace_geometry(&layout, &output, *id)))
        .collect();

    for ((id, start), (_, quarter)) in remove_start.iter().zip(&remove_quarter) {
        let end = remove_end
            .iter()
            .find(|(end_id, _)| end_id == id)
            .unwrap()
            .1;
        assert_linear_sample(*start, *quarter, end, QUARTER_PROGRESS, "remove quarter");
    }
    for ((id, start), (_, half)) in remove_start.iter().zip(&remove_half) {
        let end = remove_end
            .iter()
            .find(|(end_id, _)| end_id == id)
            .unwrap()
            .1;
        assert_linear_sample(*start, *half, end, 2. * QUARTER_PROGRESS, "remove midpoint");
    }
    assert_eq!(
        layout
            .monitor_for_output(&output)
            .unwrap()
            .workspaces_with_render_geo()
            .count(),
        3
    );
}

#[test]
fn fit_overview_interrupted_open_close_starts_at_current_geometry() {
    let mut layout = layout_with_three_populated_workspaces();
    Op::FocusWorkspace(0).apply(&mut layout);
    Op::CompleteAnimations.apply(&mut layout);
    let output = layout.outputs().next().unwrap().clone();
    let workspace_ids = populated_workspace_ids(&layout, &output);
    let normal_geometry: Vec<_> = workspace_ids
        .iter()
        .map(|id| (*id, workspace_geometry(&layout, &output, *id)))
        .collect();

    Op::ToggleOverview.apply(&mut layout);
    layout.update_render_elements(Some(&output));
    advance_and_sync(&mut layout, &output, OVERVIEW_ANIMATION_MS / 2);
    let open_mid: Vec<_> = workspace_ids
        .iter()
        .map(|id| (*id, workspace_geometry(&layout, &output, *id)))
        .collect();
    let open_mid_zoom = layout.monitor_for_output(&output).unwrap().overview_zoom();

    // Reversing the animation before it finishes must preserve the currently displayed camera
    // rather than restarting from either endpoint.
    Op::ToggleOverview.apply(&mut layout);
    layout.update_render_elements(Some(&output));
    let close_start_zoom = layout.monitor_for_output(&output).unwrap().overview_zoom();
    assert_close(
        close_start_zoom,
        open_mid_zoom,
        ZOOM_EPSILON,
        "interrupted close start zoom",
    );
    for (id, geometry) in &open_mid {
        assert_geometry_close(
            *geometry,
            workspace_geometry(&layout, &output, *id),
            "interrupted close start",
        );
    }

    advance_and_sync(&mut layout, &output, OVERVIEW_ANIMATION_MS);
    for (id, geometry) in &normal_geometry {
        assert_geometry_close(
            *geometry,
            workspace_geometry(&layout, &output, *id),
            "interrupted close endpoint",
        );
    }
}
