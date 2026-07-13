//! The renderer: a pure function of (document, gesture, simulation, camera).
//! Nothing here is stateful except the loaded fonts and the media cache.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;

use femtovg::{renderer::OpenGl, Align, Baseline, Canvas, Color, Paint, Path};
use crate::app::{App, Mode};
use diycpu::types::is_primitive;

use diycpu::project::WireNet;

use crate::camera::View;
use crate::theme;
use crate::ui::UiLayout;
use diycpu::editor::SheetRef;
use diycpu::project::{Note, ProjectAbstraction};

mod chrome;
mod dialogs;
mod media;
mod overlays;
mod parts;
mod pins;
mod wires;
mod world;

use self::media::MediaEntry;
pub(in crate::render) use world::{MAX_DEPTH, MIN_CHILD_SCALE, OPEN_T0, OPEN_T1};

/// Everything needed to draw one sheet at one place on screen.
pub(crate) struct SheetCtx<'a> {
    pub(crate) abstractions: &'a [ProjectAbstraction],
    /// While running: this sheet's instance path from the top, for keying
    /// flattened simulation values. None when not simulating.
    pub(crate) sim_path: Option<Vec<String>>,
    pub(crate) components: &'a [diycpu::project::ProjectComponent],
    pub(crate) wire_nets: &'a [WireNet],
    pub(crate) notes: &'a [Note],
    pub(crate) images: &'a [diycpu::project::MediaImage],
    pub(crate) interface: Option<&'a ProjectAbstraction>,
    pub(crate) view: View,
    pub(crate) alpha: f32,
    /// True only for the sheet being edited: selection, hover and run-mode
    /// values render here.
    pub(crate) focus: bool,
}

impl SheetCtx<'_> {
    pub(super) fn zoom(&self) -> f32 {
        self.view.scale / crate::camera::CELL
    }

    /// Grid point → screen pixels.
    pub(crate) fn px(&self, p: [f32; 2]) -> [f32; 2] {
        self.view.to_screen(p)
    }

    /// Grid rectangle → screen-pixel rectangle.
    pub(crate) fn px_rect(&self, r: diycpu::layout::Rect) -> diycpu::layout::Rect {
        let tl = self.view.to_screen([r.x, r.y]);
        diycpu::layout::Rect {
            x: tl[0],
            y: tl[1],
            w: r.w * self.view.scale,
            h: r.h * self.view.scale,
        }
    }

    // ── Simulation values, for drawing ───────────────────────────────────
    //
    // These are the ONLY places the renderer asks the simulation anything.
    // A sheet without values (edit mode, or a view rooted at an
    // abstraction) simply answers None and everything renders neutral.

    /// A component's output value RIGHT NOW, while this sheet simulates.
    pub(crate) fn sim_value(
        &self,
        app: &App,
        comp: &diycpu::project::ProjectComponent,
    ) -> Option<bool> {
        match (&app.mode, &self.sim_path) {
            (Mode::Run(session), Some(path)) => session.component_value_at(path, &comp.id),
            _ => None,
        }
    }

    /// The bit a STATEFUL component (dff/const) holds right now — the one
    /// its body tint and face digit show. None for everything else.
    pub(crate) fn stored_bit(
        &self,
        app: &App,
        comp: &diycpu::project::ProjectComponent,
    ) -> Option<bool> {
        if !matches!(comp.component_type.as_str(), "dff" | "const") {
            return None;
        }
        self.sim_value(app, comp)
    }

    /// What a stateful face shows: the live bit while running, the
    /// configured init otherwise.
    pub(crate) fn face_bit(
        &self,
        app: &App,
        comp: &diycpu::project::ProjectComponent,
    ) -> bool {
        self.stored_bit(app, comp).or(comp.init).unwrap_or(false)
    }

    /// The value behind an output port — unbundler lanes resolved to their
    /// true driver, interface-pin pseudo-ports answering None.
    pub(crate) fn output_value(
        &self,
        app: &App,
        port: &diycpu::project::PortRef,
    ) -> Option<bool> {
        let (path, session) = match (&self.sim_path, &app.mode) {
            (Some(path), Mode::Run(session)) => (path, session),
            _ => return None,
        };
        let src = crate::app::resolve_lane_display(self.components, self.wire_nets, port, 0)?;
        let is_prim = self
            .components
            .iter()
            .find(|c| c.id == src.component_id)
            .map(|c| is_primitive(&c.component_type))
            .unwrap_or(true);
        session.port_value_at(path, is_prim, &src)
    }
}

