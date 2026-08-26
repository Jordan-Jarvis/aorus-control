use std::{
    collections::HashMap,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

use eframe::egui::{
    self, Align, Color32, Context, FontId, Frame, Key, Layout, Margin, RichText, Stroke, Vec2,
};
use egui_plot::{Line, Plot, PlotPoint, Points, VLine};
use zbus::{blocking::connection::Builder as ConnectionBuilder, zvariant::OwnedValue};

const DESTINATION: &str = "io.github.aoruslinux.Control1";
const PATH: &str = "/io/github/aoruslinux/Control1";
const INTERFACE: &str = "io.github.aoruslinux.Control1";
const CURVE_POINTS: usize = 15;
const TELEMETRY_INTERVAL: Duration = Duration::from_secs(2);
const STALE_AFTER: Duration = Duration::from_secs(8);
const DBUS_METHOD_TIMEOUT: Duration = Duration::from_secs(5);

const BLUE: Color32 = Color32::from_rgb(77, 163, 255);
const CYAN: Color32 = Color32::from_rgb(62, 213, 209);
const GREEN: Color32 = Color32::from_rgb(65, 204, 126);
const AMBER: Color32 = Color32::from_rgb(244, 181, 65);
const RED: Color32 = Color32::from_rgb(238, 91, 91);
const MUTED: Color32 = Color32::from_rgb(145, 153, 168);

fn main() -> eframe::Result {
    if running_as_root() {
        eprintln!("AORUS Control must run as a regular desktop user, not root");
        std::process::exit(1);
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id("io.github.aoruslinux.Control")
            .with_title("AORUS Control")
            .with_inner_size([1120.0, 760.0])
            .with_min_inner_size([860.0, 620.0]),
        ..Default::default()
    };
    eframe::run_native(
        "AORUS Control",
        options,
        Box::new(|cc| Ok(Box::new(AorusApp::new(cc)))),
    )
}

fn running_as_root() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find(|line| line.starts_with("Uid:"))
                .and_then(|line| line.split_whitespace().nth(2))
                .and_then(|uid| uid.parse::<u32>().ok())
        })
        == Some(0)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    Dashboard,
    Fans,
    Power,
    Hardware,
}

