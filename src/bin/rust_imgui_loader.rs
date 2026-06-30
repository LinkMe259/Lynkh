#[path = "../api.rs"]
mod api;
#[path = "../config.rs"]
mod config;
#[path = "../hwid.rs"]
mod hwid;
#[allow(dead_code)]
#[path = "../models.rs"]
mod models;
#[path = "../program_profiles.rs"]
mod program_profiles;

use std::{
    collections::HashMap,
    num::NonZeroU32,
    path::Path,
    process::Command,
    rc::Rc,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

use config::AppConfig;
use glium::Surface as GliumSurface;
use glium::texture::{RawImage2d, Texture2d};
use glium::uniforms::{
    MagnifySamplerFilter, MinifySamplerFilter, SamplerBehavior, SamplerWrapFunction,
};
use glutin::{
    config::ConfigTemplateBuilder,
    context::ContextAttributesBuilder,
    display::GetGlDisplay,
    prelude::*,
    surface::{SurfaceAttributesBuilder, WindowSurface},
};
use imgui::{
    Condition, DrawListMut, FontConfig, FontGlyphRanges, FontSource, StyleColor, TextureId, Ui,
    WindowFlags,
};
use imgui_winit_support::{
    WinitPlatform,
    winit::{
        dpi::LogicalSize,
        event::{ElementState, Event, MouseButton, WindowEvent},
        event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
        window::{Window, WindowAttributes},
    },
};
use models::{LoginRequest, LoginResponse, MeResponse, Rental, RentalsResponse, UserInfo};
use program_profiles::{CoverArt, PROGRAM_PROFILES, ProgramIcon, ProgramProfile, profile_for};
use raw_window_handle::HasWindowHandle;

const WIDTH: f32 = 920.0;
const HEIGHT: f32 = 560.0;
const SIDEBAR_W: f32 = 104.0;
const PAGE_TRANSITION: f32 = 0.30;
const TOAST_TTL: f32 = 3.8;

type Col = [f32; 4];
type CoverTextures = HashMap<&'static str, CoverTexture>;

#[derive(Clone, Copy)]
struct CoverTexture {
    id: TextureId,
    size: [f32; 2],
}

fn c(r: u8, g: u8, b: u8, a: u8) -> Col {
    [
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        a as f32 / 255.0,
    ]
}

fn main() {
    let event_loop = EventLoop::new().unwrap();
    let mut runtime: Option<Runtime> = None;

    #[allow(deprecated)]
    event_loop
        .run(move |event, target| {
            target.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + Duration::from_millis(16),
            ));

            if let Some(runtime) = runtime.as_mut() {
                runtime
                    .platform
                    .handle_event(runtime.imgui.io_mut(), &runtime.window, &event);
            }

            match event {
                Event::Resumed if runtime.is_none() => {
                    runtime = Some(Runtime::new(target));
                }
                Event::NewEvents(_) => {
                    if let Some(runtime) = runtime.as_mut() {
                        let now = Instant::now();
                        runtime
                            .imgui
                            .io_mut()
                            .update_delta_time(now.duration_since(runtime.last_frame));
                        runtime.last_frame = now;
                    }
                }
                Event::AboutToWait => {
                    if let Some(runtime) = runtime.as_mut() {
                        runtime
                            .platform
                            .prepare_frame(runtime.imgui.io_mut(), &runtime.window)
                            .unwrap();
                        runtime.window.request_redraw();
                    }
                }
                Event::WindowEvent { window_id, event } => {
                    let Some(runtime) = runtime.as_mut() else {
                        return;
                    };
                    if window_id != runtime.window.id() {
                        return;
                    }

                    match event {
                        WindowEvent::RedrawRequested => {
                            if let Some(command) = runtime.render() {
                                match command {
                                    WindowCommand::Close => target.exit(),
                                    WindowCommand::Minimize => runtime.window.set_minimized(true),
                                }
                            }
                        }
                        WindowEvent::Resized(new_size) => {
                            if new_size.width > 0 && new_size.height > 0 {
                                runtime.display.resize((new_size.width, new_size.height));
                            }
                        }
                        WindowEvent::CursorMoved { position, .. } => {
                            runtime.mouse_pos = [position.x as f32, position.y as f32];
                        }
                        WindowEvent::MouseInput {
                            state: ElementState::Pressed,
                            button: MouseButton::Left,
                            ..
                        } => {
                            let inner = runtime.window.inner_size();
                            let can_drag = runtime.mouse_pos[1] <= 70.0
                                && runtime.mouse_pos[0] > SIDEBAR_W
                                && runtime.mouse_pos[0] < inner.width as f32 - 120.0;
                            if can_drag {
                                let _ = runtime.window.drag_window();
                            }
                        }
                        WindowEvent::CloseRequested => target.exit(),
                        _ => {}
                    }
                }
                _ => {}
            }
        })
        .expect("event loop error");
}

struct Runtime {
    window: Window,
    display: glium::Display<WindowSurface>,
    platform: WinitPlatform,
    imgui: imgui::Context,
    renderer: imgui_glium_renderer::Renderer,
    app: LoaderApp,
    last_frame: Instant,
    mouse_pos: [f32; 2],
}

impl Runtime {
    fn new(target: &ActiveEventLoop) -> Self {
        let (window, display) = create_window(target);
        let (platform, mut imgui) = imgui_init(&window);
        apply_style(&mut imgui);
        let mut renderer = imgui_glium_renderer::Renderer::new(&mut imgui, &display)
            .expect("failed to create renderer");
        let cover_textures = load_cover_textures(&display, &mut renderer);

        Self {
            window,
            display,
            platform,
            imgui,
            renderer,
            app: LoaderApp::new(cover_textures),
            last_frame: Instant::now(),
            mouse_pos: [-1.0, -1.0],
        }
    }

    fn render(&mut self) -> Option<WindowCommand> {
        let ui = self.imgui.frame();
        self.app.draw(ui);
        let command = self.app.take_window_command();

        let mut target_frame = self.display.draw();
        target_frame.clear_color_srgb(0.0, 0.0, 0.0, 0.0);

        self.platform.prepare_render(ui, &self.window);
        let draw_data = self.imgui.render();
        self.renderer
            .render(&mut target_frame, draw_data)
            .expect("error rendering imgui");
        target_frame.finish().expect("swap buffers failed");
        command
    }
}

fn create_window(target: &ActiveEventLoop) -> (Window, glium::Display<WindowSurface>) {
    let attrs = WindowAttributes::default()
        .with_title("NOVA Loader Rust ImGui")
        .with_inner_size(LogicalSize::new(WIDTH, HEIGHT))
        .with_min_inner_size(LogicalSize::new(760.0, 500.0))
        .with_resizable(false)
        .with_decorations(false)
        .with_transparent(true);

    let (window, cfg) = glutin_winit::DisplayBuilder::new()
        .with_window_attributes(Some(attrs))
        .build(
            target,
            ConfigTemplateBuilder::new()
                .with_alpha_size(8)
                .with_transparency(true),
            |mut configs| configs.next().unwrap(),
        )
        .expect("failed to create OpenGL window");

    let window = window.unwrap();
    let context_attribs =
        ContextAttributesBuilder::new().build(Some(window.window_handle().unwrap().as_raw()));
    let context = unsafe {
        cfg.display()
            .create_context(&cfg, &context_attribs)
            .expect("failed to create OpenGL context")
    };

    let surface_attribs = SurfaceAttributesBuilder::<WindowSurface>::new()
        .with_srgb(Some(true))
        .build(
            window.window_handle().unwrap().as_raw(),
            NonZeroU32::new(WIDTH as u32).unwrap(),
            NonZeroU32::new(HEIGHT as u32).unwrap(),
        );
    let surface = unsafe {
        cfg.display()
            .create_window_surface(&cfg, &surface_attribs)
            .expect("failed to create OpenGL surface")
    };
    let context = context
        .make_current(&surface)
        .expect("failed to make OpenGL context current");

    let display = glium::Display::from_context_surface(context, surface)
        .expect("failed to create glium display");

    (window, display)
}

fn load_cover_textures(
    display: &glium::Display<WindowSurface>,
    renderer: &mut imgui_glium_renderer::Renderer,
) -> CoverTextures {
    let mut textures = HashMap::new();

    for profile in PROGRAM_PROFILES {
        let Some(path) = profile.cover_image else {
            continue;
        };
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
        let Ok(image) = image::open(&path) else {
            continue;
        };

        let rgba = image.to_rgba8();
        let dimensions = rgba.dimensions();
        let raw = RawImage2d::from_raw_rgba(rgba.into_raw(), dimensions);
        let Ok(texture) = Texture2d::new(display, raw) else {
            continue;
        };

        let texture_id = renderer.textures().insert(imgui_glium_renderer::Texture {
            texture: Rc::new(texture),
            sampler: SamplerBehavior {
                wrap_function: (
                    SamplerWrapFunction::Clamp,
                    SamplerWrapFunction::Clamp,
                    SamplerWrapFunction::Clamp,
                ),
                minify_filter: MinifySamplerFilter::Linear,
                magnify_filter: MagnifySamplerFilter::Linear,
                ..SamplerBehavior::default()
            },
        });
        textures.insert(
            profile.id,
            CoverTexture {
                id: texture_id,
                size: [dimensions.0 as f32, dimensions.1 as f32],
            },
        );
    }

    textures
}

