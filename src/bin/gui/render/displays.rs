//! Display elements: lights, RGB lights, segment displays and the face.
//!
//! A display's body IS its visual — no chip chrome. `display_visual` is
//! the one painter, taking explicit per-lane values so the canvas (live
//! values), the placement ghost (all off) and the settings dialog (all
//! lit) share it exactly.

use femtovg::{renderer::OpenGl, Canvas, Color, Paint, Path};

use diycpu::layout::{self, Rect};
use diycpu::project::ProjectComponent;

use crate::app::App;
use crate::theme;

use super::segments::{SegDesign, FACE, FACE_VIEW, SEG16, SEG8};
use super::*;

/// Parse a lit color ("#rrggbb"), falling back to the classic LED red.
fn lit_color(comp: &ProjectComponent) -> Color {
    let hex = comp
        .display
        .as_ref()
        .and_then(|d| d.color.as_deref())
        .unwrap_or("#f4483a");
    parse_hex(hex).unwrap_or_else(|| Color::rgb(244, 72, 58))
}

fn parse_hex(s: &str) -> Option<Color> {
    let s = s.strip_prefix('#')?;
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some(Color::rgb(
        (v >> 16) as u8,
        (v >> 8) as u8,
        v as u8,
    ))
}

/// An unlit version of a lit color: most of the way toward the body tone,
/// keeping a hint of hue so an off light still says what it would show.
fn dimmed(c: Color, toward: Color, keep: f32) -> Color {
    Color::rgbf(
        toward.r + (c.r - toward.r) * keep,
        toward.g + (c.g - toward.g) * keep,
        toward.b + (c.b - toward.b) * keep,
    )
}

impl Renderer {
    /// A display on the sheet: resolve its live values, then paint.
    pub(super) fn draw_display(
        &self,
        canvas: &mut Canvas<OpenGl>,
        app: &App,
        ctx: &SheetCtx,
        comp: &ProjectComponent,
        shape: &super::parts::ComponentShape,
    ) {
        let values = ctx.display_values(app, comp);
        self.display_visual(canvas, comp, shape.px, &values, shape.zoom);

        if shape.selected {
            stroke_rounded(
                canvas,
                shape.px,
                (4.0 * shape.zoom).clamp(1.0, 6.0),
                theme::accent(),
                2.5,
            );
        }
    }

    /// The display itself, at an arbitrary screen rectangle with explicit
    /// values — shared by the canvas, ghosts and the settings dialog.
    pub(crate) fn display_visual(
        &self,
        canvas: &mut Canvas<OpenGl>,
        comp: &ProjectComponent,
        px: Rect,
        values: &[Option<bool>],
        zoom: f32,
    ) {
        match comp.component_type.as_str() {
            "light" => self.draw_light(canvas, comp, px, values, zoom, false),
            "rgb" => self.draw_light(canvas, comp, px, values, zoom, true),
            "seg" => {
                let design = if layout::seg_lanes(comp) == 8 { &SEG8 } else { &SEG16 };
                self.draw_seg(canvas, comp, px, values, zoom, design);
            }
            _ => self.draw_face(canvas, comp, px, values, zoom),
        }
    }

    /// A single lamp: square or round, lit in its color. The RGB variant
    /// mixes its three lanes into the fill.
    fn draw_light(
        &self,
        canvas: &mut Canvas<OpenGl>,
        comp: &ProjectComponent,
        px: Rect,
        values: &[Option<bool>],
        zoom: f32,
        rgb: bool,
    ) {
        let round = comp
            .display
            .as_ref()
            .and_then(|d| d.round)
            .unwrap_or(false);
        let body = theme::chip_fill();
        let (fill, lit) = if rgb {
            let bit = |k: usize| values.get(k).copied().flatten().unwrap_or(false);
            let any = bit(0) || bit(1) || bit(2);
            let on = |b: bool| if b { 0.93 } else { 0.16 };
            (
                if any || values.first().copied().flatten().is_some() {
                    Color::rgbf(on(bit(0)), on(bit(1)), on(bit(2)))
                } else {
                    dimmed(Color::rgbf(0.2, 0.2, 0.22), body, 0.35)
                },
                any,
            )
        } else {
            let color = lit_color(comp);
            match values.first().copied().flatten() {
                Some(true) => (color, true),
                _ => (dimmed(color, body, 0.22), false),
            }
        };

        let mut p = Path::new();
        if round {
            p.ellipse(px.x + px.w / 2.0, px.y + px.h / 2.0, px.w / 2.0, px.h / 2.0);
        } else {
            p.rounded_rect(px.x, px.y, px.w, px.h, (3.0 * zoom).clamp(1.0, 5.0));
        }

        // A soft glow behind a lit lamp sells the light.
        if lit {
            let glow = Paint::radial_gradient(
                px.x + px.w / 2.0,
                px.y + px.h / 2.0,
                px.w.min(px.h) * 0.2,
                px.w.max(px.h) * 0.9,
                fill,
                Color::rgba(255, 255, 255, 0),
            );
            let mut halo = Path::new();
            halo.ellipse(
                px.x + px.w / 2.0,
                px.y + px.h / 2.0,
                px.w * 0.95,
                px.h * 0.95,
            );
            canvas.fill_path(&halo, &glow);
        }
        canvas.fill_path(&p, &Paint::color(fill));
        let mut border = Paint::color(theme::chip_border());
        border.set_line_width(1.2);
        canvas.stroke_path(&p, &border);
    }

