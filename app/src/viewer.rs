//! `rustycorn run.csv`: a window with just the run page of one log. The data
//! are read-only; run information can be added
//! ("Edit run info"), appended to the end of the file.

use rc_core::SharedApp;
use rc_runview::{RunView, RunViewOptions};

use crate::runinfo_forms::{self, FormAction, InfoForm};

pub struct ViewerApp {
    view: RunView,
    form: Option<InfoForm>,
}

impl ViewerApp {
    pub fn new(app: SharedApp) -> ViewerApp {
        // the viewer shows every panel, whatever the settings say
        let start_panels = rc_core::settings::ALL_PANELS
            .iter()
            .map(|p| p.to_string())
            .collect();
        let mut view = RunView::new(
            app,
            RunViewOptions {
                viewer: true,
                start_panels,
            },
        );
        view.set_edit_run_info(true);
        let mut viewer = ViewerApp { view, form: None };
        // RUSTYCORN_SHOW_PAGES="Edit run info": the form open at start (screenshots, checks)
        if std::env::var("RUSTYCORN_SHOW_PAGES").is_ok_and(|p| {
            p.split(',')
                .any(|n| n.trim() == crate::workspace::EDIT_RUN_INFO)
        }) {
            viewer.open_form();
        }
        viewer
    }

    fn open_form(&mut self) {
        if self.form.is_none() {
            let app = self.view.app.lock();
            self.form = Some(InfoForm::viewer(
                &app.log_filename,
                &runinfo_forms::recorded(&app),
            ));
        }
    }
}

impl eframe::App for ViewerApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        crate::interrupt::close_if_requested(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let t = rc_theme::palette(ui.ctx());
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(t.dock))
            .show(ui, |ui| self.view.ui(ui));
        if self.view.take_edit_request() {
            self.open_form();
        }
        if let Some(form) = self.form.as_mut() {
            match runinfo_forms::show_form(ui.ctx(), form, egui::Id::unique("rc-viewer-runinfo")) {
                FormAction::None => {}
                FormAction::Close => self.form = None,
                FormAction::Save => {
                    // no live recording in this window
                    let result = runinfo_forms::save_viewer(&mut self.view.app.lock(), form, None);
                    match result {
                        Ok(msg) => {
                            log::info!("Run info: {msg}");
                            self.form = None;
                        }
                        Err(err) => form.error = err,
                    }
                }
            }
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        _visuals.panel_fill.to_normalized_gamma_f32()
    }
}
