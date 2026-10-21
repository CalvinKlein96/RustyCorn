//! The Settings page. Every change is saved at
//! once; changes of the data source (demo mode, serial port) restart the
//! acquisition, the others apply to new data, the next log file or new pages,
//! as each row explains. The methods folder applies at once: the link thread
//! reads `AppState::method_dir` for every method request, so no restart.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use egui::{RichText, Ui};
use rc_core::settings::{self, ALL_PANELS, REFRESH_HZ, REFRESH_OPTIONS, VOLUME_UNIT_OPTIONS};
use rc_protocol::Acquisition;
use rc_theme::{FS_BODY, FS_SMALL, mono, palette};

use crate::ports::{PortInfo, list_ports};
use crate::widgets::{feedback, form_card, page, page_header, row_separator, setting_row, toggle};

const FIELD_WIDTH: f32 = 230.0;
const CONTROL_WIDTH: f32 = 300.0;

/// Label of a refresh rate; an unknown rate shows as the default (2× per second).
pub fn refresh_label(hz: f64) -> &'static str {
    REFRESH_HZ
        .iter()
        .position(|&h| h == hz)
        .map_or(REFRESH_OPTIONS[1], |i| REFRESH_OPTIONS[i])
}

/// The volume unit choice shown for `unit_ul`: 1 µL is the first option, any
/// other value the second (10 µL).
pub fn volume_unit_index(unit_ul: f64) -> usize {
    if unit_ul == VOLUME_UNIT_OPTIONS[0].0 {
        0
    } else {
        1
    }
}

/// "%B, Cond" or "UV only".
pub fn panels_text(panels: &[String]) -> String {
    if panels.is_empty() {
        "UV only".to_string()
    } else {
        panels.join(", ")
    }
}

pub struct SettingsPage {
    port_text: String,
    prefix_text: String,
    /// the data folder as shown (Settings::data_dir creates it, so not every frame)
    folder_shown: PathBuf,
    /// the methods folder as shown (likewise)
    methods_shown: PathBuf,
    feedback: String,
    feedback_is_error: bool,
    ports: Vec<PortInfo>,
    ports_job: Option<Receiver<Vec<PortInfo>>>,
    /// a folder dialog open for this folder
    folder_job: Option<(Folder, Receiver<Option<PathBuf>>)>,
}

/// The folders chosen with a dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Folder {
    Data,
    Methods,
}

impl SettingsPage {
    pub fn new(acq: &Acquisition) -> SettingsPage {
        let s = &acq.settings;
        let folder = s.data_dir();
        SettingsPage {
            port_text: s.serial_port.clone(),
            prefix_text: s.log_prefix.clone(),
            methods_shown: s.methods_dir_in(&folder),
            folder_shown: folder,
            feedback: String::new(),
            feedback_is_error: false,
            ports: Vec::new(),
            ports_job: None,
            folder_job: None,
        }
    }

    /// Save, restart the source if asked, and say what happened.
    fn changed(&mut self, acq: &mut Acquisition, msg: String, restart: bool) {
        if let Err(err) = settings::save_settings(&acq.settings) {
            log::error!("Could not save the settings: {err}");
            self.feedback = format!("Could not save the settings: {err}");
            self.feedback_is_error = true;
        } else {
            self.feedback = msg;
            self.feedback_is_error = false;
        }
        if restart {
            acq.restart();
        }
    }