fn imgui_init(window: &Window) -> (WinitPlatform, imgui::Context) {
    let mut imgui = imgui::Context::create();
    imgui.set_ini_filename(None);

    let mut platform = WinitPlatform::new(&mut imgui);
    platform.attach_window(
        imgui.io_mut(),
        window,
        imgui_winit_support::HiDpiMode::Default,
    );

    install_font(&mut imgui);
    imgui.io_mut().font_global_scale = 1.0;

    (platform, imgui)
}

fn install_font(imgui: &mut imgui::Context) {
    let candidates = [
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        "/System/Library/Fonts/Supplemental/NotoSansThai-Regular.ttf",
        "/Library/Fonts/NotoSansThai-Regular.ttf",
        "C:\\Windows\\Fonts\\tahoma.ttf",
        "C:\\Windows\\Fonts\\segoeui.ttf",
    ];

    if let Some(bytes) = candidates
        .iter()
        .find_map(|path| std::fs::read(path).ok().filter(|bytes| !bytes.is_empty()))
    {
        let data: &'static [u8] = Box::leak(bytes.into_boxed_slice());
        imgui.fonts().add_font(&[FontSource::TtfData {
            data,
            size_pixels: 17.0,
            config: Some(FontConfig {
                oversample_h: 3,
                oversample_v: 2,
                pixel_snap_h: false,
                glyph_ranges: FontGlyphRanges::thai(),
                ..FontConfig::default()
            }),
        }]);
    } else {
        imgui
            .fonts()
            .add_font(&[FontSource::DefaultFontData { config: None }]);
    }
}

fn apply_style(imgui: &mut imgui::Context) {
    let style = imgui.style_mut();
    style.window_rounding = 18.0;
    style.child_rounding = 16.0;
    style.frame_rounding = 12.0;
    style.grab_rounding = 10.0;
    style.popup_rounding = 12.0;
    style.scrollbar_rounding = 12.0;
    style.window_border_size = 0.0;
    style.frame_border_size = 0.0;
    style.window_padding = [0.0, 0.0];
    style.item_spacing = [10.0, 9.0];
    style.item_inner_spacing = [8.0, 6.0];

    style.colors[StyleColor::Text as usize] = c(236, 242, 250, 255);
    style.colors[StyleColor::WindowBg as usize] = c(6, 8, 11, 255);
    style.colors[StyleColor::ChildBg as usize] = c(11, 13, 17, 236);
    style.colors[StyleColor::Border as usize] = c(115, 230, 255, 58);
    style.colors[StyleColor::FrameBg as usize] = c(25, 30, 38, 250);
    style.colors[StyleColor::FrameBgHovered as usize] = c(34, 44, 52, 255);
    style.colors[StyleColor::FrameBgActive as usize] = c(45, 58, 63, 255);
    style.colors[StyleColor::Button as usize] = c(26, 32, 38, 255);
    style.colors[StyleColor::ButtonHovered as usize] = c(41, 64, 66, 255);
    style.colors[StyleColor::ButtonActive as usize] = c(54, 78, 72, 255);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Home,
    Detail,
    Account,
}

#[derive(Clone, Copy)]
enum WindowCommand {
    Close,
    Minimize,
}

#[derive(Clone, Copy)]
enum NoticeKind {
    Info,
    Success,
    Warning,
    Error,
}

struct Toast {
    text: String,
    kind: NoticeKind,
    created_at: Instant,
    ttl: Duration,
}

enum AppMessage {
    Login(Result<LoginResponse, api::ApiError>),
    Me(Result<MeResponse, api::ApiError>),
    Rentals(Result<RentalsResponse, api::ApiError>),
    Logout(Result<(), api::ApiError>),
}

struct LoaderApp {
    config: AppConfig,
    username: String,
    password: String,
    hwid: String,
    token: Option<String>,
    user: Option<UserInfo>,
    expires_at: Option<String>,
    hwid_locked: Option<bool>,
    rentals: Vec<Rental>,
    selected: Option<usize>,
    page: Page,
    busy_label: Option<String>,
    status: String,
    status_kind: NoticeKind,
    toasts: Vec<Toast>,
    cover_textures: CoverTextures,
    pending_window_command: Option<WindowCommand>,
    page_changed_at: Instant,
    page_direction: f32,
    tx: Sender<AppMessage>,
    rx: Receiver<AppMessage>,
}

