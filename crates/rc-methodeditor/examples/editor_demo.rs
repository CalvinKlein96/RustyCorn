//! Visual check of the method editor: a window with a `MethodEditor` over a
//! temporary copy of a methods folder (the original is never written).
//!
//! cargo run -p rc-methodeditor --example editor_demo -- [methods folder] [--light] [--bad] [--narrow]
//!
//! `--bad` adds a method with problems (as method 1) so the validation shows;
//! `--narrow` opens the window at the app's minimum size.

use std::fs;
use std::path::PathBuf;

use rc_core::AppState;
use rc_methodeditor::MethodEditor;

struct Demo {
    editor: MethodEditor,
    _tmp: tempfile::TempDir,
}

impl eframe::App for Demo {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let t = rc_theme::palette(ui.ctx());
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(t.dock)
                    .inner_margin(egui::Margin::same(20)),
            )
            .show(ui, |ui| self.editor.ui(ui));
    }
}

fn main() -> eframe::Result {
    env_logger::init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let light = args.iter().any(|a| a == "--light");
    let bad = args.iter().any(|a| a == "--bad");
    let narrow = args.iter().any(|a| a == "--narrow");
    let src = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .map(PathBuf::from);

    let tmp = tempfile::tempdir().expect("temp dir");
    let dir = tmp.path().join("methods");
    fs::create_dir_all(&dir).expect("methods dir");
    if let Some(src) = src {
        for entry in fs::read_dir(&src).expect("methods folder").flatten() {
            if entry.path().is_file() {
                fs::copy(entry.path(), dir.join(entry.file_name())).expect("copy");
            }
        }
    }
    if bad {
        let mut m = rc_methods::MethodFile::new_method();
        m.rows.insert(
            1,
            vec!["5.0", "120", "1.0", "0.0", "9", "Inj", "0", "N", "Y"]
                .into_iter()
                .map(String::from)
                .collect(),
        );
        m.rows.insert(
            2,
            vec!["4.0", "50", "0.55", "0.0", "1", "Load", "0", "N", "N"]
                .into_iter()
                .map(String::from)
                .collect(),
        );
        m.set_setting("Max pressure (kPa)", "3000");
        rc_methods::save_method_file(&dir, 1, &m.format_text()).expect("save");
    }
    eprintln!("methods folder: {}", dir.display());

    let mut app = AppState::new();
    app.method_dir = dir;
    app.last_loaded_method = 6;
    let shared = app.shared();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("RustyCorn Editor Demo")
            .with_inner_size(if narrow {
                [1100.0, 680.0]
            } else {
                [1400.0, 860.0]
            }),
        ..Default::default()
    };
    eframe::run_native(
        "RustyCorn Editor Demo",
        options,
        Box::new(move |cc| {
            rc_theme::install_fonts(&cc.egui_ctx);
            rc_theme::apply(&cc.egui_ctx, !light);
            Ok(Box::new(Demo {
                editor: MethodEditor::new(shared),
                _tmp: tmp,
            }))
        }),
    )
}