    /// An aresluna segment glyph: dark slab, one SVG path per lane.
    fn draw_seg(
        &self,
        canvas: &mut Canvas<OpenGl>,
        comp: &ProjectComponent,
        px: Rect,
        values: &[Option<bool>],
        zoom: f32,
        design: &SegDesign,
    ) {
        // Same paper as every other component — displays belong to the
        // light theme like everything else.
        fill_rounded(canvas, px, (3.0 * zoom).clamp(1.0, 5.0), theme::chip_fill());
        stroke_rounded(
            canvas,
            px,
            (3.0 * zoom).clamp(1.0, 5.0),
            theme::chip_border(),
            1.2,
        );

        // The glyph fits the slab with a small margin, preserving aspect.
        let [vx, vy, vw, vh] = design.view;
        let margin = 0.86;
        let s = (px.w / vw).min(px.h / vh) * margin;
        let ox = px.x + (px.w - vw * s) / 2.0 - vx * s;
        let oy = px.y + (px.h - vh * s) / 2.0 - vy * s;

        let on = lit_color(comp);
        let off = theme::segment_off();
        for (k, d) in design.paths.iter().enumerate() {
            let lit = values.get(k).copied().flatten().unwrap_or(false);
            let path = svg_path(d, s, ox, oy);
            canvas.fill_path(&path, &Paint::color(if lit { on } else { off }));
        }
    }

    /// The face: the same paper as everything else, all segments in the
    /// one configured color (the artwork's colors only marked which
    /// pixels belong to which segment).
    fn draw_face(
        &self,
        canvas: &mut Canvas<OpenGl>,
        comp: &ProjectComponent,
        px: Rect,
        values: &[Option<bool>],
        zoom: f32,
    ) {
        fill_rounded(canvas, px, (3.0 * zoom).clamp(1.0, 5.0), theme::chip_fill());
        stroke_rounded(
            canvas,
            px,
            (3.0 * zoom).clamp(1.0, 5.0),
            theme::chip_border(),
            1.2,
        );

        let s = (px.w / FACE_VIEW[0]).min(px.h / FACE_VIEW[1]);
        let ox = px.x + (px.w - FACE_VIEW[0] * s) / 2.0;
        let oy = px.y + (px.h - FACE_VIEW[1] * s) / 2.0;
        let on = lit_color(comp);
        let off = theme::segment_off();
        for (k, rects) in FACE.iter().enumerate() {
            let color = if values.get(k).copied().flatten().unwrap_or(false) {
                on
            } else {
                off
            };
            let mut p = Path::new();
            for [x, y, w, h] in rects.iter() {
                p.rect(
                    ox + *x as f32 * s,
                    oy + *y as f32 * s,
                    *w as f32 * s,
                    *h as f32 * s,
                );
            }
            canvas.fill_path(&p, &Paint::color(color));
        }
    }
}

/// Replay an absolute-command SVG path (M/L/H/C/Z — all the artwork uses)
/// into a femtovg path, scaled and offset.
fn svg_path(d: &str, s: f32, ox: f32, oy: f32) -> Path {
    let mut path = Path::new();
    let mut nums: Vec<f32> = Vec::with_capacity(6);
    let mut cmd = 'M';
    let mut cur = [0.0f32, 0.0f32];
    let tx = |p: [f32; 2]| [ox + p[0] * s, oy + p[1] * s];

    let mut it = d.chars().peekable();
    loop {
        // Skip separators, read one command or number.
        while matches!(it.peek(), Some(c) if c.is_whitespace() || *c == ',') {
            it.next();
        }
        let Some(&c) = it.peek() else {
            break;
        };
        if c.is_ascii_alphabetic() {
            cmd = c;
            it.next();
            nums.clear();
            if cmd == 'Z' || cmd == 'z' {
                path.close();
            }
            continue;
        }
        // A number.
        let mut buf = String::new();
        while matches!(it.peek(), Some(c) if c.is_ascii_digit() || *c == '.' || *c == '-') {
            // A '-' after digits starts the NEXT number.
            if *it.peek().unwrap() == '-' && !buf.is_empty() {
                break;
            }
            buf.push(it.next().unwrap());
        }
        let Ok(v) = buf.parse::<f32>() else { break };
        nums.push(v);

        let take = match cmd {
            'H' | 'V' => 1,
            'M' | 'L' => 2,
            'C' => 6,
            _ => 2,
        };
        if nums.len() < take {
            continue;
        }
        match cmd {
            'M' => {
                cur = [nums[0], nums[1]];
                let p = tx(cur);
                path.move_to(p[0], p[1]);
                cmd = 'L'; // subsequent pairs are implicit line-tos
            }
            'L' => {
                cur = [nums[0], nums[1]];
                let p = tx(cur);
                path.line_to(p[0], p[1]);
            }
            'H' => {
                cur[0] = nums[0];
                let p = tx(cur);
                path.line_to(p[0], p[1]);
            }
            'V' => {
                cur[1] = nums[0];
                let p = tx(cur);
                path.line_to(p[0], p[1]);
            }
            'C' => {
                let c1 = tx([nums[0], nums[1]]);
                let c2 = tx([nums[2], nums[3]]);
                cur = [nums[4], nums[5]];
                let p = tx(cur);
                path.bezier_to(c1[0], c1[1], c2[0], c2[1], p[0], p[1]);
            }
            _ => {}
        }
        nums.clear();
    }
    path
}
