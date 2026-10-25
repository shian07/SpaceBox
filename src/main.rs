// Hide the console window on Windows release builds. Debug builds keep it so
// panics and eprintln! output stay visible.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod fsops;
mod scan;
mod sys;
mod theme;
mod treemap;
mod ui;

use fltk::{app as flapp, prelude::*, window::Window};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Embedded 256x256 window icon so the taskbar entry is recognisable on Windows
/// and Linux. Generated from `assets/spacebox.svg`.
const ICON_PNG: &[u8] = include_bytes!("../assets/spacebox.png");

fn print_usage() {
    println!(
        "spacebox {VERSION} — disk usage treemap\n\
         \n\
         Usage: spacebox [PATH]\n\
         \n\
         Arguments:\n\
           PATH            Directory to scan (default: your home directory)\n\
         \n\
         Options:\n\
           -h, --help      Print this help\n\
           -V, --version   Print version"
    );
}

fn main() {
    // Handle informational flags before touching the GUI toolkit: these are
    // commonly run from a terminal or a package manager hook and must not
    // require a display.
    let arg = std::env::args_os().nth(1);
    if let Some(arg) = arg.as_deref() {
        let text = arg.to_string_lossy();
        match text.as_ref() {
            "-h" | "--help" => {
                print_usage();
                return;
            }
            "-V" | "--version" => {
                println!("spacebox {VERSION}");
                return;
            }
            _ => {}
        }
    }

    sys::tune_allocator();
    sys::init_fonts();
    let application = flapp::App::default().with_scheme(flapp::Scheme::Gtk);
    fltk::dialog::message_set_hotspot(false);
    theme::apply();

    let root = arg.map(PathBuf::from).unwrap_or_else(app::home_dir);
    let root = std::fs::canonicalize(&root).unwrap_or(root);

    let (sender, receiver) = flapp::channel::<app::Msg>();
    let state = Rc::new(RefCell::new(app::UiState::new(root.clone(), sender)));

    let mut window = Window::new(0, 0, 1280, 800, "Spacebox");
    if let Ok(icon) = fltk::image::PngImage::from_data(ICON_PNG) {
        window.set_icon(Some(icon));
    }
    let ui = ui::build(&mut window, state.clone());
    window.show();
    flapp::set_focus(&window);

    ui::start_timer(state.clone(), ui, receiver);
    app::start_scan(&state, root);

    application.run().unwrap();
}
