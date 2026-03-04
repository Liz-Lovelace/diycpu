use femtovg::{Canvas, Color, Paint, Path, renderer::OpenGl};

use crate::workbench::Workbench;

/// Draw a microchip: a dark rounded rectangle at grid position (gx, gy)
/// with the given width and height in grid units.
pub fn draw_microchip(
    canvas: &mut Canvas<OpenGl>,
    wb: &Workbench,
    gx: f32,
    gy: f32,
    grid_w: f32,
    grid_h: f32,
) {
    let (px, py) = wb.grid_to_px(gx, gy);
    let cell = wb.cell_px();
    let w = grid_w * cell;
    let h = grid_h * cell;
    let r = cell * 0.15; // corner radius

    // Body
    let mut path = Path::new();
    path.rounded_rect(px, py, w, h, r);

    let body_paint = Paint::color(Color::rgb(30, 30, 35));
    canvas.fill_path(&path, &body_paint);

    // Border
    let mut border_paint = Paint::color(Color::rgb(70, 70, 80));
    border_paint.set_line_width(2.0);
    canvas.stroke_path(&path, &border_paint);

    // Pin notch (semicircle on top-center)
    let notch_r = cell * 0.15;
    let cx = px + w / 2.0;
    let cy = py;
    let mut notch = Path::new();
    notch.arc(cx, cy, notch_r, 0.0, std::f32::consts::PI, femtovg::Solidity::Hole);
    let mut notch_paint = Paint::color(Color::rgb(70, 70, 80));
    notch_paint.set_line_width(1.5);
    canvas.stroke_path(&notch, &notch_paint);

    // Pin markers along left and right sides
    let pins_per_side = grid_h.floor() as usize;
    let pin_w = cell * 0.22;
    let pin_h = cell * 0.12;
    let pin_paint = Paint::color(Color::rgb(160, 160, 170));

    for i in 0..pins_per_side {
        let pin_y = py + (i as f32 + 0.5) * cell - pin_h / 2.0;

        // Left pins
        let mut p = Path::new();
        p.rounded_rect(px - pin_w * 0.5, pin_y, pin_w, pin_h, 1.0);
        canvas.fill_path(&p, &pin_paint);

        // Right pins
        let mut p = Path::new();
        p.rounded_rect(px + w - pin_w * 0.5, pin_y, pin_w, pin_h, 1.0);
        canvas.fill_path(&p, &pin_paint);
    }
}
