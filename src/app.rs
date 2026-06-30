use crate::{
    api::{self, ApiError},
    config::AppConfig,
    hwid::stable_hwid,
    models::{LoginRequest, LoginResponse, MeResponse, Rental, RentalsResponse, UserInfo},
    ui_helpers::install_thai_font,
};
use eframe::egui::{
    self, Align, Align2, Color32, Layout, Margin, RichText, ScrollArea, Stroke, TextEdit, Vec2,
};
use std::{
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

pub struct ProgramLoginApp {
    config: AppConfig,
    username: String,
    password: String,
    hwid: String,
    token: Option<String>,
    user: Option<UserInfo>,
    expires_at: Option<String>,
    hwid_locked: Option<bool>,
    rentals: Vec<Rental>,
    selected_product_id: Option<String>,
    busy_label: Option<String>,
    status: StatusMessage,
    toasts: Vec<Toast>,
    sender: Sender<AppMessage>,
    receiver: Receiver<AppMessage>,
}

enum AppMessage {
    Login(Result<LoginResponse, ApiError>),
    Me(Result<MeResponse, ApiError>),
    Rentals(Result<RentalsResponse, ApiError>),
    Logout(Result<(), ApiError>),
}

#[derive(Clone)]
struct StatusMessage {
    text: String,
    kind: StatusKind,
}

#[derive(Clone)]
struct Toast {
    text: String,
    kind: StatusKind,
    created_at: Instant,
    ttl: Duration,
}

#[derive(Clone, Copy)]
enum StatusKind {
    Info,
    Success,
    Warning,
    Error,
}

impl ProgramLoginApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        install_thai_font(&cc.egui_ctx);

        let (sender, receiver) = mpsc::channel();

        Self {
            config: AppConfig::default(),
            username: String::new(),
            password: String::new(),
            hwid: stable_hwid(),
            token: None,
            user: None,
            expires_at: None,
            hwid_locked: None,
            rentals: Vec::new(),
            selected_product_id: None,
            busy_label: None,
            status: StatusMessage {
                text: "Ready.".to_owned(),
                kind: StatusKind::Info,
            },
            toasts: Vec::new(),
            sender,
            receiver,
        }
    }

    fn is_busy(&self) -> bool {
        self.busy_label.is_some()
    }

    fn poll_messages(&mut self, ctx: &egui::Context) {
        while let Ok(message) = self.receiver.try_recv() {
            self.busy_label = None;

            match message {
                AppMessage::Login(result) => match result {
                    Ok(response) => {
                        self.token = Some(response.token);
                        self.password.clear();
                        self.set_status("Login success. Syncing account...", StatusKind::Success);
                        self.push_toast("Login success.", StatusKind::Success);
                        self.start_me_request(ctx.clone());
                    }
                    Err(error) => {
                        self.clear_account();
                        self.set_api_error("Login failed", error);
                    }
                },
                AppMessage::Me(result) => match result {
                    Ok(response) => {
                        self.user = Some(response.user);
                        self.expires_at = Some(response.session.expires_at);
                        self.hwid_locked = Some(response.session.hwid_locked);
                        self.set_status("Account synced.", StatusKind::Success);
                        self.start_rentals_request(ctx.clone());
                    }
                    Err(error) => {
                        self.clear_account();
                        self.set_api_error("Account check failed", error);
                    }
                },
                AppMessage::Rentals(result) => match result {
                    Ok(response) => {
                        let count = response.rentals.len();
                        self.rentals = response.rentals;
                        self.ensure_selected_program();
                        self.set_status(format!("Loaded {count} program(s)."), StatusKind::Success);
                        self.push_toast("Program list updated.", StatusKind::Success);
                    }
                    Err(error) => {
                        if error.status == Some(401) {
                            self.clear_account();
                        }
                        self.set_api_error("Program loading failed", error);
                    }
                },
                AppMessage::Logout(result) => match result {
                    Ok(()) => {
                        self.clear_account();
                        self.set_status("Logged out.", StatusKind::Info);
                        self.push_toast("Logged out.", StatusKind::Info);
                    }
                    Err(error) => {
                        self.clear_account();
                        self.set_logout_warning(error);
                    }
                },
            }
        }
    }

    fn set_status(&mut self, text: impl Into<String>, kind: StatusKind) {
        self.status = StatusMessage {
            text: text.into(),
            kind,
        };
    }

    fn set_api_error(&mut self, prefix: &str, error: ApiError) {
        let status = error
            .status
            .map(|status| format!(" ({status})"))
            .unwrap_or_default();
        let text = format!("{prefix}{status}: {}", error.message);

        self.set_status(text.clone(), StatusKind::Error);
        self.push_toast(text, StatusKind::Error);
    }

    fn set_logout_warning(&mut self, error: ApiError) {
        let status = error
            .status
            .map(|status| format!(" ({status})"))
            .unwrap_or_default();
        let text = format!(
            "Logged out locally. Server logout failed{status}: {}",
            error.message
        );

        self.set_status(text.clone(), StatusKind::Warning);
        self.push_toast(text, StatusKind::Warning);
    }

    fn push_toast(&mut self, text: impl Into<String>, kind: StatusKind) {
        self.toasts.push(Toast {
            text: text.into(),
            kind,
            created_at: Instant::now(),
            ttl: Duration::from_secs(4),
        });

        if self.toasts.len() > 5 {
            self.toasts.remove(0);
        }
    }

    fn clear_account(&mut self) {
        self.token = None;
        self.user = None;
        self.expires_at = None;
        self.hwid_locked = None;
        self.rentals.clear();
        self.selected_product_id = None;
        self.busy_label = None;
    }

    fn can_submit_login(&self) -> bool {
        !self.is_busy() && !self.username.trim().is_empty() && !self.password.is_empty()
    }

    fn start_login_request(&mut self, ctx: egui::Context) {
        if !self.can_submit_login() {
            self.set_status("Enter username and password.", StatusKind::Warning);
            self.push_toast("Enter username and password.", StatusKind::Warning);
            return;
        }

        self.clear_account();
        self.busy_label = Some("Logging in...".to_owned());
        self.set_status("Sending login request...", StatusKind::Info);

        let sender = self.sender.clone();
        let base_url = self.config.api_base_url();
        let request = LoginRequest {
            user: self.username.trim().to_owned(),
            password: self.password.clone(),
            hwid: self.hwid.clone(),
        };

        thread::spawn(move || {
            let result = api::login(&base_url, request);
            let _ = sender.send(AppMessage::Login(result));
            ctx.request_repaint();
        });
    }

    fn start_me_request(&mut self, ctx: egui::Context) {
        let Some(token) = self.token.clone() else {
            self.set_status("Please login first.", StatusKind::Warning);
            return;
        };

        self.busy_label = Some("Syncing account...".to_owned());
        self.set_status("Calling /me...", StatusKind::Info);

        let sender = self.sender.clone();
        let base_url = self.config.api_base_url();

        thread::spawn(move || {
            let result = api::me(&base_url, &token);
            let _ = sender.send(AppMessage::Me(result));
            ctx.request_repaint();
        });
    }

    fn start_rentals_request(&mut self, ctx: egui::Context) {
        let Some(token) = self.token.clone() else {
            self.set_status("Please login first.", StatusKind::Warning);
            return;
        };

        self.busy_label = Some("Loading programs...".to_owned());
        self.set_status("Calling /rentals...", StatusKind::Info);

        let sender = self.sender.clone();
        let base_url = self.config.api_base_url();

        thread::spawn(move || {
            let result = api::rentals(&base_url, &token);
            let _ = sender.send(AppMessage::Rentals(result));
            ctx.request_repaint();
        });
    }

    fn start_logout_request(&mut self, ctx: egui::Context) {
        let Some(token) = self.token.clone() else {
            self.clear_account();
            self.set_status("Logged out.", StatusKind::Info);
            return;
        };

        self.busy_label = Some("Logging out...".to_owned());
        self.set_status("Revoking token...", StatusKind::Info);

        let sender = self.sender.clone();
        let base_url = self.config.api_base_url();

        thread::spawn(move || {
            let result = api::logout(&base_url, &token);
            let _ = sender.send(AppMessage::Logout(result));
            ctx.request_repaint();
        });
    }

    fn ensure_selected_program(&mut self) {
        let selected_exists = self
            .selected_product_id
            .as_ref()
            .is_some_and(|id| self.rentals.iter().any(|rental| rental.product_id == *id));

        if selected_exists {
            return;
        }

        self.selected_product_id = self
            .rentals
            .iter()
            .find(|rental| is_launchable(rental))
            .or_else(|| self.rentals.first())
            .map(|rental| rental.product_id.clone());
    }

    fn selected_rental(&self) -> Option<Rental> {
        let selected_id = self.selected_product_id.as_ref()?;
        self.rentals
            .iter()
            .find(|rental| rental.product_id == *selected_id)
            .cloned()
    }

    fn launch_selected_program(&mut self) {
        let Some(rental) = self.selected_rental() else {
            self.set_status("Select a program first.", StatusKind::Warning);
            self.push_toast("Select a program first.", StatusKind::Warning);
            return;
        };

        if is_launchable(&rental) {
            let text = format!("Access verified: {}", rental.product_name);
            self.set_status(text.clone(), StatusKind::Success);
            self.push_toast(text, StatusKind::Success);
        } else {
            let text = format!("Access denied: {}", rental.product_name);
            self.set_status(text.clone(), StatusKind::Warning);
            self.push_toast(text, StatusKind::Warning);
        }
    }

    fn ui_login_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, time: f64) {
        let pulse = pulse(time);

        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.12).min(72.0));
            ui.label(
                RichText::new("NOVA LOADER")
                    .size(30.0)
                    .strong()
                    .color(Color32::WHITE),
            );
            ui.label(RichText::new("Secure program access").color(mix_color(
                accent_color(),
                Color32::LIGHT_GRAY,
                pulse,
            )));
            ui.add_space(22.0);

            ui.set_width(ui.available_width().min(430.0));
            app_card(ui, |ui| {
                ui.heading("Login");
                ui.add_space(12.0);

                egui::Grid::new("login_grid")
                    .num_columns(2)
                    .spacing([14.0, 12.0])
                    .show(ui, |ui| {
                        ui.label("Username");
                        ui.add_enabled(
                            !self.is_busy(),
                            TextEdit::singleline(&mut self.username)
                                .desired_width(f32::INFINITY)
                                .hint_text("username"),
                        );
                        ui.end_row();

                        ui.label("Password");
                        let password_response = ui.add_enabled(
                            !self.is_busy(),
                            TextEdit::singleline(&mut self.password)
                                .desired_width(f32::INFINITY)
                                .password(true)
                                .hint_text("password"),
                        );
                        ui.end_row();

                        if password_response.lost_focus()
                            && ui.input(|input| input.key_pressed(egui::Key::Enter))
                        {
                            self.start_login_request(ctx.clone());
                        }
                    });

                ui.add_space(16.0);
                let button_text = self.busy_label.as_deref().unwrap_or("Login");
                if ui
                    .add_enabled(
                        self.can_submit_login(),
                        egui::Button::new(RichText::new(button_text).strong())
                            .fill(accent_color())
                            .min_size(Vec2::new(ui.available_width(), 42.0)),
                    )
                    .clicked()
                {
                    self.start_login_request(ctx.clone());
                }
            });

            self.ui_status(ui);
        });
    }

    fn ui_dashboard(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        self.ui_top_bar(ui, ctx);
        ui.add_space(12.0);

        let wide = ui.available_width() >= 780.0;

        if wide {
            ui.horizontal_top(|ui| {
                ui.set_height(ui.available_height());
                ui.vertical(|ui| {
                    ui.set_width(300.0);
                    self.ui_account_card(ui);
                    ui.add_space(12.0);
                    self.ui_session_card(ui);
                });

                ui.add_space(12.0);

                ui.vertical(|ui| {
                    ui.set_width(ui.available_width());
                    self.ui_programs_card(ui, ctx);
                    ui.add_space(12.0);
                    self.ui_selected_program_card(ui);
                });
            });
        } else {
            self.ui_account_card(ui);
            ui.add_space(12.0);
            self.ui_session_card(ui);
            ui.add_space(12.0);
            self.ui_programs_card(ui, ctx);
            ui.add_space(12.0);
            self.ui_selected_program_card(ui);
        }

        self.ui_status(ui);
    }

    fn ui_top_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        app_card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("NOVA LOADER").size(22.0).strong());
                status_pill(ui, "ONLINE", StatusKind::Success);

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add_enabled(!self.is_busy(), egui::Button::new("Logout"))
                        .clicked()
                    {
                        self.start_logout_request(ctx.clone());
                    }

                    if ui
                        .add_enabled(!self.is_busy(), egui::Button::new("Refresh"))
                        .clicked()
                    {
                        self.start_me_request(ctx.clone());
                    }
                });
            });
        });
    }

    fn ui_account_card(&self, ui: &mut egui::Ui) {
        let Some(user) = &self.user else {
            return;
        };

        app_card(ui, |ui| {
            ui.heading("Account");
            ui.add_space(10.0);
            key_value(ui, "Name", &user.name);
            key_value(ui, "Email", &user.email);
            key_value(ui, "Role", &user.role);
            key_value(ui, "User ID", &user.id);
        });
    }

    fn ui_session_card(&self, ui: &mut egui::Ui) {
        app_card(ui, |ui| {
            ui.heading("Session");
            ui.add_space(10.0);

            if let Some(expires_at) = &self.expires_at {
                key_value(ui, "Expires", expires_at);
            }

            let hwid_locked = match self.hwid_locked {
                Some(true) => "Yes",
                Some(false) => "No",
                None => "-",
            };
            key_value(ui, "HWID Lock", hwid_locked);
            key_value(ui, "Host", &self.config.host);
        });
    }

    fn ui_programs_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        app_card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Programs");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add_enabled(!self.is_busy(), egui::Button::new("Reload"))
                        .clicked()
                    {
                        self.start_rentals_request(ctx.clone());
                    }
                });
            });
            ui.add_space(10.0);

            if self.rentals.is_empty() {
                if self.is_busy() {
                    ui_loading_rows(ui);
                } else {
                    ui.label(RichText::new("No programs found.").color(Color32::LIGHT_GRAY));
                }
                return;
            }

            let rentals = self.rentals.clone();
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .max_height(260.0)
                .show(ui, |ui| {
                    for rental in rentals {
                        self.ui_program_row(ui, &rental);
                        ui.add_space(8.0);
                    }
                });
        });
    }

    fn ui_program_row(&mut self, ui: &mut egui::Ui, rental: &Rental) {
        let selected = self
            .selected_product_id
            .as_ref()
            .is_some_and(|id| id == &rental.product_id);

        let response = ui.selectable_label(
            selected,
            RichText::new(&rental.product_name)
                .strong()
                .color(if selected {
                    Color32::WHITE
                } else {
                    Color32::LIGHT_GRAY
                }),
        );

        if response.clicked() {
            self.selected_product_id = Some(rental.product_id.clone());
        }

        ui.horizontal_wrapped(|ui| {
            status_pill(ui, &rental.rental_status, status_kind_for_rental(rental));
            ui.label(RichText::new(&rental.product_status_label).color(Color32::LIGHT_GRAY));
            if let Some(seconds) = rental.remaining_seconds {
                ui.label(RichText::new(remaining_text(seconds)).color(Color32::LIGHT_GRAY));
            }
        });
    }

    fn ui_selected_program_card(&mut self, ui: &mut egui::Ui) {
        let selected = self.selected_rental();

        app_card(ui, |ui| {
            ui.heading("Selected Program");
            ui.add_space(10.0);

            let Some(rental) = selected else {
                ui.label(RichText::new("Select a program.").color(Color32::LIGHT_GRAY));
                return;
            };

            ui.horizontal(|ui| {
                ui.label(RichText::new(&rental.product_name).size(20.0).strong());
                status_pill(ui, &rental.rental_status, status_kind_for_rental(&rental));
            });

            ui.add_space(8.0);
            key_value(ui, "Product", &rental.product_status_label);
            key_value(ui, "Rental", &rental.rental_status);
            key_value(ui, "Started", &rental.started_at);
            key_value(ui, "Updated", &rental.updated_at);

            if rental.is_permanent {
                key_value(ui, "Expires", "Permanent");
            } else if let Some(expires_at) = &rental.expires_at {
                key_value(ui, "Expires", expires_at);
            }

            if let Some(seconds) = rental.remaining_seconds {
                key_value(ui, "Remaining", &remaining_text(seconds));
            }

            ui.add_space(14.0);
            let enabled = is_launchable(&rental) && !self.is_busy();
            if ui
                .add_enabled(
                    enabled,
                    egui::Button::new(RichText::new("Launch").strong())
                        .fill(if enabled {
                            accent_color()
                        } else {
                            Color32::from_rgb(58, 61, 66)
                        })
                        .min_size(Vec2::new(ui.available_width(), 42.0)),
                )
                .clicked()
            {
                self.launch_selected_program();
            }
        });
    }

    fn ui_loading_page(&self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.35).min(160.0));
            ui.spinner();
            ui.add_space(10.0);
            ui.label(
                RichText::new(self.busy_label.as_deref().unwrap_or("Loading..."))
                    .color(Color32::LIGHT_GRAY),
            );
        });
    }

    fn ui_status(&self, ui: &mut egui::Ui) {
        ui.add_space(12.0);
        app_card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                status_pill(ui, "STATUS", self.status.kind);
                ui.label(RichText::new(&self.status.text).color(status_color(self.status.kind)));

                if let Some(label) = &self.busy_label {
                    ui.spinner();
                    ui.label(label);
                }
            });
        });
    }

    fn ui_toasts(&mut self, ctx: &egui::Context) {
        self.toasts
            .retain(|toast| toast.created_at.elapsed() < toast.ttl);

        if self.toasts.is_empty() {
            return;
        }

        egui::Area::new(egui::Id::new("notification_toasts"))
            .anchor(Align2::RIGHT_TOP, Vec2::new(-18.0, 18.0))
            .interactable(false)
            .show(ctx, |ui| {
                ui.set_width(310.0);

                for toast in self.toasts.iter().rev().take(4) {
                    let age = toast.created_at.elapsed().as_secs_f32();
                    let ttl = toast.ttl.as_secs_f32();
                    let fade_in = (age / 0.18).clamp(0.0, 1.0);
                    let fade_out = ((ttl - age) / 0.35).clamp(0.0, 1.0);
                    let alpha = fade_in.min(fade_out);

                    egui::Frame::new()
                        .fill(with_alpha(
                            Color32::from_rgb(24, 27, 31),
                            (238.0 * alpha) as u8,
                        ))
                        .stroke(Stroke::new(1.0, with_alpha(status_color(toast.kind), 180)))
                        .corner_radius(8.0)
                        .inner_margin(Margin::symmetric(12, 9))
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                status_pill(ui, toast_label(toast.kind), toast.kind);
                                ui.label(RichText::new(&toast.text).color(Color32::WHITE));
                            });
                        });
                    ui.add_space(8.0);
                }
            });
    }
}

