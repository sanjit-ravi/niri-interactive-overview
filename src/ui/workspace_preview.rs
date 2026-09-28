use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gdk_pixbuf::Pixbuf;
use niri_config::Config;
use niri_ipc::WindowLayout;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::{
    MemoryRenderBuffer, MemoryRenderBufferRenderElement,
};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::Color32F;
use smithay::output::Output;
use smithay::utils::{Logical, Point, Rectangle, Size, Transform};

use crate::animation::{Animation, Clock, Curve};
use crate::layout::workspace::{Workspace, WorkspaceId};
use crate::layout::{Layout, LayoutElement};
use crate::niri_render_elements;
use crate::render_helpers::renderer::NiriRenderer;
use crate::render_helpers::solid_color::{SolidColorBuffer, SolidColorRenderElement};
use crate::render_helpers::RenderTarget;
use crate::utils::{output_size, with_toplevel_role};
use crate::window::mapped::MappedId;
use crate::window::Mapped;

const PANEL_RIGHT: f64 = 24.;
const PANEL_TOP: f64 = 24.;
const PANEL_PADDING: f64 = 14.;
const CONTENT_WIDTH: f64 = 336.;
const CONTENT_HEIGHT: f64 = CONTENT_WIDTH * 9. / 16.;
const WORKSPACE_GAP: f64 = 10.;
const WORKSPACE_INSET: f64 = 4.;
const WINDOW_GAP: f64 = 6.;
const FOCUS_OUTLINE: f64 = 2.;
const ICON_SIZE: f64 = 56.;
const ICON_RASTER_SIZE: i32 = 512;
const ICON_SOURCE_SIZE: f64 = ICON_RASTER_SIZE as f64;
const ICON_MARGIN: f64 = 8.;
const MAX_PANEL_HEIGHT: f64 = 0.48;
const PANEL_OPEN_MS: u64 = 110;
const PANEL_CLOSE_MS: u64 = 90;
const HIGHLIGHT_MORPH_MS: u64 = 140;

const fn premultiplied_color(r: f32, g: f32, b: f32, a: f32) -> Color32F {
    Color32F::new(r * a, g * a, b * a, a)
}

fn focus_colors(color: niri_config::Color) -> (Color32F, Color32F) {
    (
        premultiplied_color(color.r, color.g, color.b, 0.45),
        premultiplied_color(color.r, color.g, color.b, 1.),
    )
}

const WINDOW_ALPHA: f32 = 140. / 255.;
const WINDOW_COLOR: Color32F =
    premultiplied_color(143. / 255., 144. / 255., 153. / 255., WINDOW_ALPHA);
