//! One component on a sheet.
//!
//! `ComponentShape` answers every WHERE question once — screen rectangle,
//! corner radius, how far the instance has faded open. The draw functions
//! below it each paint one conceptual element (body, border, face, ports)
//! and never re-derive geometry. `draw_component_tree` narrates the order.

use femtovg::{renderer::OpenGl, Align, Baseline, Canvas, Paint, Path};

use diycpu::layout::{self, PortKind, Rect};
use diycpu::project::ProjectComponent;
use diycpu::types::is_primitive;

use crate::app::{App, PathSeg};
use crate::theme;
use diycpu::editor::SheetRef;

use super::*;

/// Everything about *where* one component draws, measured once.
pub(super) struct ComponentShape {
    /// Body rectangle in screen pixels.
    pub px: Rect,
    /// Grid-space footprint (nested-sheet maps are computed from it).
    pub grid: Rect,
    pub corner: f32,
    pub zoom: f32,
    pub selected: bool,
    /// 0 = closed chip, 1 = contents fully faded in. Non-zero only for
    /// abstraction instances big enough on screen.
    pub open_frac: f32,
    /// The instance's abstraction definition, when it has one.
    pub def_index: Option<usize>,
}

impl ComponentShape {
    pub(super) fn measure(
        app: &App,
        ctx: &SheetCtx,
        comp: &ProjectComponent,
        depth: usize,
    ) -> ComponentShape {
        let grid = layout::footprint(ctx.abstractions, comp);
        let px = ctx.px_rect(grid);
        let zoom = ctx.zoom();

        let def_index = ctx
            .abstractions
            .iter()
            .position(|a| a.name == comp.component_type);

        // How "open" is this instance? Driven purely by its size relative
        // to the viewport, smoothstepped for a gentle fade.
        let mut open_frac = 0.0f32;
        if let Some(i) = def_index {
            let viewport = {
                let r = app.canvas_rect();
                r.w.min(r.h)
            };
            let t = px.w.max(px.h) / viewport.max(1.0);
            if depth < MAX_DEPTH {
                let map =
                    layout::instance_inner_map(ctx.abstractions, &ctx.abstractions[i], grid);
                if ctx.view.scale * map.s >= MIN_CHILD_SCALE {
                    let raw = ((t - OPEN_T0) / (OPEN_T1 - OPEN_T0)).clamp(0.0, 1.0);
                    open_frac = raw * raw * (3.0 - 2.0 * raw);
                }
            }
        }

        ComponentShape {
            px,
            grid,
            corner: (6.0 * zoom).clamp(1.0, 9.0),
            zoom,
            selected: ctx.focus && app.selection.components.contains(&comp.id),
            open_frac,
            def_index,
        }
    }

    fn center(&self) -> [f32; 2] {
        [self.px.x + self.px.w / 2.0, self.px.y + self.px.h / 2.0]
    }

    fn on_screen(&self, app: &App) -> bool {
        self.px.x < app.window_size[0]
            && self.px.y < app.window_size[1]
            && self.px.x + self.px.w > 0.0
            && self.px.y + self.px.h > 0.0
    }
}