impl eframe::App for ProgramLoginApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_messages(ctx);
        ctx.request_repaint_after(Duration::from_millis(33));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let time = ui.input(|input| input.time);

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(bg_color())
                    .inner_margin(Margin::same(18)),
            )
            .show(ui, |ui| {
                draw_background(ui, time);

                if self.user.is_some() {
                    self.ui_dashboard(ui, &ctx);
                } else if self.token.is_some() {
                    self.ui_loading_page(ui);
                    self.ui_status(ui);
                } else {
                    self.ui_login_page(ui, &ctx, time);
                }
            });

        self.ui_toasts(&ctx);
    }
}

fn app_card<R>(
    ui: &mut egui::Ui,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    egui::Frame::new()
        .fill(panel_color())
        .stroke(Stroke::new(1.0, border_color()))
        .corner_radius(8.0)
        .inner_margin(Margin::same(14))
        .show(ui, add_contents)
}

fn key_value(ui: &mut egui::Ui, key: &str, value: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.set_min_height(24.0);
        ui.label(RichText::new(key).color(Color32::from_rgb(145, 151, 161)));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(value).color(Color32::WHITE));
        });
    });
}

fn status_pill(ui: &mut egui::Ui, label: &str, kind: StatusKind) {
    egui::Frame::new()
        .fill(with_alpha(status_color(kind), 34))
        .stroke(Stroke::new(1.0, with_alpha(status_color(kind), 150)))
        .corner_radius(8.0)
        .inner_margin(Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(
                RichText::new(label)
                    .size(11.0)
                    .strong()
                    .color(status_color(kind)),
            );
        });
}

