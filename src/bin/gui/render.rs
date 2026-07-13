//! The renderer: a pure function of (document, gesture, simulation, camera).
//! Nothing here is stateful except the loaded font.

use femtovg::{renderer::OpenGl, Align, Baseline, Canvas, Color, Paint, Path};

use diycpu::layout::{self, PortKind};
use diycpu::project::{ProjectComponent, WireNet};
use diycpu::types::is_primitive;

use crate::app::{App, Gesture, Mode, PathSeg, WireTarget};
use crate::camera::View;
use crate::theme;
use crate::ui::UiLayout;
use diycpu::editor::SheetRef;
use diycpu::project::{Note, ProjectAbstraction};

/// Instance size (relative to the viewport's smaller dimension) where the
/// closed look starts fading into the contents, and where the fade is done.
const OPEN_T0: f32 = 0.55;
const OPEN_T1: f32 = 0.95;
/// Recursion limits for drawing nested sheets.
const MAX_DEPTH: usize = 8;
const MIN_CHILD_SCALE: f32 = 2.0; // px per grid unit

/// Everything needed to draw one sheet at one place on screen.
struct SheetCtx<'a> {
    abstractions: &'a [ProjectAbstraction],
    /// While running: this sheet's instance path from the top, for keying
    /// flattened simulation values. None when not simulating.
    sim_path: Option<Vec<String>>,
    components: &'a [diycpu::project::ProjectComponent],
    wire_nets: &'a [WireNet],
    notes: &'a [Note],
    interface: Option<&'a ProjectAbstraction>,
    view: View,
    alpha: f32,
    /// True only for the sheet being edited: selection, hover and run-mode
    /// values render here.
    focus: bool,
}

impl SheetCtx<'_> {
    fn zoom(&self) -> f32 {
        self.view.scale / crate::camera::CELL
    }
}

fn make_ctx(
    project: &diycpu::project::Project,
    sheet: SheetRef,
    view: View,
    alpha: f32,
    focus: bool,
    sim_path: Option<Vec<String>>,
) -> Option<SheetCtx<'_>> {
    let (components, wire_nets) = diycpu::editor::sheet_of(project, sheet)?;
    let notes = diycpu::editor::sheet_notes(project, sheet)?;
    let interface = match sheet {
        SheetRef::Abstraction(i) => project.abstractions.get(i),
        SheetRef::Top => None,
    };
    Some(SheetCtx {
        abstractions: &project.abstractions,
        sim_path,
        components,
        wire_nets,
        notes,
        interface,
        view,
        alpha,
        focus,
    })
}

pub struct Renderer {
    font: femtovg::FontId,
}

impl Renderer {
    pub fn new(canvas: &mut Canvas<OpenGl>) -> Self {
        let font = canvas
            .add_font_mem(include_bytes!("../../../assets/DejaVuSans.ttf"))
            .expect("embedded font loads");
        Renderer { font }
    }

    pub fn draw(&self, canvas: &mut Canvas<OpenGl>, app: &App, ui: &UiLayout) {
        let [w, h] = app.window_size;
        canvas.clear_rect(0, 0, w as u32, h as u32, theme::canvas_bg());

        let view = app.canvas_rect();
        canvas.scissor(view.x, view.y, view.w, view.h);
        self.draw_world(canvas, app);
        self.draw_wire_preview(canvas, app);
        self.draw_drag_ghosts(canvas, app);
        self.draw_placement_ghost(canvas, app);
        self.draw_band(canvas, app);
        canvas.reset_scissor();

        self.draw_palette(canvas, app, ui);
        self.draw_topbar(canvas, app, ui);
        self.draw_statusbar(canvas, app);
        self.draw_tooltip(canvas, app);
        self.draw_layout_dialog(canvas, app);
        self.draw_prompt(canvas, app);
    }

    // ── Canvas layers ────────────────────────────────────────────────────

