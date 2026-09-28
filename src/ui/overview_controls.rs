use std::cell::RefCell;

use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::Color32F;
use smithay::utils::{Logical, Point, Rectangle, Size};

use crate::niri_render_elements;
use crate::render_helpers::solid_color::{SolidColorBuffer, SolidColorRenderElement};

const BUTTON_SIZE: f64 = 32.;
const BUTTON_INSET: f64 = 6.;
const BUTTON_COLOR: Color32F = Color32F::new(0.62, 0.055, 0.075, 0.94);
const ICON_COLOR: Color32F = Color32F::new(1., 1., 1., 1.);
const LABEL_BACKGROUND_COLOR: Color32F = Color32F::new(0.12, 0.32, 0.68, 0.96);
const LABEL_TEXT_COLOR: Color32F = Color32F::new(1., 1., 1., 1.);
const LABEL_WIDTH: f64 = 44.;
const LABEL_HEIGHT: f64 = 28.;
const LABEL_GAP: f64 = 10.;
const DIGIT_ROWS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b001, 0b001, 0b001],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
];

niri_render_elements! {
    OverviewControlsRenderElement => {
        SolidColor = SolidColorRenderElement,
    }
}

pub struct OverviewControls {
    background: RefCell<SolidColorBuffer>,
    icon: RefCell<[SolidColorBuffer; 5]>,
    label_background: RefCell<SolidColorBuffer>,
    label_pixel: RefCell<SolidColorBuffer>,
}

impl OverviewControls {
    pub fn new() -> Self {
        Self {
            background: RefCell::new(SolidColorBuffer::new(
                (BUTTON_SIZE, BUTTON_SIZE),
                BUTTON_COLOR,
            )),
            icon: RefCell::new(std::array::from_fn(|_| {
                SolidColorBuffer::new((0., 0.), ICON_COLOR)
            })),
            label_background: RefCell::new(SolidColorBuffer::new(
                (LABEL_WIDTH, LABEL_HEIGHT),
                LABEL_BACKGROUND_COLOR,
            )),
            label_pixel: RefCell::new(SolidColorBuffer::new((3., 3.), LABEL_TEXT_COLOR)),
        }
    }

    pub fn button_rect(window: Rectangle<f64, Logical>) -> Option<Rectangle<f64, Logical>> {
        let size = BUTTON_SIZE.min(window.size.w).min(window.size.h);
        if !size.is_finite() || size < 12. {
            return None;
        }
        let inset = BUTTON_INSET.min(size * 0.16);
        Some(Rectangle::new(
            Point::from((
                window.loc.x + window.size.w - size - inset,
                window.loc.y + inset,
            )),
            Size::from((size, size)),
        ))
    }

    pub fn workspace_label_rect(
        workspace_top_left: Point<f64, Logical>,
    ) -> Rectangle<f64, Logical> {
        Rectangle::new(
            Point::from((
                workspace_top_left.x - LABEL_WIDTH - LABEL_GAP,
                workspace_top_left.y,
            )),
            Size::from((LABEL_WIDTH, LABEL_HEIGHT)),
        )
    }

    pub fn render_workspace_label(
        &self,
        number: usize,
        workspace_top_left: Point<f64, Logical>,
        alpha: f32,
        push: &mut dyn FnMut(OverviewControlsRenderElement),
    ) {
        let rect = Self::workspace_label_rect(workspace_top_left);
        let mut digits = [0_u8; 20];
        let mut digit_count = 0;
        let mut remaining = number.max(1);
        while remaining > 0 && digit_count < digits.len() {
            digits[digit_count] = (remaining % 10) as u8;
            remaining /= 10;
            digit_count += 1;
        }

        let pixel_columns = digit_count * 3 + digit_count.saturating_sub(1);
        let pixel_size = 3_f64.min((LABEL_WIDTH - 10.) / pixel_columns as f64);
        let content_width = pixel_columns as f64 * pixel_size;
        let left = rect.loc.x + (rect.size.w - content_width) / 2.;
        let top = rect.loc.y + (rect.size.h - 5. * pixel_size) / 2.;
        self.label_pixel
            .borrow_mut()
            .update((pixel_size, pixel_size), LABEL_TEXT_COLOR);

        for visual_idx in 0..digit_count {
            let digit = digits[digit_count - visual_idx - 1] as usize;
            let digit_left = left + visual_idx as f64 * 4. * pixel_size;
            for (row, bits) in DIGIT_ROWS[digit].iter().copied().enumerate() {
                for column in 0..3 {
                    if bits & (1 << (2 - column)) == 0 {
                        continue;
                    }
                    let location = Point::from((
                        digit_left + column as f64 * pixel_size,
                        top + row as f64 * pixel_size,
                    ));
                    push(OverviewControlsRenderElement::SolidColor(
                        SolidColorRenderElement::from_buffer(
                            &self.label_pixel.borrow(),
                            location,
                            alpha,
                            Kind::Unspecified,
                        ),
                    ));
                }
            }
        }

        push(OverviewControlsRenderElement::SolidColor(
            SolidColorRenderElement::from_buffer(
                &self.label_background.borrow(),
                rect.loc,
                alpha,
                Kind::Unspecified,
            ),
        ));
    }

    pub fn render(
        &self,
        rect: Rectangle<f64, Logical>,
        push: &mut dyn FnMut(OverviewControlsRenderElement),
    ) {
        let unit = rect.size.w / BUTTON_SIZE;

        let left = rect.loc.x + 9. * unit;
        let top = rect.loc.y + 8. * unit;
        let width = 14. * unit;
        let stroke = (2. * unit).max(1.);
        let body_top = top + 5. * unit;
        let body_height = 11. * unit;
        let pieces = [
            Rectangle::new(
                Point::from((left, top + 3. * unit)),
                Size::from((width, stroke)),
            ),
            Rectangle::new(
                Point::from((left + 4. * unit, top)),
                Size::from((6. * unit, stroke)),
            ),
            Rectangle::new(
                Point::from((left + unit, body_top)),
                Size::from((stroke, body_height)),
            ),
            Rectangle::new(
                Point::from((left + width - stroke - unit, body_top)),
                Size::from((stroke, body_height)),
            ),
            Rectangle::new(
                Point::from((left + unit, body_top + body_height - stroke)),
                Size::from((width - 2. * unit, stroke)),
            ),
        ];

        let mut buffers = self.icon.borrow_mut();
        for (buffer, piece) in buffers.iter_mut().zip(pieces) {
            buffer.update(piece.size, ICON_COLOR);
            push(OverviewControlsRenderElement::SolidColor(
                SolidColorRenderElement::from_buffer(buffer, piece.loc, 1., Kind::Unspecified),
            ));
        }

        self.background.borrow_mut().update(rect.size, BUTTON_COLOR);
        push(OverviewControlsRenderElement::SolidColor(
            SolidColorRenderElement::from_buffer(
                &self.background.borrow(),
                rect.loc,
                1.,
                Kind::Unspecified,
            ),
        ));
    }
}