fn ui_loading_rows(ui: &mut egui::Ui) {
    for index in 0..4 {
        let width = ui.available_width() * (0.88 - index as f32 * 0.08);
        egui::Frame::new()
            .fill(Color32::from_rgb(28, 31, 35))
            .corner_radius(8.0)
            .inner_margin(Margin::same(10))
            .show(ui, |ui| {
                ui.allocate_space(Vec2::new(width.max(180.0), 20.0));
            });
        ui.add_space(8.0);
    }
}

fn draw_background(ui: &mut egui::Ui, time: f64) {
    let rect = ui.max_rect();
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, bg_color());

    let scan_y = rect.top() + ((time as f32 * 28.0) % rect.height().max(1.0));
    painter.line_segment(
        [
            egui::pos2(rect.left(), scan_y),
            egui::pos2(rect.right(), scan_y),
        ],
        Stroke::new(1.0, Color32::from_rgba_premultiplied(67, 185, 156, 34)),
    );

    painter.line_segment(
        [
            egui::pos2(rect.left() + 18.0, rect.top() + 8.0),
            egui::pos2(rect.right() - 18.0, rect.top() + 8.0),
        ],
        Stroke::new(2.0, with_alpha(accent_color(), 120)),
    );
}

fn is_launchable(rental: &Rental) -> bool {
    matches!(rental.rental_status.as_str(), "ACTIVE" | "PERMANENT")
        && rental.product_status == "AVAILABLE"
}