impl LoaderApp {
    fn new(cover_textures: CoverTextures) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            config: AppConfig::default(),
            username: String::new(),
            password: String::new(),
            hwid: hwid::stable_hwid(),
            token: None,
            user: None,
            expires_at: None,
            hwid_locked: None,
            rentals: Vec::new(),
            selected: None,
            page: Page::Home,
            busy_label: None,
            status: "Ready.".to_owned(),
            status_kind: NoticeKind::Info,
            toasts: Vec::new(),
            cover_textures,
            pending_window_command: None,
            page_changed_at: Instant::now(),
            page_direction: 1.0,
            tx,
            rx,
        }
    }

    fn draw(&mut self, ui: &Ui) {
        self.poll_messages();

        let size = ui.io().display_size;
        let flags = WindowFlags::NO_DECORATION
            | WindowFlags::NO_RESIZE
            | WindowFlags::NO_MOVE
            | WindowFlags::NO_SAVED_SETTINGS
            | WindowFlags::NO_BRING_TO_FRONT_ON_FOCUS
            | WindowFlags::NO_BACKGROUND;

        ui.window("NOVA_RUST_ROOT")
            .position([0.0, 0.0], Condition::Always)
            .size(size, Condition::Always)
            .flags(flags)
            .build(|| {
                self.draw_background(ui, size);

                if self.logged_in() {
                    let page_offset = self.page_offset();
                    self.draw_sidebar(ui, size);
                    match self.page {
                        Page::Home => self.draw_home(ui, size, page_offset),
                        Page::Detail => self.draw_detail(ui, size, page_offset),
                        Page::Account => self.draw_account(ui, size, page_offset),
                    }
                    self.draw_page_transition(ui, size);
                } else {
                    self.draw_login(ui, size);
                }

                self.draw_toasts(ui, size);
                self.draw_window_controls(ui, size);
                self.draw_busy(ui, size);
            });
    }

    fn take_window_command(&mut self) -> Option<WindowCommand> {
        self.pending_window_command.take()
    }

    fn set_page(&mut self, page: Page) {
        if self.page == page {
            return;
        }
        self.page_direction = if page_rank(page) >= page_rank(self.page) {
            1.0
        } else {
            -1.0
        };
        self.page = page;
        self.page_changed_at = Instant::now();
    }

    fn page_progress(&self) -> f32 {
        let elapsed = self.page_changed_at.elapsed().as_secs_f32();
        ease_out_cubic((elapsed / PAGE_TRANSITION).clamp(0.0, 1.0))
    }

    fn page_offset(&self) -> f32 {
        (1.0 - self.page_progress()) * 28.0 * self.page_direction
    }

    fn poll_messages(&mut self) {
        while let Ok(message) = self.rx.try_recv() {
            self.busy_label = None;
            match message {
                AppMessage::Login(result) => match result {
                    Ok(response) => {
                        self.token = Some(response.token);
                        self.password.clear();
                        self.set_status("Login success. Syncing account...", NoticeKind::Success);
                        self.push_toast("Login success.", NoticeKind::Success);
                        self.start_me();
                    }
                    Err(error) => {
                        self.clear_session();
                        self.set_api_error("Login failed", error);
                    }
                },
                AppMessage::Me(result) => match result {
                    Ok(response) => {
                        self.user = Some(response.user);
                        self.expires_at = Some(response.session.expires_at);
                        self.hwid_locked = Some(response.session.hwid_locked);
                        self.set_page(Page::Home);
                        self.set_status("Account synced.", NoticeKind::Success);
                        self.start_rentals();
                    }
                    Err(error) => {
                        self.clear_session();
                        self.set_api_error("Account sync failed", error);
                    }
                },
                AppMessage::Rentals(result) => match result {
                    Ok(response) => {
                        self.rentals = response.rentals;
                        self.selected = if self.rentals.is_empty() {
                            None
                        } else {
                            Some(0)
                        };
                        self.set_status("Games loaded.", NoticeKind::Success);
                        self.push_toast("Game library updated.", NoticeKind::Success);
                    }
                    Err(error) => {
                        if error.status == Some(401) {
                            self.clear_session();
                        }
                        self.set_api_error("Game loading failed", error);
                    }
                },
                AppMessage::Logout(result) => {
                    self.clear_session();
                    match result {
                        Ok(()) => {
                            self.set_status("Logged out.", NoticeKind::Info);
                            self.push_toast("Logged out.", NoticeKind::Info);
                        }
                        Err(error) => {
                            self.set_status(
                                format!("Logged out locally: {}", error.message),
                                NoticeKind::Warning,
                            );
                            self.push_toast("Logged out locally.", NoticeKind::Warning);
                        }
                    }
                }
            }
        }
    }

    fn logged_in(&self) -> bool {
        self.user.is_some()
    }

    fn is_busy(&self) -> bool {
        self.busy_label.is_some()
    }

    fn set_status(&mut self, text: impl Into<String>, kind: NoticeKind) {
        self.status = text.into();
        self.status_kind = kind;
    }

    fn set_api_error(&mut self, prefix: &str, error: api::ApiError) {
        let text = format!("{prefix}: {}", error.message);
        self.set_status(text.clone(), NoticeKind::Error);
        self.push_toast(text, NoticeKind::Error);
    }

    fn push_toast(&mut self, text: impl Into<String>, kind: NoticeKind) {
        self.toasts.push(Toast {
            text: text.into(),
            kind,
            created_at: Instant::now(),
            ttl: Duration::from_secs_f32(TOAST_TTL),
        });
        if self.toasts.len() > 5 {
            self.toasts.remove(0);
        }
    }

    fn clear_session(&mut self) {
        self.token = None;
        self.user = None;
        self.expires_at = None;
        self.hwid_locked = None;
        self.rentals.clear();
        self.selected = None;
        self.set_page(Page::Home);
        self.busy_label = None;
    }

    fn start_login(&mut self) {
        if self.is_busy() || self.username.trim().is_empty() || self.password.is_empty() {
            self.set_status("Enter username and password.", NoticeKind::Warning);
            self.push_toast("Enter username and password.", NoticeKind::Warning);
            return;
        }

        self.clear_session();
        self.busy_label = Some("Logging in...".to_owned());
        self.set_status("Sending login request...", NoticeKind::Info);

        let tx = self.tx.clone();
        let base_url = self.config.api_base_url();
        let request = LoginRequest {
            user: self.username.trim().to_owned(),
            password: self.password.clone(),
            hwid: self.hwid.clone(),
        };
        thread::spawn(move || {
            let _ = tx.send(AppMessage::Login(api::login(&base_url, request)));
        });
    }

    fn start_me(&mut self) {
        let Some(token) = self.token.clone() else {
            return;
        };
        self.busy_label = Some("Syncing account...".to_owned());
        let tx = self.tx.clone();
        let base_url = self.config.api_base_url();
        thread::spawn(move || {
            let _ = tx.send(AppMessage::Me(api::me(&base_url, &token)));
        });
    }

    fn start_rentals(&mut self) {
        let Some(token) = self.token.clone() else {
            return;
        };
        self.busy_label = Some("Loading games...".to_owned());
        let tx = self.tx.clone();
        let base_url = self.config.api_base_url();
        thread::spawn(move || {
            let _ = tx.send(AppMessage::Rentals(api::rentals(&base_url, &token)));
        });
    }

    fn start_logout(&mut self) {
        let Some(token) = self.token.clone() else {
            self.clear_session();
            return;
        };
        self.busy_label = Some("Logging out...".to_owned());
        let tx = self.tx.clone();
        let base_url = self.config.api_base_url();
        thread::spawn(move || {
            let _ = tx.send(AppMessage::Logout(api::logout(&base_url, &token)));
        });
    }

    fn launch_selected(&mut self) {
        let Some(rental) = self.selected_rental().cloned() else {
            self.push_toast("Select a game first.", NoticeKind::Warning);
            return;
        };
        if is_launchable(&rental) {
            let profile = self.profile_for_rental(&rental);
            match launch_profile(profile) {
                Ok(0) => {
                    self.set_status(
                        format!("ยังไม่ได้ตั้งค่าไฟล์ที่จะรันสำหรับ {}", rental.product_name),
                        NoticeKind::Warning,
                    );
                    self.push_toast("ยังไม่ได้ตั้งค่าไฟล์เปิดโปรแกรมนี้", NoticeKind::Warning);
                }
                Ok(count) => {
                    self.set_status(
                        format!("Launched {count} step(s): {}", rental.product_name),
                        NoticeKind::Success,
                    );
                    self.push_toast("Launch commands started.", NoticeKind::Success);
                }
                Err(error) => {
                    self.set_status(format!("Launch failed: {error}"), NoticeKind::Error);
                    self.push_toast("Launch failed.", NoticeKind::Error);
                }
            }
        } else {
            self.set_status("Access denied.", NoticeKind::Warning);
            self.push_toast("Rental is not launchable.", NoticeKind::Warning);
        }
    }

    fn selected_rental(&self) -> Option<&Rental> {
        self.selected.and_then(|index| self.rentals.get(index))
    }

    fn profile_for_rental(&self, rental: &Rental) -> &'static ProgramProfile {
        profile_for(&rental.product_name)
    }

    fn cover_texture(&self, profile: &ProgramProfile) -> Option<CoverTexture> {
        self.cover_textures.get(profile.id).copied()
    }

    fn draw_background(&self, ui: &Ui, size: [f32; 2]) {
        let draw = ui.get_window_draw_list();
        draw.add_rect([0.0, 0.0], size, c(5, 10, 13, 250))
            .filled(true)
            .rounding(30.0)
            .build();
        draw.add_rect([0.0, 0.0], size, c(28, 22, 34, 144))
            .filled(true)
            .rounding(30.0)
            .build();
        draw.add_rect([size[0] * 0.50, 0.0], size, c(99, 44, 20, 112))
            .filled(true)
            .rounding(26.0)
            .build();
        draw.add_polyline(
            vec![
                [size[0] * 0.14, 0.0],
                [size[0] * 0.34, 0.0],
                [size[0] * 0.18, size[1] - 18.0],
                [18.0, size[1] - 18.0],
            ],
            c(0, 166, 150, 26),
        )
        .filled(true)
        .build();
        draw.add_rect([0.0, size[1] * 0.52], size, c(0, 0, 0, 132))
            .filled(true)
            .rounding(26.0)
            .build();

        for i in 0..7 {
            let x = ((ui.time() as f32) * 6.0 + i as f32 * 190.0) % (size[0] + 260.0) - 160.0;
            let y = 48.0 + ((i * 71) % 250) as f32;
            let color = match i % 3 {
                0 => c(100, 255, 224, 15),
                1 => c(255, 190, 96, 14),
                _ => c(255, 118, 169, 12),
            };
            draw.add_rect([x, y], [x + 180.0, y + 26.0], color)
                .filled(true)
                .rounding(13.0)
                .build();
        }

        draw.add_rect([0.0, 0.0], size, c(0, 0, 0, 48))
            .filled(true)
            .rounding(30.0)
            .build();
        draw.add_rect([0.0, 0.0], size, c(215, 245, 255, 64))
            .rounding(30.0)
            .thickness(1.0)
            .build();
        draw.add_line([128.0, 16.0], [128.0, size[1] - 16.0], c(255, 255, 255, 34))
            .build();
    }

    fn draw_window_controls(&mut self, ui: &Ui, size: [f32; 2]) {
        let draw = ui.get_window_draw_list();
        let y = 26.0;
        if window_control_button(ui, &draw, "window_minimize", [size[0] - 74.0, y], false) {
            self.pending_window_command = Some(WindowCommand::Minimize);
        }
        if window_control_button(ui, &draw, "window_close", [size[0] - 38.0, y], true) {
            self.pending_window_command = Some(WindowCommand::Close);
        }
    }

    fn draw_sidebar(&mut self, ui: &Ui, size: [f32; 2]) {
        let draw = ui.get_window_draw_list();
        draw.add_rect([0.0, 0.0], [SIDEBAR_W, size[1]], c(4, 8, 10, 182))
            .filled(true)
            .rounding(28.0)
            .build();
        draw.add_rect([0.0, 0.0], [SIDEBAR_W, size[1]], c(120, 255, 230, 12))
            .filled(true)
            .rounding(28.0)
            .build();
        draw.add_line(
            [SIDEBAR_W - 1.0, 26.0],
            [SIDEBAR_W - 1.0, size[1] - 26.0],
            c(150, 240, 255, 34),
        )
        .build();

        let back = [54.0, 54.0];
        draw.add_circle(back, 36.0, c(8, 11, 14, 238))
            .filled(true)
            .num_segments(48)
            .build();
        draw_back_icon(&draw, back, c(224, 236, 242, 255), 1.05);
        ui.set_cursor_screen_pos([19.0, 19.0]);
        if ui.invisible_button("back_home", [70.0, 70.0]) {
            self.set_page(Page::Home);
        }

        self.nav_icon(ui, &draw, "nav_home", 0, Page::Home, 276.0);
        self.nav_icon(ui, &draw, "nav_account", 1, Page::Account, 358.0);
    }

    fn nav_icon(
        &mut self,
        ui: &Ui,
        draw: &DrawListMut<'_>,
        id: &str,
        icon: i32,
        page: Page,
        y: f32,
    ) {
        let active = self.page == page || (page == Page::Home && self.page == Page::Detail);
        let center = [54.0, y + 27.0];
        if active {
            draw.add_circle(center, 36.0, c(76, 245, 213, 26))
                .filled(true)
                .num_segments(48)
                .build();
            draw.add_circle(center, 29.0, c(242, 207, 119, 255))
                .filled(true)
                .num_segments(48)
                .build();
        } else {
            draw.add_circle(center, 29.0, c(8, 11, 14, 236))
                .filled(true)
                .num_segments(48)
                .build();
            draw.add_circle(center, 29.0, c(255, 255, 255, 16))
                .num_segments(48)
                .build();
        }
        let color = if active {
            c(11, 16, 18, 255)
        } else {
            c(214, 224, 232, 255)
        };
        if icon == 0 {
            draw_gamepad_icon(draw, center, color, 0.82);
        } else {
            draw_user_icon(draw, center, color, 0.82);
        }
        ui.set_cursor_screen_pos([27.0, y]);
        if ui.invisible_button(id, [54.0, 54.0]) {
            self.set_page(page);
        }
    }

    fn draw_login(&mut self, ui: &Ui, size: [f32; 2]) {
        let compact = size[0] < 700.0 || size[1] < 460.0;
        let panel = if compact {
            [
                (size[0] * 0.62).clamp(270.0, 340.0),
                (size[1] * 0.78).clamp(248.0, 284.0),
            ]
        } else {
            [410.0, 304.0]
        };
        let pos = [(size[0] - panel[0]) * 0.5, (size[1] - panel[1]) * 0.5];
        draw_card(ui, pos, panel, c(18, 21, 25, 226));

        let draw = ui.get_window_draw_list();
        draw.add_rect(
            [pos[0] + 1.0, pos[1] + 1.0],
            [pos[0] + panel[0] - 1.0, pos[1] + 58.0],
            c(102, 238, 221, 16),
        )
        .filled(true)
        .rounding(16.0)
        .build();
        draw.add_circle([pos[0] + 34.0, pos[1] + 32.0], 15.0, c(242, 207, 119, 255))
            .filled(true)
            .num_segments(32)
            .build();
        draw.add_text([pos[0] + 29.0, pos[1] + 23.0], c(13, 16, 18, 255), "N");

        let pad = if compact { 24.0 } else { 28.0 };
        ui.set_cursor_screen_pos([pos[0] + 58.0, pos[1] + 19.0]);
        ui.text("Nova Loader");
        ui.set_cursor_screen_pos([pos[0] + 58.0, pos[1] + 43.0]);
        ui.text_colored(c(154, 166, 180, 255), "Secure program access");

        ui.set_cursor_screen_pos([pos[0] + pad, pos[1] + 86.0]);
        ui.text_colored(c(176, 184, 194, 255), "Username");
        ui.set_cursor_screen_pos([pos[0] + pad, pos[1] + 111.0]);
        ui.set_next_item_width(panel[0] - pad * 2.0);
        ui.input_text("##username", &mut self.username).build();

        ui.set_cursor_screen_pos([pos[0] + pad, pos[1] + 148.0]);
        ui.text_colored(c(176, 184, 194, 255), "Password");
        ui.set_cursor_screen_pos([pos[0] + pad, pos[1] + 173.0]);
        ui.set_next_item_width(panel[0] - pad * 2.0);
        let submit = ui
            .input_text("##password", &mut self.password)
            .password(true)
            .enter_returns_true(true)
            .build();
        if submit {
            self.start_login();
        }

        ui.set_cursor_screen_pos([pos[0] + pad, pos[1] + panel[1] - 62.0]);
        if custom_button_on(
            ui,
            &draw,
            "login_button",
            self.busy_label.as_deref().unwrap_or("Sign In"),
            [panel[0] - pad * 2.0, 40.0],
            c(242, 207, 119, 255),
        ) {
            self.start_login();
        }
    }

    fn draw_home(&mut self, ui: &Ui, size: [f32; 2], offset_x: f32) {
        let base_left = SIDEBAR_W + 46.0;
        let left = base_left + offset_x;
        let top = 82.0;
        let width = size[0] - base_left - 42.0;
        let height = size[1] - top - 42.0;

        if self.rentals.is_empty() {
            let card = [width, height.min(260.0)];
            ui.set_cursor_screen_pos([left, top]);
            ui.invisible_button("empty_home", card);
            let draw = ui.get_window_draw_list();
            draw_cover(&draw, [left, top], card, 0, false, CoverArt::Default, None);
            draw.add_rect(
                [left, top],
                [left + card[0], top + card[1]],
                c(0, 0, 0, 108),
            )
            .filled(true)
            .rounding(24.0)
            .build();
            draw.add_text(
                [left + 34.0, top + card[1] - 86.0],
                c(255, 255, 255, 255),
                "No games found",
            );
            draw.add_text(
                [left + 34.0, top + card[1] - 58.0],
                c(202, 208, 218, 255),
                "Reload after adding a rental",
            );
            return;
        }

        let gap = 18.0;
        let half_w = (width - gap) * 0.5;
        let card_h = (height * 0.35).clamp(128.0, 164.0);
        let wide_h = (height * 0.41).clamp(150.0, 198.0);
        let mut y = top;
        let mut i = 0;
        while i < self.rentals.len() {
            if i + 1 < self.rentals.len() {
                self.draw_game_card(ui, i, [left, y], [half_w, card_h], false);
                self.draw_game_card(ui, i + 1, [left + half_w + gap, y], [half_w, card_h], false);
                i += 2;
                y += card_h + gap;
            } else {
                self.draw_game_card(ui, i, [left, y], [width, wide_h], true);
                i += 1;
                y += wide_h + gap;
            }
            if i < self.rentals.len() {
                self.draw_game_card(ui, i, [left, y], [width, wide_h], true);
                i += 1;
                y += wide_h + gap;
            }
        }
    }

    fn draw_game_card(&mut self, ui: &Ui, index: usize, pos: [f32; 2], size: [f32; 2], wide: bool) {
        ui.set_cursor_screen_pos(pos);
        if ui.invisible_button(format!("game_card_{index}"), size) {
            self.selected = Some(index);
            self.set_page(Page::Detail);
        }
        let hovered = ui.is_item_hovered();
        let lift = if hovered { -2.0 } else { 0.0 };
        let draw = ui.get_window_draw_list();
        let pos = [pos[0], pos[1] + lift];
        let rental = &self.rentals[index];
        let profile = self.profile_for_rental(rental);
        let texture = self.cover_texture(profile);
        let palette = palette_for_profile(index, &rental.product_name, profile.cover_art);
        let subtitle = rental_subtitle(rental, profile);

        draw.add_rect(
            [pos[0] + 4.0, pos[1] + 7.0],
            [pos[0] + size[0] + 4.0, pos[1] + size[1] + 7.0],
            c(0, 0, 0, 58),
        )
        .filled(true)
        .rounding(26.0)
        .build();
        draw_cover(
            &draw,
            pos,
            size,
            index as u32,
            wide,
            profile.cover_art,
            texture,
        );
        draw.add_rect(
            pos,
            [pos[0] + size[0], pos[1] + size[1]],
            c(0, 0, 0, if wide { 48 } else { 56 }),
        )
        .filled(true)
        .rounding(26.0)
        .build();

        let icon_size = if wide { 48.0 } else { 42.0 };
        draw_product_icon(
            &draw,
            [
                pos[0] + size[0] - icon_size - 24.0,
                pos[1] + size[1] - icon_size - 22.0,
            ],
            icon_size * 0.5,
            palette.accent,
            c(0, 0, 0, 96),
            profile.icon,
        );
        draw_pill(
            &draw,
            [pos[0] + 22.0, pos[1] + 20.0],
            &rental.rental_status,
            notice_color(rental_kind(rental)),
        );

        let label_target = if wide { size[0] * 0.42 } else { size[0] * 0.54 };
        let label_min = if wide { 230.0 } else { 164.0 };
        let label_max = (size[0] - 88.0).min(label_target).max(label_min);
        let label_w = label_target.max(label_min).min(label_max);
        let label_inner_w = (label_w - 36.0).max(80.0);
        let title_text = fit_text_to_width(ui, &rental.product_name, label_inner_w);
        let subtitle_text = fit_text_to_width(ui, &subtitle, label_inner_w);
        let label = [pos[0] + 24.0, pos[1] + size[1] - 68.0];
        draw.add_rect(
            label,
            [label[0] + label_w, label[1] + 54.0],
            c(9, 15, 18, 148),
        )
        .filled(true)
        .rounding(12.0)
        .build();
        draw.add_rect(
            label,
            [label[0] + label_w, label[1] + 54.0],
            c(255, 255, 255, 34),
        )
        .rounding(12.0)
        .thickness(1.0)
        .build();
        draw.add_text(
            [label[0] + 18.0, label[1] + 8.0],
            c(255, 255, 255, 255),
            title_text,
        );
        draw.add_text(
            [label[0] + 18.0, label[1] + 30.0],
            c(204, 214, 224, 232),
            subtitle_text,
        );
        draw.add_rect(
            pos,
            [pos[0] + size[0], pos[1] + size[1]],
            if hovered {
                palette.accent
            } else {
                alpha_col(palette.border, 0.72)
            },
        )
        .rounding(26.0)
        .thickness(if hovered { 2.4 } else { 1.4 })
        .build();
    }

    fn draw_detail(&mut self, ui: &Ui, size: [f32; 2], offset_x: f32) {
        let Some(index) = self.selected else {
            self.set_page(Page::Home);
            return;
        };
        let Some(rental) = self.rentals.get(index).cloned() else {
            self.set_page(Page::Home);
            return;
        };

        let base_left = SIDEBAR_W + 46.0;
        let left = base_left + offset_x;
        let top = 76.0;
        let width = size[0] - base_left - 44.0;
        let height = size[1] - top - 42.0;
        let hero_h = (height * 0.46).clamp(172.0, 238.0);
        let profile = self.profile_for_rental(&rental);
        let texture = self.cover_texture(profile);
        let palette = palette_for_profile(index, &rental.product_name, profile.cover_art);
        let draw = ui.get_window_draw_list();

        draw_cover(
            &draw,
            [left, top],
            [width, hero_h],
            index as u32,
            true,
            profile.cover_art,
            texture,
        );
        draw.add_rect([left, top], [left + width, top + hero_h], c(0, 0, 0, 58))
            .filled(true)
            .rounding(28.0)
            .build();
        draw.add_rect([left, top], [left + width, top + hero_h], palette.border)
            .rounding(28.0)
            .thickness(2.0)
            .build();
        draw_product_icon(
            &draw,
            [left + width - 76.0, top + hero_h - 76.0],
            42.0,
            palette.accent,
            c(0, 0, 0, 118),
            profile.icon,
        );

        let back_min = [left + 22.0, top + 20.0];
        ui.set_cursor_screen_pos(back_min);
        if ui.invisible_button("detail_back", [102.0, 38.0]) {
            self.set_page(Page::Home);
        }
        let back_hover = ui.is_item_hovered();
        draw.add_rect(
            back_min,
            [back_min[0] + 102.0, back_min[1] + 38.0],
            if back_hover {
                c(255, 255, 255, 40)
            } else {
                c(0, 0, 0, 96)
            },
        )
        .filled(true)
        .rounding(12.0)
        .build();
        draw_back_icon(
            &draw,
            [back_min[0] + 22.0, back_min[1] + 17.0],
            c(235, 238, 244, 255),
            0.7,
        );
        draw.add_text(
            [back_min[0] + 40.0, back_min[1] + 10.0],
            c(235, 238, 244, 255),
            "Back",
        );

        draw_pill(
            &draw,
            [left + width - 130.0, top + 22.0],
            &rental.rental_status,
            notice_color(rental_kind(&rental)),
        );
        let title = [left + 30.0, top + hero_h - 96.0];
        let title_w = width.min(460.0);
        draw.add_rect(
            title,
            [title[0] + title_w, top + hero_h - 28.0],
            c(0, 0, 0, 104),
        )
        .filled(true)
        .rounding(16.0)
        .build();
        draw.add_text(
            [title[0] + 20.0, title[1] + 12.0],
            c(255, 255, 255, 255),
            fit_text_to_width(ui, &rental.product_name, title_w - 40.0),
        );
        let subtitle = rental_subtitle(&rental, profile);
        draw.add_text(
            [title[0] + 21.0, title[1] + 45.0],
            c(214, 221, 232, 255),
            fit_text_to_width(ui, &subtitle, title_w - 42.0),
        );

        let cards_top = top + hero_h + 18.0;
        let card_h = height - hero_h - 18.0;
        let action_min_w = 236.0;
        let detail_w = (width * 0.68)
            .max(300.0)
            .min((width - action_min_w - 18.0).max(300.0));
        draw_card_on(
            &draw,
            [left, cards_top],
            [detail_w, card_h],
            c(18, 21, 25, 218),
        );
        draw.add_text(
            [left + 20.0, cards_top + 18.0],
            c(255, 255, 255, 255),
            "Program Access",
        );
        let expires = if rental.is_permanent {
            "Permanent".to_owned()
        } else {
            short_datetime(rental.expires_at.as_deref().unwrap_or("-"))
        };
        let mut rows = vec![
            ("Product", rental.product_status_label.clone()),
            ("Status", rental.product_status.clone()),
            ("Started", short_datetime(&rental.started_at)),
            ("Updated", short_datetime(&rental.updated_at)),
            ("Expires", expires),
        ];
        if let Some(seconds) = rental.remaining_seconds {
            rows.push(("Remaining", remaining_text(seconds)));
        }
        let mut y = cards_top + 54.0;
        let row_gap = if card_h < 210.0 { 22.0 } else { 25.0 };
        let value_x = left + 134.0;
        let value_w = (detail_w - 154.0).max(80.0);
        for (key, value) in rows {
            if y + 18.0 > cards_top + card_h - 16.0 {
                break;
            }
            draw.add_text([left + 20.0, y], c(148, 156, 168, 255), key);
            draw.add_text(
                [value_x, y],
                c(232, 237, 245, 255),
                fit_text_to_width(ui, &value, value_w),
            );
            y += row_gap;
        }

        let action_x = left + detail_w + 18.0;
        let action_w = width - detail_w - 18.0;
        draw_card_on(
            &draw,
            [action_x, cards_top],
            [action_w, card_h],
            c(18, 21, 25, 218),
        );
        draw_pill(
            &draw,
            [action_x + 20.0, cards_top + 20.0],
            if is_launchable(&rental) {
                "READY"
            } else {
                "LOCKED"
            },
            if is_launchable(&rental) {
                c(75, 222, 160, 255)
            } else {
                c(245, 196, 84, 255)
            },
        );
        draw.add_text(
            [action_x + 20.0, cards_top + 66.0],
            c(210, 218, 228, 255),
            if is_launchable(&rental) {
                "Access verified."
            } else {
                "Not ready to launch."
            },
        );
        draw.add_text(
            [action_x + 20.0, cards_top + 104.0],
            c(148, 156, 168, 255),
            "Launch Steps",
        );
        let button_size = [(action_w - 40.0).max(132.0), 42.0];
        let button_y = cards_top + card_h - 60.0;
        let max_step_y = button_y - 28.0;
        let mut step_y = cards_top + 130.0;
        if profile.launch_steps.is_empty() {
            if step_y < max_step_y {
                draw.add_text(
                    [action_x + 20.0, step_y],
                    c(232, 237, 245, 255),
                    "No command configured",
                );
            }
        } else {
            for step in profile.launch_steps.iter().take(4) {
                if step_y >= max_step_y {
                    break;
                }
                draw.add_text(
                    [action_x + 20.0, step_y],
                    c(232, 237, 245, 255),
                    fit_text_to_width(ui, step.label, action_w - 40.0),
                );
                step_y += 22.0;
            }
        }
        ui.set_cursor_screen_pos([action_x + 20.0, button_y]);
        if custom_button_on(
            ui,
            &draw,
            "launch_button",
            "LAUNCH",
            button_size,
            if is_launchable(&rental) {
                c(242, 207, 119, 255)
            } else {
                c(70, 73, 78, 255)
            },
        ) {
            self.launch_selected();
        }
    }

    fn draw_account(&mut self, ui: &Ui, size: [f32; 2], offset_x: f32) {
        let base_left = SIDEBAR_W + 70.0;
        let left = base_left + offset_x;
        let top = 126.0;
        let width = (size[0] - base_left - 70.0).min(620.0);
        draw_card(ui, [left, top], [width, 330.0], c(18, 21, 25, 218));
        let draw = ui.get_window_draw_list();
        draw.add_text([left + 22.0, top + 22.0], c(255, 255, 255, 255), "Account");
        if let Some(user) = &self.user {
            let mut y = top + 64.0;
            for (key, value) in [
                ("Name", user.name.as_str()),
                ("Email", user.email.as_str()),
                ("Role", user.role.as_str()),
                ("User ID", user.id.as_str()),
                ("Expires", self.expires_at.as_deref().unwrap_or("-")),
                (
                    "HWID Lock",
                    if self.hwid_locked == Some(true) {
                        "Yes"
                    } else {
                        "No"
                    },
                ),
            ] {
                draw.add_text([left + 22.0, y], c(148, 156, 168, 255), key);
                draw.add_text([left + 150.0, y], c(232, 237, 245, 255), value);
                y += 26.0;
            }
        }
        ui.set_cursor_screen_pos([left + 22.0, top + 270.0]);
        if custom_button_on(
            ui,
            &draw,
            "logout_button",
            "Logout",
            [150.0, 38.0],
            c(242, 207, 119, 255),
        ) {
            self.start_logout();
        }
        ui.set_cursor_screen_pos([left + 184.0, top + 270.0]);
        if custom_button_on(
            ui,
            &draw,
            "refresh_button",
            "Refresh",
            [150.0, 38.0],
            c(94, 236, 210, 255),
        ) {
            self.start_me();
        }
    }

    fn draw_toasts(&mut self, ui: &Ui, size: [f32; 2]) {
        let now = Instant::now();
        self.toasts
            .retain(|toast| now.duration_since(toast.created_at) <= toast.ttl);
        let draw = ui.get_window_draw_list();
        let mut y = 58.0;
        for toast in self.toasts.iter().rev() {
            let elapsed = now.duration_since(toast.created_at).as_secs_f32();
            let ttl = toast.ttl.as_secs_f32().max(0.01);
            let progress = (1.0 - elapsed / ttl).clamp(0.0, 1.0);
            let enter = ease_out_cubic((elapsed / 0.18).clamp(0.0, 1.0));
            let exit = if progress < 0.18 {
                (progress / 0.18).clamp(0.0, 1.0)
            } else {
                1.0
            };
            let alpha = enter * exit;
            let width = (size[0] * 0.36).clamp(320.0, 382.0);
            let lines = toast_lines(&toast.text, 42, 44);
            let height = if lines.len() > 1 { 86.0 } else { 70.0 };
            let x = size[0] - width - 28.0 + (1.0 - enter) * 24.0;
            let accent = notice_color(toast.kind);

            draw.add_rect(
                [x + 4.0, y + 8.0],
                [x + width + 4.0, y + height + 8.0],
                alpha_col(c(0, 0, 0, 86), alpha),
            )
            .filled(true)
            .rounding(16.0)
            .build();
            draw.add_rect(
                [x, y],
                [x + width, y + height],
                alpha_col(c(10, 14, 19, 238), alpha),
            )
            .filled(true)
            .rounding(16.0)
            .build();
            draw.add_rect(
                [x, y],
                [x + width, y + height],
                alpha_col(accent, alpha * 0.46),
            )
            .rounding(16.0)
            .thickness(1.0)
            .build();
            draw.add_rect(
                [x + 16.0, y + 18.0],
                [x + 56.0, y + 44.0],
                alpha_col(c(0, 0, 0, 92), alpha),
            )
            .filled(true)
            .rounding(10.0)
            .build();
            draw.add_text(
                [x + 24.0, y + 22.0],
                alpha_col(accent, alpha),
                notice_label(toast.kind),
            );
            draw.add_text(
                [x + 70.0, y + 14.0],
                alpha_col(c(248, 250, 253, 255), alpha),
                notice_title(toast.kind),
            );
            if let Some(first) = lines.first() {
                draw.add_text(
                    [x + 70.0, y + 38.0],
                    alpha_col(c(196, 207, 218, 255), alpha),
                    first,
                );
            }
            if let Some(second) = lines.get(1) {
                draw.add_text(
                    [x + 70.0, y + 58.0],
                    alpha_col(c(164, 176, 190, 255), alpha),
                    second,
                );
            }
            draw.add_rect(
                [x + 16.0, y + height - 7.0],
                [x + 16.0 + (width - 32.0) * progress, y + height - 4.0],
                alpha_col(accent, alpha * 0.82),
            )
            .filled(true)
            .rounding(4.0)
            .build();
            y += height + 12.0;
        }
    }

    fn draw_page_transition(&self, ui: &Ui, size: [f32; 2]) {
        let progress = self.page_progress();
        if progress >= 0.995 {
            return;
        }

        let draw = ui.get_window_draw_list();
        let fade = 1.0 - progress;
        let left = SIDEBAR_W + 1.0;
        draw.add_rect(
            [left, 0.0],
            [size[0], size[1]],
            alpha_col(c(0, 0, 0, 118), fade * 0.42),
        )
        .filled(true)
        .rounding(28.0)
        .build();

        let sweep_x = left + progress * (size[0] - left);
        draw.add_rect(
            [sweep_x - 72.0, 36.0],
            [sweep_x + 18.0, size[1] - 36.0],
            alpha_col(c(120, 255, 232, 36), fade),
        )
        .filled(true)
        .rounding(26.0)
        .build();
    }

    fn draw_busy(&self, ui: &Ui, size: [f32; 2]) {
        if let Some(label) = &self.busy_label {
            let draw = ui.get_window_draw_list();
            let center = [size[0] * 0.5, size[1] * 0.5];
            draw.add_circle(center, 28.0, c(242, 207, 119, 190))
                .num_segments(56)
                .thickness(3.0)
                .build();
            draw.add_text(
                [center[0] - 58.0, center[1] + 42.0],
                c(230, 234, 239, 255),
                label,
            );
        }
    }
}

