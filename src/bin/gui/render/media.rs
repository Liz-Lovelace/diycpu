//! Placed pictures: the GPU media cache (decode once, failures cached),
//! animated GIF frame selection, and everything that blits an image.

use femtovg::{renderer::OpenGl, Align, Baseline, Canvas, ImageFlags, Paint, Path};


use crate::app::{App, Gesture};
use crate::theme;

use super::*;


/// A decoded media file living on the GPU. Animated GIFs carry all their
/// frames; everything else is a single frame.
pub(super) struct LoadedImage {
    frames: Vec<femtovg::ImageId>,
    /// Per-frame display time in seconds (same length as `frames`).
    delays: Vec<f32>,
    total: f32,
}

pub(super) enum MediaEntry {
    Ok(LoadedImage),
    Failed,
}

impl Renderer {
    /// Decode a media file into GPU textures, caching the result (and the
    /// failure). Animated GIFs decode every frame up to a pixel budget.
    pub(super) fn media_entry<R>(
        &self,
        canvas: &mut Canvas<OpenGl>,
        path: &std::path::Path,
        f: impl FnOnce(&LoadedImage) -> R,
    ) -> Option<R> {
        let mut cache = self.media.borrow_mut();
        let entry = cache
            .entry(path.to_path_buf())
            .or_insert_with(|| Self::load_media(canvas, path));
        match entry {
            MediaEntry::Ok(img) => Some(f(img)),
            MediaEntry::Failed => None,
        }
    }