fn status_kind_for_rental(rental: &Rental) -> StatusKind {
    match rental.rental_status.as_str() {
        "ACTIVE" | "PERMANENT" => StatusKind::Success,
        "EXPIRED" => StatusKind::Error,
        _ => StatusKind::Warning,
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

fn toast_label(kind: StatusKind) -> &'static str {
    match kind {
        StatusKind::Info => "INFO",
        StatusKind::Success => "OK",
        StatusKind::Warning => "WARN",
        StatusKind::Error => "ERR",
    }
}

fn status_color(kind: StatusKind) -> Color32 {
    match kind {
        StatusKind::Info => Color32::from_rgb(125, 211, 252),
        StatusKind::Success => Color32::from_rgb(52, 211, 153),
        StatusKind::Warning => Color32::from_rgb(251, 191, 36),
        StatusKind::Error => Color32::from_rgb(251, 113, 133),
    }
}

fn bg_color() -> Color32 {
    Color32::from_rgb(11, 13, 16)
}

fn panel_color() -> Color32 {
    Color32::from_rgb(20, 23, 27)
}

fn border_color() -> Color32 {
    Color32::from_rgb(48, 54, 61)
}

fn accent_color() -> Color32 {
    Color32::from_rgb(45, 212, 191)
}

fn with_alpha(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_premultiplied(color.r(), color.g(), color.b(), alpha)
}

fn pulse(time: f64) -> f32 {
    ((time as f32 * 2.4).sin() * 0.5 + 0.5).clamp(0.0, 1.0)
}

fn mix_color(a: Color32, b: Color32, amount: f32) -> Color32 {
    let amount = amount.clamp(0.0, 1.0);
    let mix = |left: u8, right: u8| {
        (left as f32 * (1.0 - amount) + right as f32 * amount)
            .round()
            .clamp(0.0, 255.0) as u8
    };

    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}