impl Renderer {
    /// One component, plus — when it's an abstraction instance big enough
    /// on screen — its contents cross-faded in, recursively.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_component_tree(
        &self,
        canvas: &mut Canvas<OpenGl>,
        app: &App,
        project: &diycpu::project::Project,
        ctx: &SheetCtx,
        comp: &ProjectComponent,
        slice: &[PathSeg],
        on_path: bool,
        depth: usize,
        grid_clip: Option<[f32; 4]>,
    ) {
        let shape = ComponentShape::measure(app, ctx, comp, depth);

        // A display's body IS its visual — no chip chrome, just its ports.
        if layout::is_display(&comp.component_type) {
            self.draw_display(canvas, app, ctx, comp, &shape);
            self.draw_component_ports(canvas, app, ctx, comp, &shape, ctx.alpha);
            return;
        }

        self.draw_component_body(canvas, app, ctx, comp, &shape);
        if shape.open_frac > 0.0 && shape.on_screen(app) {
            self.draw_open_contents(
                canvas, app, project, ctx, comp, &shape, slice, on_path, depth, grid_clip,
            );
        }
        self.draw_component_border(canvas, &shape);

        // The closed chrome (name, glyph, value) fades out as the contents
        // fade in.
        let chrome_alpha = ctx.alpha * (1.0 - shape.open_frac);
        if chrome_alpha > 0.01 {
            canvas.set_global_alpha(chrome_alpha);
            self.draw_component_face(canvas, app, ctx, comp, &shape);
            canvas.set_global_alpha(ctx.alpha);
        }

        self.draw_component_ports(canvas, app, ctx, comp, &shape, chrome_alpha);
    }

    /// Soft shadow plus the filled body. The body is always drawn — it
    /// doubles as the child sheet's paper when the instance is open.
    fn draw_component_body(
        &self,
        canvas: &mut Canvas<OpenGl>,
        app: &App,
        ctx: &SheetCtx,
        comp: &ProjectComponent,
        shape: &ComponentShape,
    ) {
        let shadow = Rect {
            y: shape.px.y + 2.0 * shape.zoom,
            ..shape.px
        };
        fill_rounded(canvas, shadow, shape.corner, theme::chip_shadow());

        // A stateful face (dff/const) tints its whole body while running.
        let fill = match ctx.stored_bit(app, comp) {
            Some(true) => theme::badge_on_bg(),
            Some(false) => theme::badge_off_bg(),
            None => theme::chip_fill(),
        };
        fill_rounded(canvas, shape.px, shape.corner, fill);
        if shape.selected {
            fill_rounded(canvas, shape.px, shape.corner, theme::accent_soft());
        }
    }

    fn draw_component_border(&self, canvas: &mut Canvas<OpenGl>, shape: &ComponentShape) {
        let (color, width) = if shape.selected {
            (theme::accent(), 2.5)
        } else {
            (theme::chip_border(), 1.5)
        };
        stroke_rounded(canvas, shape.px, shape.corner, color, width);
    }

    /// Hand the recursion down into an open instance's sheet.
    #[allow(clippy::too_many_arguments)]
    fn draw_open_contents(
        &self,
        canvas: &mut Canvas<OpenGl>,
        app: &App,
        project: &diycpu::project::Project,
        ctx: &SheetCtx,
        comp: &ProjectComponent,
        shape: &ComponentShape,
        slice: &[PathSeg],
        on_path: bool,
        depth: usize,
        grid_clip: Option<[f32; 4]>,
    ) {
        let Some(i) = shape.def_index else { return };
        let map =
            layout::instance_inner_map(ctx.abstractions, &ctx.abstractions[i], shape.grid);

        // Does the focus path continue through THIS instance?
        let (child_slice, child_on_path): (&[PathSeg], bool) =
            match (on_path, slice.first()) {
                (true, Some(seg)) if seg.instance_id == comp.id => (&slice[1..], true),
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
            ctx.alpha * shape.open_frac,
            child_sim_path,
            child_slice,
            child_on_path,
            depth + 1,
            grid_clip,
        );
        canvas.set_global_alpha(ctx.alpha);
    }

    /// The closed look: type label, primitive glyph, stored-bit digit, and
    /// the run-mode value badge.
    fn draw_component_face(
        &self,
        canvas: &mut Canvas<OpenGl>,
        app: &App,
        ctx: &SheetCtx,
        comp: &ProjectComponent,
        shape: &ComponentShape,
    ) {
        let [cx, cy] = shape.center();

        // Cap the label so long names stay inside the body.
        let chars = comp.component_type.chars().count().max(1) as f32;
        let fit = shape.px.w * 0.78 / (chars * 0.55);
        let label_size = (13.0 * shape.zoom).min(fit).clamp(5.0, 22.0);
        let wants_label = !is_primitive(&comp.component_type)
            && !layout::is_bundle_component(&comp.component_type);
        if wants_label && label_size >= 5.0 {
            self.text(
                canvas,
                cx,
                cy - 1.0,
                &comp.component_type,
                label_size,
                theme::chip_label(),
                Align::Center,
                Baseline::Middle,
            );
        }

        let tl = [shape.px.x, shape.px.y];
        match comp.component_type.as_str() {
            "bundle" => self.draw_bundle_glyph(canvas, shape, false),
            "unbundle" => self.draw_bundle_glyph(canvas, shape, true),
            "clk" => self.draw_clock_glyph(canvas, shape),
            "nand" => self.draw_nand_glyph(canvas, shape),
            "const" => {
                // A constant signal's face is the value it feeds out, with
                // a ring saying "this is a source".
                let value = ctx.face_bit(app, comp);
                let digit_size = (14.0 * shape.zoom).clamp(6.0, 24.0);
                let color = value_digit_color(value);
                let center = [shape.px.x + shape.px.w * 0.45, cy];
                stroke_circle(
                    canvas,
                    center,
                    digit_size * 0.75,
                    color,
                    (1.6 * shape.zoom).clamp(0.6, 2.5),
                );
                self.draw_bit_digit(canvas, center, value, digit_size);
            }
            "dff" => {
                // The flip-flop's face IS its bit: init while editing, live
                // state while running. Double-click flips the init.
                let value = ctx.face_bit(app, comp);
                let digit_size = (15.0 * shape.zoom).clamp(6.0, 26.0);
                self.draw_bit_digit(
                    canvas,
                    [shape.px.x + shape.px.w * 0.58, cy],
                    value,
                    digit_size,
                );
            }
            _ => {}
        }

        // Value badge for the primitives whose face doesn't already show it.
        let face_shows_value = matches!(comp.component_type.as_str(), "dff" | "const");
        if is_primitive(&comp.component_type) && !face_shows_value {
            if let Some(v) = ctx.sim_value(app, comp) {
                self.draw_badge(canvas, shape.zoom, tl, shape.px.w, v);
            }
        }
    }

    /// The big 0/1 a stateful face wears.
    fn draw_bit_digit(
        &self,
        canvas: &mut Canvas<OpenGl>,
        center: [f32; 2],
        value: bool,
        size: f32,
    ) {
        self.text(
            canvas,
            center[0],
            center[1],
            if value { "1" } else { "0" },
            size,
            value_digit_color(value),
            Align::Center,
            Baseline::Middle,
        );
    }

    /// Port sockets (round for signals, square for bundles) and their
    /// labels. The interface stays visible even when the instance is open.
    fn draw_component_ports(
        &self,
        canvas: &mut Canvas<OpenGl>,
        app: &App,
        ctx: &SheetCtx,
        comp: &ProjectComponent,
        shape: &ComponentShape,
        chrome_alpha: f32,
    ) {
        let hovered_port = match &app.hover {
            layout::Hit::Port {
                component_id,
                port_name,
                ..
            } if ctx.focus && *component_id == comp.id => Some(port_name.clone()),
            _ => None,
        };
        let show_labels = shape.zoom >= 0.5
            && (comp.component_type == "dff" || !is_primitive(&comp.component_type));
        let skip_label = |port: &layout::PortGeom| {
            (layout::is_bundle_component(&comp.component_type) && port.info.name == "bundle")
                || layout::is_display(&comp.component_type)
        };

        for port in &layout::component_ports(ctx.abstractions, comp) {
            let s = ctx.px(port.pos);
            let r = (3.5 * shape.zoom).clamp(1.0, 6.0);
            let hovered = hovered_port.as_deref() == Some(port.info.name.as_str());

            let fill = match port.info.kind {
                PortKind::Output => {
                    let port_ref = diycpu::project::PortRef {
                        component_id: comp.id.clone(),
                        port_name: port.info.name.clone(),
                    };
                    match ctx.output_value(app, &port_ref) {
                        Some(true) => theme::value_on(),
                        Some(false) => theme::wire_off(),
                        None => theme::port_fill_output(),
                    }
                }
                PortKind::Input => theme::port_fill_input(),
            };

            // A different socket for a different kind of plug.
            let mut p = Path::new();
            match port.info.wire {
                layout::WireKind::Bundle(_) => {
                    p.rounded_rect(s[0] - r, s[1] - r, r * 2.0, r * 2.0, r * 0.3)
                }
                layout::WireKind::Signal => p.circle(s[0], s[1], r),
            }
            canvas.fill_path(&p, &Paint::color(fill));
            let mut ring = Paint::color(if hovered {
                theme::port_hover()
            } else {
                theme::port_ring()
            });
            ring.set_line_width(if hovered { 2.5 } else { 1.5 });
            canvas.stroke_path(&p, &ring);

            // Port names belong to the closed look.
            if show_labels && chrome_alpha > 0.01 && !skip_label(port) {
                canvas.set_global_alpha(chrome_alpha);
                self.draw_port_label(canvas, port, s, r + 3.0, shape.zoom);
                canvas.set_global_alpha(ctx.alpha);
            }
        }
    }

    /// A port's name, just inside the body, opposite its exit side.
    fn draw_port_label(
        &self,
        canvas: &mut Canvas<OpenGl>,
        port: &layout::PortGeom,
        s: [f32; 2],
        pad: f32,
        zoom: f32,
    ) {
        let size = (8.0 * zoom).clamp(6.0, 12.0);
        let (lx, ly, align, baseline) = match port.side {
            layout::Side::Left => (s[0] + pad, s[1], Align::Left, Baseline::Middle),
            layout::Side::Right => (s[0] - pad, s[1], Align::Right, Baseline::Middle),
            layout::Side::Top => (s[0], s[1] + pad, Align::Center, Baseline::Top),
            layout::Side::Bottom => (s[0], s[1] - pad, Align::Center, Baseline::Bottom),
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
    }

    // ── Primitive face glyphs ────────────────────────────────────────────

    /// Many thin lines gathering into one thick rail (mirrored for the
    /// unbundler).
    fn draw_bundle_glyph(
        &self,
        canvas: &mut Canvas<OpenGl>,
        shape: &ComponentShape,
        unbundle: bool,
    ) {
        let [cx, cy] = shape.center();
        let (w, h, zoom) = (shape.px.w, shape.px.h, shape.zoom);
        let span = (h * 0.28).min(w * 1.2);
        let reach = w * 0.26;
        let (many_x, one_x) = if unbundle {
            (cx + reach, cx - reach)
        } else {
            (cx - reach, cx + reach)
        };
        let mut paint = Paint::color(theme::chip_label());
        paint.set_line_width((1.6 * zoom).clamp(0.5, 2.4));
        paint.set_line_cap(femtovg::LineCap::Round);
        let mut p = Path::new();
        for i in 0..4 {
            let t = i as f32 / 3.0 - 0.5;
            p.move_to(many_x, cy + t * span);
            p.line_to(cx, cy);
        }
        canvas.stroke_path(&p, &paint);
        let mut rail = Path::new();
        rail.move_to(cx, cy);
        rail.line_to(one_x, cy);
        let mut rp = Paint::color(theme::chip_label());
        rp.set_line_width((4.0 * zoom).clamp(1.5, 6.0));
        rp.set_line_cap(femtovg::LineCap::Round);
        canvas.stroke_path(&rail, &rp);
    }

    /// A little square wave centered in the body.
    fn draw_clock_glyph(&self, canvas: &mut Canvas<OpenGl>, shape: &ComponentShape) {
        let [cx, cy] = shape.center();
        let half_w = shape.px.w * 0.28;
        let half_h = shape.px.h * 0.16;
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
        paint.set_line_width((2.0 * shape.zoom).clamp(0.6, 3.0));
        paint.set_line_join(femtovg::LineJoin::Round);
        canvas.stroke_path(&p, &paint);
    }

    /// The MIL-spec NAND symbol: a D-shape with the inversion bubble.
    fn draw_nand_glyph(&self, canvas: &mut Canvas<OpenGl>, shape: &ComponentShape) {
        let [cx, cy] = shape.center();
        let gh = shape.px.h * 0.42; // glyph height
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
        paint.set_line_width((2.0 * shape.zoom).clamp(0.6, 3.0));
        paint.set_line_join(femtovg::LineJoin::Round);
        canvas.stroke_path(&body, &paint);

        // Inversion bubble on the output.
        let rb = (r * 0.22).max(0.8);
        let mut bubble = Path::new();
        bubble.circle(flat + r + rb, cy, rb);
        canvas.stroke_path(&bubble, &paint);
    }

    /// The floating 0/1 corner badge shown while running.
    pub(super) fn draw_badge(
        &self,
        canvas: &mut Canvas<OpenGl>,
        zoom: f32,
        tl: [f32; 2],
        w: f32,
        on: bool,
    ) {
        let size = (14.0 * zoom).clamp(9.0, 18.0);
        let x = tl[0] + w - size * 0.7;
        let y = tl[1] - size * 0.3;
        let badge = Rect {
            x: x - size / 2.0,
            y: y - size / 2.0,
            w: size,
            h: size,
        };
        let (bg, fg) = if on {
            (theme::badge_on_bg(), theme::value_on())
        } else {
            (theme::badge_off_bg(), theme::value_off())
        };
        fill_rounded(canvas, badge, size * 0.25, bg);
        stroke_rounded(canvas, badge, size * 0.25, fg, 1.2);
        self.text(
            canvas,
            x,
            y,
            if on { "1" } else { "0" },
            size * 0.7,
            fg,
            Align::Center,
            Baseline::Middle,
        );
    }
}

fn value_digit_color(value: bool) -> femtovg::Color {
    if value {
        theme::value_digit_on()
    } else {
        theme::value_digit_off()
    }
}
