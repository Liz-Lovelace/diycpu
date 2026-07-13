//! The zoomable world: the focused sheet plus ancestor context behind
//! it, every big-enough instance's contents cross-faded in — recursively.
//! One canvas, any depth.

use femtovg::{renderer::OpenGl, Align, Baseline, Canvas, Paint, Path};

use diycpu::layout::{self};

use crate::app::{App, PathSeg};
use crate::camera::View;
use crate::theme;
use diycpu::editor::SheetRef;

use super::*;


/// Instance size (relative to the viewport's smaller dimension) where the
/// closed look starts fading into the contents, and where the fade is done.
pub(super) const OPEN_T0: f32 = 0.55;

pub(super) const OPEN_T1: f32 = 0.95;

/// Recursion limits for drawing nested sheets.
pub(super) const MAX_DEPTH: usize = 8;

pub(super) const MIN_CHILD_SCALE: f32 = 2.0; // px per grid unit

impl Renderer {
    /// The whole zoomable world: the focused sheet plus up to a few levels
    /// of ancestor context behind it, plus every big-enough instance's
    /// contents cross-faded in — recursively. One canvas, any depth.
    pub(super) fn draw_world(&self, canvas: &mut Canvas<OpenGl>, app: &App) {
        // Mid-drag gestures render from a temporary copy of the document
        // with the drag applied, so everything moves live.
        let project = app.display_project();
        let project: &diycpu::project::Project = &project;

        // Climb from the focus to an ancestor to start drawing from, by
        // inverting each level's map. Levels further up are entirely
        // off-screen anyway.
        let mut view = app.camera.view();
        let mut sheet = app.focus_sheet();
        let mut start = app.path.len();
        let mut climbed = 0;
        while climbed < 3 && start > 0 {
            let seg = &app.path[start - 1];
            let Some(map) = app.seg_map_in(project, seg) else { break };
            view = view.through(&map.inverse());
            sheet = seg.parent;
            start -= 1;
            climbed += 1;
        }

        // The focus sheet's grid clips to its enclosing instance body.
        let grid_clip = app.path.last().and_then(|seg| {
            let map = app.seg_map_in(project, seg)?;
            let pview = app.camera.view().through(&map.inverse());
            let (comps, _) = diycpu::editor::sheet_of(project, seg.parent)?;
            let comp = comps.iter().find(|c| c.id == seg.instance_id)?;
            let r = layout::footprint(&project.abstractions, comp);
            let tl = pview.to_screen([r.x, r.y]);
            Some([tl[0], tl[1], r.w * pview.scale, r.h * pview.scale])
        });

        // While running, sheets are keyed into the flattened simulation by
        // their instance path from the top.
        // A view rooted at an abstraction (sidebar ↗) has no unique instance
        // path, so its values render neutral.
        let root_sim_path = if app.is_running() && app.view_root == SheetRef::Top {
            Some(
                app.path[..start]
                    .iter()
                    .map(|seg| seg.instance_id.clone())
                    .collect::<Vec<_>>(),
            )
        } else {
            None
        };

        self.draw_sheet_tree(
            canvas,
            app,
            project,
            sheet,
            view,
            1.0,
            root_sim_path,
            &app.path[start..],
            true,
            0,
            grid_clip,
        );
        canvas.set_global_alpha(1.0);
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_sheet_tree(
        &self,
        canvas: &mut Canvas<OpenGl>,
        app: &App,
        project: &diycpu::project::Project,
        sheet: SheetRef,
        view: View,
        alpha: f32,
        sim_path: Option<Vec<String>>,
        slice: &[PathSeg],
        on_path: bool,
        depth: usize,
        grid_clip: Option<[f32; 4]>,
    ) {
        let focus = on_path && slice.is_empty();
        let Some(ctx) = make_ctx(project, sheet, view, alpha, focus, sim_path) else {
            return;
        };
        canvas.set_global_alpha(alpha);
        if ctx.focus {
            self.draw_focus_grid(canvas, app, view, grid_clip);
        }
        self.draw_images(canvas, app, &ctx);
        self.draw_notes(canvas, app, &ctx);
        self.draw_wires(canvas, app, &ctx);
        self.draw_derived_output_wires(canvas, app, &ctx);
        for comp in ctx.components {
            self.draw_component_tree(
                canvas, app, project, &ctx, comp, slice, on_path, depth, grid_clip,
            );
            canvas.set_global_alpha(alpha);
        }
        if ctx.interface.is_some() {
            self.draw_interface_pins(canvas, app, &ctx);
        }
    }

    pub(super) fn draw_focus_grid(
        &self,
        canvas: &mut Canvas<OpenGl>,
        app: &App,
        view: View,
        clip: Option<[f32; 4]>,
    ) {
        let cr = app.canvas_rect();
        let (mut x0, mut y0, mut x1, mut y1) = (cr.x, cr.y, cr.x + cr.w, cr.y + cr.h);
        if let Some([cx, cy, cw, ch]) = clip {
            x0 = x0.max(cx);
            y0 = y0.max(cy);
            x1 = x1.min(cx + cw);
            y1 = y1.min(cy + ch);
        }
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let cell = view.scale;
        let minor_visible = cell >= 9.0;
        if cell < 4.0 {
            return;
        }
        let g = |px: f32, o: f32| (px - o) / cell;
        for gx in (g(x0, view.offset[0]).floor() as i64)..=(g(x1, view.offset[0]).ceil() as i64) {
            let major = gx.rem_euclid(5) == 0;
            if !major && !minor_visible {
                continue;
            }
            let x = gx as f32 * cell + view.offset[0];
            if x < x0 || x > x1 {
                continue;
            }
            let mut p = Path::new();
            p.move_to(x, y0);
            p.line_to(x, y1);
            let mut paint = Paint::color(if major {
                theme::grid_line_major()
            } else {
                theme::grid_line()
            });
            paint.set_line_width(1.0);
            canvas.stroke_path(&p, &paint);
        }
        for gy in (g(y0, view.offset[1]).floor() as i64)..=(g(y1, view.offset[1]).ceil() as i64) {
            let major = gy.rem_euclid(5) == 0;
            if !major && !minor_visible {
                continue;
            }
            let y = gy as f32 * cell + view.offset[1];
            if y < y0 || y > y1 {
                continue;
            }
            let mut p = Path::new();
            p.move_to(x0, y);
            p.line_to(x1, y);
            let mut paint = Paint::color(if major {
                theme::grid_line_major()
            } else {
                theme::grid_line()
            });
            paint.set_line_width(1.0);
            canvas.stroke_path(&p, &paint);
        }
    }

    /// Sticky-note annotations: drawn behind the circuit, wrapped with the
    /// same deterministic algorithm layout uses for their hit rectangles.
    pub(super) fn draw_notes(&self, canvas: &mut Canvas<OpenGl>, app: &App, ctx: &SheetCtx) {
        let zoom = ctx.zoom();
        let cell = ctx.view.scale;
        for note in ctx.notes {
            let bounds = layout::note_bounds(note);
            let tl = ctx.view.to_screen([bounds.x, bounds.y]);
            let selected = ctx.focus && app.selection.notes.contains(&note.id);

            let mut bg = Path::new();
            bg.rounded_rect(
                tl[0],
                tl[1],
                bounds.w * cell,
                bounds.h * cell,
                (4.0 * zoom).clamp(1.5, 6.0),
            );
            canvas.fill_path(&bg, &Paint::color(theme::note_bg()));
            let mut border = Paint::color(if selected {
                theme::accent()
            } else {
                theme::note_border()
            });
            border.set_line_width(if selected { 2.0 } else { 1.2 });
            canvas.stroke_path(&bg, &border);

            let size = (12.0 * zoom).clamp(5.0, 18.0);
            if size >= 5.0 {
                let pad = layout::NOTE_PADDING * cell;
                let line_h = layout::NOTE_LINE_HEIGHT * cell;
                for (i, line) in layout::wrap_note_text(&note.text).iter().enumerate() {
                    self.text(
                        canvas,
                        tl[0] + pad,
                        tl[1] + pad + (i as f32 + 0.5) * line_h,
                        line,
                        size,
                        theme::note_text(),
                        Align::Left,
                        Baseline::Middle,
                    );
                }
            }
        }
    }
}
