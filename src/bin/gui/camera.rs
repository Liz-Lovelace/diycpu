//! Viewport transform between grid space (the document's coordinates) and
//! screen pixels.

use diycpu::layout::{Rect, SheetMap};

/// Pixels per grid cell at zoom 1.
pub const CELL: f32 = 40.0;

// Zoom limits per focus level: descending into an abstraction rebases the
// camera into the child's coordinates, so these bound each level, not the
// total depth — the zoom illusion is unbounded.
pub const MIN_ZOOM: f32 = 0.05;
pub const MAX_ZOOM: f32 = 60.0;

/// A similarity transform from some sheet's grid units to screen pixels.
#[derive(Debug, Clone, Copy)]
pub struct View {
    /// Pixels per grid unit.
    pub scale: f32,
    pub offset: [f32; 2],
}

impl View {
    // Named for readability at call sites; View is Copy so the reference
    // receiver is deliberate.
    #[allow(clippy::wrong_self_convention)]
    pub fn to_screen(&self, p: [f32; 2]) -> [f32; 2] {
        [
            p[0] * self.scale + self.offset[0],
            p[1] * self.scale + self.offset[1],
        ]
    }

    /// The view of a child sheet displayed through `map`.
    pub fn through(&self, map: &SheetMap) -> View {
        View {
            scale: self.scale * map.s,
            offset: [
                map.o[0] * self.scale + self.offset[0],
                map.o[1] * self.scale + self.offset[1],
            ],
        }
    }
}

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

    /// The camera as a grid→screen transform.
    pub fn view(&self) -> View {
        View {
            scale: self.cell_px(),
            offset: self.offset,
        }
    }

    /// Re-express the camera in another sheet's coordinates (descending
    /// into or ascending out of an abstraction) — visually a no-op.
    pub fn set_view(&mut self, view: View) {
        self.zoom = view.scale / CELL;
        self.offset = view.offset;
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
