use smithay::output::Output;
use smithay::utils::Point;

use super::*;

fn layout_with_preview_window() -> Layout<TestWindow> {
    check_ops([
        Op::AddOutput(1),
        Op::AddWindow {
            params: TestWindowParams::new(1),
        },
        Op::Communicate(1),
        Op::CompleteAnimations,
    ])
}

fn enable_surface_input(layout: &mut Layout<TestWindow>, output: &Output) {
    let window = layout
        .monitor_for_output_mut(output)
        .unwrap()
        .workspaces
        .iter_mut()
        .flat_map(Workspace::windows_mut)
        .find(|window| window.id() == &1)
        .unwrap();
    window.0.input_region.set(true);
    window.0.buffer_loc.set(Point::from((13, 7)));
}

fn advance_and_sync(layout: &mut Layout<TestWindow>, output: &Output, msec_delta: i32) {
    Op::AdvanceAnimations { msec_delta }.apply(layout);
    layout.update_render_elements(Some(output));
}

#[test]
fn fit_overview_hit_uses_displayed_buffer_origin_during_camera_animation() {
    let mut layout = layout_with_preview_window();
    let output = layout.outputs().next().unwrap().clone();
    enable_surface_input(&mut layout, &output);
    layout.open_overview();
    layout.update_render_elements(Some(&output));
    advance_and_sync(&mut layout, &output, 66);

    let visual = layout.window_visual_rect(&output, &1).unwrap();
    let (origin, zoom) = layout.overview_window_transform(&output, &1).unwrap();
    let buffer_offset = Point::from((13. * zoom, 7. * zoom));
    let expected_origin = visual.loc + buffer_offset;
    assert!((origin.x - expected_origin.x).abs() < 1.);
    assert!((origin.y - expected_origin.y).abs() < 1.);
    assert!(zoom > 0. && zoom < 1.);

    let point = expected_origin + visual.size.to_point().upscale(0.5);
    let (window, hit_origin, hit_zoom) = layout.overview_window_hit(&output, point).unwrap();
    assert_eq!(window.id(), &1);
    assert!((hit_origin.x - origin.x).abs() < 1.);
    assert!((hit_origin.y - origin.y).abs() < 1.);
    assert_eq!(hit_zoom, zoom);
}

#[test]
fn fit_overview_mapping_rejects_clipped_and_closed_preview() {
    let mut layout = layout_with_preview_window();
    let output = layout.outputs().next().unwrap().clone();
    assert!(layout.overview_window_transform(&output, &1).is_none());

    layout.open_overview();
    Op::CompleteAnimations.apply(&mut layout);
    layout.update_render_elements(Some(&output));
    assert!(layout
        .overview_window_hit(&output, Point::from((-1., 0.)))
        .is_none());

    layout.close_overview();
    assert!(layout.overview_window_transform(&output, &1).is_none());
}

#[test]
fn fit_overview_mapping_rejects_snapshot_open_animation() {
    let mut layout = layout_with_preview_window();
    let output = layout.outputs().next().unwrap().clone();
    layout.open_overview();
    Op::CompleteAnimations.apply(&mut layout);
    layout.update_render_elements(Some(&output));

    layout.start_open_animation_for_window(&1);
    assert!(layout.overview_window_transform(&output, &1).is_none());
}

#[test]
fn overview_hit_does_not_claim_activation_only_tile_margin() {
    let mut options = Options::default();
    options.layout.border.off = false;
    options.layout.border.width = 4.;
    let mut layout = check_ops_with_options(
        options,
        [
            Op::AddOutput(1),
            Op::AddWindow {
                params: TestWindowParams::new(1),
            },
            Op::Communicate(1),
            Op::CompleteAnimations,
        ],
    );
    let output = layout.outputs().next().unwrap().clone();
    layout.open_overview();
    Op::CompleteAnimations.apply(&mut layout);
    layout.update_render_elements(Some(&output));

    let visual = layout.window_visual_rect(&output, &1).unwrap();
    let zoom = layout.monitor_for_output(&output).unwrap().overview_zoom();
    let point = visual.loc + Point::from((-3. * zoom, visual.size.h / 2.));
    assert!(layout.overview_window_hit(&output, point).is_none());
}