impl Tab {
    const ALL: [Self; 4] = [Self::Dashboard, Self::Fans, Self::Power, Self::Hardware];
    fn label(self) -> &'static str {
        match self {
            Self::Dashboard => "Dashboard",
            Self::Fans => "Fans",
            Self::Power => "Power & Battery",
            Self::Hardware => "Hardware / Diagnostics",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FanMode {
    Normal = 0,
    Silent = 1,
    Gaming = 2,
    Custom = 3,
    Auto = 4,
    Fixed = 5,
}

impl FanMode {
    const SELECTABLE: [Self; 4] = [Self::Normal, Self::Silent, Self::Gaming, Self::Custom];
    fn from_wire(value: u8) -> Option<Self> {
        Some(match value {
            0 => Self::Normal,
            1 => Self::Silent,
            2 => Self::Gaming,
            3 => Self::Custom,
            4 => Self::Auto,
            5 => Self::Fixed,
            _ => return None,
        })
    }
    fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Silent => "Silent",
            Self::Gaming => "Gaming",
            Self::Custom => "Custom",
            Self::Auto => "Auto",
            Self::Fixed => "Fixed",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct CurvePoint {
    temperature: u8,
    speed: u8,
}

#[derive(Clone, Debug, Default)]
struct Status {
    daemon_mode: Option<String>,
    driver_available: Option<bool>,
    power_profile: Option<String>,
    fan_mode: Option<FanMode>,
    temperatures: [Option<i32>; 3],
    fans: [Option<u32>; 4],
    charge_mode: Option<u8>,
    charge_limit: Option<u8>,
    battery_cycles: Option<u32>,
    gpu_boost: Option<u8>,
    usb_charge_s3: Option<bool>,
    usb_charge_s4: Option<bool>,
    graphics_mode: Option<String>,
    graphics_power: Option<bool>,
    custom_curve_available: Option<bool>,
    fan_modes: Vec<FanMode>,
    fan_curve_points: Option<u8>,
    charge_mode_supported: Option<bool>,
    charge_limit_supported: Option<bool>,
    usb_charge_s3_supported: Option<bool>,
    usb_charge_s4_supported: Option<bool>,
    gpu_boost_values: Vec<u8>,
    product_name: Option<String>,
    product_version: Option<String>,
    bios_version: Option<String>,
    bios_date: Option<String>,
    kernel_release: Option<String>,
    driver_module_version: Option<String>,
    platform_path: Option<String>,
    hwmon_path: Option<String>,
    last_error: Option<String>,
    unknown_keys: Vec<String>,
}

impl Status {
    fn write_enabled(&self) -> bool {
        self.daemon_mode.as_deref() == Some("write-enabled")
    }
    fn temperature_c(&self, index: usize) -> Option<f32> {
        self.temperatures[index].map(|value| value as f32 / 1000.0)
    }
}

#[derive(Debug)]
enum Command {
    Refresh,
    RefreshCurve,
    SetPowerProfile(String),
    SetFanMode(FanMode),
    ReapplyFanProfile,
    SetFanCurve([CurvePoint; CURVE_POINTS]),
    SetProfileMappings(HashMap<String, FanMode>),
    SetChargeMode(u8),
    SetChargeLimit(u8),
    SetGpuBoost(u8),
}

impl Command {
    fn label(&self) -> &'static str {
        match self {
            Self::Refresh => "Refresh",
            Self::RefreshCurve => "Reload fan curve",
            Self::SetPowerProfile(_) => "Change power profile",
            Self::SetFanMode(_) => "Change fan profile",
            Self::ReapplyFanProfile => "Reapply fan profile",
            Self::SetFanCurve(_) => "Apply fan curve",
            Self::SetProfileMappings(_) => "Save profile mappings",
            Self::SetChargeMode(_) => "Change charge mode",
            Self::SetChargeLimit(_) => "Set charge limit",
            Self::SetGpuBoost(_) => "Set GPU boost",
        }
    }
}

#[derive(Debug)]
enum WorkerEvent {
    Snapshot {
        status: Box<Status>,
        received_at: Instant,
    },
    Curve(Result<[CurvePoint; CURVE_POINTS], String>),
    ProfileMappings(Result<HashMap<String, FanMode>, String>),
    ActionStarted(&'static str),
    ActionFinished {
        action: &'static str,
        result: Result<(), String>,
    },
    ConnectionError(String),
}

struct AorusApp {
    tab: Tab,
    status: Option<Status>,
    last_snapshot: Option<Instant>,
    connection_error: Option<String>,
    action_message: Option<(bool, String)>,
    action_in_flight: Option<&'static str>,
    command_tx: Sender<Command>,
    event_rx: Receiver<WorkerEvent>,
    firmware_curve: Option<[CurvePoint; CURVE_POINTS]>,
    edited_curve: [CurvePoint; CURVE_POINTS],
    curve_loaded: bool,
    selected_point: usize,
    dragging_point: Option<usize>,
    mappings: HashMap<String, FanMode>,
    saved_mappings: HashMap<String, FanMode>,
    mappings_loaded: bool,
    charge_limit_edit: u8,
    charge_limit_dirty: bool,
}

impl AorusApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        configure_style(&cc.egui_ctx);
        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        spawn_dbus_worker(command_rx, event_tx, cc.egui_ctx.clone());
        let mappings = HashMap::from([
            ("performance".to_owned(), FanMode::Gaming),
            ("balanced".to_owned(), FanMode::Normal),
            ("battery".to_owned(), FanMode::Silent),
        ]);
        Self {
            tab: Tab::Dashboard,
            status: None,
            last_snapshot: None,
            connection_error: None,
            action_message: None,
            action_in_flight: None,
            command_tx,
            event_rx,
            firmware_curve: None,
            edited_curve: default_curve(),
            curve_loaded: false,
            selected_point: 0,
            dragging_point: None,
            saved_mappings: mappings.clone(),
            mappings,
            mappings_loaded: false,
            charge_limit_edit: 80,
            charge_limit_dirty: false,
        }
    }

    fn drain_events(&mut self) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                WorkerEvent::Snapshot {
                    status,
                    received_at,
                } => {
                    if !self.charge_limit_dirty {
                        self.charge_limit_edit =
                            status.charge_limit.unwrap_or(self.charge_limit_edit);
                    }
                    self.status = Some(*status);
                    self.last_snapshot = Some(received_at);
                    self.connection_error = None;
                }
                WorkerEvent::Curve(result) => match result {
                    Ok(curve) => {
                        let dirty = self.curve_dirty();
                        self.firmware_curve = Some(curve);
                        if !dirty || !self.curve_loaded {
                            self.edited_curve = curve;
                        }
                        self.curve_loaded = true;
                    }
                    Err(error) => {
                        self.firmware_curve = None;
                        self.curve_loaded = false;
                        self.action_message =
                            Some((false, format!("Fan curve could not be loaded: {error}")));
                    }
                },
                WorkerEvent::ProfileMappings(result) => match result {
                    Ok(mappings) => {
                        if !self.mappings_dirty() {
                            self.mappings = mappings.clone();
                        }
                        self.saved_mappings = mappings;
                        self.mappings_loaded = true;
                    }
                    Err(error) => {
                        self.mappings_loaded = false;
                        self.action_message = Some((
                            false,
                            format!("Profile mappings could not be loaded: {error}"),
                        ));
                    }
                },
                WorkerEvent::ActionStarted(action) => {
                    self.action_in_flight = Some(action);
                    self.action_message = None;
                }
                WorkerEvent::ActionFinished { action, result } => {
                    self.action_in_flight = None;
                    if result.is_ok() {
                        if action == "Save profile mappings" {
                            self.saved_mappings = self.mappings.clone();
                        } else if action == "Set charge limit" {
                            self.charge_limit_dirty = false;
                        }
                    }
                    self.action_message = Some(match result {
                        Ok(()) => (
                            true,
                            format!("{action} completed and status was refreshed."),
                        ),
                        Err(error) => (false, format!("{action} failed: {error}")),
                    });
                }
                WorkerEvent::ConnectionError(error) => {
                    self.connection_error = Some(error);
                    self.status = None;
                    self.last_snapshot = None;
                    self.firmware_curve = None;
                    self.curve_loaded = false;
                    self.mappings_loaded = false;
                }
            }
        }
    }

    fn send(&mut self, command: Command) {
        let label = command.label();
        if self.command_tx.send(command).is_err() {
            self.action_message = Some((false, format!("{label} failed: D-Bus worker stopped")));
        }
    }
    fn write_enabled(&self) -> bool {
        self.status.as_ref().is_some_and(Status::write_enabled) && self.action_in_flight.is_none()
    }
    fn connected(&self) -> bool {
        self.status.is_some() && self.connection_error.is_none()
    }
    fn power_control_enabled(&self) -> bool {
        self.connected() && self.action_in_flight.is_none()
    }
    fn curve_dirty(&self) -> bool {
        self.firmware_curve
            .is_some_and(|curve| curve != self.edited_curve)
    }
    fn curve_supported(&self) -> bool {
        self.status.as_ref().and_then(|s| s.fan_curve_points) == Some(CURVE_POINTS as u8)
    }
    fn mappings_dirty(&self) -> bool {
        self.mappings != self.saved_mappings
    }

    fn show_sidebar(&mut self, root: &mut egui::Ui) {
        egui::Panel::left("navigation")
            .resizable(false)
            .exact_size(220.0)
            .frame(Frame::new().fill(Color32::from_rgb(20, 23, 30)))
            .show(root, |ui| {
                ui.add_space(18.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("◉").size(24.0).color(CYAN));
                    ui.vertical(|ui| {
                        ui.label(RichText::new("AORUS CONTROL").strong().size(17.0));
                        ui.label(RichText::new("Linux native").small().color(MUTED));
                    });
                });
                ui.add_space(24.0);
                for tab in Tab::ALL {
                    let button = egui::Button::new(RichText::new(tab.label()).size(14.0))
                        .selected(self.tab == tab)
                        .min_size(Vec2::new(192.0, 38.0));
                    if ui.add(button).clicked() {
                        self.tab = tab;
                    }
                    ui.add_space(4.0);
                }
                ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
                    ui.add_space(14.0);
                    let (color, title, detail) = self.sidebar_health();
                    Frame::new()
                        .fill(Color32::from_rgb(27, 31, 40))
                        .corner_radius(8.0)
                        .inner_margin(Margin::same(12))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("●").color(color));
                                ui.label(RichText::new(title).strong());
                            });
                            ui.label(RichText::new(detail).small().color(MUTED));
                        });
                });
            });
    }

    fn sidebar_health(&self) -> (Color32, &'static str, String) {
        if let Some(error) = &self.connection_error {
            return (RED, "Daemon unavailable", compact(error, 82));
        }
        let Some(status) = &self.status else {
            return (AMBER, "Connecting", "Waiting for system D-Bus…".to_owned());
        };
        if self
            .last_snapshot
            .is_some_and(|at| at.elapsed() > STALE_AFTER)
        {
            return (
                AMBER,
                "Telemetry stale",
                "Last refresh was more than 8 seconds ago".to_owned(),
            );
        }
        if status.driver_available == Some(false) {
            return (
                RED,
                "Driver missing",
                "AORUS platform controls were not discovered".to_owned(),
            );
        }
        if let Some(error) = &status.last_error {
            return (RED, "Daemon error", compact(error, 82));
        }
        if status.write_enabled() {
            (
                GREEN,
                "Write enabled",
                "Hardware changes require polkit approval".to_owned(),
            )
        } else {
            (
                BLUE,
                "Shadow mode",
                "Monitoring only; Python remains authoritative".to_owned(),
            )
        }
    }

    fn show_top_bar(&mut self, root: &mut egui::Ui) {
        egui::Panel::top("top_bar")
            .exact_size(62.0)
            .frame(
                Frame::new()
                    .fill(Color32::from_rgb(25, 28, 36))
                    .inner_margin(Margin::symmetric(20, 12)),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(self.tab.label()).size(20.0).strong());
                        ui.label(
                            RichText::new("GIGABYTE AERO / AORUS system control")
                                .small()
                                .color(MUTED),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add_enabled(
                                self.action_in_flight.is_none(),
                                egui::Button::new("Refresh"),
                            )
                            .clicked()
                        {
                            self.send(Command::Refresh);
                        }
                        if let Some(action) = self.action_in_flight {
                            ui.spinner();
                            ui.label(RichText::new(action).color(AMBER));
                        } else if let Some(at) = self.last_snapshot {
                            ui.label(
                                RichText::new(format!(
                                    "Updated {:.0}s ago",
                                    at.elapsed().as_secs_f32()
                                ))
                                .small()
                                .color(MUTED),
                            );
                        }
                    });
                });
            });
    }

    fn show_content(&mut self, root: &mut egui::Ui) {
        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(Color32::from_rgb(18, 20, 26))
                    .inner_margin(Margin::same(22)),
            )
            .show(root, |ui| {
                if let Some((success, message)) = &self.action_message {
                    banner(ui, if *success { GREEN } else { RED }, message);
                    ui.add_space(12.0);
                }
                egui::ScrollArea::vertical().show(ui, |ui| match self.tab {
                    Tab::Dashboard => self.dashboard(ui),
                    Tab::Fans => self.fans(ui),
                    Tab::Power => self.power_and_battery(ui),
                    Tab::Hardware => self.hardware(ui),
                });
            });
    }

    fn dashboard(&mut self, ui: &mut egui::Ui) {
        self.health_banner(ui);
        ui.add_space(14.0);
        ui.heading("Thermals");
        ui.add_space(8.0);
        ui.columns(3, |columns| {
            for (index, (label, color)) in [
                ("EC Temp 1", RED),
                ("EC Temp 2", AMBER),
                ("EC Temp 3", CYAN),
            ]
            .into_iter()
            .enumerate()
            {
                let value = self
                    .status
                    .as_ref()
                    .and_then(|status| status.temperature_c(index));
                metric_card(
                    &mut columns[index],
                    label,
                    value
                        .map(|v| format!("{v:.0} °C"))
                        .as_deref()
                        .unwrap_or("—"),
                    "EC sensor",
                    color,
                );
            }
        });
        ui.add_space(12.0);
        ui.columns(2, |columns| {
            for (index, label) in ["Fan 1", "Fan 2"].into_iter().enumerate() {
                let value = self.status.as_ref().and_then(|status| status.fans[index]);
                metric_card(
                    &mut columns[index],
                    label,
                    value.map(|v| format!("{v} RPM")).as_deref().unwrap_or("—"),
                    "Measured speed",
                    BLUE,
                );
            }
        });
        ui.add_space(20.0);
        ui.heading("Profiles");
        ui.add_space(8.0);
        card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("System power");
                ui.label(profile_badge(
                    self.status
                        .as_ref()
                        .and_then(|s| s.power_profile.as_deref())
                        .unwrap_or("Unknown"),
                    BLUE,
                ));
                ui.separator();
                ui.label("Firmware fans");
                ui.label(profile_badge(
                    self.status
                        .as_ref()
                        .and_then(|s| s.fan_mode)
                        .map(FanMode::label)
                        .unwrap_or("Unknown"),
                    CYAN,
                ));
            });
            ui.add_space(12.0);
            ui.horizontal_wrapped(|ui| {
                for profile in ["battery", "balanced", "performance"] {
                    let active = self
                        .status
                        .as_ref()
                        .and_then(|s| s.power_profile.as_deref())
                        == Some(profile);
                    if ui
                        .add_enabled(
                            self.power_control_enabled(),
                            egui::Button::new(title_case(profile))
                                .selected(active)
                                .min_size([130.0, 34.0].into()),
                        )
                        .clicked()
                    {
                        self.send(Command::SetPowerProfile(profile.to_owned()));
                    }
                }
            });
            if !self.power_control_enabled() {
                ui.label(
                    RichText::new(self.write_disabled_reason())
                        .small()
                        .color(MUTED),
                );
            }
        });
    }

    fn health_banner(&self, ui: &mut egui::Ui) {
        let (color, title, detail) = self.sidebar_health();
        Frame::new()
            .fill(color.gamma_multiply(0.12))
            .stroke(Stroke::new(1.0, color.gamma_multiply(0.55)))
            .corner_radius(8.0)
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("●").color(color));
                    ui.vertical(|ui| {
                        ui.label(RichText::new(title).strong().color(color));
                        ui.label(detail);
                    });
                });
            });
    }

    fn fans(&mut self, ui: &mut egui::Ui) {
        ui.heading("Firmware fan profile");
        ui.add_space(8.0);
        card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for mode in FanMode::SELECTABLE {
                    let active = self.status.as_ref().and_then(|s| s.fan_mode) == Some(mode);
                    let supported = self
                        .status
                        .as_ref()
                        .is_some_and(|s| s.fan_modes.contains(&mode));
                    let custom_available = mode != FanMode::Custom
                        || (self.curve_loaded
                            && self.status.as_ref().and_then(|s| s.custom_curve_available)
                                == Some(true));
                    if ui
                        .add_enabled(
                            self.write_enabled() && supported && custom_available,
                            egui::Button::new(mode.label())
                                .selected(active)
                                .min_size([110.0, 34.0].into()),
                        )
                        .clicked()
                    {
                        self.send(Command::SetFanMode(mode));
                    }
                }
                if ui
                    .add_enabled(self.write_enabled(), egui::Button::new("Reapply current"))
                    .clicked()
                {
                    self.send(Command::ReapplyFanProfile);
                }
            });
            ui.label(RichText::new("Normal, Silent, Gaming, and Custom are firmware-controlled profiles. Fixed fan speed is intentionally not exposed.").small().color(MUTED));
        });
        ui.add_space(18.0);
        ui.horizontal(|ui| {
            ui.heading("Custom fan curve");
            if self.curve_dirty() {
                ui.label(profile_badge("Unsaved changes", AMBER));
            }
        });
        ui.label(RichText::new("Drag any point, select it and use arrow keys, or edit exact values below. Changes remain local until Apply.").color(MUTED));
        ui.add_space(8.0);
        card(ui, |ui| {
            let points: Vec<[f64; 2]> = self
                .edited_curve
                .iter()
                .map(|point| [point.temperature as f64, point.speed as f64])
                .collect();
            let selected = self.edited_curve[self.selected_point];
            let temps: Vec<f64> = self
                .status
                .as_ref()
                .into_iter()
                .flat_map(|s| s.temperatures)
                .flatten()
                .map(|v| v as f64 / 1000.0)
                .collect();
            let response = Plot::new("fan_curve")
                .height(360.0)
                .allow_drag(false)
                .allow_zoom(false)
                .allow_scroll(false)
                .show_grid(true)
                .include_x(0.0)
                .include_x(100.0)
                .include_y(0.0)
                .include_y(255.0)
                .x_axis_label("Temperature (°C)")
                .y_axis_label("Firmware fan level (0–255)")
                .show(ui, |plot_ui| {
                    plot_ui.line(
                        Line::new("Curve", points.clone())
                            .color(CYAN)
                            .width(2.5)
                            .fill(0.0)
                            .fill_alpha(0.06),
                    );
                    plot_ui.points(
                        Points::new("Points", points.clone())
                            .color(BLUE)
                            .radius(6.0),
                    );
                    plot_ui.points(
                        Points::new(
                            "Selected",
                            vec![[selected.temperature as f64, selected.speed as f64]],
                        )
                        .color(AMBER)
                        .radius(9.0),
                    );
                    for (index, temperature) in temps.into_iter().enumerate() {
                        plot_ui.vline(
                            VLine::new(format!("EC {}", index + 1), temperature)
                                .color([RED, AMBER, GREEN][index])
                                .width(1.5),
                        );
                    }
                    let pointer = plot_ui.pointer_coordinate();
                    if plot_ui.response().drag_started() {
                        self.dragging_point = pointer
                            .and_then(|pointer| nearest_curve_point(&self.edited_curve, pointer));
                        if let Some(index) = self.dragging_point {
                            self.selected_point = index;
                        }
                    }
                    if plot_ui.response().dragged()
                        && let (Some(index), Some(pointer)) = (self.dragging_point, pointer)
                    {
                        self.set_curve_point(
                            index,
                            pointer.x.round() as i32,
                            pointer.y.round() as i32,
                        );
                    }
                    if plot_ui.response().drag_stopped() {
                        self.dragging_point = None;
                    }
                    if plot_ui.response().clicked()
                        && let Some(pointer) = pointer
                        && let Some(index) = nearest_curve_point(&self.edited_curve, pointer)
                    {
                        self.selected_point = index;
                    }
                });
            response.response.on_hover_text("Drag a point to edit. The point is clamped between its neighbours so the curve remains monotonic.");
            self.curve_keyboard(ui);
            ui.separator();
            self.curve_point_editor(ui);
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        self.write_enabled()
                            && self.curve_supported()
                            && self.curve_dirty()
                            && self.curve_loaded,
                        egui::Button::new("Apply curve").fill(CYAN.gamma_multiply(0.35)),
                    )
                    .clicked()
                {
                    self.send(Command::SetFanCurve(self.edited_curve));
                }
                if ui
                    .add_enabled(self.curve_dirty(), egui::Button::new("Discard / reload"))
                    .clicked()
                    && let Some(curve) = self.firmware_curve
                {
                    self.edited_curve = curve;
                }
                if ui
                    .add_enabled(
                        self.connected()
                            && self.curve_supported()
                            && self.action_in_flight.is_none(),
                        egui::Button::new("Reload curve from daemon"),
                    )
                    .clicked()
                {
                    self.send(Command::RefreshCurve);
                }
                ui.label(
                    RichText::new(format!(
                        "Point {} of {CURVE_POINTS}",
                        self.selected_point + 1
                    ))
                    .small()
                    .color(MUTED),
                );
            });
            if !self.curve_loaded {
                ui.label(RichText::new("No complete 15-point curve is available from the daemon. The preview is illustrative and cannot be applied.").color(AMBER));
            } else if !self.write_enabled() {
                ui.label(
                    RichText::new(self.write_disabled_reason())
                        .small()
                        .color(MUTED),
                );
            }
        });
        ui.add_space(18.0);
        self.profile_mappings(ui);
    }

    fn curve_keyboard(&mut self, ui: &mut egui::Ui) {
        if ui.memory(|memory| memory.focused().is_some()) {
            return;
        }
        let (left, right, up, down) = ui.input(|input| {
            (
                input.key_pressed(Key::ArrowLeft),
                input.key_pressed(Key::ArrowRight),
                input.key_pressed(Key::ArrowUp),
                input.key_pressed(Key::ArrowDown),
            )
        });
        let point = self.edited_curve[self.selected_point];
        if left || right || up || down {
            self.set_curve_point(
                self.selected_point,
                point.temperature as i32 + i32::from(right) - i32::from(left),
                point.speed as i32 + i32::from(up) - i32::from(down),
            );
        }
    }

    fn curve_point_editor(&mut self, ui: &mut egui::Ui) {
        let index = self.selected_point;
        let mut temperature = self.edited_curve[index].temperature as i32;
        let mut speed = self.edited_curve[index].speed as i32;
        ui.horizontal_wrapped(|ui| {
            ui.label("Selected point");
            ui.add(
                egui::Slider::new(&mut self.selected_point, 0..=CURVE_POINTS - 1)
                    .custom_formatter(|value, _| format!("{}", value as usize + 1))
                    .show_value(true),
            );
            ui.separator();
            ui.label("Temperature");
            let temp_changed = ui
                .add(
                    egui::DragValue::new(&mut temperature)
                        .range(0..=100)
                        .suffix(" °C"),
                )
                .changed();
            ui.label("Fan level");
            let speed_changed = ui
                .add(egui::DragValue::new(&mut speed).range(0..=255))
                .changed();
            ui.label(RichText::new(format!("({:.0}%)", speed as f32 / 255.0 * 100.0)).color(MUTED));
            if temp_changed || speed_changed {
                self.set_curve_point(index, temperature, speed);
            }
        });
    }

    fn set_curve_point(&mut self, index: usize, temperature: i32, speed: i32) {
        let min_temp = index
            .checked_sub(1)
            .map_or(0, |i| self.edited_curve[i].temperature);
        let max_temp = self
            .edited_curve
            .get(index + 1)
            .map_or(100, |p| p.temperature);
        let min_speed = index
            .checked_sub(1)
            .map_or(0, |i| self.edited_curve[i].speed);
        let max_speed = self.edited_curve.get(index + 1).map_or(255, |p| p.speed);
        self.edited_curve[index] = CurvePoint {
            temperature: temperature.clamp(min_temp as i32, max_temp as i32) as u8,
            speed: speed.clamp(min_speed as i32, max_speed as i32) as u8,
        };
    }

    fn profile_mappings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Power → fan profile mappings");
        ui.add_space(8.0);
        card(ui, |ui| {
            for profile in ["battery", "balanced", "performance"] {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(title_case(profile)).strong());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let selected = *self.mappings.get(profile).unwrap_or(&FanMode::Normal);
                        ui.add_enabled_ui(self.connected() && self.mappings_loaded, |ui| {
                            egui::ComboBox::from_id_salt(("mapping", profile))
                                .selected_text(selected.label())
                                .show_ui(ui, |ui| {
                                    for mode in FanMode::SELECTABLE {
                                        let supported = self
                                            .status
                                            .as_ref()
                                            .is_some_and(|s| s.fan_modes.contains(&mode));
                                        let custom_available = mode != FanMode::Custom
                                            || (self.curve_loaded
                                                && self
                                                    .status
                                                    .as_ref()
                                                    .and_then(|s| s.custom_curve_available)
                                                    == Some(true));
                                        if ui
                                            .add_enabled(
                                                supported && custom_available,
                                                egui::Button::selectable(
                                                    selected == mode,
                                                    mode.label(),
                                                ),
                                            )
                                            .clicked()
                                        {
                                            self.mappings.insert(profile.to_owned(), mode);
                                        }
                                    }
                                });
                        });
                    });
                });
                ui.separator();
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        self.write_enabled() && self.mappings_loaded && self.mappings_dirty(),
                        egui::Button::new("Save mappings"),
                    )
                    .clicked()
                {
                    self.send(Command::SetProfileMappings(self.mappings.clone()));
                }
                if ui
                    .add_enabled(
                        self.mappings_loaded && self.mappings_dirty(),
                        egui::Button::new("Discard mapping edits"),
                    )
                    .clicked()
                {
                    self.mappings = self.saved_mappings.clone();
                }
            });
            ui.label(RichText::new("Mappings are loaded from the daemon. Unsaved local edits are preserved across telemetry refreshes.").small().color(MUTED));
            if !self.mappings_loaded {
                ui.label(
                    RichText::new(
                        "Persisted mappings are unavailable; editing and saving are disabled.",
                    )
                    .color(AMBER),
                );
            }
        });
    }

    fn power_and_battery(&mut self, ui: &mut egui::Ui) {
        ui.heading("System power profile");
        ui.add_space(8.0);
        card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for profile in ["battery", "balanced", "performance"] {
                    let active = self
                        .status
                        .as_ref()
                        .and_then(|s| s.power_profile.as_deref())
                        == Some(profile);
                    if ui
                        .add_enabled(
                            self.power_control_enabled(),
                            egui::Button::new(title_case(profile))
                                .selected(active)
                                .min_size([140.0, 38.0].into()),
                        )
                        .clicked()
                    {
                        self.send(Command::SetPowerProfile(profile.to_owned()));
                    }
                }
            });
            ui.label(
                RichText::new(
                    "System76 Power remains the authority for CPU and system power policy.",
                )
                .small()
                .color(MUTED),
            );
        });
        ui.add_space(18.0);
        ui.heading("Battery care");
        ui.add_space(8.0);
        card(ui, |ui| {
            let charge_mode = self.status.as_ref().and_then(|s| s.charge_mode);
            let charge_mode_supported =
                self.status.as_ref().and_then(|s| s.charge_mode_supported) == Some(true);
            let charge_limit_supported =
                self.status.as_ref().and_then(|s| s.charge_limit_supported) == Some(true);
            ui.horizontal_wrapped(|ui| {
                ui.label("Charge mode");
                for (value, label) in [(0, "Normal"), (1, "Custom limit")] {
                    if ui
                        .add_enabled(
                            self.write_enabled() && charge_mode_supported,
                            egui::Button::new(label).selected(charge_mode == Some(value)),
                        )
                        .clicked()
                    {
                        self.send(Command::SetChargeMode(value));
                    }
                }
            });
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.label("Charge limit");
                if ui
                    .add_enabled(
                        self.connected() && charge_limit_supported,
                        egui::Slider::new(&mut self.charge_limit_edit, 60..=100).suffix("%"),
                    )
                    .changed()
                {
                    self.charge_limit_dirty = true;
                }
                let changed = self.charge_limit_dirty
                    || self.status.as_ref().and_then(|s| s.charge_limit)
                        != Some(self.charge_limit_edit);
                if ui
                    .add_enabled(
                        self.write_enabled() && charge_limit_supported && changed,
                        egui::Button::new("Set limit"),
                    )
                    .clicked()
                {
                    self.send(Command::SetChargeLimit(self.charge_limit_edit));
                }
            });
            ui.horizontal(|ui| {
                ui.label("Battery cycles");
                ui.label(
                    RichText::new(
                        self.status
                            .as_ref()
                            .and_then(|s| s.battery_cycles)
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "Unavailable".to_owned()),
                    )
                    .strong(),
                );
            });
            if !charge_mode_supported && !charge_limit_supported {
                ui.label(
                    RichText::new(
                        "AORUS charging controls are not reported by this hardware/daemon.",
                    )
                    .color(MUTED),
                );
            }
        });
        ui.add_space(18.0);
        ui.heading("GPU");
        ui.add_space(8.0);
        card(ui, |ui| {
            let current = self.status.as_ref().and_then(|s| s.gpu_boost);
            let supported_values = self
                .status
                .as_ref()
                .map(|s| s.gpu_boost_values.clone())
                .unwrap_or_default();
            ui.horizontal_wrapped(|ui| {
                ui.label("Boost level");
                for value in supported_values.iter().copied() {
                    if ui
                        .add_enabled(
                            self.write_enabled() && current.is_some(),
                            egui::Button::new(value.to_string()).selected(current == Some(value)),
                        )
                        .clicked()
                    {
                        self.send(Command::SetGpuBoost(value));
                    }
                }
            });
            if supported_values.is_empty() {
                ui.label(
                    RichText::new("GPU boost is unavailable or has not been proven on this model.")
                        .color(MUTED),
                );
            } else {
                ui.label(RichText::new("The daemon is responsible for rejecting values not proven safe on this laptop.").small().color(MUTED));
            }
            ui.separator();
            readonly_row(
                ui,
                "Graphics mode",
                self.status
                    .as_ref()
                    .and_then(|s| s.graphics_mode.as_deref())
                    .unwrap_or("Unavailable"),
            );
            readonly_row(
                ui,
                "Discrete graphics power",
                self.status
                    .as_ref()
                    .and_then(|s| s.graphics_power)
                    .map(on_off)
                    .unwrap_or("Unavailable"),
            );
        });
    }

    fn hardware(&mut self, ui: &mut egui::Ui) {
        ui.heading("Service and hardware status");
        ui.add_space(8.0);
        card(ui, |ui| {
            let status = self.status.as_ref();
            readonly_row(ui, "D-Bus destination", DESTINATION);
            readonly_row(
                ui,
                "Daemon mode",
                status
                    .and_then(|s| s.daemon_mode.as_deref())
                    .unwrap_or("Unavailable"),
            );
            readonly_row(
                ui,
                "AORUS driver",
                status
                    .and_then(|s| s.driver_available)
                    .map(|v| if v { "Available" } else { "Missing" })
                    .unwrap_or("Unknown"),
            );
            readonly_row(
                ui,
                "Product",
                status
                    .and_then(|s| s.product_name.as_deref())
                    .unwrap_or("Unavailable"),
            );
            readonly_row(
                ui,
                "Product version",
                status
                    .and_then(|s| s.product_version.as_deref())
                    .unwrap_or("Unavailable"),
            );
            readonly_row(
                ui,
                "BIOS",
                status
                    .and_then(|s| s.bios_version.as_deref())
                    .unwrap_or("Unavailable"),
            );
            readonly_row(
                ui,
                "BIOS date",
                status
                    .and_then(|s| s.bios_date.as_deref())
                    .unwrap_or("Unavailable"),
            );
            readonly_row(
                ui,
                "Kernel",
                status
                    .and_then(|s| s.kernel_release.as_deref())
                    .unwrap_or("Unavailable"),
            );
            readonly_row(
                ui,
                "Driver module version",
                status
                    .and_then(|s| s.driver_module_version.as_deref())
                    .unwrap_or("Unavailable"),
            );
            readonly_row(
                ui,
                "Platform path",
                status
                    .and_then(|s| s.platform_path.as_deref())
                    .unwrap_or("Unavailable"),
            );
            readonly_row(
                ui,
                "Hardware monitor path",
                status
                    .and_then(|s| s.hwmon_path.as_deref())
                    .unwrap_or("Unavailable"),
            );
            readonly_row(
                ui,
                "Custom curve",
                status
                    .and_then(|s| s.custom_curve_available)
                    .map(|v| if v { "Available" } else { "Unavailable" })
                    .unwrap_or("Unknown"),
            );
            readonly_row(
                ui,
                "Fan curve points",
                &status
                    .and_then(|s| s.fan_curve_points)
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "Unsupported".to_owned()),
            );
            readonly_row(
                ui,
                "Supported fan profiles",
                &status
                    .map(|s| {
                        s.fan_modes
                            .iter()
                            .map(|mode| mode.label())
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| "None reported".to_owned()),
            );
            readonly_row(
                ui,
                "USB charging in S3",
                status
                    .and_then(|s| s.usb_charge_s3)
                    .map(on_off)
                    .unwrap_or("Unavailable"),
            );
            readonly_row(
                ui,
                "USB charging in S4",
                status
                    .and_then(|s| s.usb_charge_s4)
                    .map(on_off)
                    .unwrap_or("Unavailable"),
            );
        });
        ui.add_space(18.0);
        ui.heading("Sensor channels");
        ui.add_space(8.0);
        card(ui, |ui| {
            for index in 0..3 {
                readonly_row(
                    ui,
                    &format!("EC temperature {}", index + 1),
                    &self
                        .status
                        .as_ref()
                        .and_then(|s| s.temperature_c(index))
                        .map(|v| format!("{v:.1} °C"))
                        .unwrap_or_else(|| "Unavailable".to_owned()),
                );
            }
            for index in 0..4 {
                readonly_row(
                    ui,
                    &format!("Fan {}", index + 1),
                    &self
                        .status
                        .as_ref()
                        .and_then(|s| s.fans[index])
                        .map(|v| format!("{v} RPM"))
                        .unwrap_or_else(|| "Unavailable".to_owned()),
                );
            }
        });
        ui.add_space(18.0);
        ui.heading("Diagnostics");
        ui.add_space(8.0);
        card(ui, |ui| {
            let mut report = self.diagnostic_report();
            ui.horizontal(|ui| {
                if ui.button("Copy diagnostic report").clicked() {
                    ui.ctx().copy_text(report.clone());
                    self.action_message =
                        Some((true, "Diagnostic report copied to clipboard.".to_owned()));
                }
                ui.label(
                    RichText::new(
                        "Contains status values only; no secrets or direct sysfs access.",
                    )
                    .small()
                    .color(MUTED),
                );
            });
            ui.add_space(8.0);
            ui.add(
                egui::TextEdit::multiline(&mut report)
                    .font(egui::TextStyle::Monospace)
                    .desired_rows(14)
                    .interactive(false),
            );
        });
    }

    fn diagnostic_report(&self) -> String {
        let Some(status) = &self.status else {
            return format!(
                "AORUS Control\nD-Bus: {DESTINATION}\nStatus: unavailable\nError: {}",
                self.connection_error.as_deref().unwrap_or("connecting")
            );
        };
        format!(
            "AORUS Control diagnostic report\nD-Bus: {DESTINATION}\nDaemon mode: {}\nDriver available: {:?}\nPower profile: {}\nFan mode: {}\nEC temperatures (m°C): {:?}\nFans (RPM): {:?}\nCharge mode: {:?}\nCharge limit: {:?}\nBattery cycles: {:?}\nGPU boost: {:?}\nGPU boost values: {:?}\nFan modes: {:?}\nFan curve points: {:?}\nCharge mode/limit capabilities: {:?}/{:?}\nUSB S3/S4 capabilities: {:?}/{:?}\nGraphics mode: {:?}\nGraphics power: {:?}\nUSB S3/S4: {:?}/{:?}\nCustom curve available: {:?}\nProduct: {:?} {:?}\nBIOS: {:?} ({:?})\nKernel: {:?}\nDriver module: {:?}\nPlatform path: {:?}\nHwmon path: {:?}\nLast error: {}\nUnknown status keys: {:?}",
            status.daemon_mode.as_deref().unwrap_or("unknown"),
            status.driver_available,
            status.power_profile.as_deref().unwrap_or("unknown"),
            status.fan_mode.map(FanMode::label).unwrap_or("unknown"),
            status.temperatures,
            status.fans,
            status.charge_mode,
            status.charge_limit,
            status.battery_cycles,
            status.gpu_boost,
            status.gpu_boost_values,
            status.fan_modes,
            status.fan_curve_points,
            status.charge_mode_supported,
            status.charge_limit_supported,
            status.usb_charge_s3_supported,
            status.usb_charge_s4_supported,
            status.graphics_mode,
            status.graphics_power,
            status.usb_charge_s3,
            status.usb_charge_s4,
            status.custom_curve_available,
            status.product_name,
            status.product_version,
            status.bios_version,
            status.bios_date,
            status.kernel_release,
            status.driver_module_version,
            status.platform_path,
            status.hwmon_path,
            status.last_error.as_deref().unwrap_or("none"),
            status.unknown_keys,
        )
    }

    fn write_disabled_reason(&self) -> &'static str {
        if self.action_in_flight.is_some() {
            "A hardware action is already in progress."
        } else if self.status.as_ref().and_then(|s| s.daemon_mode.as_deref()) == Some("shadow") {
            "Controls are disabled in shadow mode; the Python service remains authoritative."
        } else {
            "Controls require a connected write-enabled daemon."
        }
    }
}