    pub(super) fn load_media(canvas: &mut Canvas<OpenGl>, path: &std::path::Path) -> MediaEntry {
        let Ok(bytes) = std::fs::read(path) else {
            return MediaEntry::Failed;
        };
        if bytes.len() as u64 > crate::app::MAX_MEDIA_BYTES {
            return MediaEntry::Failed;
        }
        let format = image::guess_format(&bytes).ok();
        if format == Some(image::ImageFormat::Gif) {
            use image::AnimationDecoder;
            let decoded = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(&bytes[..]))
                .ok()
                .map(|dec| {
                    let mut frames = Vec::new();
                    let mut delays = Vec::new();
                    // Cap decoded animation data so a huge GIF can't eat
                    // the GPU: ~256 MB of RGBA.
                    let mut budget: i64 = 64_000_000;
                    for frame in dec.into_frames() {
                        let Ok(frame) = frame else { break };
                        let (num, den) = frame.delay().numer_denom_ms();
                        let delay = (num as f32 / den.max(1) as f32 / 1000.0).max(0.02);
                        let buf = frame.into_buffer();
                        budget -= (buf.width() * buf.height()) as i64;
                        let dynimg = image::DynamicImage::ImageRgba8(buf);
                        let Ok(src) = femtovg::ImageSource::try_from(&dynimg) else {
                            break;
                        };
                        let Ok(id) = canvas.create_image(src, ImageFlags::empty()) else {
                            break;
                        };
                        frames.push(id);
                        delays.push(delay);
                        if budget <= 0 {
                            break; // show what fits; the loop still plays
                        }
                    }
                    (frames, delays)
                });
            if let Some((frames, delays)) = decoded {
                if !frames.is_empty() {
                    let total = delays.iter().sum::<f32>().max(0.02);
                    return MediaEntry::Ok(LoadedImage {
                        frames,
                        delays,
                        total,
                    });
                }
            }
            // fall through to a static decode of the first frame
        }
        let Ok(dynimg) = image::load_from_memory(&bytes) else {
            return MediaEntry::Failed;
        };
        let dynimg = image::DynamicImage::ImageRgba8(dynimg.to_rgba8());
        let Ok(src) = femtovg::ImageSource::try_from(&dynimg) else {
            return MediaEntry::Failed;
        };
        let Ok(id) = canvas.create_image(src, ImageFlags::empty()) else {
            return MediaEntry::Failed;
        };
        MediaEntry::Ok(LoadedImage {
            frames: vec![id],
            delays: vec![f32::INFINITY],
            total: f32::INFINITY,
        })
    }

    /// The frame to show right now.
    pub(super) fn current_frame(&self, img: &LoadedImage, seconds: f32) -> femtovg::ImageId {
        if img.frames.len() <= 1 {
            return img.frames[0];
        }
        self.animating.set(true);
        let mut t = seconds % img.total;
        for (k, d) in img.delays.iter().enumerate() {
            if t < *d {
                return img.frames[k];
            }
            t -= d;
        }
        img.frames[0]
    }

    /// Placed pictures: the backdrop layer of a sheet.
    pub(super) fn draw_images(&self, canvas: &mut Canvas<OpenGl>, app: &App, ctx: &SheetCtx) {
        for img in ctx.images {
            let bounds = diycpu::layout::image_bounds(img);
            let tl = ctx.view.to_screen([bounds.x, bounds.y]);
            let (w, h) = (bounds.w * ctx.view.scale, bounds.h * ctx.view.scale);
            let selected = ctx.focus && app.selection.images.contains(&img.id);
            self.draw_media_rect(
                canvas,
                app,
                app.resolve_media(&img.file),
                diycpu::layout::Rect {
                    x: tl[0],
                    y: tl[1],
                    w,
                    h,
                },
                ctx.zoom(),
                selected,
                1.0,
            );
        }
    }

    /// One picture at a screen rectangle: the media file if it loads, a
    /// tidy placeholder if it doesn't. Shared by sheets, ghosts and the
    /// image dialog.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_media_rect(
        &self,
        canvas: &mut Canvas<OpenGl>,
        app: &App,
        path: Option<PathBuf>,
        r: diycpu::layout::Rect,
        zoom: f32,
        selected: bool,
        alpha: f32,
    ) {
        let corner = (4.0 * zoom).clamp(1.0, 6.0);
        let mut body = Path::new();
        body.rounded_rect(r.x, r.y, r.w, r.h, corner);

        let drawn = path.and_then(|p| {
            self.media_entry(canvas, &p, |img| {
                self.current_frame(img, app.anim_seconds)
            })
        });
        match drawn {
            Some(frame) => {
                let paint = Paint::image(frame, r.x, r.y, r.w, r.h, 0.0, alpha);
                canvas.fill_path(&body, &paint);
            }
            None => {
                // Missing or undecodable: keep the object usable.
                canvas.fill_path(&body, &Paint::color(theme::chip_fill()));
                let mut cross = Path::new();
                cross.move_to(r.x, r.y);
                cross.line_to(r.x + r.w, r.y + r.h);
                cross.move_to(r.x + r.w, r.y);
                cross.line_to(r.x, r.y + r.h);
                let mut cp = Paint::color(theme::panel_border());
                cp.set_line_width(1.0);
                canvas.stroke_path(&cross, &cp);
                let size = (11.0 * zoom).clamp(6.0, 14.0);
                self.text(
                    canvas,
                    r.x + r.w / 2.0,
                    r.y + r.h / 2.0,
                    "no image",
                    size,
                    theme::text_dim(),
                    Align::Center,
                    Baseline::Middle,
                );
            }
        }
        let mut border = Paint::color(if selected {
            theme::accent()
        } else {
            theme::note_border()
        });
        border.set_line_width(if selected { 2.5 } else { 1.2 });
        canvas.stroke_path(&body, &border);
    }

    /// The picked file follows the cursor before placement commits.
    pub(super) fn draw_image_placement_ghost(&self, canvas: &mut Canvas<OpenGl>, app: &App) {
        let Gesture::PlacingImage { source, size } = &app.gesture else {
            return;
        };
        if !app.canvas_rect().contains(app.cursor) {
            return;
        }
        let g = app.cursor_grid();
        let pos = [
            ((g[0] - size[0] / 2.0) * 2.0).round() / 2.0,
            ((g[1] - size[1] / 2.0) * 2.0).round() / 2.0,
        ];
        let tl = app.camera.grid_to_screen(pos);
        let cell = app.camera.cell_px();
        canvas.set_global_alpha(0.55);
        self.draw_media_rect(
            canvas,
            app,
            Some(source.clone()),
            diycpu::layout::Rect {
                x: tl[0],
                y: tl[1],
                w: size[0] * cell,
                h: size[1] * cell,
            },
            app.camera.zoom,
            false,
            1.0,
        );
        canvas.set_global_alpha(1.0);
    }
}
