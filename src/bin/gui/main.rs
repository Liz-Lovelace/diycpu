mod demo;
mod microchip;
mod wire;
mod workbench;

use femtovg::{renderer::OpenGl, Canvas, Color};
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
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowAttributes},
};

use workbench::Workbench;

struct App {
    context: PossiblyCurrentContext,
    surface: Surface<WindowSurface>,
    window: Window,
    canvas: Canvas<OpenGl>,
    workbench: Workbench,
}

impl App {
    fn new(event_loop: &ActiveEventLoop) -> Self {
        let window_attrs = WindowAttributes::default()
            .with_title("diycpu")
            .with_inner_size(PhysicalSize::new(1280, 720));

        let config_template = ConfigTemplateBuilder::new()
            .with_alpha_size(8)
            .with_stencil_size(8);

        let (window, gl_config) = DisplayBuilder::new()
            .with_window_attributes(Some(window_attrs))
            .build(event_loop, config_template, |configs| {
                configs
                    .reduce(|a, b| {
                        if a.num_samples() > b.num_samples() {
                            a
                        } else {
                            b
                        }
                    })
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
            size.width.try_into().unwrap(),
            size.height.try_into().unwrap(),
        );

        let surface = unsafe {
            gl_display
                .create_window_surface(&gl_config, &surface_attrs)
                .unwrap()
        };

        let context = gl_context.make_current(&surface).unwrap();

        let renderer =
            unsafe { OpenGl::new_from_function_cstr(|s| gl_display.get_proc_address(s).cast()) }
                .unwrap();

        let canvas = Canvas::new(renderer).unwrap();

        App {
            context,
            surface,
            window,
            canvas,
            workbench: Workbench::new(),
        }
    }

    fn render(&mut self) {
        let size = self.window.inner_size();
        let dpi = self.window.scale_factor() as f32;

        self.canvas.set_size(size.width, size.height, dpi);
        self.canvas.clear_rect(
            0,
            0,
            size.width,
            size.height,
            Color::rgbf(0.1, 0.1, 0.12),
        );

        self.workbench.draw_grid(&mut self.canvas, size.width, size.height);
        demo::draw_demo(&mut self.canvas, &self.workbench);

        self.canvas.flush();
        self.surface.swap_buffers(&self.context).unwrap();
    }
}

enum AppState {
    Uninitialized,
    Running(App),
}

impl ApplicationHandler for AppState {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if matches!(self, AppState::Uninitialized) {
            *self = AppState::Running(App::new(event_loop));
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let AppState::Running(app) = self else {
            return;
        };

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(_) => app.window.request_redraw(),
            WindowEvent::RedrawRequested => app.render(),
            _ => {}
        }
    }
}

fn main() {
    let event_loop = EventLoop::new().unwrap();
    let mut app = AppState::Uninitialized;
    event_loop.run_app(&mut app).unwrap();
}