    /// Results of the background jobs (port list, folder dialog); call every frame.
    pub fn poll(&mut self, acq: &mut Acquisition) {
        if let Some(rx) = &self.ports_job {
            match rx.try_recv() {
                Ok(ports) => {
                    self.ports = ports;
                    self.ports_job = None;
                }
                Err(TryRecvError::Disconnected) => self.ports_job = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        if let Some((which, rx)) = &self.folder_job {
            let which = *which;
            match rx.try_recv() {
                Ok(dir) => {
                    self.folder_job = None;
                    match (which, dir) {
                        (Folder::Data, Some(dir)) => self.set_data_folder(acq, dir),
                        (Folder::Methods, Some(dir)) => {
                            self.set_methods_folder(acq, dir.to_string_lossy().into_owned())
                        }
                        (_, None) => {}
                    }
                }
                Err(TryRecvError::Disconnected) => self.folder_job = None,
                Err(TryRecvError::Empty) => {}
            }
        }
    }

    fn refresh_ports(&mut self, ctx: &egui::Context) {
        if self.ports_job.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("list ports".into())
            .spawn(move || {
                let _ = tx.send(list_ports());
                ctx.request_repaint();
            });
        if spawned.is_ok() {
            self.ports_job = Some(rx);
        }
    }

    /// The folder dialog, off the GUI thread (it blocks until closed).
    fn choose_folder(&mut self, ctx: &egui::Context, which: Folder) {
        if self.folder_job.is_some() {
            return;
        }
        let (start, title) = match which {
            Folder::Data => (self.folder_shown.clone(), "Data folder"),
            Folder::Methods => (self.methods_shown.clone(), "Methods folder"),
        };
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("folder dialog".into())
            .spawn(move || {
                let dir = rfd::FileDialog::new()
                    .set_title(title)
                    .set_directory(&start)
                    .pick_folder();
                let _ = tx.send(dir);
                ctx.request_repaint();
            });
        match spawned {
            Ok(_) => self.folder_job = Some((which, rx)),
            Err(err) => log::error!("Could not open the folder dialog: {err}"),
        }
    }

    fn set_data_folder(&mut self, acq: &mut Acquisition, dir: PathBuf) {
        let dir_text = dir.to_string_lossy().into_owned();
        if dir_text.is_empty() || dir_text == acq.settings.data_dir {
            return;
        }
        acq.settings.data_dir = dir_text;
        let folder = acq.settings.data_dir();
        // the default methods folder is in the data folder and moves with it
        let follows = acq.settings.methods_dir_is_default();
        let methods = acq.settings.methods_dir_in(&folder);
        if follows {
            rc_methods::install_example_methods_logged(&methods);
        }
        {
            let mut app = acq.app.lock();
            app.log_dir = folder.clone();
            if follows {
                app.method_dir = methods.clone();
                app.methods_changed += 1;
            }
            app.touch();
        }
        self.folder_shown = folder.clone();
        self.methods_shown = methods.clone();
        let msg = if follows {
            format!(
                "Data folder set to {}; the next log file goes there. The methods folder moved with it: {}.",
                folder.display(),
                methods.display()
            )
        } else {
            format!(
                "Data folder set to {}; the next log file goes there.",
                folder.display()
            )
        };
        self.changed(acq, msg, false);
    }

    /// A methods folder of its own (`dir`), or "" for `<data folder>/methods`.
    /// Applies at once: the method editor and the Akta's next save or load use it.
    fn set_methods_folder(&mut self, acq: &mut Acquisition, dir: String) {
        if dir == acq.settings.methods_dir {
            return;
        }
        acq.settings.methods_dir = dir;
        let methods = acq.settings.methods_dir_in(&self.folder_shown);
        rc_methods::install_example_methods_logged(&methods);
        {
            let mut app = acq.app.lock();
            app.method_dir = methods.clone();
            app.methods_changed += 1;
            app.touch();
        }
        self.methods_shown = methods.clone();
        let msg = if acq.settings.methods_dir_is_default() {
            format!(
                "Methods folder back to the default in the data folder: {}.",
                methods.display()
            )
        } else {
            format!(
                "Methods folder set to {}; the Akta saves and loads methods there from now on.",
                methods.display()
            )
        };
        self.changed(acq, msg, false);
    }

    fn apply_port(&mut self, acq: &mut Acquisition, text: &str) {
        let port = text.trim().to_string();
        self.port_text = port.clone();
        if port.is_empty() || port == acq.settings.serial_port {
            self.port_text = acq.settings.serial_port.clone();
            return;
        }
        acq.settings.serial_port = port.clone();
        let restart = !acq.settings.demo_mode;
        let msg = format!(
            "Serial port set to {port}.{}",
            if restart { " Reconnecting." } else { "" }
        );
        self.changed(acq, msg, restart);
    }

    fn apply_prefix(&mut self, acq: &mut Acquisition) {
        let prefix = self.prefix_text.trim().to_string();
        if prefix.is_empty() || prefix == acq.settings.log_prefix {
            self.prefix_text = acq.settings.log_prefix.clone();
            return;
        }
        self.prefix_text = prefix.clone();
        acq.settings.log_prefix = prefix.clone();
        {
            let mut app = acq.app.lock();
            app.log_prefix = prefix.clone();
            app.touch();
        }
        self.changed(
            acq,
            format!("Log prefix set to \"{prefix}\"; the next log file will use it."),
            false,
        );
    }

    pub fn ui(&mut self, ui: &mut Ui, acq: &mut Acquisition) {
        let t = palette(ui.ctx());
        page(ui, "settings-page", |ui| {
            page_header(
                ui,
                "Settings",
                "Every change is saved at once; changing the data source reconnects.",
            );
            ui.add_space(8.0);
            let color = if self.feedback_is_error { t.err } else { t.ok };
            feedback(ui, &self.feedback, color);
            ui.add_space(14.0);

            form_card(ui, "Instrument", |ui| self.instrument_rows(ui, acq));
            ui.add_space(16.0);
            form_card(ui, "Logging", |ui| self.logging_rows(ui, acq));
            ui.add_space(16.0);
            form_card(ui, "Folders", |ui| self.folder_rows(ui, acq));
            ui.add_space(16.0);
            form_card(ui, "Display", |ui| self.display_rows(ui, acq));

            ui.add_space(14.0);
            ui.label(
                RichText::new(format!(
                    "Settings file: {}",
                    settings::settings_path().display()
                ))
                .font(egui::FontId::proportional(FS_SMALL))
                .color(t.text_faint),
            );
        });
    }

    fn instrument_rows(&mut self, ui: &mut Ui, acq: &mut Acquisition) {
        let t = palette(ui.ctx());
        let mut demo = acq.settings.demo_mode;
        let r = setting_row(
            ui,
            "Demo mode",
            "Simulated data for testing; no instrument needed.",
            60.0,
            |ui| toggle(ui, &mut demo),
        );
        if r.changed() && demo != acq.settings.demo_mode {
            acq.settings.demo_mode = demo;
            let msg = if demo {
                "Demo mode on — showing simulated data.".to_string()
            } else {
                format!(
                    "Demo mode off — connecting to {}.",
                    acq.settings.serial_port
                )
            };
            self.changed(acq, msg, true);
        }
        row_separator(ui);

        let note = "Press Enter to apply, e.g. /dev/ttyUSB0 (Linux) or COM3 (Windows), \
                    or pick a port from the list.";
        let mut picked: Option<String> = None;
        let mut submitted = false;
        setting_row(ui, "Serial port", note, CONTROL_WIDTH + 40.0, |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let list = ui.menu_button(
                RichText::new("⏷")
                    .font(egui::FontId::proportional(14.0))
                    .color(t.text_dim),
                |ui| {
                    ui.set_min_width(300.0);
                    if self.ports_job.is_some() {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(RichText::new("Looking for serial ports…").color(t.text_dim));
                        });
                    } else if self.ports.is_empty() {
                        ui.label(RichText::new("No serial ports found").color(t.text_dim));
                    } else {
                        for p in &self.ports {
                            let current = p.name == acq.settings.serial_port;
                            let label = RichText::new(&p.name).font(mono(12.0));
                            let resp = ui.selectable_label(current, label);
                            let resp = if p.description.is_empty() {
                                resp
                            } else {
                                resp.on_hover_text(&p.description)
                            };
                            if !p.description.is_empty() {
                                ui.label(
                                    RichText::new(&p.description)
                                        .font(egui::FontId::proportional(FS_SMALL))
                                        .color(t.text_faint),
                                );
                            }
                            if resp.clicked() {
                                picked = Some(p.name.clone());
                                ui.close();
                            }
                        }
                    }
                },
            );
            if list.response.clicked() {
                self.refresh_ports(ui.ctx());
            }
            list.response.on_hover_text("Serial ports of this computer");
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.port_text)
                    .desired_width(FIELD_WIDTH)
                    .font(mono(12.5)),
            );
            submitted = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if !field.has_focus() && self.port_text.trim() != acq.settings.serial_port {
                ui.label(
                    RichText::new("not applied")
                        .font(egui::FontId::proportional(FS_SMALL))
                        .color(t.warn),
                )
                .on_hover_text("Press Enter in the field to apply the port");
            }
        });
        if let Some(port) = picked {
            self.apply_port(acq, &port);
        } else if submitted {
            let text = self.port_text.clone();
            self.apply_port(acq, &text);
        }
        row_separator(ui);

        let mut unit_idx = volume_unit_index(acq.settings.volume_unit_ul);
        let before = unit_idx;
        setting_row(
            ui,
            "Volume unit",
            "Applies to data received from now on.",
            CONTROL_WIDTH,
            |ui| {
                egui::ComboBox::from_id_salt("volume-unit")
                    .width(FIELD_WIDTH + 30.0)
                    .selected_text(VOLUME_UNIT_OPTIONS[unit_idx].1)
                    .show_ui(ui, |ui| {
                        for (i, (_, label)) in VOLUME_UNIT_OPTIONS.iter().enumerate() {
                            ui.selectable_value(&mut unit_idx, i, *label);
                        }
                    });
            },
        );
        if unit_idx != before {
            let unit = VOLUME_UNIT_OPTIONS[unit_idx].0;
            if acq.settings.volume_unit_ul != unit {
                acq.settings.volume_unit_ul = unit;
                {
                    let mut app = acq.app.lock();
                    app.volume_unit_ul = unit;
                    app.touch();
                }
                self.changed(
                    acq,
                    format!("Volume unit set to {unit} µL per count."),
                    false,
                );
            }
        }
    }

    fn logging_rows(&mut self, ui: &mut Ui, acq: &mut Acquisition) {
        let t = palette(ui.ctx());
        let note = "Press Enter to apply. Used for the next log file: <prefix>_<date>_<time>.csv";
        let mut submitted = false;
        setting_row(ui, "Log file prefix", note, CONTROL_WIDTH, |ui| {
            let field = ui
                .add(egui::TextEdit::singleline(&mut self.prefix_text).desired_width(FIELD_WIDTH));
            submitted = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if !field.has_focus() && self.prefix_text.trim() != acq.settings.log_prefix {
                ui.label(
                    RichText::new("not applied")
                        .font(egui::FontId::proportional(FS_SMALL))
                        .color(t.warn),
                )
                .on_hover_text("Press Enter in the field to apply the prefix");
            }
        });
        if submitted {
            self.apply_prefix(acq);
        }
        row_separator(ui);

        let mut raw = acq.settings.save_raw;
        let note = "For troubleshooting: also saves every instrument message (_raw.txt) and method dumps (.met). \
                    From the next log file on.";
        let r = setting_row(ui, "Save raw instrument data", note, 60.0, |ui| {
            toggle(ui, &mut raw)
        });
        if r.changed() && raw != acq.settings.save_raw {
            acq.settings.save_raw = raw;
            let msg = format!(
                "Raw instrument data will {}be saved, from the next log file on.",
                if raw { "" } else { "not " }
            );
            self.changed(acq, msg, false);
        }
    }

    fn folder_rows(&mut self, ui: &mut Ui, acq: &mut Acquisition) {
        let busy = self.folder_job.as_ref().map(|(which, _)| *which);
        let note = "CSV logs, raw captures (_raw.txt), method dumps (.met) and exported PDFs. \
                    From the next log file on.";
        let mut choose = None;
        setting_row(ui, "Data folder", note, CONTROL_WIDTH, |ui| {
            if ui
                .add_enabled(busy.is_none(), egui::Button::new("Choose…"))
                .clicked()
            {
                choose = Some(Folder::Data);
            }
            if busy == Some(Folder::Data) {
                ui.spinner();
            }
        });
        path_box(ui, &self.folder_shown, "");
        ui.add_space(4.0);
        row_separator(ui);

        let note = "The methods the Akta saves to and loads from this PC: PrimeMeth_NNN.txt, names.txt \
                    and the run setups. Applies at once, also while connected. Unless chosen, \
                    the methods folder in the data folder.";
        let default = acq.settings.methods_dir_is_default();
        let mut use_default = false;
        setting_row(ui, "Methods folder", note, CONTROL_WIDTH, |ui| {
            if ui
                .add_enabled(busy.is_none(), egui::Button::new("Choose…"))
                .clicked()
            {
                choose = Some(Folder::Methods);
            }
            if !default {
                use_default = ui
                    .add_enabled(busy.is_none(), egui::Button::new("Use default"))
                    .on_hover_text("Back to the methods folder in the data folder")
                    .clicked();
            }
            if busy == Some(Folder::Methods) {
                ui.spinner();
            }
        });
        path_box(
            ui,
            &self.methods_shown,
            if default {
                "default: in the data folder"
            } else {
                ""
            },
        );
        ui.add_space(4.0);
        if let Some(which) = choose {
            self.choose_folder(ui.ctx(), which);
        }
        if use_default {
            self.set_methods_folder(acq, String::new());
        }
    }

    fn display_rows(&mut self, ui: &mut Ui, acq: &mut Acquisition) {
        let mut dark = acq.settings.dark_mode;
        let r = setting_row(
            ui,
            "Dark mode",
            "Dark or light window colours; applies at once.",
            60.0,
            |ui| toggle(ui, &mut dark),
        );
        if r.changed() && dark != acq.settings.dark_mode {
            acq.settings.dark_mode = dark;
            rc_theme::apply(ui.ctx(), dark);
            self.changed(
                acq,
                format!("{} mode saved.", if dark { "Dark" } else { "Light" }),
                false,
            );
        }
        row_separator(ui);

        let note = "How often live plots redraw. Lower is lighter on the computer; data is recorded in full either way.";
        let current = refresh_label(acq.settings.refresh_hz);
        let mut chosen: Option<usize> = None;
        setting_row(ui, "Display refresh", note, CONTROL_WIDTH, |ui| {
            egui::ComboBox::from_id_salt("display-refresh")
                .width(FIELD_WIDTH + 30.0)
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for (i, label) in REFRESH_OPTIONS.iter().enumerate() {
                        if ui.selectable_label(*label == current, *label).clicked() {
                            chosen = Some(i);
                        }
                    }
                });
        });
        if let Some(i) = chosen {
            let hz = REFRESH_HZ[i];
            if acq.settings.refresh_hz != hz {
                acq.settings.refresh_hz = hz;
                self.changed(
                    acq,
                    format!("Display refresh set to {}.", REFRESH_OPTIONS[i]),
                    false,
                );
            }
        }
        row_separator(ui);

        let note =
            "Plots switched on when the program starts or a run is opened (UV is always shown).";
        let mut toggled = false;
        setting_row(ui, "Panels shown at start", note, 340.0, |ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            // right-to-left layout: add in reverse so they read in order
            for name in ALL_PANELS.iter().rev() {
                let mut on = acq.settings.start_panels.iter().any(|p| p == name);
                if ui
                    .checkbox(
                        &mut on,
                        RichText::new(*name).font(egui::FontId::proportional(FS_BODY)),
                    )
                    .changed()
                {
                    toggled = true;
                    let mut set: Vec<&str> = acq
                        .settings
                        .start_panels
                        .iter()
                        .map(String::as_str)
                        .collect();
                    if on {
                        set.push(name);
                    } else {
                        set.retain(|p| p != name);
                    }
                    acq.settings.start_panels = ALL_PANELS
                        .iter()
                        .filter(|p| set.contains(p))
                        .map(|p| p.to_string())
                        .collect();
                }
            }
        });
        if toggled {
            let msg = format!(
                "Panels shown at start: {}.",
                panels_text(&acq.settings.start_panels)
            );
            self.changed(acq, msg, false);
        }
    }
}