niri_render_elements! {
    WorkspacePreviewRenderElement<R> => {
        SolidColor = SolidColorRenderElement,
        AppIcon = MemoryRenderBufferRenderElement<R>,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreviewState {
    Hidden,
    Opening,
    Open,
    Closing,
}


#[derive(Debug, Clone, PartialEq)]
struct PreviewWindowRect<Id> {
    id: Id,
    app_id: Option<String>,
    title: Option<String>,
    rect: Rectangle<f64, Logical>,
    fullscreen: bool,
    tiled: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct PreviewWorkspaceRect<Id> {
    id: WorkspaceId,
    rect: Rectangle<f64, Logical>,
    interior: Rectangle<f64, Logical>,
    focus_view: Option<Rectangle<f64, Logical>>,
    windows: Vec<PreviewWindowRect<Id>>,
}

#[derive(Debug, Clone, PartialEq)]
struct PreviewDiagram<Id> {
    panel: Rectangle<f64, Logical>,
    workspaces: Vec<PreviewWorkspaceRect<Id>>,
}

#[derive(Debug, Clone)]
struct PreviewWindowInput<Id> {
    id: Id,
    app_id: Option<String>,
    title: Option<String>,
    layout: WindowLayout,
    position: Option<Point<f64, Logical>>,
    fullscreen: bool,
}

#[derive(Debug, Clone)]
struct PreviewWorkspaceInput<Id> {
    id: WorkspaceId,
    view_size: Size<f64, Logical>,
    view_pos: f64,
    target_view_pos: f64,
    windows: Vec<PreviewWindowInput<Id>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HighlightIdentity {
    workspace: WorkspaceId,
    windows: Vec<MappedId>,
}
#[derive(Clone)]
struct LoadedAppIcon {
    buffer: MemoryRenderBuffer,
    window_color: Color32F,
}

pub struct WorkspacePreview {
    state: PreviewState,
    output: Option<Output>,
    diagram: Option<PreviewDiagram<MappedId>>,
    window_buffers: Vec<SolidColorBuffer>,
    window_colors: Vec<Color32F>,
    icon_buffers: Vec<Option<MemoryRenderBuffer>>,
    icon_cache: HashMap<Option<String>, Option<LoadedAppIcon>>,
    highlight_buffer: RefCell<SolidColorBuffer>,
    highlight_outline_buffers: RefCell<[SolidColorBuffer; 4]>,
    highlight_rect: Rectangle<f64, Logical>,
    highlight_target: Rectangle<f64, Logical>,
    highlight_identity: Option<HighlightIdentity>,
    panel_animation: Option<Animation>,
    highlight_animation: Option<[Animation; 4]>,
    focus_fill_color: Color32F,
    focus_outline_color: Color32F,
    config: Rc<RefCell<Config>>,
    clock: Clock,
}

impl WorkspacePreview {
    pub fn new(clock: Clock, config: Rc<RefCell<Config>>) -> Self {
        let (focus_fill_color, focus_outline_color) =
            focus_colors(config.borrow().layout.focus_ring.active_color);
        Self {
            state: PreviewState::Hidden,
            output: None,
            diagram: None,
            window_buffers: Vec::new(),
            window_colors: Vec::new(),
            icon_buffers: Vec::new(),
            icon_cache: HashMap::new(),
            highlight_buffer: RefCell::new(SolidColorBuffer::new((0., 0.), focus_fill_color)),
            highlight_outline_buffers: RefCell::new(std::array::from_fn(|_| {
                SolidColorBuffer::new((0., 0.), focus_outline_color)
            })),
            highlight_rect: Rectangle::default(),
            highlight_target: Rectangle::default(),
            highlight_identity: None,
            panel_animation: None,
            highlight_animation: None,
            focus_fill_color,
            focus_outline_color,
            config,
            clock,
        }
    }
}

impl WorkspacePreview {
    pub fn open(&mut self, layout: &Layout<Mapped>, output: Output) {
        self.update_focus_colors();
        let output_changed = self.output.as_ref() != Some(&output);
        let was_hidden = self.state == PreviewState::Hidden;
        let was_closing = self.state == PreviewState::Closing;
        let panel_from = self.panel_progress();

        if !output_changed && !was_hidden && !was_closing {
            self.sync(layout);
            return;
        }

        self.output = Some(output.clone());
        if let Some((diagram, identity, target)) = build_preview(layout, &output) {
            self.apply_diagram(diagram, identity, target, false);
        } else {
            self.hide();
            return;
        }

        self.panel_animation = Some(Animation::ease(
            self.clock.clone(),
            panel_from,
            1.,
            0.,
            PANEL_OPEN_MS,
            Curve::EaseOutCubic,
        ));
        self.state = PreviewState::Opening;
    }

    pub fn sync(&mut self, layout: &Layout<Mapped>) {
        self.update_focus_colors();
        if self.state == PreviewState::Hidden {
            return;
        }
        let Some(output) = self.output.clone() else {
            return;
        };
        let Some((diagram, identity, target)) = build_preview(layout, &output) else {
            self.hide();
            return;
        };

        self.apply_diagram(diagram, identity, target, true);
    }

    fn update_focus_colors(&mut self) {
        let active_color = self.config.borrow().layout.focus_ring.active_color;
        let (fill, outline) = focus_colors(active_color);
        self.focus_fill_color = fill;
        self.focus_outline_color = outline;
    }

    pub fn close(&mut self) {
        if self.state == PreviewState::Hidden {
            return;
        }

        let panel_from = self.panel_progress();
        self.panel_animation = Some(Animation::ease(
            self.clock.clone(),
            panel_from,
            0.,
            0.,
            PANEL_CLOSE_MS,
            Curve::EaseOutCubic,
        ));
        self.state = PreviewState::Closing;
    }

    pub fn hide(&mut self) {
        self.state = PreviewState::Hidden;
        self.output = None;
        self.diagram = None;
        self.panel_animation = None;
        self.highlight_animation = None;
        self.highlight_identity = None;
    }

    pub fn advance_animations(&mut self) {
        if self
            .panel_animation
            .as_ref()
            .is_some_and(Animation::is_done)
        {
            self.panel_animation = None;
            match self.state {
                PreviewState::Opening => self.state = PreviewState::Open,
                PreviewState::Closing => self.hide(),
                PreviewState::Hidden | PreviewState::Open => {}
            }
        }

        if self
            .highlight_animation
            .as_ref()
            .is_some_and(|animations| animations.iter().all(Animation::is_done))
        {
            self.highlight_rect = self.highlight_target;
            self.highlight_animation = None;
        }
    }

    pub fn are_animations_ongoing(&self) -> bool {
        self.state == PreviewState::Opening
            || self.state == PreviewState::Closing
            || self.highlight_animation.is_some()
    }

    pub fn render_output<R: NiriRenderer>(
        &self,
        renderer: &mut R,
        output: &Output,
        target: RenderTarget,
        push: &mut dyn FnMut(WorkspacePreviewRenderElement<R>),
    ) {
        if target != RenderTarget::Output || self.output.as_ref() != Some(output) {
            return;
        }
        let Some(diagram) = &self.diagram else {
            return;
        };
        if self.state == PreviewState::Hidden {
            return;
        }

        // Smithay renders the element vector in reverse. Emit front-most layers first.
        let progress = self.panel_progress().clamp(0., 1.) as f32;
        let delta = Point::from(((1. - progress as f64) * 8., 0.));
        let output_scale = output.current_scale().fractional_scale();
        let highlight = self.current_highlight_rect();
        let border = FOCUS_OUTLINE.min(highlight.size.w).min(highlight.size.h);

        {
            let mut buffers = self.highlight_outline_buffers.borrow_mut();
            buffers[0].update((highlight.size.w, border), self.focus_outline_color);
            buffers[1].update((highlight.size.w, border), self.focus_outline_color);
            buffers[2].update((border, highlight.size.h), self.focus_outline_color);
            buffers[3].update((border, highlight.size.h), self.focus_outline_color);
            let locations = outline_locations(highlight, border);
            for (buffer, location) in buffers.iter().zip(locations) {
                push(WorkspacePreviewRenderElement::SolidColor(
                    SolidColorRenderElement::from_buffer(
                        buffer,
                        location + delta,
                        progress,
                        Kind::Unspecified,
                    ),
                ));
            }
        }

        let mut window_index = 0;
        for workspace in &diagram.workspaces {
            for window in &workspace.windows {
                if let Some(icon) = &self.icon_buffers[window_index] {
                    let (icon_location, icon_size) =
                        icon_geometry(window.rect);
                    if icon_size.w > 0. && icon_size.h > 0. {
                        let icon_size =
                            Size::from((icon_size.w.round() as i32, icon_size.h.round() as i32));
                        if let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
                            renderer,
                            icon_location.to_physical_precise_round(output_scale),
                            icon,
                            Some(progress),
                            Some(Rectangle::from_size(Size::from((
                                ICON_SOURCE_SIZE,
                                ICON_SOURCE_SIZE,
                            )))),
                            Some(icon_size),
                            Kind::Unspecified,
                        ) {
                            push(WorkspacePreviewRenderElement::AppIcon(element));
                        }
                    }
                }
                window_index += 1;
            }
        }

        {
            let mut buffer = self.highlight_buffer.borrow_mut();
            buffer.update(highlight.size, self.focus_fill_color);
            push(WorkspacePreviewRenderElement::SolidColor(
                SolidColorRenderElement::from_buffer(
                    &buffer,
                    highlight.loc + delta,
                    progress,
                    Kind::Unspecified,
                ),
            ));
        }

        window_index = 0;
        for workspace in &diagram.workspaces {
            for window in &workspace.windows {
                push(WorkspacePreviewRenderElement::SolidColor(
                    SolidColorRenderElement::from_buffer(
                        &self.window_buffers[window_index],
                        window.rect.loc + delta,
                        progress,
                        Kind::Unspecified,
                    ),
                ));
                window_index += 1;
            }
        }
    }

    fn apply_diagram(
        &mut self,
        diagram: PreviewDiagram<MappedId>,
        identity: HighlightIdentity,
        target: Rectangle<f64, Logical>,
        animate_highlight: bool,
    ) {
        let target_changed =
            self.highlight_identity.as_ref() != Some(&identity) || self.highlight_target != target;
        if animate_highlight && target_changed {
            let from = self.current_highlight_rect();
            self.highlight_animation = Some(std::array::from_fn(|index| {
                let (from, to) = rect_component(from, target, index);
                Animation::ease(
                    self.clock.clone(),
                    from,
                    to,
                    0.,
                    HIGHLIGHT_MORPH_MS,
                    Curve::EaseOutCubic,
                )
            }));
        } else if !animate_highlight {
            self.highlight_animation = None;
            self.highlight_rect = target;
        }

        self.highlight_identity = Some(identity);
        self.highlight_target = target;
        self.diagram = Some(diagram);
        self.update_buffers();
    }

    fn update_buffers(&mut self) {
        let Some(diagram) = &self.diagram else {
            return;
        };

        let window_count = diagram.workspaces.iter().map(|ws| ws.windows.len()).sum();
        self.window_buffers.resize_with(window_count, || {
            SolidColorBuffer::new((0., 0.), WINDOW_COLOR)
        });
        self.window_colors.resize(window_count, WINDOW_COLOR);
        self.icon_buffers.resize_with(window_count, || None);

        let mut window_index = 0;
        for workspace in &diagram.workspaces {
            for window in &workspace.windows {
                let key = window.app_id.clone();
                let icon = if let Some(icon) = self.icon_cache.get(&key) {
                    icon.clone()
                } else {
                    let icon = load_app_icon(window.app_id.as_deref());
                    self.icon_cache.insert(key, icon.clone());
                    icon
                };
                self.window_colors[window_index] =
                    window_color(icon.as_ref().map(|icon| icon.window_color));
                self.icon_buffers[window_index] = icon.map(|icon| icon.buffer);
                self.window_buffers[window_index]
                    .update(window.rect.size, self.window_colors[window_index]);
                window_index += 1;
            }
        }
    }

    fn panel_progress(&self) -> f64 {
        self.panel_animation
            .as_ref()
            .map(Animation::clamped_value)
            .unwrap_or(if self.state == PreviewState::Hidden {
                0.
            } else {
                1.
            })
    }

    fn current_highlight_rect(&self) -> Rectangle<f64, Logical> {
        let Some(animations) = &self.highlight_animation else {
            return self.highlight_rect;
        };
        Rectangle::new(
            Point::from((animations[0].value(), animations[1].value())),
            Size::from((animations[2].value(), animations[3].value())),
        )
    }
}

fn outline_locations(rect: Rectangle<f64, Logical>, width: f64) -> [Point<f64, Logical>; 4] {
    let width = width.min(rect.size.w).min(rect.size.h);
    [
        rect.loc,
        Point::from((rect.loc.x, rect.loc.y + rect.size.h - width)),
        rect.loc,
        Point::from((rect.loc.x + rect.size.w - width, rect.loc.y)),
    ]
}

fn build_preview(
    layout: &Layout<Mapped>,
    output: &Output,
) -> Option<(
    PreviewDiagram<MappedId>,
    HighlightIdentity,
    Rectangle<f64, Logical>,
)> {
    let monitor = layout.monitor_for_output(output)?;
    let inputs = monitor
        .workspaces()
        .iter()
        .map(workspace_input)
        .collect::<Vec<_>>();
    let diagram = layout_diagram(output_size(output), &inputs);
    let active_workspace_id = monitor.active_workspace_ref().id();
    let workspace = diagram
        .workspaces
        .iter()
        .find(|workspace| workspace.id == active_workspace_id)?;
    let active_window = monitor
        .active_workspace_ref()
        .active_window()
        .map(|window| window.id());
    let (focused_windows, target) = highlight_target(workspace, active_window.as_ref());
    let identity = HighlightIdentity {
        workspace: workspace.id,
        windows: focused_windows,
    };
    Some((diagram, identity, target))
}

fn workspace_input(workspace: &Workspace<Mapped>) -> PreviewWorkspaceInput<MappedId> {
    let view_pos = workspace.view_pos();
    let target_view_pos = workspace.target_view_pos();
    PreviewWorkspaceInput {
        id: workspace.id(),
        view_size: workspace.view_size(),
        view_pos,
        target_view_pos,
        windows: workspace
            .tiles_with_ipc_layouts()
            .map(|(tile, layout)| {
                let position = layout.pos_in_scrolling_layout.and_then(|_| {
                    workspace
                        .tiles_with_render_positions()
                        .find(|(positioned_tile, _, _)| {
                            positioned_tile.window().id() == tile.window().id()
                        })
                        .map(|(_, position, _)| Point::from((position.x + view_pos, position.y)))
                });
                PreviewWindowInput {
                    id: tile.window().id(),
                    app_id: with_toplevel_role(tile.window().toplevel(), |role| {
                        role.app_id.clone()
                    }),
                    title: with_toplevel_role(tile.window().toplevel(), |role| role.title.clone()),
                    layout,
                    position,
                    fullscreen: tile.window().sizing_mode().is_fullscreen(),
                }
            })
            .collect(),
    }
}

fn layout_diagram<Id: Clone + PartialEq>(
    output_size: Size<f64, Logical>,
    workspaces: &[PreviewWorkspaceInput<Id>],
) -> PreviewDiagram<Id> {
    let natural_workspace_widths = workspaces
        .iter()
        .map(natural_workspace_width)
        .collect::<Vec<_>>();
    let natural_workspace_heights = vec![CONTENT_HEIGHT; workspaces.len()];
    let natural_content_width = natural_workspace_widths.iter().copied().fold(0., f64::max);
    let natural_panel: Size<f64, Logical> = Size::from((
        natural_content_width + PANEL_PADDING * 2.,
        PANEL_PADDING * 2.
            + natural_workspace_heights.iter().sum::<f64>()
            + WORKSPACE_GAP * workspaces.len().saturating_sub(1) as f64,
    ));
    let max_height = output_size.h.max(0.) * MAX_PANEL_HEIGHT;
    let max_width = (output_size.w - PANEL_RIGHT).max(0.);
    let scale = if natural_panel.w > 0. && natural_panel.h > 0. {
        (1.0f64)
            .min(max_width / natural_panel.w)
            .min(max_height / natural_panel.h)
    } else {
        1.
    };
    let scale = scale.max(0.);
    let panel = Rectangle::new(
        Point::from((
            output_size.w - PANEL_RIGHT - natural_panel.w * scale,
            PANEL_TOP,
        )),
        Size::from((natural_panel.w * scale, natural_panel.h * scale)),
    );

    let padding = PANEL_PADDING * scale;
    let gap = WORKSPACE_GAP * scale;
    let mut workspace_y = panel.loc.y + padding;
    let mut diagram_workspaces = Vec::with_capacity(workspaces.len());
    for (workspace, width) in workspaces.iter().zip(natural_workspace_widths) {
        let size = Size::from((width * scale, CONTENT_HEIGHT * scale));
        let workspace_x = panel.loc.x + panel.size.w - padding - size.w;
        let rect = Rectangle::new(Point::from((workspace_x, workspace_y)), size);
        let interior = inset_rect(rect, WORKSPACE_INSET * scale);
        let coordinate_scale = workspace_coordinate_scale(workspace, scale);
        let focus_view = workspace_focus_view(workspace, interior, coordinate_scale);
        let mut windows = layout_workspace(workspace, interior, coordinate_scale);
        let alignment = right_align_windows(&mut windows, interior);
        for window in &mut windows {
            window.fullscreen = workspace
                .windows
                .iter()
                .any(|input| input.id == window.id && input.fullscreen);
        }
        let focus_view = focus_view.map(|mut focus_view| {
            focus_view.loc.x += alignment;
            focus_view
        });
        diagram_workspaces.push(PreviewWorkspaceRect {
            id: workspace.id,
            rect,
            interior,
            focus_view,
            windows,
        });
        workspace_y += rect.size.h + gap;
    }

    PreviewDiagram {
        panel,
        workspaces: diagram_workspaces,
    }
}

fn natural_workspace_width<Id>(workspace: &PreviewWorkspaceInput<Id>) -> f64 {
    let view_width = workspace.view_size.w;
    if !view_width.is_finite() || view_width <= 0. {
        return CONTENT_WIDTH;
    }
    let mut positioned_width: Option<f64> = None;
    let mut all_tiled_windows_positioned = true;
    for window in &workspace.windows {
        let Some((column, row)) = window.layout.pos_in_scrolling_layout else {
            continue;
        };
        if column == 0 || row == 0 {
            continue;
        }
        let Some(position) = window.position else {
            all_tiled_windows_positioned = false;
            continue;
        };
        let size = sanitized_size(window.layout.tile_size);
        let right = position.x + size.w;
        if !position.x.is_finite() || !position.y.is_finite() || !right.is_finite() {
            all_tiled_windows_positioned = false;
            continue;
        }
        positioned_width = Some(positioned_width.map_or(right, |max| max.max(right)));
    }
    if all_tiled_windows_positioned {
        if let Some(max_x) = positioned_width {
            let content_width = max_x.max(0.);
            let width_scale = if content_width > 0. {
                (content_width / view_width).max(1.)
            } else {
                1.
            };
            return CONTENT_WIDTH * width_scale;
        }
    }

    let max_column = workspace
        .windows
        .iter()
        .filter_map(|window| window.layout.pos_in_scrolling_layout)
        .filter(|(column, row)| *column > 0 && *row > 0)
        .map(|(column, _)| column)
        .max()
        .unwrap_or(0);
    if max_column == 0 {
        return CONTENT_WIDTH;
    }

    let mut column_widths = vec![0.0f64; max_column];
    for window in &workspace.windows {
        let Some((column, row)) = window.layout.pos_in_scrolling_layout else {
            continue;
        };
        if column == 0 || row == 0 {
            continue;
        }
        column_widths[column - 1] =
            column_widths[column - 1].max(sanitized_size(window.layout.tile_size).w);
    }
    let content_width = column_widths.iter().sum::<f64>();
    let width_scale = if content_width > 0. {
        (content_width / view_width).max(1.)
    } else {
        1.
    };
    CONTENT_WIDTH * width_scale
}

fn workspace_coordinate_scale<Id>(
    workspace: &PreviewWorkspaceInput<Id>,
    diagram_scale: f64,
) -> f64 {
    let view_size = workspace.view_size;
    if !view_size.w.is_finite()
        || !view_size.h.is_finite()
        || view_size.w <= 0.
        || view_size.h <= 0.
    {
        return 0.;
    }
    let width = (CONTENT_WIDTH - WORKSPACE_INSET * 2.).max(0.) / view_size.w;
    let height = (CONTENT_HEIGHT - WORKSPACE_INSET * 2.).max(0.) / view_size.h;
    diagram_scale * width.min(height)
}

fn workspace_focus_view<Id>(
    workspace: &PreviewWorkspaceInput<Id>,
    interior: Rectangle<f64, Logical>,
    coordinate_scale: f64,
) -> Option<Rectangle<f64, Logical>> {
    let is_tiled = |window: &PreviewWindowInput<Id>| {
        window
            .layout
            .pos_in_scrolling_layout
            .is_some_and(|(column, row)| column > 0 && row > 0)
    };
    if !workspace.windows.iter().any(is_tiled)
        || coordinate_scale <= 0.
        || !coordinate_scale.is_finite()
    {
        return None;
    }
    if !workspace
        .windows
        .iter()
        .filter(|window| is_tiled(window))
        .all(|window| {
            window
                .position
                .is_some_and(|position| position.x.is_finite() && position.y.is_finite())
        })
    {
        return None;
    }

    let view_pos = if workspace.target_view_pos.is_finite() {
        workspace.target_view_pos
    } else if workspace.view_pos.is_finite() {
        workspace.view_pos
    } else {
        return None;
    };
    let width = workspace.view_size.w * coordinate_scale;
    if !width.is_finite() || width <= 0. {
        return None;
    }

    Some(Rectangle::new(
        Point::from((interior.loc.x + view_pos * coordinate_scale, interior.loc.y)),
        Size::from((width, interior.size.h)),
    ))
}

fn layout_workspace<Id: Clone + PartialEq>(
    workspace: &PreviewWorkspaceInput<Id>,
    interior: Rectangle<f64, Logical>,
    coordinate_scale: f64,
) -> Vec<PreviewWindowRect<Id>> {
    let mut tiled = Vec::new();
    let mut floating = Vec::new();
    let mut overflow = Vec::new();
    for (index, window) in workspace.windows.iter().enumerate() {
        let valid_position = window
            .layout
            .pos_in_scrolling_layout
            .filter(|(column, row)| *column > 0 && *row > 0);
        if let Some((column, row)) = valid_position {
            let size = sanitized_size(window.layout.tile_size);
            if size.w > 0. && size.h > 0. {
                tiled.push((
                    column,
                    row,
                    index,
                    window.id.clone(),
                    window.app_id.clone(),
                    window.title.clone(),
                    size,
                    window.position,
                ));
                continue;
            }
        }
        if let Some(pos) = window.layout.tile_pos_in_workspace_view {
            if pos.0.is_finite() && pos.1.is_finite() {
                floating.push((
                    index,
                    window.id.clone(),
                    window.app_id.clone(),
                    window.title.clone(),
                    pos,
                    sanitized_size(window.layout.tile_size),
                ));
                continue;
            }
        }
        overflow.push((
            index,
            window.id.clone(),
            window.app_id.clone(),
            window.title.clone(),
            sanitized_size(window.layout.tile_size),
        ));
    }

    tiled.sort_by(|a, b| (a.0, a.1, a.2).cmp(&(b.0, b.1, b.2)));
    let max_column = tiled.iter().map(|tile| tile.0).max().unwrap_or(0);
    let mut column_widths = vec![0.0f64; max_column];
    let mut column_heights = vec![0.0f64; max_column];
    for (column, _row, _index, _id, _app_id, _title, size, _position) in &tiled {
        column_widths[*column - 1] = column_widths[*column - 1].max(size.w);
        column_heights[*column - 1] += size.h;
    }
    let positioned = !tiled.is_empty() && tiled.iter().all(|tile| tile.7.is_some());
    let (fit, origin_x, origin_y) = if positioned {
        let max_x = tiled
            .iter()
            .filter_map(|tile| tile.7.map(|position| position.x + tile.6.w))
            .fold(0., f64::max);
        let max_y = tiled
            .iter()
            .filter_map(|tile| tile.7.map(|position| position.y + tile.6.h))
            .fold(0., f64::max);
        let fallback_fit = if max_x > 0. && max_y > 0. {
            (interior.size.w / max_x).min(interior.size.h / max_y)
        } else {
            0.
        };
        (
            if coordinate_scale > 0. {
                coordinate_scale
            } else {
                fallback_fit
            },
            interior.loc.x,
            interior.loc.y,
        )
    } else {
        let natural_width = column_widths.iter().sum::<f64>();
        let natural_height = column_heights.iter().copied().fold(0., f64::max);
        let fit = if natural_width > 0. && natural_height > 0. {
            (interior.size.w / natural_width).min(interior.size.h / natural_height)
        } else {
            0.
        };
        let tiled_width = natural_width * fit;
        let tiled_height = natural_height * fit;
        (
            fit,
            interior.loc.x + (interior.size.w - tiled_width) / 2.,
            interior.loc.y + (interior.size.h - tiled_height) / 2.,
        )
    };
    let mut column_x = origin_x;
    let mut tiled_rects = Vec::with_capacity(tiled.len());
    for column in 0..max_column {
        let mut y = origin_y;
        for (_column, _row, _index, id, app_id, title, size, position) in
            tiled.iter().filter(|tile| tile.0 == column + 1)
        {
            let location = position
                .map(|position| {
                    Point::from((origin_x + position.x * fit, origin_y + position.y * fit))
                })
                .unwrap_or_else(|| Point::from((column_x, y)));
            let rect = inset_rect(
                clamp_rect(
                    Rectangle::new(location, Size::from((size.w * fit, size.h * fit))),
                    interior,
                ),
                WINDOW_GAP * fit / 2.,
            );
            tiled_rects.push(PreviewWindowRect {
                id: id.clone(),
                app_id: app_id.clone(),
                title: title.clone(),
                fullscreen: false,
                rect,
                tiled: true,
            });
            y += size.h * fit;
        }
        column_x += column_widths[column] * fit;
    }

    let mut windows = tiled_rects;
    let view_width = workspace.view_size.w.max(1.);
    let view_height = workspace.view_size.h.max(1.);
    for (_index, id, app_id, title, pos, size) in floating {
        let rect = Rectangle::new(
            Point::from((
                interior.loc.x + pos.0 / view_width * interior.size.w,
                interior.loc.y + pos.1 / view_height * interior.size.h,
            )),
            Size::from((
                size.w / view_width * interior.size.w,
                size.h / view_height * interior.size.h,
            )),
        );
        windows.push(PreviewWindowRect {
            id,
            app_id,
            title,
            fullscreen: false,
            rect: inset_rect(clamp_rect(rect, interior), WINDOW_GAP / 2.),
            tiled: false,
        });
    }

    if !overflow.is_empty() {
        let gap = 2.;
        let height = (interior.size.h * 0.12).max(1.).min(interior.size.h);
        let width = ((interior.size.w - gap * (overflow.len().saturating_sub(1) as f64))
            / overflow.len() as f64)
            .max(1.);
        let y = interior.loc.y + interior.size.h - height;
        for (index, (_source_index, id, app_id, title, _size)) in overflow.into_iter().enumerate() {
            let x = interior.loc.x + index as f64 * (width + gap);
            windows.push(PreviewWindowRect {
                id,
                app_id,
                title,
                fullscreen: false,
                rect: inset_rect(
                    clamp_rect(
                        Rectangle::new(Point::from((x, y)), Size::from((width, height))),
                        interior,
                    ),
                    WINDOW_GAP / 2.,
                ),
                tiled: false,
            });
        }
    }

    windows
}

fn right_align_windows<Id>(
    windows: &mut [PreviewWindowRect<Id>],
    workspace: Rectangle<f64, Logical>,
) -> f64 {
    let Some(rightmost) = windows
        .iter()
        .map(|window| window.rect.loc.x + window.rect.size.w)
        .reduce(f64::max)
    else {
        return 0.;
    };
    let offset = (workspace.loc.x + workspace.size.w - rightmost).max(0.);
    for window in windows {
        window.rect.loc.x += offset;
    }
    offset
}

fn highlight_target<Id: Clone + PartialEq>(
    workspace: &PreviewWorkspaceRect<Id>,
    active_window: Option<&Id>,
) -> (Vec<Id>, Rectangle<f64, Logical>) {
    let Some(active_window) = active_window else {
        return (Vec::new(), workspace.interior);
    };
    let Some(active) = workspace
        .windows
        .iter()
        .find(|window| window.id == *active_window)
    else {
        return (Vec::new(), workspace.interior);
    };
    // A fullscreen tile owns the viewport. Other off-screen tiles can overlap its
    // miniature card, so viewport intersection would highlight the wrong windows.
    if active.fullscreen {
        return (vec![active.id.clone()], active.rect);
    }

    let mut focused = workspace
        .focus_view
        .filter(|_| active.tiled)
        .map(|focus_view| {
            workspace
                .windows
                .iter()
                .filter(|window| window.tiled && focus_view.intersection(window.rect).is_some())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !focused.iter().any(|window| window.id == *active_window) {
        focused.clear();
        focused.push(active);
    }

    let target = focused
        .iter()
        .filter_map(|window| {
            workspace
                .focus_view
                .filter(|_| active.tiled)
                .map_or(Some(window.rect), |view| view.intersection(window.rect))
        })
        .reduce(union_rect)
        .unwrap_or(active.rect);
    let identity = focused
        .into_iter()
        .map(|window| window.id.clone())
        .collect();
    (identity, target)
}

fn union_rect(
    first: Rectangle<f64, Logical>,
    second: Rectangle<f64, Logical>,
) -> Rectangle<f64, Logical> {
    let left = first.loc.x.min(second.loc.x);
    let top = first.loc.y.min(second.loc.y);
    let right = (first.loc.x + first.size.w).max(second.loc.x + second.size.w);
    let bottom = (first.loc.y + first.size.h).max(second.loc.y + second.size.h);
    Rectangle::new(
        Point::from((left, top)),
        Size::from((right - left, bottom - top)),
    )
}
fn sanitized_size((width, height): (f64, f64)) -> Size<f64, Logical> {
    Size::from((
        if width.is_finite() { width.max(0.) } else { 0. },
        if height.is_finite() {
            height.max(0.)
        } else {
            0.
        },
    ))
}

fn inset_rect(rect: Rectangle<f64, Logical>, inset: f64) -> Rectangle<f64, Logical> {
    let inset = inset.min(rect.size.w / 2.).min(rect.size.h / 2.).max(0.);
    Rectangle::new(
        Point::from((rect.loc.x + inset, rect.loc.y + inset)),
        Size::from((rect.size.w - inset * 2., rect.size.h - inset * 2.)),
    )
}

fn clamp_rect(
    rect: Rectangle<f64, Logical>,
    bounds: Rectangle<f64, Logical>,
) -> Rectangle<f64, Logical> {
    let width = rect.size.w.max(0.).min(bounds.size.w);
    let height = rect.size.h.max(0.).min(bounds.size.h);
    let x = rect
        .loc
        .x
        .max(bounds.loc.x)
        .min(bounds.loc.x + bounds.size.w - width);
    let y = rect
        .loc
        .y
        .max(bounds.loc.y)
        .min(bounds.loc.y + bounds.size.h - height);
    Rectangle::new(Point::from((x, y)), Size::from((width, height)))
}

fn rect_component(
    from: Rectangle<f64, Logical>,
    to: Rectangle<f64, Logical>,
    index: usize,
) -> (f64, f64) {
    match index {
        0 => (from.loc.x, to.loc.x),
        1 => (from.loc.y, to.loc.y),
        2 => (from.size.w, to.size.w),
        _ => (from.size.h, to.size.h),
    }
}


fn icon_geometry(rect: Rectangle<f64, Logical>) -> (Point<f64, Logical>, Size<f64, Logical>) {
    let size = ICON_SIZE
        .min(rect.size.w - ICON_MARGIN)
        .min(rect.size.h - ICON_MARGIN)
        .max(0.);
    let location = Point::from((
        rect.loc.x + (rect.size.w - size) / 2.,
        rect.loc.y + (rect.size.h - size) / 2.,
    ));
    (location, Size::from((size, size)))
}

fn window_color(icon_color: Option<Color32F>) -> Color32F {
    icon_color.unwrap_or(WINDOW_COLOR)
}

fn load_app_icon(app_id: Option<&str>) -> Option<LoadedAppIcon> {
    let icon_name = app_id.and_then(desktop_icon_name);
    icon_name
        .as_deref()
        .and_then(resolve_icon_path)
        .and_then(|path| decode_icon(&path))
        .or_else(|| Some(fallback_icon(app_id)))
}



fn desktop_icon_name(app_id: &str) -> Option<String> {
    let app_id = app_id.strip_suffix(".desktop").unwrap_or(app_id);
    let mut names: Vec<String> = Vec::new();
    for name in [app_id, app_id.rsplit('/').next().unwrap_or(app_id)] {
        if !name.is_empty() && !names.iter().any(|known| known == name) {
            names.push(name.to_owned());
        }
    }
    if let Some(name) = app_id.rsplit('.').next() {
        if !name.is_empty() && !names.iter().any(|known| known == name) {
            names.push(name.to_owned());
        }
    }

    for directory in xdg_data_dirs() {
        let directory = directory.join("applications");
        for name in &names {
            let path = directory.join(format!("{name}.desktop"));
            let Ok(contents) = fs::read_to_string(path) else {
                continue;
            };
            if let Some(icon) = contents
                .lines()
                .find_map(|line| line.strip_prefix("Icon="))
                .map(str::trim)
                .filter(|icon| !icon.is_empty())
            {
                return Some(icon.to_owned());
            }
        }
    }

    names.into_iter().next()
}

fn xdg_data_dirs() -> Vec<PathBuf> {
    let mut directories = Vec::new();
    let mut push_unique = |directory: PathBuf| {
        if !directory.as_os_str().is_empty() && !directories.contains(&directory) {
            directories.push(directory);
        }
    };

    if let Ok(data_home) = std::env::var("XDG_DATA_HOME") {
        push_unique(PathBuf::from(data_home));
    } else if let Ok(home) = std::env::var("HOME") {
        push_unique(PathBuf::from(home).join(".local/share"));
    }
    if let Ok(data_dirs) = std::env::var("XDG_DATA_DIRS") {
        for directory in data_dirs
            .split(':')
            .filter(|directory| !directory.is_empty())
        {
            push_unique(PathBuf::from(directory));
        }
    } else {
        push_unique(PathBuf::from("/usr/local/share"));
        push_unique(PathBuf::from("/usr/share"));
    }
    push_unique(PathBuf::from("/usr/local/share"));
    push_unique(PathBuf::from("/usr/share"));
    directories
}

fn icon_file_names(icon_name: &str) -> Vec<String> {
    let has_image_extension = Path::new(icon_name)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("png") || extension.eq_ignore_ascii_case("svg")
        });
    if has_image_extension {
        vec![icon_name.to_owned()]
    } else {
        vec![format!("{icon_name}.png"), format!("{icon_name}.svg")]
    }
}

fn resolve_icon_path(icon_name: &str) -> Option<PathBuf> {
    if icon_name.is_empty() {
        return None;
    }
    let icon_path = Path::new(icon_name);
    if icon_path.is_absolute() {
        return icon_path.is_file().then(|| icon_path.to_owned());
    }
    if icon_name.contains('/') || icon_name.contains('\\') {
        return None;
    }

    let sizes = [
        "512x512", "384x384", "310x310", "256x256", "192x192", "150x150", "128x128", "96x96",
        "64x64", "48x48", "32x32", "24x24", "22x22", "16x16", "scalable",
    ];
    let themes = ["hicolor", "breeze", "breeze-dark", "Adwaita"];
    let file_names = icon_file_names(icon_name);
    for data_dir in xdg_data_dirs() {
        let root = data_dir.join("icons");
        for theme in themes {
            for size in sizes {
                for file_name in &file_names {
                    let path = root.join(theme).join(size).join("apps").join(file_name);
                    if path.is_file() {
                        return Some(path);
                    }
                }
            }
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        let root = PathBuf::from(home).join(".icons");
        for theme in themes {
            for size in sizes {
                for file_name in &file_names {
                    let path = root.join(theme).join(size).join("apps").join(file_name);
                    if path.is_file() {
                        return Some(path);
                    }
                }
            }
        }
    }
    for data_dir in xdg_data_dirs() {
        for file_name in &file_names {
            let pixmap = data_dir.join("pixmaps").join(file_name);
            if pixmap.is_file() {
                return Some(pixmap);
            }
        }
    }
    None
}

fn decode_icon(path: &Path) -> Option<LoadedAppIcon> {
    if fs::metadata(path).ok()?.len() > 8 * 1024 * 1024 {
        return None;
    }
    let pixbuf =
        Pixbuf::from_file_at_scale(path, ICON_RASTER_SIZE, ICON_RASTER_SIZE, false).ok()?;
    decode_pixbuf(pixbuf)
}


#[inline]
fn premultiply_channel(channel: u8, alpha: u8) -> u8 {
    ((u16::from(channel) * u16::from(alpha) + 127) / 255) as u8
}

fn decode_pixbuf(pixbuf: Pixbuf) -> Option<LoadedAppIcon> {
    let width = usize::try_from(pixbuf.width()).ok()?;
    let height = usize::try_from(pixbuf.height()).ok()?;
    let channels = usize::try_from(pixbuf.n_channels()).ok()?;
    let rowstride = usize::try_from(pixbuf.rowstride()).ok()?;
    if width == 0 || height == 0 || width > 512 || height > 512 || !(channels == 3 || channels == 4)
    {
        return None;
    }

    let row_bytes = width.checked_mul(channels)?;
    let mut bgra = Vec::with_capacity(width.checked_mul(height)?.checked_mul(4)?);
    let mut vivid_sum = [0.; 3];
    let mut vivid_weight = 0.;
    let mut dark_sum = [0.; 3];
    let mut dark_weight = 0.;
    let pixels = unsafe { pixbuf.pixels() };
    for row in pixels.chunks(rowstride).take(height) {
        let row = row.get(..row_bytes)?;
        for pixel in row.chunks_exact(channels) {
            let alpha = if channels == 4 { pixel[3] } else { 255 };
            let red = f64::from(pixel[0]) / 255.;
            let green = f64::from(pixel[1]) / 255.;
            let blue = f64::from(pixel[2]) / 255.;
            let luma = 0.2126 * red + 0.7152 * green + 0.0722 * blue;
            let chroma = red.max(green).max(blue) - red.min(green).min(blue);
            let alpha_weight = f64::from(alpha) / 255.;
            if alpha >= 16 {
                let dark_weight_for_pixel = alpha_weight * (1. - luma).powi(2);
                dark_sum[0] += red * dark_weight_for_pixel;
                dark_sum[1] += green * dark_weight_for_pixel;
                dark_sum[2] += blue * dark_weight_for_pixel;
                dark_weight += dark_weight_for_pixel;
                if chroma > 0.06 {
                    let vivid_weight_for_pixel = alpha_weight * chroma;
                    vivid_sum[0] += red * vivid_weight_for_pixel;
                    vivid_sum[1] += green * vivid_weight_for_pixel;
                    vivid_sum[2] += blue * vivid_weight_for_pixel;
                    vivid_weight += vivid_weight_for_pixel;
                }
            }
            // Smithay's GL renderer blends premultiplied ARGB textures.
            bgra.extend_from_slice(&[
                premultiply_channel(pixel[2], alpha),
                premultiply_channel(pixel[1], alpha),
                premultiply_channel(pixel[0], alpha),
                alpha,
            ]);
        }
    }
    if bgra.len() != width.checked_mul(height)?.checked_mul(4)? {
        return None;
    }

    let palette = if vivid_weight > 0.02 {
        [
            vivid_sum[0] / vivid_weight,
            vivid_sum[1] / vivid_weight,
            vivid_sum[2] / vivid_weight,
        ]
    } else if dark_weight > 0. {
        [
            dark_sum[0] / dark_weight,
            dark_sum[1] / dark_weight,
            dark_sum[2] / dark_weight,
        ]
    } else {
        [0.56, 0.56, 0.60]
    };
    Some(LoadedAppIcon {
        buffer: MemoryRenderBuffer::from_slice(
            &bgra,
            Fourcc::Argb8888,
            (width as i32, height as i32),
            1,
            Transform::Normal,
            None,
        ),
        window_color: premultiplied_color(
            palette[0] as f32,
            palette[1] as f32,
            palette[2] as f32,
            WINDOW_ALPHA,
        ),
    })
}

fn app_hash(app_id: Option<&str>) -> u32 {
    app_id
        .unwrap_or("?")
        .bytes()
        .fold(0x811c9dc5u32, |hash, byte| {
            hash.wrapping_mul(16777619).wrapping_add(u32::from(byte))
        })
}

fn fallback_icon(app_id: Option<&str>) -> LoadedAppIcon {
    let hash = app_hash(app_id);
    let background = [
        56 + ((hash >> 16) & 0x3f) as u8,
        64 + ((hash >> 8) & 0x5f) as u8,
        88 + (hash & 0x5f) as u8,
    ];
    let variant = hash & 3;
    let mut pixels = vec![0u8; 64 * 64 * 4];
    for y in 0usize..64 {
        for x in 0usize..64 {
            let dx = x as f64 - 31.5;
            let dy = y as f64 - 31.5;
            let radius = dx * dx + dy * dy;
            if radius > 29. * 29. {
                continue;
            }
            let ax = dx.abs();
            let ay = dy.abs();
            let diagonal = ax + ay;
            let ring = match variant {
                0 => (15. <= ax && ax <= 21. && ay <= 21.) || (15. <= ay && ay <= 21. && ax <= 21.),
                1 => (ax <= 5. && ay <= 21.) || (ay <= 5. && ax <= 21.),
                2 => 14. <= diagonal && diagonal <= 20.,
                _ => {
                    let inner = radius >= 12. * 12.;
                    let outer = radius <= 21. * 21.;
                    inner && outer
                }
            };
            let offset = (y * 64 + x) * 4;
            let color = if ring {
                [240, 255, 247, 255]
            } else {
                [background[2], background[1], background[0], 255]
            };
            pixels[offset..offset + 4].copy_from_slice(&color);
        }
    }
    LoadedAppIcon {
        buffer: MemoryRenderBuffer::from_slice(
            &pixels,
            Fourcc::Argb8888,
            (64, 64),
            1,
            Transform::Normal,
            None,
        ),
        window_color: premultiplied_color(
            f32::from(background[0]) / 255.,
            f32::from(background[1]) / 255.,
            f32::from(background[2]) / 255.,
            WINDOW_ALPHA,
        ),
    }
}


#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn input(
        id: u64,
        view_size: (f64, f64),
        windows: impl IntoIterator<Item = (u64, WindowLayout)>,
    ) -> PreviewWorkspaceInput<u64> {
        PreviewWorkspaceInput {
            id: WorkspaceId::specific(id),
            view_size: Size::from(view_size),
            view_pos: 0.,
            target_view_pos: 0.,
            windows: windows
                .into_iter()
                .map(|(id, layout)| PreviewWindowInput {
                    id,
                    app_id: None,
                    title: None,
                    layout,
                    position: None,
                    fullscreen: false,
                })
                .collect(),
        }
    }

    fn positioned_input(
        id: u64,
        view_size: (f64, f64),
        windows: impl IntoIterator<Item = (u64, WindowLayout, (f64, f64))>,
    ) -> PreviewWorkspaceInput<u64> {
        positioned_input_with_view(id, view_size, 0., windows)
    }

    fn positioned_input_with_view(
        id: u64,
        view_size: (f64, f64),
        target_view_pos: f64,
        windows: impl IntoIterator<Item = (u64, WindowLayout, (f64, f64))>,
    ) -> PreviewWorkspaceInput<u64> {
        PreviewWorkspaceInput {
            id: WorkspaceId::specific(id),
            view_size: Size::from(view_size),
            view_pos: 0.,
            target_view_pos,
            windows: windows
                .into_iter()
                .map(|(id, layout, position)| PreviewWindowInput {
                    id,
                    app_id: None,
                    title: None,
                    layout,
                    position: Some(Point::from(position)),
                    fullscreen: false,
                })
                .collect(),
        }
    }

    fn tiled(column: usize, row: usize, width: f64, height: f64) -> WindowLayout {
        WindowLayout {
            pos_in_scrolling_layout: Some((column, row)),
            tile_size: (width, height),
            window_size: (width as i32, height as i32),
            tile_pos_in_workspace_view: None,
            window_offset_in_tile: (0., 0.),
        }
    }

    #[test]
    fn multi_workspace_columns_are_contained_and_ordered_left_to_right() {
        let workspaces = vec![
            input(
                1,
                (1920., 1080.),
                [
                    (1, tiled(2, 2, 300., 200.)),
                    (2, tiled(1, 2, 400., 100.)),
                    (3, tiled(1, 1, 400., 200.)),
                    (4, tiled(2, 1, 300., 300.)),
                ],
            ),
            input(2, (1080., 1920.), []),
        ];
        let diagram = layout_diagram(Size::from((1920., 1080.)), &workspaces);
        let ws = &diagram.workspaces[0];
        assert_eq!(ws.windows[0].id, 3);
        assert_eq!(ws.windows[1].id, 2);
        assert_eq!(ws.windows[2].id, 4);
        assert_eq!(ws.windows[3].id, 1);
        for window in &ws.windows {
            assert!(contains_rect(ws.interior, window.rect));
        }
        assert_eq!(diagram.workspaces.len(), 2);
    }

    #[test]
    fn empty_workspace_target_is_its_inset_frame() {
        let workspaces = vec![input(1, (1920., 1080.), [])];
        let diagram = layout_diagram(Size::from((1920., 1080.)), &workspaces);
        let (identity, target) = highlight_target(&diagram.workspaces[0], None);
        assert!(identity.is_empty());
        assert_eq!(target, diagram.workspaces[0].interior);
        assert_eq!(
            diagram.workspaces[0].interior,
            inset_rect(diagram.workspaces[0].rect, 4.)
        );
    }

    #[test]
    fn active_window_resolves_to_the_center_card() {
        let workspaces = vec![positioned_input(
            1,
            (1920., 1080.),
            [
                (7, tiled(1, 1, 960., 540.), (0., 0.)),
                (8, tiled(2, 1, 960., 540.), (960., 0.)),
                (9, tiled(3, 1, 960., 540.), (1920., 0.)),
            ],
        )];
        let diagram = layout_diagram(Size::from((1920., 1080.)), &workspaces);
        let active = 8;
        let rect = diagram.workspaces[0]
            .windows
            .iter()
            .find(|window| window.id == active)
            .map(|window| window.rect)
            .unwrap();
        assert_eq!(rect, diagram.workspaces[0].windows[1].rect);
        assert!(contains_rect(diagram.workspaces[0].interior, rect));
    }

    #[test]
    fn highlight_follows_the_two_columns_in_the_target_view() {
        let workspaces = vec![
            positioned_input_with_view(
                1,
                (1920., 1080.),
                0.,
                [
                    (1, tiled(1, 1, 960., 1080.), (0., 0.)),
                    (2, tiled(2, 1, 960., 1080.), (960., 0.)),
                    (3, tiled(3, 1, 960., 1080.), (1920., 0.)),
                ],
            ),
            positioned_input_with_view(
                2,
                (1920., 1080.),
                960.,
                [
                    (4, tiled(1, 1, 960., 1080.), (0., 0.)),
                    (5, tiled(2, 1, 960., 1080.), (960., 0.)),
                    (6, tiled(3, 1, 960., 1080.), (1920., 0.)),
                ],
            ),
        ];
        let diagram = layout_diagram(Size::from((1920., 1080.)), &workspaces);

        let left_workspace = &diagram.workspaces[0];
        let left_active = 1;
        let (left_identity, left_target) = highlight_target(left_workspace, Some(&left_active));
        assert_eq!(left_identity, vec![1, 2]);
        assert!(contains_rect(left_workspace.interior, left_target,));
        assert!(left_target.loc.x < left_workspace.windows[2].rect.loc.x);

        let right_workspace = &diagram.workspaces[1];
        let right_active = 5;
        let (right_identity, right_target) = highlight_target(right_workspace, Some(&right_active));
        assert_eq!(right_identity, vec![5, 6]);
        assert!(contains_rect(right_workspace.interior, right_target,));
        assert!(right_target.loc.x > right_workspace.windows[0].rect.loc.x);
    }

    #[test]
    fn mixed_width_cards_do_not_overlap_and_highlight_stays_in_view() {
        for view_pos in [0., 1920., 3840.] {
            let workspace = positioned_input_with_view(
                1,
                (3840., 2160.),
                view_pos,
                [
                    (1, tiled(1, 1, 1920., 2160.), (0., 0.)),
                    (2, tiled(2, 1, 3840., 2160.), (1920., 0.)),
                    (3, tiled(3, 1, 1920., 2160.), (5760., 0.)),
                ],
            );
            let diagram = layout_diagram(Size::from((3840., 2160.)), &[workspace]);
            let preview = &diagram.workspaces[0];
            for pair in preview.windows.windows(2) {
                assert!(
                    pair[0].rect.loc.x + pair[0].rect.size.w <= pair[1].rect.loc.x,
                    "adjacent cards overlap: {:?}",
                    pair
                );
            }
            let active = if view_pos == 0. {
                1
            } else if view_pos == 1920. {
                2
            } else {
                3
            };
            let (_, target) = highlight_target(preview, Some(&active));
            let view = preview.focus_view.unwrap();
            assert!(target.loc.x >= view.loc.x - 1e-6);
            assert!(target.loc.x + target.size.w <= view.loc.x + view.size.w + 1e-6);
        }
    }

    #[test]
    fn fullscreen_active_window_is_highlighted_alone() {
        let mut workspace = positioned_input_with_view(
            1,
            (3840., 2160.),
            1920.,
            [
                (1, tiled(1, 1, 1920., 2160.), (0., 0.)),
                (2, tiled(2, 1, 3840., 2160.), (1920., 0.)),
                (3, tiled(3, 1, 1920., 2160.), (5760., 0.)),
            ],
        );
        workspace.windows[1].fullscreen = true;

        let diagram = layout_diagram(Size::from((3840., 2160.)), &[workspace]);
        let preview = &diagram.workspaces[0];
        let active = 2;
        let active_rect = preview
            .windows
            .iter()
            .find(|window| window.id == active)
            .map(|window| window.rect)
            .unwrap();
        let (identity, target) = highlight_target(preview, Some(&active));

        assert_eq!(identity, vec![active]);
        assert_eq!(target, active_rect);
    }

    #[test]
    fn fullscreen_neighbor_highlight_covers_only_its_visible_part() {
        let mut workspace = positioned_input_with_view(
            1,
            (3840., 2160.),
            0.,
            [
                (1, tiled(1, 1, 1920., 2160.), (0., 0.)),
                (2, tiled(2, 1, 3840., 2160.), (1920., 0.)),
                (3, tiled(3, 1, 1920., 2160.), (5760., 0.)),
            ],
        );
        workspace.windows[1].fullscreen = true;

        let diagram = layout_diagram(Size::from((3840., 2160.)), &[workspace]);
        let preview = &diagram.workspaces[0];
        let active = 1;
        let active_rect = preview
            .windows
            .iter()
            .find(|window| window.id == active)
            .map(|window| window.rect)
            .unwrap();
        let (identity, target) = highlight_target(preview, Some(&active));

        assert_eq!(identity, vec![1, 2]);
        assert_eq!(target.loc, active_rect.loc);
        let view = preview.focus_view.unwrap();
        assert!((target.loc.x + target.size.w - view.loc.x - view.size.w).abs() < 1e-6);
        assert!(
            target.loc.x + target.size.w
                < preview.windows[1].rect.loc.x + preview.windows[1].rect.size.w
        );
    }

    #[test]
    fn lower_tiles_align_with_the_visible_pair_side() {
        let left = positioned_input_with_view(
            1,
            (1920., 1080.),
            0.,
            [
                (1, tiled(1, 1, 960., 540.), (0., 0.)),
                (2, tiled(2, 1, 960., 540.), (960., 0.)),
                (3, tiled(3, 1, 960., 540.), (1920., 0.)),
                (4, tiled(1, 2, 960., 540.), (0., 540.)),
            ],
        );
        let right = positioned_input_with_view(
            2,
            (1920., 1080.),
            960.,
            [
                (5, tiled(1, 1, 960., 540.), (0., 0.)),
                (6, tiled(2, 1, 960., 540.), (960., 0.)),
                (7, tiled(3, 1, 960., 540.), (1920., 0.)),
                (8, tiled(2, 2, 960., 540.), (960., 540.)),
            ],
        );
        let diagram = layout_diagram(Size::from((1920., 1080.)), &[left, right]);

        let left_workspace = &diagram.workspaces[0];
        let left_top = left_workspace
            .windows
            .iter()
            .find(|window| window.id == 1)
            .unwrap();
        let left_bottom = left_workspace
            .windows
            .iter()
            .find(|window| window.id == 4)
            .unwrap();
        assert!((left_bottom.rect.loc.x - left_top.rect.loc.x).abs() < 1e-6);

        let right_workspace = &diagram.workspaces[1];
        let right_top = right_workspace
            .windows
            .iter()
            .find(|window| window.id == 6)
            .unwrap();
        let right_bottom = right_workspace
            .windows
            .iter()
            .find(|window| window.id == 8)
            .unwrap();
        assert!((right_bottom.rect.loc.x - right_top.rect.loc.x).abs() < 1e-6);
    }

    #[test]
    fn workspace_width_matches_content_and_height_stays_equal() {
        let workspaces = vec![
            input(1, (1920., 1080.), [(1, tiled(1, 1, 1920., 1080.))]),
            input(
                2,
                (1920., 1080.),
                [(2, tiled(1, 1, 960., 540.)), (3, tiled(2, 1, 960., 540.))],
            ),
            input(
                3,
                (1920., 1080.),
                [
                    (4, tiled(1, 1, 1920., 1080.)),
                    (5, tiled(2, 1, 1920., 1080.)),
                ],
            ),
        ];
        let diagram = layout_diagram(Size::from((1920., 1080.)), &workspaces);
        let single = &diagram.workspaces[0].rect;
        let halves = &diagram.workspaces[1].rect;
        let wide = &diagram.workspaces[2].rect;
        let right_edge = single.loc.x + single.size.w;
        assert!((halves.loc.x + halves.size.w - right_edge).abs() < 1e-6);
        assert!((wide.loc.x + wide.size.w - right_edge).abs() < 1e-6);
        assert!((halves.size.w - single.size.w).abs() < 1e-6);
        assert!((halves.size.h - single.size.h).abs() < 1e-6);
        assert!((wide.size.h - single.size.h).abs() < 1e-6);
    }

    #[test]
    fn positioned_tiles_keep_overview_alignment_across_workspaces() {
        let workspaces = vec![
            positioned_input(
                1,
                (1920., 1080.),
                [
                    (1, tiled(1, 1, 960., 540.), (0., 0.)),
                    (2, tiled(2, 1, 960., 540.), (960., 0.)),
                ],
            ),
            positioned_input(
                2,
                (1920., 1080.),
                [(3, tiled(2, 1, 960., 540.), (960., 540.))],
            ),
        ];
        let diagram = layout_diagram(Size::from((1920., 1080.)), &workspaces);
        let top_right = diagram.workspaces[0]
            .windows
            .iter()
            .find(|window| window.id == 2)
            .unwrap()
            .rect;
        let lower = diagram.workspaces[1].windows[0].rect;
        assert!((lower.loc.x - top_right.loc.x).abs() < 1e-6);
        assert!(lower.loc.y > top_right.loc.y);
    }

    #[test]
    fn scrolling_window_cards_remain_separate() {
        let workspace = input(
            1,
            (1920., 1080.),
            [
                (1, tiled(1, 1, 500., 700.)),
                (2, tiled(2, 1, 500., 700.)),
                (3, tiled(3, 1, 500., 700.)),
            ],
        );
        let diagram = layout_diagram(Size::from((1920., 1080.)), &[workspace]);
        let windows = &diagram.workspaces[0].windows;
        assert!(windows[0].rect.loc.x + windows[0].rect.size.w < windows[1].rect.loc.x);
        assert!(windows[1].rect.loc.x + windows[1].rect.size.w < windows[2].rect.loc.x);
    }
    #[test]
    fn window_cards_align_to_workspace_right_edge() {
        let workspace =
            positioned_input(1, (1920., 1080.), [(1, tiled(1, 1, 960., 1080.), (0., 0.))]);
        let diagram = layout_diagram(Size::from((1920., 1080.)), &[workspace]);
        let preview = &diagram.workspaces[0];
        let rect = preview.windows[0].rect;
        let right_edge = preview.interior.loc.x + preview.interior.size.w;

        assert!(rect.loc.x > preview.interior.loc.x);
        assert!((rect.loc.x + rect.size.w - right_edge).abs() < 1e-6);
    }

    #[test]
    fn highlight_morph_keeps_center_when_active_card_resizes() {
        let clock = Clock::with_time(Duration::ZERO);
        let from = Rectangle::new(Point::from((10., 10.)), Size::from((20., 20.)));
        let to = Rectangle::new(Point::from((0., 0.)), Size::from((40., 40.)));
        let animations: [Animation; 4] = std::array::from_fn(|index| {
            let (from, to) = rect_component(from, to, index);
            Animation::ease(clock.clone(), from, to, 0., 140, Curve::EaseOutCubic)
        });
        let mut clock = clock;
        clock.set_unadjusted(Duration::from_millis(70));
        let current: Rectangle<f64, Logical> = Rectangle::new(
            Point::from((animations[0].value(), animations[1].value())),
            Size::from((animations[2].value(), animations[3].value())),
        );
        assert!((current.loc.x + current.size.w / 2. - 20.).abs() < 1e-6);
        assert!((current.loc.y + current.size.h / 2. - 20.).abs() < 1e-6);
    }
    #[test]
    fn repeated_target_changes_start_at_current_interpolated_rectangle() {
        let clock = Clock::with_time(Duration::ZERO);
        let first = Rectangle::new(Point::from((0., 0.)), Size::from((10., 10.)));
        let second = Rectangle::new(Point::from((100., 20.)), Size::from((20., 30.)));
        let third = Rectangle::new(Point::from((40., 80.)), Size::from((40., 15.)));
        let animations: [Animation; 4] = std::array::from_fn(|index| {
            let (from, to) = rect_component(first, second, index);
            Animation::ease(clock.clone(), from, to, 0., 140, Curve::EaseOutCubic)
        });
        let mut clock = clock;
        clock.set_unadjusted(Duration::from_millis(70));
        let current = Rectangle::new(
            Point::from((animations[0].value(), animations[1].value())),
            Size::from((animations[2].value(), animations[3].value())),
        );
        let restarted: [Animation; 4] = std::array::from_fn(|index| {
            let (from, to) = rect_component(current, third, index);
            Animation::ease(clock.clone(), from, to, 0., 140, Curve::EaseOutCubic)
        });
        assert_eq!(restarted[0].from(), current.loc.x);
        assert_eq!(restarted[1].from(), current.loc.y);
        assert_eq!(restarted[2].from(), current.size.w);
        assert_eq!(restarted[3].from(), current.size.h);
    }

    #[test]
    fn many_workspaces_fit_inside_panel_bounds() {
        let workspaces = (0..20)
            .map(|id| input(id, (2560., 1440.), []))
            .collect::<Vec<_>>();
        let diagram = layout_diagram(Size::from((2560., 1440.)), &workspaces);
        let output = Rectangle::from_size(Size::from((2560., 1440.)));
        assert!(contains_rect(output, diagram.panel));
        assert!(diagram.panel.size.h <= 1440. * MAX_PANEL_HEIGHT + 1e-6);
        for workspace in &diagram.workspaces {
            assert!(contains_rect(diagram.panel, workspace.rect));
        }
    }



    #[test]
    fn icon_edges_use_premultiplied_alpha() {
        assert_eq!(premultiply_channel(255, 0), 0);
        assert_eq!(premultiply_channel(255, 128), 128);
        assert_eq!(premultiply_channel(255, 255), 255);
    }


    fn contains_rect(outer: Rectangle<f64, Logical>, inner: Rectangle<f64, Logical>) -> bool {
        inner.loc.x >= outer.loc.x
            && inner.loc.y >= outer.loc.y
            && inner.loc.x + inner.size.w <= outer.loc.x + outer.size.w
            && inner.loc.y + inner.size.h <= outer.loc.y + outer.size.h
    }
}
