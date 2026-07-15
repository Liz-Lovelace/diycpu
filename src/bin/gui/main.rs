mod app;
mod camera;
mod render;
mod sim;
mod theme;
mod ui;

use std::path::PathBuf;
use std::time::Instant;

use femtovg::{renderer::OpenGl, Canvas};
use glutin::{
    config::ConfigTemplateBuilder,
    context::{ContextApi, ContextAttributesBuilder, PossiblyCurrentContext},
    display::GetGlDisplay,
    prelude::*,
    surface::{Surface, SurfaceAttributesBuilder, WindowSurface},
};
use glutin_winit::DisplayBuilder;
use raw_window_handle::HasWindowHandle;
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{Key, NamedKey},
    window::{CursorIcon, Window, WindowAttributes},
};

use app::{App, Gesture};
use diycpu::layout::Hit;
use render::Renderer;
use ui::{ui_layout, ButtonId};

struct CliArgs {
    project: Option<PathBuf>,
    screenshot: Option<PathBuf>,
    run_steps: Option<usize>,
    sheet: Option<String>,
    dialog: bool,
    /// Open the image-settings dialog for the sheet's first image.
    image_dialog: bool,
    /// Open the display-settings dialog for the sheet's first display.
    display_dialog: bool,
    camera: Option<(f32, f32, f32)>,
    /// Park the cursor at a grid point before the screenshot (hover states).
    cursor: Option<(f32, f32)>,
    /// Sample animated images at this many seconds.
    anim_t: Option<f32>,
    size: (u32, u32),
}

fn parse_args() -> CliArgs {
    let mut args = CliArgs {
        project: None,
        screenshot: None,
        run_steps: None,
        sheet: None,
        dialog: false,
        image_dialog: false,
        display_dialog: false,
        camera: None,
        cursor: None,
        anim_t: None,
        size: (1280, 800),
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--screenshot" => args.screenshot = it.next().map(PathBuf::from),
            "--run" => args.run_steps = it.next().and_then(|s| s.parse().ok()),
            "--sheet" => args.sheet = it.next(),
            "--dialog" => args.dialog = true,
            "--image-dialog" => args.image_dialog = true,
            "--display-dialog" => args.display_dialog = true,
            "--anim-t" => args.anim_t = it.next().and_then(|s| s.parse().ok()),
            "--cursor" => {
                if let Some(s) = it.next() {
                    let parts: Vec<f32> =
                        s.split(',').filter_map(|p| p.parse().ok()).collect();
                    if let [x, y] = parts[..] {
                        args.cursor = Some((x, y));
                    }
                }
            }
            "--camera" => {
                if let Some(s) = it.next() {
                    let parts: Vec<f32> =
                        s.split(',').filter_map(|p| p.parse().ok()).collect();
                    if let [zoom, x, y] = parts[..] {
                        args.camera = Some((zoom, x, y));
                    }
                }
            }
            "--size" => {
                if let Some(s) = it.next() {
                    if let Some((w, h)) = s.split_once('x') {
                        if let (Ok(w), Ok(h)) = (w.parse(), h.parse()) {
                            args.size = (w, h);
                        }
                    }
                }
            }
            other if !other.starts_with("--") => args.project = Some(PathBuf::from(other)),
            other => eprintln!("Unknown flag: {other}"),
        }
    }
    args
}

/// Wall-clock time between clock steps while the simulation plays.
const PLAY_INTERVAL: std::time::Duration = std::time::Duration::from_millis(300);
/// Redraw cadence while an animated image is on screen.
const ANIM_FRAME: std::time::Duration = std::time::Duration::from_millis(33);

struct Shell {
    context: PossiblyCurrentContext,
    surface: Surface<WindowSurface>,
    window: Window,
    canvas: Canvas<OpenGl>,
    renderer: Renderer,
    app: App,
    screenshot: Option<PathBuf>,
    last_press: Option<(Instant, [f32; 2])>,
    next_play_tick: Instant,
    /// Launch time; drives animated images.
    start: Instant,
    /// Screenshot helpers: hover this grid point / sample animations here.
    cursor_at: Option<(f32, f32)>,
    anim_t: Option<f32>,
    /// The last frame showed an animated image — keep redrawing.
    anim_active: bool,
}

