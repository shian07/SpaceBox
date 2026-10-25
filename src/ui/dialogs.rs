use fltk::dialog;

pub fn shortcuts() {
    dialog::message_default(
        "Keyboard shortcuts\n\n\
         Arrows            move selection\n\
         Enter             open folder / reveal file\n\
         Backspace         go up one level\n\
         Alt+Left          go up one level\n\
         Delete            move to Trash\n\
         Shift+Delete      delete permanently\n\
         Ctrl+C            copy path\n\
         Ctrl+F            focus search\n\
         Ctrl+O            choose folder\n\
         Ctrl+H            scan home\n\
         Ctrl+R / F5       rescan\n\
         Escape            clear selection / cancel scan\n\n\
         Right-click a box for Open, Reveal, Copy path, Trash and Delete.",
    );
}

pub fn about() {
    dialog::message_default(&format!(
        "Spacebox {}\n\n\
         A compact treemap disk-space browser for Linux, macOS and Windows.\n\
         Written in Rust with FLTK; no webview, no Electron.\n\n\
         Allocated size is on by default and reflects real disk usage.\n\
         Files below 1 MiB are grouped per folder to keep memory low.\n\
         The status bar shows live RSS.",
        env!("CARGO_PKG_VERSION"),
    ));
}