fn draw_card(ui: &Ui, pos: [f32; 2], size: [f32; 2], bg: Col) {
    let draw = ui.get_window_draw_list();
    draw_card_on(&draw, pos, size, bg);
}

fn draw_card_on(draw: &DrawListMut<'_>, pos: [f32; 2], size: [f32; 2], bg: Col) {
    draw.add_rect(pos, [pos[0] + size[0], pos[1] + size[1]], bg)
        .filled(true)
        .rounding(16.0)
        .build();
    draw.add_rect(
        pos,
        [pos[0] + size[0], pos[1] + size[1]],
        c(220, 226, 232, 70),
    )
    .rounding(16.0)
    .thickness(1.0)
    .build();
}

fn custom_button_on(
    ui: &Ui,
    draw: &DrawListMut<'_>,
    id: &str,
    label: &str,
    size: [f32; 2],
    fill: Col,
) -> bool {
    let clicked = ui.invisible_button(id, size);
    let min = ui.item_rect_min();
    let max = ui.item_rect_max();
    let hovered = ui.is_item_hovered();
    let color = if hovered {
        mix_col(fill, c(255, 255, 255, 255), 0.10)
    } else {
        fill
    };
    draw.add_rect(min, max, color)
        .filled(true)
        .rounding(14.0)
        .build();
    let text_size = ui.calc_text_size(label);
    draw.add_text(
        [
            min[0] + (size[0] - text_size[0]) * 0.5,
            min[1] + (size[1] - text_size[1]) * 0.5,
        ],
        c(14, 15, 17, 255),
        label,
    );
    clicked
}