impl Shell {
    fn new(event_loop: &ActiveEventLoop, args: &CliArgs) -> Self {
        let window_attrs = WindowAttributes::default()
            .with_title("diycpu")
            .with_inner_size(PhysicalSize::new(args.size.0, args.size.1));

        let config_template = ConfigTemplateBuilder::new()
            .with_alpha_size(8)
            .with_stencil_size(8);

        let (window, gl_config) = DisplayBuilder::new()
            .with_window_attributes(Some(window_attrs))
            .build(event_loop, config_template, |configs| {
                configs
                    .reduce(|a, b| if a.num_samples() > b.num_samples() { a } else { b })
                    .unwrap()
            })
            .unwrap();

        let window = window.unwrap();
        let gl_display = gl_config.display();

        let context_attrs = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::OpenGl(None))
            .build(Some(window.window_handle().unwrap().as_raw()));

        let gl_context =
            unsafe { gl_display.create_context(&gl_config, &context_attrs).unwrap() };

        let size = window.inner_size();
        let surface_attrs = SurfaceAttributesBuilder::<WindowSurface>::new().build(
            window.window_handle().unwrap().as_raw(),
            size.width.max(1).try_into().unwrap(),
            size.height.max(1).try_into().unwrap(),
        );

        let surface = unsafe {
            gl_display
                .create_window_surface(&gl_config, &surface_attrs)
                .unwrap()
        };

        let context = gl_context.make_current(&surface).unwrap();

        let gl_renderer =
            unsafe { OpenGl::new_from_function_cstr(|s| gl_display.get_proc_address(s).cast()) }
                .unwrap();

        let mut canvas = Canvas::new(gl_renderer).unwrap();
        let renderer = Renderer::new(&mut canvas);

        // A project may be given as its directory or its json file; with
        // no argument, the most recent project in the projects folder opens
        // (created on first launch).
        let project_path = args
            .project
            .clone()
            .map(|p| {
                if p.is_dir() {
                    p.join("project.json")
                } else {
                    p
                }
            })
            .or_else(|| {
                let base = app::projects_base_dir()?;
                match app::default_project_path(&base) {
                    Ok(p) => Some(p),
                    Err(e) => {
                        eprintln!("Could not prepare the projects folder: {e}");
                        None
                    }
                }
            });
        // The projects folder exists from the very first launch, whatever
        // path the session was started with.
        if let Some(base) = app::projects_base_dir() {
            let _ = std::fs::create_dir_all(&base);
        }
        let mut app = App::new(project_path);
        app.window_size = [size.width as f32, size.height as f32];
        if let Some(sheet_name) = &args.sheet {
            if let Some(i) = app
                .editor
                .project
                .abstractions
                .iter()
                .position(|a| &a.name == sheet_name)
            {
                app.open_as_root(i);
            }
        }
        if args.dialog {
            app.action_open_layout_dialog();
        }
        if args.image_dialog {
            if let Some(id) = app.editor.sheet_images().first().map(|i| i.id.clone()) {
                app.action_open_image_dialog(&id);
            }
        }
        if args.display_dialog {
            let first = app
                .editor
                .sheet_data()
                .0
                .iter()
                .find(|c| diycpu::layout::is_display(&c.component_type))
                .map(|c| c.id.clone());
            if let Some(id) = first {
                app.action_open_display_dialog(&id);
            }
        }
        if let Some((zoom, x, y)) = args.camera {
            app.camera.zoom = zoom;
            app.camera.offset = [x, y];
            app.pending_fit = false;
            app.update_focus();
        }
        if let Some(steps) = args.run_steps {
            app.action_run_toggle();
            for _ in 0..steps {
                app.action_step_forward();
            }
        }