    /// The whole zoomable world: the focused sheet plus up to a few levels
    /// of ancestor context behind it, plus every big-enough instance's
    /// contents cross-faded in — recursively. One canvas, any depth.
    fn draw_world(&self, canvas: &mut Canvas<OpenGl>, app: &App) {
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
        let root_sim_path = if app.is_running() {
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
    fn draw_sheet_tree(
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

    fn draw_focus_grid(
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
    fn draw_notes(&self, canvas: &mut Canvas<OpenGl>, app: &App, ctx: &SheetCtx) {
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

    /// The display color of a whole net.
    fn net_color(&self, app: &App, ctx: &SheetCtx, net: &WireNet) -> Color {
        if let (Some(path), Mode::Run(session)) = (&ctx.sim_path, &app.mode) {
            let energized = net.source.as_ref().and_then(|src| {
                if src.component_id.starts_with(diycpu::convert::INPUT_PREFIX) {
                    return None; // pin values arrive via the instance inputs
                }
                let comp = ctx.components.iter().find(|c| c.id == src.component_id)?;
                session.port_value_at(path, is_primitive(&comp.component_type), src)
            });
            return match energized {
                Some(true) => theme::wire_on(),
                Some(false) => theme::wire_off(),
                None => theme::wire(),
            };
        }
        if !ctx.focus {
            return theme::wire();
        }
        if app.selection.net.as_deref() == Some(net.id.as_str()) {
            theme::wire_selected()
        } else {
            theme::wire()
        }
    }

    fn draw_wires(&self, canvas: &mut Canvas<OpenGl>, app: &App, ctx: &SheetCtx) {
        let abstractions = ctx.abstractions;
        let (components, wire_nets) = (ctx.components, ctx.wire_nets);
        let zoom = ctx.zoom();
        let width = (2.5 * zoom).clamp(0.5, 4.5);

        for net in wire_nets {
            let color = self.net_color(app, ctx, net);
            let selected =
                ctx.focus && app.selection.net.as_deref() == Some(net.id.as_str());
            let mut paint = Paint::color(color);
            paint.set_line_width(if selected { width + 1.0 } else { width });
            paint.set_line_join(femtovg::LineJoin::Round);
            paint.set_line_cap(femtovg::LineCap::Round);

            for i in 0..net.segments.len() {
                let Some(pts) =
                    layout::route_segment(abstractions, components, ctx.interface, net, i)
                else {
                    continue;
                };
                let mut p = Path::new();
                for (k, pt) in pts.iter().enumerate() {
                    let s = ctx.view.to_screen(*pt);
                    if k == 0 {
                        p.move_to(s[0], s[1]);
                    } else {
                        p.line_to(s[0], s[1]);
                    }
                }
                canvas.stroke_path(&p, &paint);
            }

            for junction in &net.junctions {
                let hovered = ctx.focus
                    && matches!(
                        &app.hover,
                        layout::Hit::Junction { junction_id, net_id }
                            if *junction_id == junction.id && *net_id == net.id
                    );
                let s = ctx.view.to_screen(junction.pos);
                let mut p = Path::new();
                p.circle(s[0], s[1], (3.5 * zoom).clamp(1.0, 5.5));
                canvas.fill_path(&p, &Paint::color(color));
                if hovered && !app.is_running() {
                    let mut ring_path = Path::new();
                    ring_path.circle(s[0], s[1], (6.5 * zoom).clamp(4.5, 9.0));
                    let mut ring = Paint::color(theme::accent());
                    ring.set_line_width(2.0);
                    canvas.stroke_path(&ring_path, &ring);
                }
            }
        }
    }

    /// Resolve where a wire anchor sits on the sheet.
    fn anchor_geom(&self, app: &App, anchor: &diycpu::editor::WirePoint) -> Option<([f32; 2], layout::ExitDir)> {
        use diycpu::editor::WirePoint;
        let abstractions = &app.editor.project.abstractions;
        let (components, wire_nets) = app.editor.sheet_data();
        match anchor {
            WirePoint::Port(p) => {
                layout::port_geom_any(abstractions, components, app.interface(), p)
                    .map(|(pos, side)| (pos, Some(side)))
            }
            WirePoint::Junction {
                net_id,
                junction_id,
            } => wire_nets
                .iter()
                .find(|n| &n.id == net_id)?
                .junctions
                .iter()
                .find(|j| &j.id == junction_id)
                .map(|j| (j.pos, None)),
            WirePoint::New(pos) => Some((*pos, None)),
        }
    }

    fn draw_wire_preview(&self, canvas: &mut Canvas<OpenGl>, app: &App) {
        let Gesture::DrawingWire { anchor } = &app.gesture else {
            return;
        };
        let abstractions = &app.editor.project.abstractions;
        let (components, wire_nets) = app.editor.sheet_data();
        let Some((from_pos, from_dir)) = self.anchor_geom(app, anchor) else {
            return;
        };

        let target = app.wire_target();
        let (end, end_dir, color) = match &target {
            WireTarget::Port(port) => {
                match layout::port_geom_any(abstractions, components, app.interface(), port) {
                    Some((pos, side)) => (pos, Some(side), theme::wire_preview()),
                    None => return,
                }
            }
            WireTarget::Node {
                net_id,
                junction_id,
            } => {
                let pos = wire_nets
                    .iter()
                    .find(|n| &n.id == net_id)
                    .and_then(|n| n.junctions.iter().find(|j| &j.id == junction_id))
                    .map(|j| j.pos);
                match pos {
                    Some(p) => (p, None, theme::wire_preview()),
                    None => return,
                }
            }
            WireTarget::Incompatible => (app.cursor_grid(), None, theme::wire_invalid()),
            WireTarget::Free(p) => (*p, None, theme::wire_preview()),
        };

        // The exact same routing as the committed wire will get.
        let pts = layout::route(from_pos, from_dir, end, end_dir);
        let zoom = app.camera.zoom;
        let mut paint = Paint::color(color);
        paint.set_line_width((2.5 * zoom).clamp(1.5, 4.5));
        paint.set_line_join(femtovg::LineJoin::Round);
        paint.set_line_cap(femtovg::LineCap::Round);
        let mut p = Path::new();
        for (k, pt) in pts.iter().enumerate() {
            let s = app.camera.grid_to_screen(*pt);
            if k == 0 {
                p.move_to(s[0], s[1]);
            } else {
                p.line_to(s[0], s[1]);
            }
        }
        canvas.stroke_path(&p, &paint);

        // Where the wire would land: node dot on empty space, ring on a
        // snappable port or node.
        let s = app.camera.grid_to_screen(end);
        match &target {
            WireTarget::Free(_) => {
                let mut dot = Path::new();
                dot.circle(s[0], s[1], (3.5 * zoom).clamp(2.0, 5.5));
                canvas.fill_path(&dot, &Paint::color(color));
            }
            WireTarget::Port(_) | WireTarget::Node { .. } => {
                let mut ring_path = Path::new();
                ring_path.circle(s[0], s[1], (7.0 * zoom).clamp(5.0, 10.0));
                let mut ring = Paint::color(theme::accent());
                ring.set_line_width(2.0);
                canvas.stroke_path(&ring_path, &ring);
            }
            WireTarget::Incompatible => {}
        }
    }

    /// While a node drags, ghost its segments at the target position (the
    /// One component, plus — when it's an abstraction instance big enough
    /// on screen — its contents cross-faded in, recursively.
    #[allow(clippy::too_many_arguments)]
    fn draw_component_tree(
        &self,
        canvas: &mut Canvas<OpenGl>,
        app: &App,
        project: &diycpu::project::Project,
        ctx: &SheetCtx,
        comp: &diycpu::project::ProjectComponent,
        slice: &[PathSeg],
        on_path: bool,
        depth: usize,
        grid_clip: Option<[f32; 4]>,
    ) {
        let abstractions = ctx.abstractions;
        let rect = layout::footprint(abstractions, comp);
        let tl = ctx.view.to_screen([rect.x, rect.y]);
        let cell = ctx.view.scale;
        let (w, h) = (rect.w * cell, rect.h * cell);
        let zoom = ctx.zoom();
        let selected = ctx.focus && app.selection.components.contains(&comp.id);
        let corner = (6.0 * zoom).clamp(1.0, 9.0);

        // How "open" is this instance? 0 = closed chip, 1 = showing its
        // contents. Driven purely by its size relative to the viewport.
        let def_index = abstractions
            .iter()
            .position(|a| a.name == comp.component_type);
        let mut open_frac = 0.0f32;
        if let Some(i) = def_index {
            let viewport = {
                let r = app.canvas_rect();
                r.w.min(r.h)
            };
            let t = w.max(h) / viewport.max(1.0);
            if depth < MAX_DEPTH {
                let map =
                    diycpu::layout::instance_inner_map(abstractions, &abstractions[i], rect);
                if ctx.view.scale * map.s >= MIN_CHILD_SCALE {
                    open_frac = ((t - OPEN_T0) / (OPEN_T1 - OPEN_T0)).clamp(0.0, 1.0);
                    // Smoothstep for a gentler fade.
                    open_frac = open_frac * open_frac * (3.0 - 2.0 * open_frac);
                }
            }
        }

        // Soft shadow + body (always drawn: the body doubles as the child
        // sheet's paper).
        let mut shadow = Path::new();
        shadow.rounded_rect(tl[0], tl[1] + 2.0 * zoom, w, h, corner);
        canvas.fill_path(&shadow, &Paint::color(theme::chip_shadow()));

        let mut body = Path::new();
        body.rounded_rect(tl[0], tl[1], w, h, corner);
        let fill = if comp.component_type == "dff" || comp.component_type == "const" {
            match (&app.mode, &ctx.sim_path) {
                (Mode::Run(session), Some(path)) => {
                    match session.component_value_at(path, &comp.id) {
                        Some(true) => theme::badge_on_bg(),
                        Some(false) => theme::badge_off_bg(),
                        None => theme::chip_fill(),
                    }
                }
                _ => theme::chip_fill(),
            }
        } else {
            theme::chip_fill()
        };
        canvas.fill_path(&body, &Paint::color(fill));
        if selected {
            canvas.fill_path(&body, &Paint::color(theme::accent_soft()));
        }

        // Contents (under the border and ports).
        if open_frac > 0.0 {
            if let Some(i) = def_index {
                let onscreen = tl[0] < app.window_size[0]
                    && tl[1] < app.window_size[1]
                    && tl[0] + w > 0.0
                    && tl[1] + h > 0.0;
                if onscreen {
                    let map = diycpu::layout::instance_inner_map(
                        abstractions,
                        &abstractions[i],
                        rect,
                    );
                    let (child_slice, child_on_path): (&[PathSeg], bool) =
                        match (on_path, slice.first()) {
                            (true, Some(seg)) if seg.instance_id == comp.id => {
                                (&slice[1..], true)
                            }
                            _ => (&[], false),
                        };
                    let child_sim_path = ctx.sim_path.as_ref().map(|p| {
                        let mut p = p.clone();
                        p.push(comp.id.clone());
                        p
                    });
                    self.draw_sheet_tree(
                        canvas,
                        app,
                        project,
                        SheetRef::Abstraction(i),
                        ctx.view.through(&map),
                        ctx.alpha * open_frac,
                        child_sim_path,
                        child_slice,
                        child_on_path,
                        depth + 1,
                        grid_clip,
                    );
                    canvas.set_global_alpha(ctx.alpha);
                }
            }
        }

        let mut border = Paint::color(if selected {
            theme::accent()
        } else {
            theme::chip_border()
        });
        border.set_line_width(if selected { 2.5 } else { 1.5 });
        canvas.stroke_path(&body, &border);

        // The closed chrome (name, glyph, value) fades out as the contents
        // fade in.
        let chrome_alpha = ctx.alpha * (1.0 - open_frac);
        if chrome_alpha > 0.01 {
            canvas.set_global_alpha(chrome_alpha);

            let label_size = (13.0 * zoom).clamp(5.0, 22.0);
            if label_size >= 5.0 && !is_primitive(&comp.component_type) {
                self.text(
                    canvas,
                    tl[0] + w / 2.0,
                    tl[1] + h / 2.0 - 1.0,
                    &comp.component_type,
                    label_size,
                    theme::chip_label(),
                    Align::Center,
                    Baseline::Middle,
                );
            }

            match comp.component_type.as_str() {
                "clk" => self.draw_clock_glyph(canvas, zoom, tl, w, h),
                "nand" => self.draw_nand_glyph(canvas, zoom, tl, w, h),
                "const" => {
                    // A constant signal: its face is the value it feeds out.
                    let value = match (&app.mode, &ctx.sim_path) {
                        (Mode::Run(session), Some(path)) => {
                            session.component_value_at(path, &comp.id)
                        }
                        _ => comp.init,
                    }
                    .unwrap_or(false);
                    let digit_size = (18.0 * zoom).clamp(6.0, 30.0);
                    // A ring around the digit says "this is a source".
                    let cx = tl[0] + w * 0.45;
                    let cy = tl[1] + h / 2.0;
                    let mut ring = Path::new();
                    ring.circle(cx, cy, digit_size * 0.75);
                    let mut rp = Paint::color(if value {
                        theme::value_digit_on()
                    } else {
                        theme::value_digit_off()
                    });
                    rp.set_line_width((1.6 * zoom).clamp(0.6, 2.5));
                    canvas.stroke_path(&ring, &rp);
                    self.text(
                        canvas,
                        cx,
                        cy,
                        if value { "1" } else { "0" },
                        digit_size,
                        if value {
                            theme::value_digit_on()
                        } else {
                            theme::value_digit_off()
                        },
                        Align::Center,
                        Baseline::Middle,
                    );
                }
                "dff" => {
                    // The flip-flop's face IS its bit: init while editing,
                    // live state while running. Double-click flips the init.
                    let value = match (&app.mode, &ctx.sim_path) {
                        (Mode::Run(session), Some(path)) => {
                            session.component_value_at(path, &comp.id)
                        }
                        _ => comp.init,
                    }
                    .unwrap_or(false);
                    let digit_size = (20.0 * zoom).clamp(6.0, 32.0);
                    self.text(
                        canvas,
                        tl[0] + w * 0.58,
                        tl[1] + h / 2.0,
                        if value { "1" } else { "0" },
                        digit_size,
                        if value {
                            theme::value_digit_on()
                        } else {
                            theme::value_digit_off()
                        },
                        Align::Center,
                        Baseline::Middle,
                    );
                }
                _ => {}
            }

            // Run-mode value badge for the primitives whose face doesn't
            // already show it.
            if let (Mode::Run(session), Some(path)) = (&app.mode, &ctx.sim_path) {
                if is_primitive(&comp.component_type)
                    && comp.component_type != "dff"
                    && comp.component_type != "const"
                {
                    if let Some(v) = session.component_value_at(path, &comp.id) {
                        self.draw_badge(canvas, zoom, tl, w, v);
                    }
                }
            }
            canvas.set_global_alpha(ctx.alpha);
        }

        // Ports (the interface stays visible even when open).
        let hover_port = match &app.hover {
            layout::Hit::Port {
                component_id,
                port_name,
                ..
            } if ctx.focus && *component_id == comp.id => Some(port_name.clone()),
            _ => None,
        };
        let ports = layout::component_ports(abstractions, comp);
        let show_port_labels = zoom >= 0.5
            && (comp.component_type == "dff" || !is_primitive(&comp.component_type));
        for port in &ports {
            let s = ctx.view.to_screen(port.pos);
            let r = (5.0 * zoom).clamp(1.0, 8.0);
            let hovered = hover_port.as_deref() == Some(port.info.name.as_str());

            let fill = match (&app.mode, &ctx.sim_path, port.info.kind) {
                (Mode::Run(session), Some(path), PortKind::Output) => {
                    let port_ref = diycpu::project::PortRef {
                        component_id: comp.id.clone(),
                        port_name: port.info.name.clone(),
                    };
                    match session.port_value_at(
                        path,
                        is_primitive(&comp.component_type),
                        &port_ref,
                    ) {
                        Some(true) => theme::value_on(),
                        Some(false) => theme::wire_off(),
                        None => theme::port_fill_output(),
                    }
                }
                (_, _, PortKind::Output) => theme::port_fill_output(),
                (_, _, PortKind::Input) => theme::port_fill_input(),
            };

            let mut p = Path::new();
            p.circle(s[0], s[1], r);
            canvas.fill_path(&p, &Paint::color(fill));
            let mut ring = Paint::color(if hovered {
                theme::port_hover()
            } else {
                theme::port_ring()
            });
            ring.set_line_width(if hovered { 2.5 } else { 1.5 });
            canvas.stroke_path(&p, &ring);

            if show_port_labels {
                // Port names belong to the closed look.
                if chrome_alpha > 0.01 {
                    canvas.set_global_alpha(chrome_alpha);
                    let size = (9.0 * zoom).clamp(7.0, 13.0);
                    let pad = r + 4.0;
                    // Label sits just inside the body, opposite the exit.
                    let (lx, ly, align, baseline) = match port.side {
                        layout::Side::Left => {
                            (s[0] + pad, s[1], Align::Left, Baseline::Middle)
                        }
                        layout::Side::Right => {
                            (s[0] - pad, s[1], Align::Right, Baseline::Middle)
                        }
                        layout::Side::Top => {
                            (s[0], s[1] + pad, Align::Center, Baseline::Top)
                        }
                        layout::Side::Bottom => {
                            (s[0], s[1] - pad, Align::Center, Baseline::Bottom)
                        }
                    };
                    self.text(
                        canvas,
                        lx,
                        ly,
                        &port.info.name,
                        size,
                        theme::port_label(),
                        align,
                        baseline,
                    );
                    canvas.set_global_alpha(ctx.alpha);
                }
            }
        }
    }

    fn draw_clock_glyph(
        &self,
        canvas: &mut Canvas<OpenGl>,
        zoom: f32,
        tl: [f32; 2],
        w: f32,
        h: f32,
    ) {
        // A little square wave centered in the body.
        let cx = tl[0] + w / 2.0;
        let cy = tl[1] + h / 2.0;
        let half_w = w * 0.28;
        let half_h = h * 0.16;
        let mut p = Path::new();
        p.move_to(cx - half_w, cy + half_h);
        p.line_to(cx - half_w / 2.0, cy + half_h);
        p.line_to(cx - half_w / 2.0, cy - half_h);
        p.line_to(cx, cy - half_h);
        p.line_to(cx, cy + half_h);
        p.line_to(cx + half_w / 2.0, cy + half_h);
        p.line_to(cx + half_w / 2.0, cy - half_h);
        p.line_to(cx + half_w, cy - half_h);
        let mut paint = Paint::color(theme::chip_label());
        paint.set_line_width((2.0 * zoom).clamp(0.6, 3.0));
        paint.set_line_join(femtovg::LineJoin::Round);
        canvas.stroke_path(&p, &paint);
    }

    /// The MIL-spec NAND symbol: a D-shape with the inversion bubble.
    fn draw_nand_glyph(
        &self,
        canvas: &mut Canvas<OpenGl>,
        zoom: f32,
        tl: [f32; 2],
        w: f32,
        h: f32,
    ) {
        let cx = tl[0] + w / 2.0;
        let cy = tl[1] + h / 2.0;
        let gh = h * 0.42; // glyph height
        let r = gh / 2.0;
        let left = cx - gh * 0.62;
        let flat = cx + gh * 0.05; // where the flat part meets the arc

        let mut body = Path::new();
        body.move_to(flat, cy - r);
        body.line_to(left, cy - r);
        body.line_to(left, cy + r);
        body.line_to(flat, cy + r);
        body.arc(
            flat,
            cy,
            r,
            std::f32::consts::FRAC_PI_2,
            -std::f32::consts::FRAC_PI_2,
            femtovg::Solidity::Solid,
        );
        let mut paint = Paint::color(theme::chip_label());
        paint.set_line_width((2.0 * zoom).clamp(0.6, 3.0));
        paint.set_line_join(femtovg::LineJoin::Round);
        canvas.stroke_path(&body, &paint);

        // Inversion bubble on the output.
        let rb = (r * 0.22).max(0.8);
        let mut bubble = Path::new();
        bubble.circle(flat + r + rb, cy, rb);
        canvas.stroke_path(&bubble, &paint);
    }

    fn draw_badge(&self, canvas: &mut Canvas<OpenGl>, zoom: f32, tl: [f32; 2], w: f32, on: bool) {
        let size = (14.0 * zoom).clamp(9.0, 18.0);
        let x = tl[0] + w - size * 0.7;
        let y = tl[1] - size * 0.3;
        let mut p = Path::new();
        p.rounded_rect(x - size / 2.0, y - size / 2.0, size, size, size * 0.25);
        canvas.fill_path(
            &p,
            &Paint::color(if on {
                theme::badge_on_bg()
            } else {
                theme::badge_off_bg()
            }),
        );
        let mut ring = Paint::color(if on { theme::value_on() } else { theme::value_off() });
        ring.set_line_width(1.2);
        canvas.stroke_path(&p, &ring);
        self.text(
            canvas,
            x,
            y,
            if on { "1" } else { "0" },
            size * 0.7,
            if on { theme::value_on() } else { theme::value_off() },
            Align::Center,
            Baseline::Middle,
        );
    }

    /// Interface pins on the rails beside an abstraction's content: a port
    /// circle plus a clickable name plate.
    fn draw_interface_pins(&self, canvas: &mut Canvas<OpenGl>, app: &App, ctx: &SheetCtx) {
        let Some(def) = ctx.interface else {
            return;
        };
        let abstractions = ctx.abstractions;
        let zoom = ctx.zoom();
        let cell = ctx.view.scale;

        for pin in layout::interface_pins(abstractions, def) {
            let s = ctx.view.to_screen(pin.pos);
            let selected = ctx.focus
                && app
                    .selection
                    .pin
                    .as_ref()
                    .is_some_and(|(k, n)| *k == pin.io && *n == pin.name);
            let hovered_port = ctx.focus
                && matches!(
                    &app.hover,
                    layout::Hit::Port { component_id, .. }
                        if *component_id == pin.port_ref().component_id
                );

            // Name plate.
            let plate = pin.plate;
            let tl = ctx.view.to_screen([plate.x, plate.y]);
            let mut p = Path::new();
            p.rounded_rect(
                tl[0],
                tl[1],
                plate.w * cell,
                plate.h * cell,
                (5.0 * zoom).clamp(2.0, 8.0),
            );
            canvas.fill_path(
                &p,
                &Paint::color(match pin.io {
                    PortKind::Input => theme::pin_plate_input_bg(),
                    PortKind::Output => theme::pin_plate_output_bg(),
                }),
            );
            let mut border = Paint::color(if selected {
                theme::accent()
            } else {
                theme::pin_plate_border()
            });
            border.set_line_width(if selected { 2.0 } else { 1.2 });
            canvas.stroke_path(&p, &border);

            let label_size = (12.0 * zoom).clamp(4.0, 16.0);
            self.text(
                canvas,
                tl[0] + plate.w * cell / 2.0,
                tl[1] + plate.h * cell / 2.0,
                &pin.name,
                label_size,
                theme::pin_label(),
                Align::Center,
                Baseline::Middle,
            );

            // Port circle (wire attachment point). Input pins drive wires,
            // so they get the filled "source" look.
            let r = (5.0 * zoom).clamp(2.5, 8.0);
            let mut c = Path::new();
            c.circle(s[0], s[1], r);
            canvas.fill_path(
                &c,
                &Paint::color(match pin.io {
                    PortKind::Input => theme::port_fill_output(),
                    PortKind::Output => theme::port_fill_input(),
                }),
            );
            let mut ring = Paint::color(if hovered_port {
                theme::port_hover()
            } else {
                theme::port_ring()
            });
            ring.set_line_width(if hovered_port { 2.5 } else { 1.5 });
            canvas.stroke_path(&c, &ring);

            // Tie plate and circle together with a short leader.
            let mut leader = Path::new();
            match pin.io {
                PortKind::Input => {
                    leader.move_to(tl[0] + plate.w * cell, s[1]);
                    leader.line_to(s[0] - r, s[1]);
                }
                PortKind::Output => {
                    leader.move_to(s[0] + r, s[1]);
                    leader.line_to(tl[0], s[1]);
                }
            }
            let mut lp = Paint::color(theme::pin_plate_border());
            lp.set_line_width(1.2);
            canvas.stroke_path(&leader, &lp);
        }
    }

    /// Output pin connections are stored in the abstraction's `outputs` map,
    /// so their wires are derived: routed fresh every frame from the target
    /// port to the pin. No stored geometry to go stale.
    fn draw_derived_output_wires(
        &self,
        canvas: &mut Canvas<OpenGl>,
        _app: &App,
        ctx: &SheetCtx,
    ) {
        let Some(def) = ctx.interface else {
            return;
        };
        let abstractions = ctx.abstractions;
        let zoom = ctx.zoom();

        let pins = layout::interface_pins(abstractions, def);
        for (name, target) in &def.outputs {
            if target.is_empty() {
                continue;
            }
            let Some(pin) = pins
                .iter()
                .find(|p| p.io == PortKind::Output && &p.name == name)
            else {
                continue;
            };
            let port = layout::output_target_port(target);
            let Some((from, from_side)) =
                layout::port_geom_any(abstractions, ctx.components, Some(def), &port)
            else {
                continue;
            };
            let pts = layout::route(from, Some(from_side), pin.pos, Some(layout::Side::Left));
            let mut p = Path::new();
            for (k, pt) in pts.iter().enumerate() {
                let s = ctx.view.to_screen(*pt);
                if k == 0 {
                    p.move_to(s[0], s[1]);
                } else {
                    p.line_to(s[0], s[1]);
                }
            }
            let mut paint = Paint::color(theme::wire());
            paint.set_line_width((2.5 * zoom).clamp(0.5, 4.5));
            paint.set_line_join(femtovg::LineJoin::Round);
            paint.set_line_cap(femtovg::LineCap::Round);
            canvas.stroke_path(&p, &paint);
        }
    }

    /// The component-layout dialog: drag ports around the closed
    /// rectangle's edges, drag the corner to resize.
    fn draw_layout_dialog(&self, canvas: &mut Canvas<OpenGl>, app: &App) {
        let Some(geom) = app.dialog_geometry() else {
            return;
        };
        let Some(def) = app.dialog_def() else {
            return;
        };
        let [w, h] = app.window_size;

        let mut dim = Path::new();
        dim.rect(0.0, 0.0, w, h);
        canvas.fill_path(&dim, &Paint::color(femtovg::Color::rgba(30, 33, 40, 70)));

        let p = &geom.panel;
        let mut panel = Path::new();
        panel.rounded_rect(p.x, p.y, p.w, p.h, 10.0);
        canvas.fill_path(&panel, &Paint::color(theme::canvas_bg()));
        let mut border = Paint::color(theme::button_border());
        border.set_line_width(1.5);
        canvas.stroke_path(&panel, &border);

        self.text(
            canvas,
            p.x + 16.0,
            p.y + 24.0,
            &format!("Component layout — {}", def.name),
            15.0,
            theme::text(),
            Align::Left,
            Baseline::Middle,
        );
        self.text(
            canvas,
            geom.close_rect.x + geom.close_rect.w / 2.0,
            geom.close_rect.y + geom.close_rect.h / 2.0,
            "✕",
            14.0,
            if geom.close_rect.contains(app.cursor) {
                theme::danger()
            } else {
                theme::text_dim()
            },
            Align::Center,
            Baseline::Middle,
        );
        self.text(
            canvas,
            p.x + 16.0,
            p.y + 46.0,
            "Drag a port to any edge · drag the corner to resize · Esc closes",
            11.5,
            theme::text_dim(),
            Align::Left,
            Baseline::Middle,
        );

        // The component preview.
        let r = &geom.preview;
        let mut body = Path::new();
        body.rounded_rect(r.x, r.y, r.w, r.h, 8.0);
        canvas.fill_path(&body, &Paint::color(theme::chip_fill()));
        let mut bp = Paint::color(theme::chip_border());
        bp.set_line_width(2.0);
        canvas.stroke_path(&body, &bp);
        self.text(
            canvas,
            r.x + r.w / 2.0,
            r.y + r.h / 2.0,
            &def.name,
            15.0,
            theme::chip_label(),
            Align::Center,
            Baseline::Middle,
        );

        for (name, pos, is_input) in &geom.ports {
            let hovered = {
                let dx = app.cursor[0] - pos[0];
                let dy = app.cursor[1] - pos[1];
                (dx * dx + dy * dy).sqrt() <= 12.0
            };
            let mut c = Path::new();
            c.circle(pos[0], pos[1], 7.0);
            canvas.fill_path(
                &c,
                &Paint::color(if *is_input {
                    theme::port_fill_input()
                } else {
                    theme::port_fill_output()
                }),
            );
            let mut ring = Paint::color(if hovered {
                theme::accent()
            } else {
                theme::port_ring()
            });
            ring.set_line_width(if hovered { 2.5 } else { 1.5 });
            canvas.stroke_path(&c, &ring);

            // Label outside the body, next to the port.
            let inward = [
                (r.x + r.w / 2.0 - pos[0]).signum(),
                (r.y + r.h / 2.0 - pos[1]).signum(),
            ];
            let on_vertical_edge = (pos[0] - r.x).abs() < 1.0 || (pos[0] - (r.x + r.w)).abs() < 1.0;
            if on_vertical_edge {
                self.text(
                    canvas,
                    pos[0] - inward[0] * 12.0,
                    pos[1],
                    name,
                    12.0,
                    theme::text(),
                    if inward[0] > 0.0 { Align::Right } else { Align::Left },
                    Baseline::Middle,
                );
            } else {
                self.text(
                    canvas,
                    pos[0],
                    pos[1] - inward[1] * 16.0,
                    name,
                    12.0,
                    theme::text(),
                    Align::Center,
                    Baseline::Middle,
                );
            }
        }

        // Resize handle + size readout.
        let hr = &geom.resize_handle;
        let mut handle = Path::new();
        handle.rounded_rect(hr.x, hr.y, hr.w, hr.h, 4.0);
        canvas.fill_path(
            &handle,
            &Paint::color(if hr.contains(app.cursor) {
                theme::accent()
            } else {
                theme::button_border()
            }),
        );
        self.text(
            canvas,
            p.x + p.w / 2.0,
            p.y + p.h - 26.0,
            &format!("{} × {} cells", geom.size.0, geom.size.1),
            12.5,
            theme::text_dim(),
            Align::Center,
            Baseline::Middle,
        );
    }

    fn draw_prompt(&self, canvas: &mut Canvas<OpenGl>, app: &App) {
        let Some(prompt) = &app.prompt else {
            return;
        };
        let [w, h] = app.window_size;

        // Dim the world so focus is unambiguous.
        let mut dim = Path::new();
        dim.rect(0.0, 0.0, w, h);
        canvas.fill_path(&dim, &Paint::color(femtovg::Color::rgba(30, 33, 40, 70)));

        let bw = 380.0;
        let bh = 110.0;
        let bx = (w - bw) / 2.0;
        let by = (h - bh) / 2.5;

        let mut boxp = Path::new();
        boxp.rounded_rect(bx, by, bw, bh, 10.0);
        canvas.fill_path(&boxp, &Paint::color(theme::chip_fill()));
        let mut border = Paint::color(theme::button_border());
        border.set_line_width(1.5);
        canvas.stroke_path(&boxp, &border);

        self.text(
            canvas,
            bx + 16.0,
            by + 26.0,
            &prompt.title,
            14.0,
            theme::text(),
            Align::Left,
            Baseline::Middle,
        );

        // Input field with caret.
        let mut field = Path::new();
        field.rounded_rect(bx + 16.0, by + 44.0, bw - 32.0, 30.0, 6.0);
        canvas.fill_path(&field, &Paint::color(theme::panel_bg()));
        let mut fb = Paint::color(theme::accent());
        fb.set_line_width(1.5);
        canvas.stroke_path(&field, &fb);
        self.text(
            canvas,
            bx + 24.0,
            by + 59.0,
            &format!("{}▏", prompt.value),
            14.0,
            theme::text(),
            Align::Left,
            Baseline::Middle,
        );

        self.text(
            canvas,
            bx + 16.0,
            by + 92.0,
            "Enter to confirm  ·  Esc to cancel",
            11.0,
            theme::text_dim(),
            Align::Left,
            Baseline::Middle,
        );
    }

    /// Dragged items render live in the world; this overlay only flags an
    /// invalid drop position.
    fn draw_drag_ghosts(&self, canvas: &mut Canvas<OpenGl>, app: &App) {
        let Gesture::DraggingComponents {
            ids,
            note_ids,
            delta,
            ..
        } = &app.gesture
        else {
            return;
        };
        if *delta == [0, 0] {
            return;
        }
        let valid = app.drag_is_valid(ids, *delta);
        if valid {
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
            canvas.fill_path(
                &p,
                &Paint::color(if valid {
                    theme::accent_soft()
                } else {
                    theme::ghost_invalid()
                }),
            );
            let mut stroke = Paint::color(if valid { theme::accent() } else { theme::danger() });
            stroke.set_line_width(1.5);
            canvas.stroke_path(&p, &stroke);
        }
    }

    fn draw_placement_ghost(&self, canvas: &mut Canvas<OpenGl>, app: &App) {
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
            id: String::new(),
            component_type: type_name.clone(),
            init: None,
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

    fn draw_band(&self, canvas: &mut Canvas<OpenGl>, app: &App) {
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

    // ── Chrome ───────────────────────────────────────────────────────────

    fn draw_palette(&self, canvas: &mut Canvas<OpenGl>, app: &App, ui: &UiLayout) {
        let [_, h] = app.window_size;
        let mut p = Path::new();
        p.rect(0.0, 0.0, theme::PALETTE_WIDTH, h);
        canvas.fill_path(&p, &Paint::color(theme::panel_bg()));
        let mut divider = Path::new();
        divider.move_to(theme::PALETTE_WIDTH - 0.5, 0.0);
        divider.line_to(theme::PALETTE_WIDTH - 0.5, h);
        let mut dp = Paint::color(theme::panel_border());
        dp.set_line_width(1.0);
        canvas.stroke_path(&divider, &dp);

        for (label, y) in &ui.section_headers {
            self.text(
                canvas,
                12.0,
                *y,
                label,
                10.0,
                theme::text_dim(),
                Align::Left,
                Baseline::Middle,
            );
        }

        for item in &ui.palette_items {
            let r = &item.rect;
            let hovered = r.contains(app.cursor) && item.enabled;
            if item.armed || hovered {
                let mut bg = Path::new();
                bg.rounded_rect(r.x, r.y, r.w, r.h, 6.0);
                canvas.fill_path(
                    &bg,
                    &Paint::color(if item.armed {
                        theme::panel_item_armed()
                    } else {
                        theme::panel_item_hover()
                    }),
                );
                if item.armed {
                    let mut stroke = Paint::color(theme::accent());
                    stroke.set_line_width(1.5);
                    canvas.stroke_path(&bg, &stroke);
                }
            }

            // Mini glyph: a sticky note for the note tool, a chip otherwise.
            let glyph_x = r.x + 8.0;
            let glyph_y = r.y + r.h / 2.0;
            let color = if item.enabled {
                theme::chip_border()
            } else {
                theme::text_disabled()
            };
            let mut cp = Paint::color(color);
            cp.set_line_width(1.3);
            if item.type_name == "note" {
                let mut sticky = Path::new();
                sticky.rounded_rect(glyph_x, glyph_y - 7.0, 18.0, 14.0, 2.0);
                canvas.stroke_path(&sticky, &cp);
                for dy in [-2.5, 1.5] {
                    let mut line = Path::new();
                    line.move_to(glyph_x + 3.5, glyph_y + dy);
                    line.line_to(glyph_x + 14.5, glyph_y + dy);
                    canvas.stroke_path(&line, &cp);
                }
            } else {
                let mut chip = Path::new();
                chip.rounded_rect(glyph_x, glyph_y - 7.0, 18.0, 14.0, 2.5);
                canvas.stroke_path(&chip, &cp);
                for dy in [-3.0, 3.0] {
                    let mut pin = Path::new();
                    pin.move_to(glyph_x - 3.0, glyph_y + dy);
                    pin.line_to(glyph_x, glyph_y + dy);
                    pin.move_to(glyph_x + 18.0, glyph_y + dy);
                    pin.line_to(glyph_x + 21.0, glyph_y + dy);
                    canvas.stroke_path(&pin, &cp);
                }
            }

            self.text(
                canvas,
                r.x + 36.0,
                glyph_y,
                &item.type_name,
                13.0,
                if item.enabled { theme::text() } else { theme::text_disabled() },
                Align::Left,
                Baseline::Middle,
            );

            // Open-for-editing affordance on abstraction rows.
            if let Some(er) = &item.edit_rect {
                let hovered_edit = er.contains(app.cursor);
                if hovered_edit || r.contains(app.cursor) {
                    let mut bg = Path::new();
                    bg.rounded_rect(er.x, er.y, er.w, er.h, 4.0);
                    if hovered_edit {
                        canvas.fill_path(&bg, &Paint::color(theme::accent_soft()));
                    }
                    self.text(
                        canvas,
                        er.x + er.w / 2.0,
                        er.y + er.h / 2.0,
                        "↗",
                        13.0,
                        if hovered_edit { theme::accent() } else { theme::text_dim() },
                        Align::Center,
                        Baseline::Middle,
                    );
                }
            }
        }
    }

    fn draw_topbar(&self, canvas: &mut Canvas<OpenGl>, app: &App, ui: &UiLayout) {
        let [w, _] = app.window_size;
        let mut p = Path::new();
        p.rect(0.0, 0.0, w, theme::TOPBAR_HEIGHT);
        canvas.fill_path(&p, &Paint::color(theme::panel_bg()));
        let mut divider = Path::new();
        divider.move_to(0.0, theme::TOPBAR_HEIGHT - 0.5);
        divider.line_to(w, theme::TOPBAR_HEIGHT - 0.5);
        let mut dp = Paint::color(theme::panel_border());
        dp.set_line_width(1.0);
        canvas.stroke_path(&divider, &dp);

        // Title: the zoom breadcrumb.
        let mut title = app.breadcrumb().join(" ▸ ");
        if app.editor.dirty {
            title.push_str(" •");
        }
        // The title is clickable (rename); hint with a hover underline.
        if ui.title_rect.contains(app.cursor) && !app.is_running() {
            let mut underline = Path::new();
            let y = theme::TOPBAR_HEIGHT / 2.0 + 9.0;
            underline.move_to(12.0, y);
            underline.line_to(12.0 + title.chars().count() as f32 * 7.5, y);
            let mut up = Paint::color(theme::text_dim());
            up.set_line_width(1.0);
            canvas.stroke_path(&underline, &up);
        }
        self.text(
            canvas,
            12.0,
            theme::TOPBAR_HEIGHT / 2.0,
            &title,
            14.0,
            theme::text(),
            Align::Left,
            Baseline::Middle,
        );

        for button in &ui.buttons {
            let r = &button.rect;
            let hovered = r.contains(app.cursor) && button.enabled;
            let mut bg = Path::new();
            bg.rounded_rect(r.x, r.y, r.w, r.h, 6.0);
            canvas.fill_path(
                &bg,
                &Paint::color(if button.active {
                    theme::run_active_bg()
                } else if hovered {
                    theme::button_hover_bg()
                } else {
                    theme::button_bg()
                }),
            );
            let mut stroke = Paint::color(if button.active {
                theme::run_active_border()
            } else {
                theme::button_border()
            });
            stroke.set_line_width(1.2);
            canvas.stroke_path(&bg, &stroke);

            let label_color = if button.enabled {
                theme::text()
            } else {
                theme::text_disabled()
            };
            match button.hotkey {
                // Blender-style: label on the left, dim hotkey on the right.
                Some(hotkey) => {
                    self.text(
                        canvas,
                        r.x + 10.0,
                        r.y + r.h / 2.0,
                        &button.label,
                        12.5,
                        label_color,
                        Align::Left,
                        Baseline::Middle,
                    );
                    self.text(
                        canvas,
                        r.x + r.w - 8.0,
                        r.y + r.h / 2.0,
                        hotkey,
                        10.0,
                        theme::text_dim(),
                        Align::Right,
                        Baseline::Middle,
                    );
                }
                None => self.text(
                    canvas,
                    r.x + r.w / 2.0,
                    r.y + r.h / 2.0,
                    &button.label,
                    12.5,
                    label_color,
                    Align::Center,
                    Baseline::Middle,
                ),
            }
        }

        if let Some((label, x, y)) = &ui.step_label {
            self.text(
                canvas,
                *x,
                *y,
                label,
                13.0,
                theme::text(),
                Align::Center,
                Baseline::Middle,
            );
        }
    }

    fn draw_statusbar(&self, canvas: &mut Canvas<OpenGl>, app: &App) {
        let [w, h] = app.window_size;
        let y = h - theme::STATUSBAR_HEIGHT;
        let mut p = Path::new();
        p.rect(0.0, y, w, theme::STATUSBAR_HEIGHT);
        canvas.fill_path(&p, &Paint::color(theme::panel_bg()));
        let mut divider = Path::new();
        divider.move_to(0.0, y + 0.5);
        divider.line_to(w, y + 0.5);
        let mut dp = Paint::color(theme::panel_border());
        dp.set_line_width(1.0);
        canvas.stroke_path(&divider, &dp);

        let mid = y + theme::STATUSBAR_HEIGHT / 2.0;
        self.text(
            canvas,
            12.0,
            mid,
            &app.status.text,
            12.0,
            if app.status.is_error {
                theme::status_error()
            } else {
                theme::status_info()
            },
            Align::Left,
            Baseline::Middle,
        );

        let g = app.cursor_grid();
        let right = format!(
            "{}%  ·  {:.0}, {:.0}",
            (app.camera.zoom * 100.0).round(),
            g[0].floor(),
            g[1].floor()
        );
        self.text(
            canvas,
            w - 12.0,
            mid,
            &right,
            12.0,
            theme::text_dim(),
            Align::Right,
            Baseline::Middle,
        );
    }

    fn draw_tooltip(&self, canvas: &mut Canvas<OpenGl>, app: &App) {
        if !matches!(app.gesture, Gesture::Idle)
            || app.layout_dialog.is_some()
            || app.prompt.is_some()
        {
            return;
        }
        let lines: Vec<String> = match &app.hover {
            layout::Hit::Port {
                component_id,
                port_name,
                kind,
            } => {
                if let Some(pin) = component_id
                    .strip_prefix("INPUT:")
                    .or_else(|| component_id.strip_prefix("OUTPUT:"))
                {
                    vec![format!(
                        "pin \"{pin}\" — drag to wire it {}",
                        match kind {
                            PortKind::Output => "into the circuit",
                            PortKind::Input => "from a component",
                        }
                    )]
                } else {
                    vec![format!(
                        "{} \"{port_name}\"",
                        match kind {
                            PortKind::Input => "input",
                            PortKind::Output => "output",
                        }
                    )]
                }
            }
            layout::Hit::Pin { name, kind } => vec![format!(
                "{} pin \"{name}\" — double-click renames, Delete removes",
                match kind {
                    PortKind::Input => "input",
                    PortKind::Output => "output",
                }
            )],
            layout::Hit::Junction { .. } => {
                if app.is_running() {
                    return;
                }
                vec!["node — click to wire from it, drag to move, Delete removes".into()]
            }
            layout::Hit::Wire { .. } => {
                if app.is_running() {
                    return;
                }
                vec![
                    "wire — double-click adds a node, Delete removes this segment".into(),
                ]
            }
            layout::Hit::Note { .. } => {
                if app.is_running() {
                    return;
                }
                vec!["note — double-click to edit".into()]
            }
            layout::Hit::Component { component_id } => {
                let (components, _) = app.editor.sheet_data();
                let Some(comp) = components.iter().find(|c| &c.id == component_id) else {
                    return;
                };
                let mut lines = match comp.component_type.as_str() {
                    "clk" => vec!["clock — toggles every step".to_string()],
                    "nand" => vec!["NAND gate".to_string()],
                    "dff" | "const" => vec![],
                    other => vec![format!("{other} — zoom in or double-click to open")],
                };
                if let Mode::Run(session) = &app.mode {
                    if let Some(v) = session.component_value(&comp.id) {
                        lines.push(format!("value: {}", if v { "1" } else { "0" }));
                    }
                } else if comp.component_type == "dff" {
                    lines.push(format!(
                        "flip-flop — starts at {}, double-click to flip",
                        if comp.init.unwrap_or(false) { "1" } else { "0" }
                    ));
                } else if comp.component_type == "const" {
                    lines.push(format!(
                        "constant {} — double-click to flip",
                        if comp.init.unwrap_or(false) { "1" } else { "0" }
                    ));
                }
                if lines.is_empty() {
                    return;
                }
                lines
            }
            _ => return,
        };

        let size = 12.0;
        let line_h = size + 5.0;
        let mut paint = Paint::color(theme::tooltip_text());
        paint.set_font(&[self.font]);
        paint.set_font_size(size);
        let width = lines
            .iter()
            .map(|l| {
                canvas
                    .measure_text(0.0, 0.0, l, &paint)
                    .map(|m| m.width())
                    .unwrap_or(80.0)
            })
            .fold(0.0f32, f32::max);

        let x = (app.cursor[0] + 16.0).min(app.window_size[0] - width - 24.0);
        let y = (app.cursor[1] + 18.0)
            .min(app.window_size[1] - theme::STATUSBAR_HEIGHT - lines.len() as f32 * line_h - 16.0);

        let mut bg = Path::new();
        bg.rounded_rect(
            x,
            y,
            width + 16.0,
            lines.len() as f32 * line_h + 10.0,
            5.0,
        );
        canvas.fill_path(&bg, &Paint::color(theme::tooltip_bg()));
        for (i, line) in lines.iter().enumerate() {
            self.text(
                canvas,
                x + 8.0,
                y + 5.0 + i as f32 * line_h + line_h / 2.0,
                line,
                size,
                theme::tooltip_text(),
                Align::Left,
                Baseline::Middle,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn text(
        &self,
        canvas: &mut Canvas<OpenGl>,
        x: f32,
        y: f32,
        s: &str,
        size: f32,
        color: Color,
        align: Align,
        baseline: Baseline,
    ) {
        let mut paint = Paint::color(color);
        paint.set_font(&[self.font]);
        // Quantize size and snap the anchor to whole pixels: glyphs land on
        // the pixel grid instead of being smeared across it.
        paint.set_font_size((size * 2.0).round() / 2.0);
        paint.set_text_align(align);
        paint.set_text_baseline(baseline);
        let _ = canvas.fill_text(x.round(), y.round(), s, &paint);
    }
}
