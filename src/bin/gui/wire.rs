use femtovg::{Canvas, Color, Paint, Path, renderer::OpenGl};

use crate::workbench::Workbench;

/// Draw a wire from grid point `start` to `end`.
/// If `energized` is true the wire is yellow, otherwise gray.
/// Small arrows along the wire indicate direction (start → end).
pub fn draw_wire(
    canvas: &mut Canvas<OpenGl>,
    wb: &Workbench,
    start: (f32, f32),
    end: (f32, f32),
    energized: bool,
) {
    let (x1, y1) = wb.grid_to_px(start.0, start.1);
    let (x2, y2) = wb.grid_to_px(end.0, end.1);

    let color = if energized {
        Color::rgb(255, 210, 50)
    } else {
        Color::rgb(110, 110, 120)
    };

    // Wire line
    let mut paint = Paint::color(color);
    paint.set_line_width(if energized { 2.5 } else { 2.0 });

    let mut path = Path::new();
    path.move_to(x1, y1);
    path.line_to(x2, y2);
    canvas.stroke_path(&path, &paint);

    // Direction arrows along the wire
    let dx = x2 - x1;
    let dy = y2 - y1;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1.0 {
        return;
    }

    let ux = dx / len;
    let uy = dy / len;

    // Perpendicular
    let px = -uy;
    let py = ux;

    let arrow_size = wb.cell_px() * 0.18;
    let arrow_paint = Paint::color(color);

    // Place arrows at 25% and 75% along the wire (or just one at 50% if short)
    let cell = wb.cell_px();
    let positions: &[f32] = if len > cell * 2.5 {
        &[0.3, 0.7]
    } else {
        &[0.5]
    };

    for &t in positions {
        let cx = x1 + dx * t;
        let cy = y1 + dy * t;

        let mut arrow = Path::new();
        arrow.move_to(cx + ux * arrow_size, cy + uy * arrow_size);
        arrow.line_to(cx - ux * arrow_size + px * arrow_size, cy - uy * arrow_size + py * arrow_size);
        arrow.line_to(cx - ux * arrow_size - px * arrow_size, cy - uy * arrow_size - py * arrow_size);
        arrow.close();
        canvas.fill_path(&arrow, &arrow_paint);
    }

    // Small circles at endpoints
    let dot_r = if energized { 3.5 } else { 3.0 };
    for &(x, y) in &[(x1, y1), (x2, y2)] {
        let mut dot = Path::new();
        dot.circle(x, y, dot_r);
        canvas.fill_path(&dot, &Paint::color(color));
    }
}