impl eframe::App for AorusApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.drain_events();
        self.show_sidebar(ui);
        self.show_top_bar(ui);
        self.show_content(ui);
        ui.ctx().request_repaint_after(Duration::from_millis(500));
    }
}

fn spawn_dbus_worker(command_rx: Receiver<Command>, event_tx: Sender<WorkerEvent>, ctx: Context) {
    thread::Builder::new()
        .name("aorus-dbus".to_owned())
        .spawn(move || {
            refresh_status(&event_tx);
            refresh_curve(&event_tx);
            refresh_profile_mappings(&event_tx);
            ctx.request_repaint();

            let mut next_refresh = Instant::now() + TELEMETRY_INTERVAL;
            loop {
                let timeout = next_refresh.saturating_duration_since(Instant::now());
                match command_rx.recv_timeout(timeout) {
                    Ok(Command::Refresh) => refresh_status(&event_tx),
                    Ok(Command::RefreshCurve) => refresh_curve(&event_tx),
                    Ok(command) => {
                        let action = command.label();
                        let reload_curve = matches!(&command, Command::SetFanCurve(_));
                        let reload_mappings = matches!(&command, Command::SetProfileMappings(_));
                        let _ = event_tx.send(WorkerEvent::ActionStarted(action));
                        ctx.request_repaint();
                        let result = perform_command(command);
                        let succeeded = result.is_ok();
                        let _ = event_tx.send(WorkerEvent::ActionFinished { action, result });
                        refresh_status(&event_tx);
                        if succeeded && reload_curve {
                            refresh_curve(&event_tx);
                        }
                        if succeeded && reload_mappings {
                            refresh_profile_mappings(&event_tx);
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => refresh_status(&event_tx),
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
                next_refresh = Instant::now() + TELEMETRY_INTERVAL;
                ctx.request_repaint();
            }
        })
        .expect("failed to start D-Bus worker");
}

fn refresh_status(event_tx: &Sender<WorkerEvent>) {
    match fetch_status() {
        Ok(status) => {
            let _ = event_tx.send(WorkerEvent::Snapshot {
                status: Box::new(status),
                received_at: Instant::now(),
            });
        }
        Err(error) => {
            let _ = event_tx.send(WorkerEvent::ConnectionError(error));
        }
    }
}

fn refresh_curve(event_tx: &Sender<WorkerEvent>) {
    let _ = event_tx.send(WorkerEvent::Curve(fetch_curve()));
}

fn refresh_profile_mappings(event_tx: &Sender<WorkerEvent>) {
    let _ = event_tx.send(WorkerEvent::ProfileMappings(fetch_profile_mappings()));
}

fn with_proxy<T>(
    f: impl FnOnce(&zbus::blocking::Proxy<'_>) -> Result<T, String>,
) -> Result<T, String> {
    let connection = ConnectionBuilder::system()
        .map_err(display_dbus_error)?
        .method_timeout(DBUS_METHOD_TIMEOUT)
        .build()
        .map_err(display_dbus_error)?;
    let proxy = zbus::blocking::Proxy::new(&connection, DESTINATION, PATH, INTERFACE)
        .map_err(display_dbus_error)?;
    f(&proxy)
}

fn fetch_status() -> Result<Status, String> {
    with_proxy(|proxy| {
        let values: HashMap<String, OwnedValue> =
            proxy.call("GetStatus", &()).map_err(display_dbus_error)?;
        Ok(status_from_wire(values))
    })
}

fn fetch_curve() -> Result<[CurvePoint; CURVE_POINTS], String> {
    with_proxy(|proxy| {
        let points: Vec<(u8, u8)> = proxy.call("GetFanCurve", &()).map_err(display_dbus_error)?;
        curve_from_wire(points)
    })
}

fn fetch_profile_mappings() -> Result<HashMap<String, FanMode>, String> {
    with_proxy(|proxy| {
        let mappings: HashMap<String, u8> = proxy
            .call("GetProfileMappings", &())
            .map_err(display_dbus_error)?;
        mappings
            .into_iter()
            .map(|(profile, mode)| {
                let fan_mode = FanMode::from_wire(mode)
                    .filter(|mode| FanMode::SELECTABLE.contains(mode))
                    .ok_or_else(|| {
                        format!("daemon returned invalid fan mode {mode} for {profile}")
                    })?;
                Ok((profile, fan_mode))
            })
            .collect()
    })
}

fn perform_command(command: Command) -> Result<(), String> {
    with_proxy(|proxy| match command {
        Command::Refresh | Command::RefreshCurve => Ok(()),
        Command::SetPowerProfile(profile) => proxy
            .call::<_, _, ()>("SetPowerProfile", &profile)
            .map_err(display_dbus_error),
        Command::SetFanMode(mode) => proxy
            .call::<_, _, ()>("SetFanMode", &(mode as u8))
            .map_err(display_dbus_error),
        Command::ReapplyFanProfile => proxy
            .call::<_, _, ()>("ReapplyFanProfile", &())
            .map_err(display_dbus_error),
        Command::SetFanCurve(curve) => {
            let wire: Vec<(u8, u8)> = curve
                .into_iter()
                .map(|p| (p.temperature, p.speed))
                .collect();
            proxy
                .call::<_, _, ()>("SetFanCurve", &wire)
                .map_err(display_dbus_error)
        }
        Command::SetProfileMappings(mappings) => {
            let wire: HashMap<String, u8> = mappings
                .into_iter()
                .map(|(profile, mode)| (profile, mode as u8))
                .collect();
            proxy
                .call::<_, _, ()>("SetProfileMappings", &wire)
                .map_err(display_dbus_error)
        }
        Command::SetChargeMode(mode) => proxy
            .call::<_, _, ()>("SetChargeMode", &mode)
            .map_err(display_dbus_error),
        Command::SetChargeLimit(limit) => proxy
            .call::<_, _, ()>("SetChargeLimit", &limit)
            .map_err(display_dbus_error),
        Command::SetGpuBoost(boost) => proxy
            .call::<_, _, ()>("SetGpuBoost", &boost)
            .map_err(display_dbus_error),
    })
}

fn status_from_wire(mut values: HashMap<String, OwnedValue>) -> Status {
    let mut status = Status {
        daemon_mode: take::<String>(&mut values, "daemon_mode"),
        driver_available: take::<bool>(&mut values, "driver_available"),
        power_profile: take::<String>(&mut values, "power_profile"),
        fan_mode: take::<u8>(&mut values, "fan_mode").and_then(FanMode::from_wire),
        temperatures: [
            take::<i32>(&mut values, "temp1_millicelsius"),
            take::<i32>(&mut values, "temp2_millicelsius"),
            take::<i32>(&mut values, "temp3_millicelsius"),
        ],
        fans: [
            take::<u32>(&mut values, "fan1_rpm"),
            take::<u32>(&mut values, "fan2_rpm"),
            take::<u32>(&mut values, "fan3_rpm"),
            take::<u32>(&mut values, "fan4_rpm"),
        ],
        charge_mode: take::<u8>(&mut values, "charge_mode"),
        charge_limit: take::<u8>(&mut values, "charge_limit_percent"),
        battery_cycles: take::<u32>(&mut values, "battery_cycles"),
        gpu_boost: take::<u8>(&mut values, "gpu_boost"),
        usb_charge_s3: take::<bool>(&mut values, "usb_charge_s3"),
        usb_charge_s4: take::<bool>(&mut values, "usb_charge_s4"),
        graphics_mode: take::<String>(&mut values, "graphics_mode"),
        graphics_power: take::<bool>(&mut values, "graphics_power"),
        custom_curve_available: take::<bool>(&mut values, "custom_curve_available"),
        fan_modes: take::<Vec<u8>>(&mut values, "cap_fan_modes")
            .unwrap_or_default()
            .into_iter()
            .filter_map(FanMode::from_wire)
            .collect(),
        fan_curve_points: take::<u8>(&mut values, "cap_fan_curve_points"),
        charge_mode_supported: take::<bool>(&mut values, "cap_charge_mode"),
        charge_limit_supported: take::<bool>(&mut values, "cap_charge_limit"),
        usb_charge_s3_supported: take::<bool>(&mut values, "cap_usb_charge_s3"),
        usb_charge_s4_supported: take::<bool>(&mut values, "cap_usb_charge_s4"),
        gpu_boost_values: take::<Vec<u8>>(&mut values, "cap_gpu_boost_values").unwrap_or_default(),
        product_name: take::<String>(&mut values, "product_name"),
        product_version: take::<String>(&mut values, "product_version"),
        bios_version: take::<String>(&mut values, "bios_version"),
        bios_date: take::<String>(&mut values, "bios_date"),
        kernel_release: take::<String>(&mut values, "kernel_release"),
        driver_module_version: take::<String>(&mut values, "driver_module_version"),
        platform_path: take::<String>(&mut values, "platform_path"),
        hwmon_path: take::<String>(&mut values, "hwmon_path"),
        last_error: take::<String>(&mut values, "last_error"),
        unknown_keys: Vec::new(),
    };
    status.unknown_keys = values.into_keys().collect();
    status.unknown_keys.sort();
    status
}

fn take<T>(values: &mut HashMap<String, OwnedValue>, key: &str) -> Option<T>
where
    T: TryFrom<OwnedValue>,
{
    values.remove(key).and_then(|value| T::try_from(value).ok())
}

fn curve_from_wire(points: Vec<(u8, u8)>) -> Result<[CurvePoint; CURVE_POINTS], String> {
    let points: [CurvePoint; CURVE_POINTS] = points
        .into_iter()
        .map(|(temperature, speed)| CurvePoint { temperature, speed })
        .collect::<Vec<_>>()
        .try_into()
        .map_err(|points: Vec<_>| {
            format!(
                "expected {CURVE_POINTS} curve points, received {}",
                points.len()
            )
        })?;
    if points
        .windows(2)
        .any(|pair| pair[0].temperature > pair[1].temperature || pair[0].speed > pair[1].speed)
    {
        return Err("daemon returned a non-monotonic fan curve".to_owned());
    }
    Ok(points)
}

fn display_dbus_error(error: zbus::Error) -> String {
    let message = error.to_string();
    if message.contains("ServiceUnknown") || message.contains("NameHasNoOwner") {
        "aorusd is not running on the system bus".to_owned()
    } else {
        message
    }
}

fn default_curve() -> [CurvePoint; CURVE_POINTS] {
    std::array::from_fn(|index| CurvePoint {
        temperature: (20 + index * 5).min(100) as u8,
        speed: (38 + index * 14).min(255) as u8,
    })
}

fn nearest_curve_point(curve: &[CurvePoint; CURVE_POINTS], pointer: PlotPoint) -> Option<usize> {
    curve
        .iter()
        .enumerate()
        .map(|(index, point)| {
            let dx = (point.temperature as f64 - pointer.x) / 100.0;
            let dy = (point.speed as f64 - pointer.y) / 255.0;
            (index, dx * dx + dy * dy)
        })
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .filter(|(_, distance)| *distance <= 0.0035)
        .map(|(index, _)| index)
}

fn configure_style(ctx: &Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = Color32::from_rgb(18, 20, 26);
    visuals.window_fill = Color32::from_rgb(25, 28, 36);
    visuals.extreme_bg_color = Color32::from_rgb(14, 16, 21);
    visuals.faint_bg_color = Color32::from_rgb(31, 35, 44);
    visuals.selection.bg_fill = BLUE.gamma_multiply(0.45);
    visuals.widgets.inactive.corner_radius = 6.0.into();
    visuals.widgets.hovered.corner_radius = 6.0.into();
    visuals.widgets.active.corner_radius = 6.0.into();
    ctx.set_visuals(visuals);
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = Vec2::new(10.0, 8.0);
        style.spacing.button_padding = Vec2::new(12.0, 7.0);
        style
            .text_styles
            .insert(egui::TextStyle::Heading, FontId::proportional(22.0));
    });
}

fn card(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui)) {
    Frame::new()
        .fill(Color32::from_rgb(25, 28, 36))
        .stroke(Stroke::new(1.0, Color32::from_rgb(45, 50, 61)))
        .corner_radius(10.0)
        .inner_margin(Margin::same(16))
        .show(ui, body);
}
fn metric_card(ui: &mut egui::Ui, label: &str, value: &str, detail: &str, color: Color32) {
    card(ui, |ui| {
        ui.label(RichText::new(label).color(MUTED));
        ui.label(RichText::new(value).size(30.0).strong().color(color));
        ui.label(RichText::new(detail).small().color(MUTED));
    });
}
fn banner(ui: &mut egui::Ui, color: Color32, message: &str) {
    Frame::new()
        .fill(color.gamma_multiply(0.12))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.5)))
        .corner_radius(7.0)
        .inner_margin(Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("●").color(color));
                ui.label(message);
            });
        });
}
fn readonly_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(MUTED));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(value).strong());
        });
    });
    ui.separator();
}
fn profile_badge(text: &str, color: Color32) -> RichText {
    RichText::new(format!("  {text}  ")).strong().color(color)
}
fn title_case(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}
fn on_off(value: bool) -> &'static str {
    if value { "On" } else { "Off" }
}
fn compact(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_owned()
    } else {
        format!(
            "{}…",
            text.chars()
                .take(max_chars.saturating_sub(1))
                .collect::<String>()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn curve_wire_requires_fifteen_monotonic_points() {
        let valid: Vec<_> = (0..CURVE_POINTS)
            .map(|i| ((i * 5) as u8, (i * 10) as u8))
            .collect();
        assert!(curve_from_wire(valid.clone()).is_ok());
        assert!(curve_from_wire(valid[..14].to_vec()).is_err());
        let mut invalid = valid;
        invalid[8].1 = 1;
        assert!(curve_from_wire(invalid).is_err());
    }
    #[test]
    fn nearest_point_has_a_bounded_hit_target() {
        let curve = default_curve();
        let point = curve[4];
        assert_eq!(
            nearest_curve_point(&curve, PlotPoint::new(point.temperature, point.speed)),
            Some(4)
        );
        assert_eq!(
            nearest_curve_point(&curve, PlotPoint::new(1000.0, 1000.0)),
            None
        );
    }
}