fn window_control_button(
    ui: &Ui,
    draw: &DrawListMut<'_>,
    id: &str,
    center: [f32; 2],
    close: bool,
) -> bool {
    ui.set_cursor_screen_pos([center[0] - 14.0, center[1] - 14.0]);
    let clicked = ui.invisible_button(id, [28.0, 28.0]);
    let hovered = ui.is_item_hovered();
    let bg = if close && hovered {
        c(255, 92, 116, 232)
    } else if hovered {
        c(255, 255, 255, 34)
    } else {
        c(0, 0, 0, 74)
    };
    let icon = if close && hovered {
        c(14, 15, 18, 255)
    } else {
        c(232, 238, 245, 245)
    };

    draw.add_circle(center, 14.0, bg)
        .filled(true)
        .num_segments(32)
        .build();
    if close {
        draw.add_line(
            [center[0] - 5.0, center[1] - 5.0],
            [center[0] + 5.0, center[1] + 5.0],
            icon,
        )
        .thickness(2.0)
        .build();
        draw.add_line(
            [center[0] + 5.0, center[1] - 5.0],
            [center[0] - 5.0, center[1] + 5.0],
            icon,
        )
        .thickness(2.0)
        .build();
    } else {
        draw.add_line(
            [center[0] - 6.0, center[1]],
            [center[0] + 6.0, center[1]],
            icon,
        )
        .thickness(2.0)
        .build();
    }
    clicked
}

