//! Interface pins on an abstraction's sheet: port circle, name plate,
//! leader line.

use femtovg::{renderer::OpenGl, Align, Baseline, Canvas, Paint, Path};

use diycpu::layout::{self, PortKind};

use crate::app::App;
use crate::theme;

use super::*;


impl Renderer {
    /// Interface pins on the rails beside an abstraction's content: a port
    /// circle plus a clickable name plate.
    pub(super) fn draw_interface_pins(&self, canvas: &mut Canvas<OpenGl>, app: &App, ctx: &SheetCtx) {
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
            // The lane count lives in the pin's socket, not the plate.
            let label = pin.name.clone();
            self.text(
                canvas,
                tl[0] + plate.w * cell / 2.0,
                tl[1] + plate.h * cell / 2.0,
                &label,
                label_size,
                theme::pin_label(),
                Align::Center,
                Baseline::Middle,
            );

            // Port circle (wire attachment point) — square when the pin
            // carries a bundle. Input pins drive wires, so they get the
            // filled "source" look.
            let r = if pin.bundle.is_some() {
                (6.0 * zoom).clamp(2.5, 10.0)
            } else {
                (4.0 * zoom).clamp(2.0, 6.5)
            };
            let mut c = Path::new();
            if pin.bundle.is_some() {
                c.rounded_rect(s[0] - r, s[1] - r, r * 2.0, r * 2.0, r * 0.3);
            } else {
                c.circle(s[0], s[1], r);
            }
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
            if let Some(w) = pin.bundle {
                if r >= 4.5 {
                    let digit_color = match pin.io {
                        PortKind::Input => femtovg::Color::white(),
                        PortKind::Output => theme::text(),
                    };
                    self.text(
                        canvas,
                        s[0],
                        s[1],
                        &format!("{w}"),
                        (r * 1.25).min(12.0),
                        digit_color,
                        Align::Center,
                        Baseline::Middle,
                    );
                }
            }

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
}