/// A folder in use, in full, below its row; `tag` (e.g. "default") at the right.
fn path_box(ui: &mut Ui, path: &std::path::Path, tag: &str) {
    let t = palette(ui.ctx());
    egui::Frame::new()
        .fill(t.field)
        .stroke(egui::Stroke::new(1.0, t.field_border))
        .corner_radius(egui::CornerRadius::same(t.radius_sm as u8))
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let path = path.display().to_string();
            ui.horizontal(|ui| {
                if !tag.is_empty() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(tag)
                                .font(egui::FontId::proportional(FS_SMALL))
                                .color(t.text_faint),
                        );
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&path).font(mono(12.0)).color(t.text),
                                )
                                .truncate(),
                            )
                            .on_hover_text(&path);
                        });
                    });
                } else {
                    ui.add(
                        egui::Label::new(RichText::new(&path).font(mono(12.0)).color(t.text))
                            .truncate(),
                    )
                    .on_hover_text(&path);
                }
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_apply_at_once() {
        // never the user's settings (left pointing here: nothing else in this binary saves)
        let tmp = tempfile::tempdir().unwrap();
        settings::set_config_dir_override(Some(tmp.path().join("config")));
        let data = tmp.path().join("data");
        let s = settings::Settings {
            demo_mode: true,
            data_dir: data.display().to_string(),
            ..Default::default()
        };
        let app = rc_core::AppState::new().shared();
        let mut acq = Acquisition::new(app.clone(), s); // never started
        let mut page = SettingsPage::new(&acq);
        assert_eq!(page.methods_shown, data.join("methods"));

        // a methods folder of its own: the link and the editor use it at once
        let methods = tmp.path().join("methods elsewhere");
        page.set_methods_folder(&mut acq, methods.display().to_string());
        assert_eq!(app.lock().method_dir, methods);
        assert_eq!(app.lock().methods_changed, 1);
        // a new methods folder gets the example method
        let example = vec![rc_methods::EXAMPLE_METHOD_NUMBER];
        assert_eq!(rc_methods::stored_methods(&methods), example);
        assert!(
            methods.is_dir() && !page.feedback_is_error,
            "{}",
            page.feedback
        );
        assert_eq!(
            settings::load_settings().methods_dir,
            methods.display().to_string()
        );

        // the data folder moves; the chosen methods folder stays
        let data2 = tmp.path().join("data2");
        page.set_data_folder(&mut acq, data2.clone());
        assert_eq!(app.lock().log_dir, data2);
        assert_eq!(app.lock().method_dir, methods);
        assert_eq!(app.lock().methods_changed, 1);

        // back to the default: <data folder>/methods, which moves with the data folder
        page.set_methods_folder(&mut acq, String::new());
        assert_eq!(app.lock().method_dir, data2.join("methods"));
        assert_eq!(app.lock().methods_changed, 2);
        assert_eq!(rc_methods::stored_methods(&data2.join("methods")), example);
        assert_eq!(settings::load_settings().methods_dir, "");
        page.set_data_folder(&mut acq, data.clone());
        assert_eq!(app.lock().method_dir, data.join("methods"));
        assert_eq!(app.lock().methods_changed, 3);
        assert_eq!(rc_methods::stored_methods(&data.join("methods")), example);
        assert!(page.feedback.contains("moved with it"), "{}", page.feedback);
        assert_eq!(
            settings::load_settings().data_dir,
            data.display().to_string()
        );

        // a folder in use is left alone: its deleted method does not come back
        let path = rc_methods::method_path(&methods, example[0]);
        rc_methods::retire_method_file(&path).unwrap();
        page.set_methods_folder(&mut acq, methods.display().to_string());
        assert!(rc_methods::stored_methods(&methods).is_empty());
        page.set_methods_folder(&mut acq, String::new());
    }

    #[test]
    fn refresh_labels() {
        assert_eq!(refresh_label(1.0), "1× per second");
        assert_eq!(refresh_label(10.0), "10× per second");
        assert_eq!(refresh_label(3.0), "2× per second");
    }

    #[test]
    fn volume_units() {
        assert_eq!(volume_unit_index(1.0), 0);
        assert_eq!(volume_unit_index(10.0), 1);
        assert_eq!(volume_unit_index(5.0), 1);
    }

    #[test]
    fn panel_lists() {
        assert_eq!(panels_text(&[]), "UV only");
        assert_eq!(panels_text(&["%B".to_string(), "pH".to_string()]), "%B, pH");
    }
}