// ── Tiny paint vocabulary ────────────────────────────────────────────────────
//
// The draw functions above these talk in rectangles and colors; the raw
// femtovg path plumbing lives here once.

pub(crate) fn fill_rounded(
    canvas: &mut Canvas<OpenGl>,
    r: diycpu::layout::Rect,
    radius: f32,
    color: Color,
) {
    let mut p = Path::new();
    p.rounded_rect(r.x, r.y, r.w, r.h, radius);
    canvas.fill_path(&p, &Paint::color(color));
}

pub(crate) fn stroke_rounded(
    canvas: &mut Canvas<OpenGl>,
    r: diycpu::layout::Rect,
    radius: f32,
    color: Color,
    width: f32,
) {
    let mut p = Path::new();
    p.rounded_rect(r.x, r.y, r.w, r.h, radius);
    let mut paint = Paint::color(color);
    paint.set_line_width(width);
    canvas.stroke_path(&p, &paint);
}

pub(crate) fn stroke_circle(
    canvas: &mut Canvas<OpenGl>,
    center: [f32; 2],
    radius: f32,
    color: Color,
    width: f32,
) {
    let mut p = Path::new();
    p.circle(center[0], center[1], radius);
    let mut paint = Paint::color(color);
    paint.set_line_width(width);
    canvas.stroke_path(&p, &paint);
}

pub(crate) fn make_ctx(
    project: &diycpu::project::Project,
    sheet: SheetRef,
    view: View,
    alpha: f32,
    focus: bool,
    sim_path: Option<Vec<String>>,
) -> Option<SheetCtx<'_>> {
    let (components, wire_nets) = diycpu::editor::sheet_of(project, sheet)?;
    let notes = diycpu::editor::sheet_notes(project, sheet)?;
    let images = diycpu::editor::sheet_images(project, sheet)?;
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
        images,
        interface,
        view,
        alpha,
        focus,
    })
}

pub struct Renderer {
    /// Inter first for all UI text; DejaVu as the fallback for symbols
    /// Inter doesn't cover (⟲ ◂ ▸ ✕ …).
    fonts: [femtovg::FontId; 2],
    /// Media decoded at most once per absolute path; failures cached too so
    /// a broken file doesn't re-decode every frame.
    media: RefCell<HashMap<PathBuf, MediaEntry>>,
    /// Set while drawing when any animated image is visible — the shell
    /// keeps redrawing while this holds.
    animating: Cell<bool>,
}

impl Renderer {
    pub fn new(canvas: &mut Canvas<OpenGl>) -> Self {
        let inter = canvas
            .add_font_mem(include_bytes!("../../../../assets/Inter-Regular.otf"))
            .expect("embedded font loads");
        let fallback = canvas
            .add_font_mem(include_bytes!("../../../../assets/DejaVuSans.ttf"))
            .expect("embedded font loads");
        Renderer {
            fonts: [inter, fallback],
            media: RefCell::new(HashMap::new()),
            animating: Cell::new(false),
        }
    }

    /// True when the last draw showed an animated image (keep redrawing).
    pub fn wants_animation(&self) -> bool {
        self.animating.get()
    }

    pub fn draw(&self, canvas: &mut Canvas<OpenGl>, app: &App, ui: &UiLayout) {
        self.animating.set(false);
        let [w, h] = app.window_size;
        canvas.clear_rect(0, 0, w as u32, h as u32, theme::canvas_bg());

        let view = app.canvas_rect();
        canvas.scissor(view.x, view.y, view.w, view.h);
        self.draw_world(canvas, app);
        self.draw_wire_preview(canvas, app);
        self.draw_drag_ghosts(canvas, app);
        self.draw_placement_ghost(canvas, app);
        self.draw_image_placement_ghost(canvas, app);
        self.draw_band(canvas, app);
        self.draw_cog(canvas, app);
        canvas.reset_scissor();

        self.draw_palette(canvas, app, ui);
        self.draw_topbar(canvas, app, ui);
        self.draw_statusbar(canvas, app);
        self.draw_layout_dialog(canvas, app);
        self.draw_image_dialog(canvas, app);
        self.draw_prompt(canvas, app);
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn text(
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
        paint.set_font(&self.fonts);
        // Quantize size and snap the anchor to whole pixels: glyphs land on
        // the pixel grid instead of being smeared across it.
        paint.set_font_size((size * 2.0).round() / 2.0);
        paint.set_text_align(align);
        paint.set_text_baseline(baseline);
        let _ = canvas.fill_text(x.round(), y.round(), s, &paint);
    }
}
