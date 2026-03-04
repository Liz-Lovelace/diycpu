use femtovg::{Canvas, Color, Paint, Path, renderer::OpenGl};

/// Grid-unit size in pixels (at 1x zoom).
const CELL_SIZE: f32 = 40.0;

/// Camera/viewport state for the infinite canvas.
pub struct Workbench {
    /// Offset in pixels (pan).
    pub offset_x: f32,
    pub offset_y: f32,
    pub zoom: f32,
}

impl Workbench {
    pub fn new() -> Self {
        Self {
            offset_x: 0.0,
            offset_y: 0.0,
            zoom: 1.0,
        }
    }

    /// Convert grid coordinates to pixel coordinates.
    pub fn grid_to_px(&self, gx: f32, gy: f32) -> (f32, f32) {
        let px = gx * CELL_SIZE * self.zoom + self.offset_x;
        let py = gy * CELL_SIZE * self.zoom + self.offset_y;
        (px, py)
    }

    /// Size of one grid unit in current pixels.
    pub fn cell_px(&self) -> f32 {
        CELL_SIZE * self.zoom
    }

    /// Draw the background grid across the entire viewport.
    pub fn draw_grid(&self, canvas: &mut Canvas<OpenGl>, width: u32, height: u32) {
        let cell = self.cell_px();
        if cell < 4.0 {
            return; // too zoomed out to draw grid
        }

        let mut paint = Paint::color(Color::rgbaf(1.0, 1.0, 1.0, 0.06));
        paint.set_line_width(1.0);

        // Vertical lines
        let start_gx = (-self.offset_x / cell).floor() as i32;
        let end_gx = ((width as f32 - self.offset_x) / cell).ceil() as i32;
        for gx in start_gx..=end_gx {
            let px = gx as f32 * cell + self.offset_x;
            let mut path = Path::new();
            path.move_to(px, 0.0);
            path.line_to(px, height as f32);
            canvas.stroke_path(&path, &paint);
        }

        // Horizontal lines
        let start_gy = (-self.offset_y / cell).floor() as i32;
        let end_gy = ((height as f32 - self.offset_y) / cell).ceil() as i32;
        for gy in start_gy..=end_gy {
            let py = gy as f32 * cell + self.offset_y;
            let mut path = Path::new();
            path.move_to(0.0, py);
            path.line_to(width as f32, py);
            canvas.stroke_path(&path, &paint);
        }
    }
}
