//! Wires: committed nets (signal and bundle), the drawing preview, and
//! the derived wires that feed output pins.

use femtovg::{renderer::OpenGl, Align, Baseline, Canvas, Color, Paint, Path};

use diycpu::layout::{self, PortKind};
use diycpu::project::WireNet;

use crate::app::{App, Gesture, WireTarget};
use crate::theme;

use super::*;


impl Renderer {
    /// The display color of a whole net. Bundles carry many values, so
    /// they stay neutral; a signal net wears its driver's value.
    pub(super) fn net_color(&self, app: &App, ctx: &SheetCtx, net: &WireNet) -> Color {
        if net.bundle {
            return theme::wire();
        }
        let energized = net
            .source
            .as_ref()
            .and_then(|src| ctx.output_value(app, src));
        match energized {
            Some(true) => theme::wire_on(),
            Some(false) => theme::wire_off(),
            None => theme::wire(),
        }
    }

    pub(super) fn draw_wires(&self, canvas: &mut Canvas<OpenGl>, app: &App, ctx: &SheetCtx) {
        let abstractions = ctx.abstractions;
        let (components, wire_nets) = (ctx.components, ctx.wire_nets);
        let zoom = ctx.zoom();
        let width = (2.5 * zoom).clamp(0.5, 4.5);

        for net in wire_nets {
            let color = self.net_color(app, ctx, net);
            // Bundles draw as a wide rail with a light core: visibly "many
            // wires running together".
            let width = if net.bundle {
                (6.0 * zoom).clamp(2.5, 10.0)
            } else {
                width
            };

            for i in 0..net.segments.len() {
                let Some(pts) =
                    layout::route_segment(abstractions, components, ctx.interface, net, i)
                else {
                    continue;
                };
                // Segments select individually.
                let selected = ctx.focus
                    && app
                        .selection
                        .segments
                        .iter()
                        .any(|(id, k)| *id == net.id && *k == i);
                let mut paint =
                    Paint::color(if selected { theme::wire_selected() } else { color });
                paint.set_line_width(if selected { width + 1.0 } else { width });
                paint.set_line_join(femtovg::LineJoin::Round);
                paint.set_line_cap(femtovg::LineCap::Round);
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
                if net.bundle {
                    let mut core = Paint::color(theme::canvas_bg());
                    core.set_line_width((width * 0.3).max(1.0));
                    core.set_line_join(femtovg::LineJoin::Round);
                    canvas.stroke_path(&p, &core);
                }
            }

            // The classic bus tick with the lane count, on the first segment.
            if net.bundle && zoom >= 0.45 {
                if let Some(pts) =
                    layout::route_segment(abstractions, components, ctx.interface, net, 0)
                {
                    if let [a, b] = pts[..] {
                        let m = ctx.view.to_screen([(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]);
                        let t = (9.0 * zoom).clamp(6.0, 13.0);
                        let mut tick = Path::new();
                        tick.move_to(m[0] - t * 0.4, m[1] + t * 0.6);
                        tick.line_to(m[0] + t * 0.4, m[1] - t * 0.6);
                        let mut tp = Paint::color(theme::wire());
                        tp.set_line_width(1.5);
                        canvas.stroke_path(&tick, &tp);
                        let w = net
                            .source
                            .as_ref()
                            .and_then(|s| app.port_lane(s))
                            .and_then(|k| match k {
                                layout::WireKind::Bundle(w) => Some(w),
                                layout::WireKind::Signal => None,
                            });
                        if let Some(w) = w {
                            self.text(
                                canvas,
                                m[0] + t * 0.7,
                                m[1] - t * 0.6,
                                &format!("{w}"),
                                t,
                                theme::text_dim(),
                                Align::Left,
                                Baseline::Middle,
                            );
                        }
                    }
                }
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
    pub(super) fn anchor_geom(&self, app: &App, anchor: &diycpu::editor::WirePoint) -> Option<[f32; 2]> {
        use diycpu::editor::WirePoint;
        let abstractions = &app.editor.project.abstractions;
        let (components, wire_nets) = app.editor.sheet_data();
        match anchor {
            WirePoint::Port(p) => {
                layout::port_geom_any(abstractions, components, app.interface(), p)
                    .map(|(pos, _side)| pos)
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
                .map(|j| j.pos),
            WirePoint::New(pos) => Some(*pos),
        }
    }

    pub(super) fn draw_wire_preview(&self, canvas: &mut Canvas<OpenGl>, app: &App) {
        let Gesture::DrawingWire { anchor } = &app.gesture else {
            return;
        };
        let abstractions = &app.editor.project.abstractions;
        let (components, wire_nets) = app.editor.sheet_data();
        let Some(from_pos) = self.anchor_geom(app, anchor) else {
            return;
        };

        let target = app.wire_target();
        let (end, color) = match &target {
            WireTarget::Port(port) => {
                match layout::port_geom_any(abstractions, components, app.interface(), port) {
                    Some((pos, _side)) => (pos, theme::wire_preview()),
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
                    Some(p) => (p, theme::wire_preview()),
                    None => return,
                }
            }
            WireTarget::Incompatible => (app.cursor_grid(), theme::wire_invalid()),
            WireTarget::Free(p) => (*p, theme::wire_preview()),
        };

        // The exact same straight line the committed wire will get.
        let pts = layout::route(from_pos, end);
        let zoom = app.camera.zoom;
        let bundle_preview = match anchor {
            diycpu::editor::WirePoint::Port(p) => matches!(
                app.port_lane(p),
                Some(layout::WireKind::Bundle(_))
            ),
            diycpu::editor::WirePoint::Junction { net_id, .. } => wire_nets
                .iter()
                .find(|n| &n.id == net_id)
                .is_some_and(|n| n.bundle),
            _ => false,
        };
        let mut paint = Paint::color(color);
        paint.set_line_width(if bundle_preview {
            (6.0 * zoom).clamp(2.5, 10.0)
        } else {
            (2.5 * zoom).clamp(1.5, 4.5)
        });
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

    /// Output pin connections are stored in the abstraction's `outputs` map,
    /// so their wires are derived: routed fresh every frame from the target
    /// port to the pin. No stored geometry to go stale.
    pub(super) fn draw_derived_output_wires(
        &self,
        canvas: &mut Canvas<OpenGl>,
        app: &App,
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
            let Some((from, _side)) =
                layout::port_geom_any(abstractions, ctx.components, Some(def), &port)
            else {
                continue;
            };
            let pts = layout::route(from, pin.pos);
            let mut p = Path::new();
            for (k, pt) in pts.iter().enumerate() {
                let s = ctx.view.to_screen(*pt);
                if k == 0 {
                    p.move_to(s[0], s[1]);
                } else {
                    p.line_to(s[0], s[1]);
                }
            }
            // Live value of the driving port while simulating; selection
            // highlight while editing.
            let color = if app.is_running() {
                match ctx.output_value(app, &port) {
                    Some(true) => theme::wire_on(),
                    Some(false) => theme::wire_off(),
                    None => theme::wire(),
                }
            } else if ctx.focus && app.selection.pin_wires.contains(name) {
                theme::wire_selected()
            } else {
                theme::wire()
            };
            let mut paint = Paint::color(color);
            paint.set_line_width((2.5 * zoom).clamp(0.5, 4.5));
            paint.set_line_join(femtovg::LineJoin::Round);
            paint.set_line_cap(femtovg::LineCap::Round);
            canvas.stroke_path(&p, &paint);
        }
    }
}
