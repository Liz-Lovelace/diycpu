//! Transient things drawn over the world: selection band, invalid-drop
//! ghosts, placement ghosts, and the hover cog.

use femtovg::{renderer::OpenGl, Canvas, Paint, Path};

use diycpu::layout::{self};
use diycpu::project::ProjectComponent;

use crate::app::{App, Gesture};
use crate::theme;

use super::*;


impl Renderer {
    /// The hover gear: settings live here.
    pub(super) fn draw_cog(&self, canvas: &mut Canvas<OpenGl>, app: &App) {
        let Some((r, _)) = app.cog() else {
            return;
        };
        let hovered = r.contains(app.cursor);
        let cx = r.x + r.w / 2.0;
        let cy = r.y + r.h / 2.0;

        let mut disc = Path::new();
        disc.circle(cx, cy, r.w / 2.0);
        canvas.fill_path(
            &disc,
            &Paint::color(if hovered {
                theme::button_hover_bg()
            } else {
                theme::chip_fill()
            }),
        );
        let mut ring = Paint::color(if hovered {
            theme::accent()
        } else {
            theme::button_border()
        });
        ring.set_line_width(1.2);
        canvas.stroke_path(&disc, &ring);

        // The gear: a ring with teeth and a hub.
        let color = if hovered { theme::accent() } else { theme::text_dim() };
        let mut gear = Paint::color(color);
        gear.set_line_width(1.6);
        let rr = r.w * 0.22;
        let mut body = Path::new();
        body.circle(cx, cy, rr);
        canvas.stroke_path(&body, &gear);
        let mut teeth = Path::new();
        for k in 0..6 {
            let a = k as f32 * std::f32::consts::FRAC_PI_3 + 0.4;
            teeth.move_to(cx + a.cos() * (rr + 1.0), cy + a.sin() * (rr + 1.0));
            teeth.line_to(cx + a.cos() * (rr + 3.5), cy + a.sin() * (rr + 3.5));
        }
        let mut tp = Paint::color(color);
        tp.set_line_width(2.0);
        tp.set_line_cap(femtovg::LineCap::Round);
        canvas.stroke_path(&teeth, &tp);
    }

    /// Dragged items render live in the world; this overlay only flags an
    /// invalid drop position.
    pub(super) fn draw_drag_ghosts(&self, canvas: &mut Canvas<OpenGl>, app: &App) {
        let Gesture::DraggingComponents {
            ids,
            note_ids,
            delta,
            copying,
            ..
        } = &app.gesture
        else {
            return;
        };
        if *delta == [0, 0] {
            return;
        }
        if app.drag_is_valid(ids, *delta, *copying) {
            return;
        }
        let abstractions = &app.editor.project.abstractions;
        let (components, _) = app.editor.sheet_data();
        let cell = app.camera.cell_px();

        let mut ghost_rects: Vec<layout::Rect> = components
            .iter()
            .filter(|c| ids.contains(&c.id))
            .map(|c| layout::footprint(abstractions, c))
            .collect();
        ghost_rects.extend(
            app.editor
                .sheet_notes()
                .iter()
                .filter(|n| note_ids.contains(&n.id))
                .map(layout::note_bounds),
        );

        for rect in ghost_rects {
            let tl = app.camera.grid_to_screen([
                rect.x + delta[0] as f32,
                rect.y + delta[1] as f32,
            ]);
            let mut p = Path::new();
            p.rounded_rect(
                tl[0],
                tl[1],
                rect.w * cell,
                rect.h * cell,
                (6.0 * app.camera.zoom).clamp(2.0, 9.0),
            );
            canvas.fill_path(&p, &Paint::color(theme::ghost_invalid()));
            let mut stroke = Paint::color(theme::danger());
            stroke.set_line_width(1.5);
            canvas.stroke_path(&p, &stroke);
        }
    }

    pub(super) fn draw_placement_ghost(&self, canvas: &mut Canvas<OpenGl>, app: &App) {
        let Gesture::Placing { type_name } = &app.gesture else {
            return;
        };
        if !app.canvas_rect().contains(app.cursor) {
            return;
        }
        let pos = app.placement_pos(type_name);
        if type_name == "note" {
            // A blank sticky-note outline; the text comes right after.
            let size = app.placement_size(type_name);
            let tl = app.camera.grid_to_screen([pos[0] as f32, pos[1] as f32]);
            let cell = app.camera.cell_px();
            canvas.set_global_alpha(0.6);
            let mut p = Path::new();
            p.rounded_rect(
                tl[0],
                tl[1],
                size[0] * cell,
                size[1] * cell,
                (4.0 * app.camera.zoom).clamp(1.5, 6.0),
            );
            canvas.fill_path(&p, &Paint::color(theme::note_bg()));
            let mut stroke = Paint::color(theme::note_border());
            stroke.set_line_width(1.2);
            canvas.stroke_path(&p, &stroke);
            canvas.set_global_alpha(1.0);
            return;
        }
        let valid = app.placement_is_valid(type_name);
        let ghost = ProjectComponent {
            display: None,
            id: String::new(),
            component_type: type_name.clone(),
            init: None,
            bits: None,
            pos: [pos[0] as f32, pos[1] as f32],
        };
        let project = &app.editor.project;
        if let Some(ctx) =
            make_ctx(project, app.editor.sheet, app.camera.view(), 0.55, false, None)
        {
            canvas.set_global_alpha(0.55);
            // MAX_DEPTH suppresses the open-contents recursion for ghosts.
            self.draw_component_tree(
                canvas, app, project, &ctx, &ghost, &[], false, MAX_DEPTH, None,
            );
            canvas.set_global_alpha(1.0);
        }
        if !valid {
            let abstractions = &app.editor.project.abstractions;
            let rect = layout::footprint(abstractions, &ghost);
            let tl = app.camera.grid_to_screen([rect.x, rect.y]);
            let cell = app.camera.cell_px();
            let mut p = Path::new();
            p.rounded_rect(
                tl[0],
                tl[1],
                rect.w * cell,
                rect.h * cell,
                (6.0 * app.camera.zoom).clamp(2.0, 9.0),
            );
            canvas.fill_path(&p, &Paint::color(theme::ghost_invalid()));
        }
    }

    pub(super) fn draw_band(&self, canvas: &mut Canvas<OpenGl>, app: &App) {
        let Gesture::BandSelecting { start_grid } = &app.gesture else {
            return;
        };
        let a = app.camera.grid_to_screen(*start_grid);
        let b = app.cursor;
        let (x, y) = (a[0].min(b[0]), a[1].min(b[1]));
        let (w, h) = ((a[0] - b[0]).abs(), (a[1] - b[1]).abs());
        let mut p = Path::new();
        p.rect(x, y, w, h);
        canvas.fill_path(&p, &Paint::color(theme::accent_soft()));
        let mut stroke = Paint::color(theme::accent());
        stroke.set_line_width(1.0);
        canvas.stroke_path(&p, &stroke);
    }
}