fn launch_profile(profile: &ProgramProfile) -> Result<usize, String> {
    let mut launched = 0;
    for step in profile.launch_steps {
        if step.command.trim().is_empty() {
            continue;
        }

        Command::new(step.command)
            .args(step.args)
            .spawn()
            .map_err(|error| format!("{} ({})", step.label, error))?;
        launched += 1;
    }

    Ok(launched)
}

fn draw_cover(
    draw: &DrawListMut<'_>,
    pos: [f32; 2],
    size: [f32; 2],
    seed: u32,
    wide: bool,
    art: CoverArt,
    texture: Option<CoverTexture>,
) {
    let palette = palette_for_profile(seed as usize, "cover", art);
    let end = [pos[0] + size[0], pos[1] + size[1]];
    if let Some(texture) = texture {
        let (uv_min, uv_max) = cover_uv(texture.size, size);
        draw.add_image_rounded(texture.id, pos, end, 26.0)
            .uv_min(uv_min)
            .uv_max(uv_max)
            .build();
        draw.add_rect(pos, end, c(0, 0, 0, 44))
            .filled(true)
            .rounding(26.0)
            .build();
        return;
    }

    draw.add_rect(pos, end, palette.tl)
        .filled(true)
        .rounding(26.0)
        .build();
    draw.add_rect(
        [pos[0] + size[0] * 0.38, pos[1]],
        end,
        alpha_col(palette.tr, 0.86),
    )
    .filled(true)
    .rounding(26.0)
    .build();
    draw.add_rect(
        [pos[0], pos[1] + size[1] * 0.56],
        end,
        alpha_col(palette.bl, 0.28),
    )
    .filled(true)
    .rounding(26.0)
    .build();
    draw.add_rect(
        [pos[0] + size[0] * 0.50, pos[1] + size[1] * 0.48],
        end,
        alpha_col(palette.br, 0.72),
    )
    .filled(true)
    .rounding(26.0)
    .build();
    draw.add_rect([pos[0], pos[1] + size[1] * 0.60], end, c(0, 0, 0, 64))
        .filled(true)
        .rounding(26.0)
        .build();
    draw.add_circle(
        [pos[0] + size[0] * 0.22, pos[1] + size[1] * 0.22],
        size[1] * 0.68,
        alpha_col(palette.accent, 0.10),
    )
    .filled(true)
    .num_segments(48)
    .build();
    for i in 0..5 {
        let x = pos[0] + 24.0 + size[0] * (0.02 + 0.17 * i as f32);
        draw.add_polyline(
            vec![
                [x + size[0] * 0.14, pos[1] + 12.0],
                [x + size[0] * 0.24, pos[1] + 12.0],
                [x + size[0] * 0.01, end[1] - 12.0],
                [x - size[0] * 0.09, end[1] - 12.0],
            ],
            c(255, 255, 255, 14),
        )
        .filled(true)
        .build();
    }
    let count = if wide { 4 } else { 2 };
    for i in 0..count {
        let sx = pos[0]
            + size[0]
                * if wide {
                    0.48 + 0.14 * i as f32
                } else {
                    0.62 + 0.18 * i as f32
                };
        let base = pos[1] + size[1] * if wide { 0.76 } else { 0.72 };
        let scale = if wide { 0.82 - 0.04 * i as f32 } else { 0.68 };
        draw.add_circle(
            [sx, base - 54.0 * scale],
            21.0 * scale,
            c(86, 102, 118, 112),
        )
        .filled(true)
        .num_segments(30)
        .build();
        draw.add_rect(
            [sx - 28.0 * scale, base - 32.0 * scale],
            [sx + 30.0 * scale, base + 56.0 * scale],
            c(86, 102, 118, 112),
        )
        .filled(true)
        .rounding(9.0)
        .build();
        draw.add_line(
            [sx - 48.0 * scale, base - 6.0 * scale],
            [sx + 52.0 * scale, base - 26.0 * scale],
            c(8, 13, 18, 152),
        )
        .thickness(6.0 * scale)
        .build();
    }
}