        Shell {
            context,
            surface,
            window,
            canvas,
            renderer,
            app,
            screenshot: args.screenshot.clone(),
            last_press: None,
            next_play_tick: Instant::now(),
            start: Instant::now(),
            cursor_at: args.cursor,
            anim_t: args.anim_t,
            anim_active: false,
        }
    }

    fn render(&mut self, event_loop: &ActiveEventLoop) {
        let size = self.window.inner_size();
        self.app.window_size = [size.width as f32, size.height as f32];
        if self.app.pending_fit {
            self.app.fit_view();
            self.app.pending_fit = false;
        }

        // dpi is fixed at 1.0: we draw in physical pixels ourselves, and a
        // higher value would make femtovg rasterize glyphs oversized and
        // scale them back down — blurry text on HiDPI screens.
        self.canvas.set_size(size.width, size.height, 1.0);
        self.app.anim_seconds = self
            .anim_t
            .unwrap_or_else(|| self.start.elapsed().as_secs_f32());
        if let Some((gx, gy)) = self.cursor_at.take() {
            // After the pending fit so the point lands where intended.
            let s = self.app.camera.grid_to_screen([gx, gy]);
            self.app.mouse_moved(s);
        }
        self.app.recompute_hover();
        let ui = ui_layout(&self.app);
        self.renderer.draw(&mut self.canvas, &self.app, &ui);
        self.canvas.flush();
        self.anim_active = self.renderer.wants_animation();

        if let Some(path) = self.screenshot.take() {
            match self.save_screenshot(&path) {
                Ok(()) => println!("Wrote {}", path.display()),
                Err(e) => {
                    eprintln!("Screenshot failed: {e}");
                    std::process::exit(1);
                }
            }
            event_loop.exit();
            return;
        }

        self.surface.swap_buffers(&self.context).unwrap();
    }

    fn save_screenshot(&mut self, path: &std::path::Path) -> Result<(), String> {
        let img = self.canvas.screenshot().map_err(|e| format!("{e:?}"))?;
        let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
        let mut encoder = png::Encoder::new(
            std::io::BufWriter::new(file),
            img.width() as u32,
            img.height() as u32,
        );
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        let mut data = Vec::with_capacity(img.width() * img.height() * 4);
        for row in img.rows() {
            for px in row {
                data.extend_from_slice(&[px.r, px.g, px.b, px.a]);
            }
        }
        writer.write_image_data(&data).map_err(|e| e.to_string())
    }

    fn update_cursor_icon(&self) {
        let icon = match &self.app.gesture {
            Gesture::Panning { .. } => CursorIcon::Grabbing,
            Gesture::Placing { .. }
            | Gesture::PlacingImage { .. }
            | Gesture::DrawingWire { .. } => CursorIcon::Crosshair,
            Gesture::DraggingComponents { .. } => CursorIcon::Move,
            Gesture::DraggingJunction { .. } => CursorIcon::Grabbing,
            _ => match self.app.hover {
                Hit::Port { .. } => CursorIcon::Crosshair,
                Hit::Junction { .. } => CursorIcon::Grab,
                Hit::Component { .. } | Hit::Note { .. } | Hit::Image { .. } => {
                    CursorIcon::Pointer
                }
                _ => CursorIcon::Default,
            },
        };
        self.window.set_cursor(icon);
    }

    /// Left press: dialog first, then chrome, then canvas (with
    /// double-click detection).
    fn left_pressed(&mut self) {
        // Modal layers swallow every click: a prompt dismisses (like
        // clicking outside any dialog), the layout dialog handles its own.
        if self.app.prompt.is_some() {
            self.app.dismiss_prompt();
            return;
        }
        if self.app.display_dialog.is_some() {
            self.app.display_dialog_pressed();
            return;
        }
        if self.app.image_dialog.is_some() {
            self.app.img_dialog_pressed();
            return;
        }
        if self.app.layout_dialog.is_some() {
            self.app.dialog_mouse_pressed();
            return;
        }
        let cursor = self.app.cursor;
        let ui = ui_layout(&self.app);

        for button in &ui.buttons {
            if button.rect.contains(cursor) {
                if !button.enabled {
                    return;
                }
                match button.id {
                    ButtonId::RunToggle => self.app.action_run_toggle(),
                    ButtonId::PlayPause => self.app.action_play_toggle(),
                    ButtonId::Rewind => self.app.action_rewind(),
                    ButtonId::StepBack => self.app.action_step_back(),
                    ButtonId::StepForward => self.app.action_step_forward(),
                    ButtonId::Fit => self.app.fit_view(),
                    ButtonId::Save => self.app.save_as(),
                    ButtonId::Open => self.app.open(),
                    ButtonId::NewAbstraction => self.app.action_new_abstraction(),
                    ButtonId::AddInputPin => {
                        self.app.action_add_pin(diycpu::layout::PortKind::Input)
                    }
                    ButtonId::AddOutputPin => {
                        self.app.action_add_pin(diycpu::layout::PortKind::Output)
                    }
                    ButtonId::RenameAbstraction => self.app.action_rename_title(),
                    ButtonId::LayoutDialog => self.app.action_open_layout_dialog(),
                    ButtonId::DeleteAbstraction => self.app.action_delete_abstraction(),
                    ButtonId::BackToTop => self.app.action_back_to_top(),
                }
                return;
            }
        }

        if ui.title_rect.contains(cursor) {
            self.app.action_rename_title();
            return;
        }

        for item in &ui.palette_items {
            if let Some(er) = &item.edit_rect {
                if er.contains(cursor) {
                    if let Some(i) = self
                        .app
                        .editor
                        .project
                        .abstractions
                        .iter()
                        .position(|a| a.name == item.type_name)
                    {
                        self.app.open_as_root(i);
                    }
                    return;
                }
            }
            if item.rect.contains(cursor) {
                if item.enabled {
                    self.app.palette_clicked(&item.type_name);
                }
                return;
            }
        }

        // Double-click detection (only meaningful on the canvas).
        let now = Instant::now();
        let is_double = self
            .last_press
            .map(|(t, p)| {
                now.duration_since(t).as_millis() < 350
                    && (p[0] - cursor[0]).abs() < 6.0
                    && (p[1] - cursor[1]).abs() < 6.0
            })
            .unwrap_or(false);

        if is_double && matches!(self.app.gesture, Gesture::Idle) {
            self.last_press = None;
            self.app.double_clicked();
        } else {
            self.last_press = Some((now, cursor));
            self.app.left_pressed();
        }
    }
}

