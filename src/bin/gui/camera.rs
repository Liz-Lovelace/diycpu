//! Viewport transform between grid space (the document's coordinates) and
//! screen pixels.

use diycpu::layout::Rect;

/// Pixels per grid cell at zoom 1.
pub const CELL: f32 = 40.0;

pub const MIN_ZOOM: f32 = 0.2;
pub const MAX_ZOOM: f32 = 4.0;

pub struct Camera {
    /// Screen-pixel position of grid origin.
    pub offset: [f32; 2],
    pub zoom: f32,
}

impl Camera {
    pub fn new() -> Self {
        Camera {
            offset: [0.0, 0.0],
            zoom: 1.0,
        }
    }

    pub fn cell_px(&self) -> f32 {
        CELL * self.zoom
    }

    pub fn grid_to_screen(&self, g: [f32; 2]) -> [f32; 2] {
        [
            g[0] * self.cell_px() + self.offset[0],
            g[1] * self.cell_px() + self.offset[1],
        ]
    }

    pub fn screen_to_grid(&self, s: [f32; 2]) -> [f32; 2] {
        [
            (s[0] - self.offset[0]) / self.cell_px(),
            (s[1] - self.offset[1]) / self.cell_px(),
        ]
    }

    pub fn pan(&mut self, dx: f32, dy: f32) {
        self.offset[0] += dx;
        self.offset[1] += dy;
    }

    /// Zoom by `factor`, keeping the grid point under `anchor` (screen px)
    /// fixed on screen.
    pub fn zoom_at(&mut self, anchor: [f32; 2], factor: f32) {
        let before = self.screen_to_grid(anchor);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let after = self.grid_to_screen(before);
        self.offset[0] += anchor[0] - after[0];
        self.offset[1] += anchor[1] - after[1];
    }

    /// Fit a grid-space rectangle into the given screen viewport.
    pub fn fit(&mut self, content: Rect, viewport: Rect, margin_px: f32) {
        if content.w <= 0.0 || content.h <= 0.0 {
            self.zoom = 1.0;
            self.offset = [
                viewport.x + viewport.w / 2.0 - content.x * CELL,
                viewport.y + viewport.h / 2.0 - content.y * CELL,
            ];
            return;
        }
        let avail_w = (viewport.w - margin_px * 2.0).max(50.0);
        let avail_h = (viewport.h - margin_px * 2.0).max(50.0);
        let zoom_w = avail_w / (content.w * CELL);
        let zoom_h = avail_h / (content.h * CELL);
        self.zoom = zoom_w.min(zoom_h).clamp(MIN_ZOOM, 1.25);
        let cell = self.cell_px();
        self.offset = [
            viewport.x + (viewport.w - content.w * cell) / 2.0 - content.x * cell,
            viewport.y + (viewport.h - content.h * cell) / 2.0 - content.y * cell,
        ];
    }
}