#[derive(Clone, Copy)]
struct Palette {
    tl: Col,
    tr: Col,
    bl: Col,
    br: Col,
    border: Col,
    accent: Col,
}

fn palette_for_profile(index: usize, name: &str, art: CoverArt) -> Palette {
    match art {
        CoverArt::TacticalBlue => Palette {
            tl: c(3, 82, 166, 255),
            tr: c(8, 18, 32, 255),
            bl: c(0, 196, 223, 255),
            br: c(5, 8, 16, 255),
            border: c(117, 236, 255, 255),
            accent: c(118, 245, 216, 255),
        },
        CoverArt::MiningPink => Palette {
            tl: c(0, 130, 150, 255),
            tr: c(167, 28, 84, 255),
            bl: c(5, 54, 70, 255),
            br: c(9, 10, 17, 255),
            border: c(255, 175, 208, 255),
            accent: c(255, 124, 174, 255),
        },
        CoverArt::CompetitiveCyan => Palette {
            tl: c(4, 159, 184, 255),
            tr: c(22, 44, 62, 255),
            bl: c(2, 88, 114, 255),
            br: c(7, 10, 15, 255),
            border: c(203, 238, 255, 255),
            accent: c(141, 225, 255, 255),
        },
        CoverArt::StealthGreen => Palette {
            tl: c(24, 150, 95, 255),
            tr: c(15, 47, 58, 255),
            bl: c(6, 88, 66, 255),
            br: c(6, 10, 14, 255),
            border: c(170, 255, 220, 255),
            accent: c(92, 255, 183, 255),
        },
        CoverArt::AmberOps => Palette {
            tl: c(177, 99, 21, 255),
            tr: c(44, 27, 16, 255),
            bl: c(207, 64, 43, 255),
            br: c(9, 10, 15, 255),
            border: c(255, 222, 147, 255),
            accent: c(255, 196, 87, 255),
        },
        CoverArt::Default => palette_for_seed(index, name),
    }
}

fn palette_for_seed(index: usize, name: &str) -> Palette {
    let seed = hash_text(name).wrapping_add(index as u32 * 97) % 6;
    match seed {
        0 => Palette {
            tl: c(8, 109, 191, 255),
            tr: c(12, 22, 38, 255),
            bl: c(0, 185, 171, 255),
            br: c(5, 8, 16, 255),
            border: c(117, 236, 255, 255),
            accent: c(118, 245, 216, 255),
        },
        1 => Palette {
            tl: c(0, 147, 171, 255),
            tr: c(148, 36, 87, 255),
            bl: c(8, 42, 63, 255),
            br: c(8, 10, 17, 255),
            border: c(255, 175, 208, 255),
            accent: c(255, 124, 174, 255),
        },
        2 => Palette {
            tl: c(36, 160, 113, 255),
            tr: c(20, 64, 88, 255),
            bl: c(6, 92, 115, 255),
            br: c(7, 10, 16, 255),
            border: c(170, 255, 220, 255),
            accent: c(92, 255, 183, 255),
        },
        3 => Palette {
            tl: c(177, 99, 21, 255),
            tr: c(44, 27, 16, 255),
            bl: c(207, 64, 43, 255),
            br: c(9, 10, 15, 255),
            border: c(255, 222, 147, 255),
            accent: c(255, 196, 87, 255),
        },
        4 => Palette {
            tl: c(63, 111, 229, 255),
            tr: c(37, 40, 78, 255),
            bl: c(0, 189, 202, 255),
            br: c(8, 11, 18, 255),
            border: c(203, 220, 255, 255),
            accent: c(141, 199, 255, 255),
        },
        _ => Palette {
            tl: c(80, 98, 122, 255),
            tr: c(18, 25, 37, 255),
            bl: c(23, 159, 174, 255),
            br: c(7, 10, 15, 255),
            border: c(232, 240, 255, 255),
            accent: c(220, 235, 255, 255),
        },
    }
}

fn draw_pill(draw: &DrawListMut<'_>, pos: [f32; 2], text: &str, fg: Col) {
    let width = 22.0 + text.len() as f32 * 7.0;
    draw.add_rect(pos, [pos[0] + width, pos[1] + 28.0], c(0, 0, 0, 100))
        .filled(true)
        .rounding(10.0)
        .build();
    draw.add_text([pos[0] + 11.0, pos[1] + 6.0], fg, text);
}

fn draw_back_icon(draw: &DrawListMut<'_>, center: [f32; 2], color: Col, scale: f32) {
    draw.add_line(
        [center[0] + 7.0 * scale, center[1] - 11.0 * scale],
        [center[0] - 7.0 * scale, center[1]],
        color,
    )
    .thickness(2.4 * scale)
    .build();
    draw.add_line(
        [center[0] - 7.0 * scale, center[1]],
        [center[0] + 7.0 * scale, center[1] + 11.0 * scale],
        color,
    )
    .thickness(2.4 * scale)
    .build();
}

fn draw_gamepad_icon(draw: &DrawListMut<'_>, center: [f32; 2], color: Col, scale: f32) {
    draw.add_rect(
        [center[0] - 18.0 * scale, center[1] - 10.0 * scale],
        [center[0] + 18.0 * scale, center[1] + 11.0 * scale],
        color,
    )
    .rounding(8.0 * scale)
    .thickness(2.2 * scale)
    .build();
    draw.add_line(
        [center[0] - 10.0 * scale, center[1]],
        [center[0] - 2.0 * scale, center[1]],
        color,
    )
    .thickness(2.2 * scale)
    .build();
    draw.add_line(
        [center[0] - 6.0 * scale, center[1] - 4.0 * scale],
        [center[0] - 6.0 * scale, center[1] + 4.0 * scale],
        color,
    )
    .thickness(2.2 * scale)
    .build();
    draw.add_circle(
        [center[0] + 7.0 * scale, center[1] - 2.0 * scale],
        2.2 * scale,
        color,
    )
    .filled(true)
    .build();
    draw.add_circle(
        [center[0] + 13.0 * scale, center[1] + 3.0 * scale],
        2.2 * scale,
        color,
    )
    .filled(true)
    .build();
}

