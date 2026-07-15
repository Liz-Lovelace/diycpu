//! The fixed UI around the canvas: palette, toolbar, status bar.

use femtovg::{renderer::OpenGl, Align, Baseline, Canvas, Paint, Path};


use crate::app::App;
use crate::theme;
use crate::ui::UiLayout;

use super::*;


impl Renderer {
    pub(super) fn draw_palette(&self, canvas: &mut Canvas<OpenGl>, app: &App, ui: &UiLayout) {
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
            } else if item.type_name == "bundle" || item.type_name == "unbundle" {
                // Lines gathering into a rail.
                let flip = item.type_name == "unbundle";
                let (many_x, one_x) = if flip {
                    (glyph_x + 18.0, glyph_x)
                } else {
                    (glyph_x, glyph_x + 18.0)
                };
                let mid = glyph_y;
                let mut fan = Path::new();
                for dy in [-6.0, -2.0, 2.0, 6.0] {
                    fan.move_to(many_x, mid + dy);
                    fan.line_to(glyph_x + 9.0, mid);
                }
                canvas.stroke_path(&fan, &cp);
                let mut rail = Path::new();
                rail.move_to(glyph_x + 9.0, mid);
                rail.line_to(one_x, mid);
                let mut rp = Paint::color(color);
                rp.set_line_width(3.0);
                canvas.stroke_path(&rail, &rp);
            } else if item.type_name == "light" {
                let mut lamp = Path::new();
                lamp.circle(glyph_x + 9.0, glyph_y, 6.5);
                canvas.stroke_path(&lamp, &cp);
                // rays
                let mut rays = Path::new();
                for (dx, dy) in [(0.0, -1.0), (0.9, -0.5), (-0.9, -0.5)] {
                    rays.move_to(glyph_x + 9.0 + dx * 8.5, glyph_y + dy * 8.5);
                    rays.line_to(glyph_x + 9.0 + dx * 11.5, glyph_y + dy * 11.5);
                }
                canvas.stroke_path(&rays, &cp);
            } else if item.type_name == "rgb" {
                let mut lamp = Path::new();
                lamp.circle(glyph_x + 9.0, glyph_y, 6.5);
                canvas.stroke_path(&lamp, &cp);
                for (i, c) in [(0, (220, 60, 60)), (1, (50, 180, 80)), (2, (70, 110, 235))] {
                    let mut dot = Path::new();
                    dot.circle(glyph_x + 5.0 + i as f32 * 4.0, glyph_y, 1.6);
                    canvas.fill_path(&dot, &Paint::color(femtovg::Color::rgb(c.0, c.1, c.2)));
                }
            } else if item.type_name == "segment" {
                // A tiny "8": two stacked boxes of bars.
                let mut segs = Path::new();
                for (x0, y0, x1, y1) in [
                    (3.0, -8.0, 13.0, -8.0),
                    (3.0, 0.0, 13.0, 0.0),
                    (3.0, 8.0, 13.0, 8.0),
                    (3.0, -8.0, 3.0, 8.0),
                    (13.0, -8.0, 13.0, 8.0),
                ] {
                    segs.move_to(glyph_x + x0, glyph_y + y0);
                    segs.line_to(glyph_x + x1, glyph_y + y1);
                }
                canvas.stroke_path(&segs, &cp);
            } else if item.type_name == "image" {
                // A tiny landscape: frame, sun, mountains.
                let mut frame = Path::new();
                frame.rounded_rect(glyph_x, glyph_y - 7.0, 18.0, 14.0, 2.0);
                canvas.stroke_path(&frame, &cp);
                let mut sun = Path::new();
                sun.circle(glyph_x + 6.0, glyph_y - 2.5, 1.8);
                canvas.stroke_path(&sun, &cp);
                let mut hills = Path::new();
                hills.move_to(glyph_x + 1.5, glyph_y + 6.0);
                hills.line_to(glyph_x + 7.0, glyph_y + 0.5);
                hills.line_to(glyph_x + 10.5, glyph_y + 4.0);
                hills.line_to(glyph_x + 13.0, glyph_y + 1.5);
                hills.line_to(glyph_x + 16.5, glyph_y + 6.0);
                canvas.stroke_path(&hills, &cp);
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

    pub(super) fn draw_topbar(&self, canvas: &mut Canvas<OpenGl>, app: &App, ui: &UiLayout) {
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

    pub(super) fn draw_statusbar(&self, canvas: &mut Canvas<OpenGl>, app: &App) {
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
}