// One instance for the whole program; variant size is irrelevant.
#[allow(clippy::large_enum_variant)]
enum AppState {
    Uninitialized(CliArgs),
    Running(Shell),
}

impl ApplicationHandler for AppState {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let AppState::Uninitialized(args) = self {
            *self = AppState::Running(Shell::new(event_loop, args));
        }
    }

    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: winit::event::StartCause) {
        // A timer fired: the autoplay clock, an animation frame, or both.
        if matches!(cause, winit::event::StartCause::ResumeTimeReached { .. }) {
            if let AppState::Running(shell) = self {
                if shell.app.playing && Instant::now() >= shell.next_play_tick {
                    shell.app.play_tick();
                    shell.next_play_tick = Instant::now() + PLAY_INTERVAL;
                }
                if shell.app.playing || shell.anim_active {
                    shell.window.request_redraw();
                }
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let AppState::Running(shell) = self {
            let mut next: Option<Instant> = None;
            if shell.app.playing {
                next = Some(shell.next_play_tick);
            }
            if shell.anim_active {
                let frame = Instant::now() + ANIM_FRAME;
                next = Some(next.map_or(frame, |n| n.min(frame)));
            }
            if let Some(at) = next {
                event_loop
                    .set_control_flow(winit::event_loop::ControlFlow::WaitUntil(at));
                return;
            }
        }
        event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let AppState::Running(shell) = self else {
            return;
        };

        match event {
            WindowEvent::CloseRequested => {
                if shell.app.request_close() {
                    event_loop.exit();
                } else {
                    shell.window.request_redraw();
                }
            }
            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    self_surface_resize(shell, size);
                }
                shell.window.request_redraw();
            }
            WindowEvent::RedrawRequested => shell.render(event_loop),
            WindowEvent::CursorMoved { position, .. } => {
                if shell.app.display_dialog.is_some() {
                    shell.app.cursor = [position.x as f32, position.y as f32];
                    shell.app.display_dialog_moved();
                } else if shell.app.image_dialog.is_some() {
                    shell.app.cursor = [position.x as f32, position.y as f32];
                    shell.app.img_dialog_moved();
                } else if shell.app.layout_dialog.is_some() {
                    shell.app.cursor = [position.x as f32, position.y as f32];
                    shell.app.dialog_mouse_moved();
                } else {
                    shell
                        .app
                        .mouse_moved([position.x as f32, position.y as f32]);
                }
                shell.update_cursor_icon();
                shell.window.request_redraw();
            }
            WindowEvent::MouseInput { state, button, .. } => {
                match (button, state) {
                    (MouseButton::Left, ElementState::Pressed) => shell.left_pressed(),
                    (MouseButton::Left, ElementState::Released) => {
                        if shell.app.display_dialog.is_some() {
                            shell.app.display_dialog_released();
                        } else if shell.app.image_dialog.is_some() {
                            shell.app.img_dialog_released();
                        } else if shell.app.layout_dialog.is_some() {
                            shell.app.dialog_mouse_released();
                        } else {
                            shell.app.left_released();
                        }
                    }
                    (MouseButton::Middle | MouseButton::Right, ElementState::Pressed) => {
                        shell.app.middle_or_right_pressed();
                    }
                    (MouseButton::Middle | MouseButton::Right, ElementState::Released) => {
                        shell.app.pan_released()
                    }
                    _ => {}
                }
                shell.update_cursor_icon();
                shell.window.request_redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let amount = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y * 120.0,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32,
                };
                shell.app.scrolled(amount);
                shell.window.request_redraw();
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                shell.app.ctrl =
                    modifiers.state().control_key() || modifiers.state().super_key();
                shell.app.shift = modifiers.state().shift_key();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return;
                }
                let key = match &event.logical_key {
                    Key::Named(NamedKey::Escape) => Some(app::KeyInput::Escape),
                    Key::Named(NamedKey::Delete) => Some(app::KeyInput::Delete),
                    Key::Named(NamedKey::Backspace) => Some(app::KeyInput::Backspace),
                    Key::Named(NamedKey::Enter) => Some(app::KeyInput::Enter),
                    Key::Named(NamedKey::ArrowRight) => Some(app::KeyInput::ArrowRight),
                    Key::Named(NamedKey::ArrowLeft) => Some(app::KeyInput::ArrowLeft),
                    Key::Named(NamedKey::Home) => Some(app::KeyInput::Home),
                    Key::Named(NamedKey::Space) if !event.repeat => {
                        Some(app::KeyInput::Space)
                    }
                    Key::Character(c) => Some(app::KeyInput::Char(c.as_str())),
                    _ => None,
                };
                if let Some(key) = key {
                    shell.app.handle_key(key);
                }
                shell.update_cursor_icon();
                shell.window.request_redraw();
            }
            WindowEvent::Focused(false) => {
                shell.app.focus_lost();
                shell.window.request_redraw();
            }
            _ => {}
        }
        shell.app.autosave();
    }
}

fn self_surface_resize(shell: &Shell, size: PhysicalSize<u32>) {
    shell.surface.resize(
        &shell.context,
        size.width.try_into().unwrap(),
        size.height.try_into().unwrap(),
    );
}

fn main() {
    let args = parse_args();
    let event_loop = EventLoop::new().unwrap();
    let mut state = AppState::Uninitialized(args);
    event_loop.run_app(&mut state).unwrap();
}