fn draw_user_icon(draw: &DrawListMut<'_>, center: [f32; 2], color: Col, scale: f32) {
    draw.add_circle([center[0], center[1] - 8.0 * scale], 7.0 * scale, color)
        .num_segments(24)
        .thickness(2.2 * scale)
        .build();
    draw.add_bezier_curve(
        [center[0] - 16.0 * scale, center[1] + 14.0 * scale],
        [center[0] - 12.0 * scale, center[1] + 2.0 * scale],
        [center[0] + 12.0 * scale, center[1] + 2.0 * scale],
        [center[0] + 16.0 * scale, center[1] + 14.0 * scale],
        color,
    )
    .thickness(2.4 * scale)
    .build();
}

fn draw_product_icon(
    draw: &DrawListMut<'_>,
    center: [f32; 2],
    radius: f32,
    accent: Col,
    bg: Col,
    icon: ProgramIcon,
) {
    draw.add_circle(center, radius, bg)
        .filled(true)
        .num_segments(48)
        .build();
    draw.add_circle(center, radius, c(255, 255, 255, 42))
        .num_segments(48)
        .build();
    match icon {
        ProgramIcon::Gamepad => draw_gamepad_icon(draw, center, accent, radius / 30.0),
        ProgramIcon::Shield => {
            let pts = vec![
                [center[0], center[1] - 17.0 * radius / 30.0],
                [
                    center[0] + 15.0 * radius / 30.0,
                    center[1] - 8.0 * radius / 30.0,
                ],
                [
                    center[0] + 10.0 * radius / 30.0,
                    center[1] + 12.0 * radius / 30.0,
                ],
                [center[0], center[1] + 19.0 * radius / 30.0],
                [
                    center[0] - 10.0 * radius / 30.0,
                    center[1] + 12.0 * radius / 30.0,
                ],
                [
                    center[0] - 15.0 * radius / 30.0,
                    center[1] - 8.0 * radius / 30.0,
                ],
            ];
            draw.add_polyline(pts, accent)
                .thickness(2.2 * radius / 30.0)
                .build();
        }
        ProgramIcon::Radar => {
            draw.add_circle(center, 17.0 * radius / 30.0, accent)
                .num_segments(36)
                .thickness(2.0)
                .build();
            draw.add_circle(center, 7.0 * radius / 30.0, accent)
                .num_segments(28)
                .thickness(2.0)
                .build();
        }
        ProgramIcon::Bolt => {
            draw.add_triangle(
                [center[0] + 2.0, center[1] - 18.0],
                [center[0] - 8.0, center[1] + 4.0],
                [center[0] + 12.0, center[1] - 4.0],
                accent,
            )
            .thickness(2.2)
            .build();
        }
    }
}

fn rental_kind(rental: &Rental) -> NoticeKind {
    match rental.rental_status.as_str() {
        "ACTIVE" | "PERMANENT" => NoticeKind::Success,
        "EXPIRED" => NoticeKind::Error,
        _ => NoticeKind::Warning,
    }
}

fn notice_color(kind: NoticeKind) -> Col {
    match kind {
        NoticeKind::Success => c(75, 222, 160, 255),
        NoticeKind::Warning => c(245, 196, 84, 255),
        NoticeKind::Error => c(250, 104, 132, 255),
        NoticeKind::Info => c(125, 211, 252, 255),
    }
}

fn notice_label(kind: NoticeKind) -> &'static str {
    match kind {
        NoticeKind::Success => "OK",
        NoticeKind::Warning => "WARN",
        NoticeKind::Error => "ERR",
        NoticeKind::Info => "INFO",
    }
}

fn notice_title(kind: NoticeKind) -> &'static str {
    match kind {
        NoticeKind::Success => "Success",
        NoticeKind::Warning => "Warning",
        NoticeKind::Error => "Error",
        NoticeKind::Info => "Info",
    }
}

fn compact_toast_text(text: &str, max_chars: usize) -> String {
    let mut chars = text.trim().chars();
    let mut output = String::new();
    for _ in 0..max_chars {
        let Some(ch) = chars.next() else {
            return output;
        };
        output.push(if ch == '\n' || ch == '\r' { ' ' } else { ch });
    }
    if chars.next().is_some() {
        output.push_str("...");
    }
    output
}

fn compact_label_text(text: &str, max_chars: usize) -> String {
    let mut chars = text.trim().chars();
    let mut output = String::new();
    for _ in 0..max_chars {
        let Some(ch) = chars.next() else {
            return output;
        };
        output.push(if ch == '\n' || ch == '\r' { ' ' } else { ch });
    }
    if chars.next().is_some() {
        output.push_str("...");
    }
    output
}

fn fit_text_to_width(ui: &Ui, text: &str, max_width: f32) -> String {
    let cleaned = text.trim().replace(['\n', '\r'], " ");
    if ui.calc_text_size(&cleaned)[0] <= max_width {
        return cleaned;
    }

    let suffix = "...";
    let suffix_w = ui.calc_text_size(suffix)[0];
    let mut output = String::new();
    for ch in cleaned.chars() {
        let candidate = format!("{output}{ch}");
        if ui.calc_text_size(&candidate)[0] + suffix_w > max_width {
            break;
        }
        output.push(ch);
    }
    if output.is_empty() {
        suffix.to_owned()
    } else {
        output.push_str(suffix);
        output
    }
}

fn short_datetime(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "-" {
        return "-".to_owned();
    }

    let without_ms = trimmed
        .split('.')
        .next()
        .unwrap_or(trimmed)
        .trim_end_matches('Z')
        .replace('T', " ");
    compact_label_text(&without_ms, 16)
}

fn toast_lines(text: &str, first_max: usize, second_max: usize) -> Vec<String> {
    let compact = compact_toast_text(text, first_max + second_max + 1);
    let mut chars = compact.chars();
    let mut first = String::new();
    for _ in 0..first_max {
        let Some(ch) = chars.next() else {
            return vec![first];
        };
        first.push(ch);
    }
    let rest: String = chars.collect::<String>().trim().to_owned();
    if rest.is_empty() {
        vec![first]
    } else {
        vec![first, compact_label_text(&rest, second_max)]
    }
}

fn page_rank(page: Page) -> i32 {
    match page {
        Page::Home => 0,
        Page::Detail => 1,
        Page::Account => 2,
    }
}

fn ease_out_cubic(t: f32) -> f32 {
    let inv = 1.0 - t.clamp(0.0, 1.0);
    1.0 - inv * inv * inv
}

fn alpha_col(mut color: Col, alpha: f32) -> Col {
    color[3] *= alpha.clamp(0.0, 1.0);
    color
}

fn cover_uv(image_size: [f32; 2], target_size: [f32; 2]) -> ([f32; 2], [f32; 2]) {
    let image_aspect = (image_size[0] / image_size[1]).max(0.01);
    let target_aspect = (target_size[0] / target_size[1]).max(0.01);
    if image_aspect > target_aspect {
        let visible = target_aspect / image_aspect;
        let pad = (1.0 - visible) * 0.5;
        ([pad, 0.0], [1.0 - pad, 1.0])
    } else {
        let visible = image_aspect / target_aspect;
        let pad = (1.0 - visible) * 0.44;
        ([0.0, pad], [1.0, (pad + visible).min(1.0)])
    }
}

fn is_launchable(rental: &Rental) -> bool {
    matches!(rental.rental_status.as_str(), "ACTIVE" | "PERMANENT")
        && rental.product_status == "AVAILABLE"
}

fn rental_subtitle(rental: &Rental, profile: &ProgramProfile) -> String {
    if !profile.subtitle.is_empty() {
        profile.subtitle.to_owned()
    } else if rental.is_permanent || rental.rental_status == "PERMANENT" {
        "Permanent access".to_owned()
    } else if let Some(seconds) = rental.remaining_seconds {
        remaining_text(seconds)
    } else if !rental.product_status_label.is_empty() {
        rental.product_status_label.clone()
    } else {
        "Program access".to_owned()
    }
}

fn remaining_text(seconds: i64) -> String {
    if seconds <= 0 {
        return "Expired".to_owned();
    }
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;
    if days > 0 {
        format!("{days}d {hours}h left")
    } else if hours > 0 {
        format!("{hours}h {minutes}m left")
    } else {
        format!("{minutes}m left")
    }
}

fn hash_text(text: &str) -> u32 {
    text.bytes().fold(2166136261u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(16777619)
    })
}

fn mix_col(a: Col, b: Col, t: f32) -> Col {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
        a[3] + (b[3] - a[3]) * t,
    ]
}
