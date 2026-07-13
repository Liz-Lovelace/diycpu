//! Modal layers: the layout dialog, the image dialog, and text prompts.

use femtovg::{renderer::OpenGl, Align, Baseline, Canvas, Paint, Path};


use crate::app::App;
use crate::theme;

use super::*;


impl Renderer {
    /// The image settings dialog: preview, file swap, size sliders.
    pub(super) fn draw_image_dialog(&self, canvas: &mut Canvas<OpenGl>, app: &App) {
        let Some(geom) = app.image_dialog_geometry() else {
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
            "Image settings",
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

        // The picture itself, aspect-fit.
        let r = &geom.preview;
        self.draw_media_rect(
            canvas,
            app,
            app.resolve_media(&geom.file_label),
            diycpu::layout::Rect {
                x: r.x,
                y: r.y,
                w: r.w,
                h: r.h,
            },
            1.0,
            false,
            1.0,
        );

        // Choose-file button with the current file name on it.
        let c = &geom.choose_rect;
        let hovered = c.contains(app.cursor);
        let mut btn = Path::new();
        btn.rounded_rect(c.x, c.y, c.w, c.h, 6.0);
        canvas.fill_path(
            &btn,
            &Paint::color(if hovered {
                theme::button_hover_bg()
            } else {
                theme::button_bg()
            }),
        );
        let mut bb = Paint::color(theme::button_border());
        bb.set_line_width(1.2);
        canvas.stroke_path(&btn, &bb);
        self.text(
            canvas,
            c.x + c.w / 2.0,
            c.y + c.h / 2.0,
            &format!("Choose image…   ({})", geom.file_label),
            12.0,
            theme::text(),
            Align::Center,
            Baseline::Middle,
        );

        for slider in &geom.sliders {
            self.draw_slider(
                canvas,
                app,
                slider,
                crate::app::IMG_SIZE_MIN,
                crate::app::IMG_SIZE_MAX,
            );
        }
    }

    /// The component-layout dialog: drag ports around the closed
    /// rectangle's edges, drag the corner to resize.
    pub(super) fn draw_layout_dialog(&self, canvas: &mut Canvas<OpenGl>, app: &App) {
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
            "Drag a port to any edge · size below (click a value to type it) · Esc closes",
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

        // Size sliders: label, track with a filled knob, and a clickable
        // value that opens a type-in prompt.
        for slider in &geom.sliders {
            self.draw_slider(
                canvas,
                app,
                slider,
                crate::app::DIALOG_SIZE_MIN,
                crate::app::DIALOG_SIZE_MAX,
            );
        }
    }

    /// One dialog slider row: label, rail with a knob, and the clickable
    /// value that opens a type-in prompt.
    pub(super) fn draw_slider(
        &self,
        canvas: &mut Canvas<OpenGl>,
        app: &App,
        slider: &crate::app::DialogSlider,
        min: f32,
        max: f32,
    ) {
        self.text(
            canvas,
            slider.track.x - 12.0,
            slider.track.y + slider.track.h / 2.0,
            slider.label,
            12.5,
            theme::text(),
            Align::Right,
            Baseline::Middle,
        );

        let t = &slider.track;
        let mid_y = t.y + t.h / 2.0;
        let mut rail = Path::new();
        rail.rounded_rect(t.x, mid_y - 2.0, t.w, 4.0, 2.0);
        canvas.fill_path(&rail, &Paint::color(theme::panel_border()));

        let f = ((slider.value - min) / (max - min)).clamp(0.0, 1.0);
        let mut fill = Path::new();
        fill.rounded_rect(t.x, mid_y - 2.0, t.w * f, 4.0, 2.0);
        canvas.fill_path(&fill, &Paint::color(theme::accent()));
        let mut knob = Path::new();
        knob.circle(t.x + t.w * f, mid_y, 7.0);
        canvas.fill_path(&knob, &Paint::color(theme::chip_fill()));
        let mut ring = Paint::color(theme::accent());
        ring.set_line_width(2.0);
        canvas.stroke_path(&knob, &ring);

        // The value doubles as a "type it" button.
        let v = &slider.value_rect;
        let hovered = v.contains(app.cursor);
        let mut vbox = Path::new();
        vbox.rounded_rect(v.x, v.y, v.w, v.h, 5.0);
        canvas.fill_path(
            &vbox,
            &Paint::color(if hovered {
                theme::button_hover_bg()
            } else {
                theme::button_bg()
            }),
        );
        let mut vb = Paint::color(theme::button_border());
        vb.set_line_width(1.2);
        canvas.stroke_path(&vbox, &vb);
        self.text(
            canvas,
            v.x + v.w / 2.0,
            v.y + v.h / 2.0,
            &format!("{} cells", slider.value),
            12.0,
            theme::text(),
            Align::Center,
            Baseline::Middle,
        );
    }

    pub(super) fn draw_prompt(&self, canvas: &mut Canvas<OpenGl>, app: &App) {
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
}
