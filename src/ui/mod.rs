pub mod breadcrumbs;
pub mod dialogs;
pub mod panel;
pub mod widgets;

use crate::app::{self, Msg, SideMode, UiState};
use crate::scan::Category;
use crate::sys;
use crate::theme;
use crate::treemap::Treemap;
use breadcrumbs::Breadcrumbs;
use fltk::{
    app as flapp,
    button::Button,
    draw,
    enums::{Align, CallbackTrigger, Event, Font, FrameType, Key, Shortcut},
    frame::Frame,
    group::Flex,
    input::Input,
    menu::{MenuBar, MenuFlag},
    prelude::*,
    window::Window,
};
use std::cell::RefCell;
use std::rc::Rc;

const RSS_BUDGET_KB: u64 = 20 * 1024;

struct ShortcutContext {
    state: Rc<RefCell<UiState>>,
    search: Input,
    window: flapp::WindowPtr,
}

thread_local! {
    static SHORTCUTS: RefCell<Option<ShortcutContext>> = const { RefCell::new(None) };
}

fn install_shortcut_dispatch(state: Rc<RefCell<UiState>>, search: Input, window: flapp::WindowPtr) {
    SHORTCUTS.with(|cell| {
        *cell.borrow_mut() = Some(ShortcutContext {
            state,
            search,
            window,
        });
    });
    unsafe {
        flapp::event_dispatch(dispatch_event);
    }
}

fn dispatch_event(event: Event, window: flapp::WindowPtr) -> bool {
    if event == Event::KeyDown && flapp::modal().is_none() {
        let context = SHORTCUTS.with(|cell| {
            cell.borrow()
                .as_ref()
                .map(|ctx| (ctx.state.clone(), ctx.search.clone(), ctx.window))
        });
        if let Some((state, mut search, main_window)) = context {
            if window == main_window {
                let key = flapp::event_key();
                let search_focused = search.has_focus();
                if (!search_focused || matches!(key, Key::Escape | Key::F5))
                    && handle_shortcut(&state, &mut search, event)
                {
                    return true;
                }
            }
        }
    }
    unsafe { flapp::handle_raw(event, window) }
}

#[derive(Clone)]
pub struct Ui {
    pub window: Window,
    pub treemap: Frame,
    pub status: Frame,
    pub rss: Frame,
    pub errors: Button,
    pub scan: Button,
    pub up: Button,
    pub sidebar: panel::Sidebar,
    pub crumbs: Frame,
}

fn toolbar_button(label: &str) -> Button {
    widgets::flat_button(label, false)
}

pub fn build(window: &mut Window, state: Rc<RefCell<UiState>>) -> Ui {
    window.set_color(theme::BG);
    window.set_label("Spacebox");
    window.size_range(900, 600, 0, 0);

    let mut root = Flex::default_fill().column();
    root.set_pad(0);
    root.set_margins(0, 0, 0, 0);

    let mut menu = MenuBar::default();
    menu.set_frame(FrameType::FlatBox);
    menu.set_color(theme::PANEL);
    menu.set_selection_color(theme::ACCENT_DIM);
    menu.set_text_color(theme::TEXT);
    menu.set_text_size(12);
    root.fixed(&menu, 26);

    let mut toolbar = Flex::default().row();
    toolbar.set_pad(6);
    toolbar.set_margins(4, 4, 4, 4);

    let mut up = toolbar_button("Up");
    {
        let state = state.clone();
        up.set_callback(move |_| app::go_up(&state));
    }
    toolbar.fixed(&up, 42);

    let choose = toolbar_button("Choose folder");
    {
        let state = state.clone();
        let mut choose = choose.clone();
        choose.set_callback(move |_| app::choose_folder(&state));
    }
    toolbar.fixed(&choose, 104);

    let mut scan = toolbar_button("Scan");
    {
        let state = state.clone();
        scan.set_callback(move |_| app::toggle_scan(&state));
    }
    toolbar.fixed(&scan, 84);

    let crumbs = Breadcrumbs::new(state.clone());

    let mut search = Input::default();
    search.set_trigger(CallbackTrigger::EnterKey);
    search.set_frame(FrameType::DownBox);
    search.set_color(theme::PANEL_ALT);
    search.set_text_color(theme::TEXT);
    search.set_selection_color(theme::ACCENT_DIM);
    search.set_text_size(11);
    search.set_tooltip("Search names, then press Enter. Escape clears.");
    {
        let state = state.clone();
        search.set_callback(move |input| {
            let value = input.value();
            app::set_search(&state, value);
        });
    }
    toolbar.fixed(&search, 230);

    toolbar.end();
    root.fixed(&toolbar, 40);

    let mut main = Flex::default().row();
    main.set_pad(0);

    let sidebar = panel::build(&state);
    main.fixed(&sidebar.flex, 350);

    let mut right = Flex::default().column();
    right.set_pad(0);

    let treemap = Treemap::new(0, 0, 0, 0, state.clone());
    let legend = legend_frame();
    right.fixed(&legend, 22);
    right.end();

    main.end();

    let mut status_row = Flex::default().row();
    status_row.set_pad(0);

    let mut status = Frame::default();
    status.set_frame(FrameType::NoBox);
    status.set_color(theme::PANEL);
    {
        let state = state.clone();
        status.draw(move |frame| paint_status(&state, frame));
    }

    let mut errors = widgets::flat_button("Errors 0", false);
    errors.set_label_color(theme::WARNING);
    errors.set_label_size(10);
    errors.set_tooltip("Show folders that could not be read");
    {
        let state = state.clone();
        errors.set_callback(move |_| app::show_errors(&state));
    }
    status_row.fixed(&errors, 86);

    let mut rss = Frame::default();
    rss.set_frame(FrameType::NoBox);
    rss.set_color(theme::PANEL);
    rss.set_tooltip("Resident memory. Idle target is under 20 MiB.");
    {
        let state = state.clone();
        rss.draw(move |frame| paint_rss(&state, frame));
    }
    status_row.fixed(&rss, 136);

    // The status label takes every pixel the fixed cells do not claim. A Flex
    // child that is neither fixed nor resizable keeps a stale geometry.
    status_row.resizable(&status);

    status_row.end();
    root.fixed(&status_row, 24);

    add_menus(&mut menu, &state, &right, &legend);

    root.end();

    {
        let search_probe = search.clone();
        window.handle(move |window, event| {
            if event == Event::Push {
                let (mx, my) = flapp::event_coords();
                let inside_search = mx >= search_probe.x()
                    && mx < search_probe.x() + search_probe.w()
                    && my >= search_probe.y()
                    && my < search_probe.y() + search_probe.h();
                if !inside_search {
                    flapp::set_focus(window);
                }
            }
            false
        });
    }

    window.resizable(&root);
    window.end();

    install_shortcut_dispatch(state, search, window.as_widget_ptr().cast());

    Ui {
        window: window.clone(),
        treemap: treemap.frame.clone(),
        status,
        rss,
        errors,
        scan,
        up,
        sidebar,
        crumbs: crumbs.frame,
    }
}

fn add_menus(menu: &mut MenuBar, state: &Rc<RefCell<UiState>>, right: &Flex, legend: &Frame) {
    {
        let state = state.clone();
        menu.add(
            "&File/&Scan Home",
            Shortcut::Ctrl | 'h',
            MenuFlag::Normal,
            move |_| app::scan_home(&state),
        );
    }
    {
        let state = state.clone();
        menu.add(
            "&File/&Choose Folder…",
            Shortcut::Ctrl | 'o',
            MenuFlag::Normal,
            move |_| app::choose_folder(&state),
        );
    }
    {
        let state = state.clone();
        menu.add(
            "&File/&Rescan",
            Shortcut::Ctrl | 'r',
            MenuFlag::Normal,
            move |_| app::rescan(&state),
        );
    }
    menu.add(
        "&File/&Quit",
        Shortcut::Ctrl | 'q',
        MenuFlag::Normal,
        |_| flapp::quit(),
    );

    {
        let state = state.clone();
        menu.add(
            "&View/&Allocated usage",
            Shortcut::None,
            MenuFlag::Normal,
            move |_| app::set_size_mode(&state, true),
        );
    }
    {
        let state = state.clone();
        menu.add(
            "&View/&Logical size",
            Shortcut::None,
            MenuFlag::Normal,
            move |_| app::set_size_mode(&state, false),
        );
    }
    {
        let state = state.clone();
        menu.add(
            "&View/Largest &files",
            Shortcut::None,
            MenuFlag::Normal,
            move |_| app::set_side_mode(&state, SideMode::Files),
        );
    }
    {
        let state = state.clone();
        menu.add(
            "&View/Largest f&olders",
            Shortcut::None,
            MenuFlag::Normal,
            move |_| app::set_side_mode(&state, SideMode::Folders),
        );
    }

    let mut legend_visible = true;
    let mut legend_toggle = legend.clone();
    let mut right_toggle = right.clone();
    menu.add(
        "&View/&Legend",
        Shortcut::None,
        MenuFlag::Normal,
        move |_| {
            legend_visible = !legend_visible;
            if legend_visible {
                legend_toggle.show();
                right_toggle.fixed(&legend_toggle, 22);
            } else {
                legend_toggle.hide();
                right_toggle.fixed(&legend_toggle, 0);
            }
            right_toggle.recalc();
        },
    );

    menu.add(
        "&Help/&Shortcuts",
        Shortcut::from_key(Key::F1),
        MenuFlag::Normal,
        |_| dialogs::shortcuts(),
    );
    menu.add("&Help/&About", Shortcut::None, MenuFlag::Normal, |_| {
        dialogs::about()
    });
}

fn legend_frame() -> Frame {
    let mut frame = Frame::default();
    frame.set_frame(FrameType::NoBox);
    frame.set_color(theme::PANEL);
    frame.draw(|frame| {
        let (ox, oy, w, h) = (frame.x(), frame.y(), frame.w(), frame.h());
        draw::draw_rect_fill(ox, oy, w, h, theme::PANEL);
        draw::set_draw_color(theme::BORDER);
        draw::draw_line(ox, oy, ox + w, oy);

        let mut items: Vec<(&str, (u8, u8, u8))> = Vec::with_capacity(11);
        items.push(("Folders", theme::dir_rgb(1)));
        items.push(("Small files", theme::SMALL_FILES_RGB));
        for category in Category::ALL {
            items.push((category.label(), theme::category_rgb(category)));
        }

        let mut x = ox + 8;
        let y = oy + 4;
        let right = ox + w - 4;
        for (label, rgb) in items {
            draw::set_font(Font::Helvetica, 10);
            let text_w = draw::measure(label, false).0;
            if x + 10 + 4 + text_w > right {
                break;
            }
            draw::draw_rect_fill(x, y + 3, 9, 9, theme::rgb_color(rgb));
            draw::set_draw_color(theme::rgb_color(theme::blend(rgb, (0, 0, 0), 0.5)));
            draw::draw_rect(x, y + 3, 9, 9);
            draw::set_draw_color(theme::MUTED);
            draw::draw_text2(
                label,
                x + 13,
                y,
                text_w + 4,
                h - 8,
                Align::Left | Align::Inside,
            );
            x += 9 + 4 + text_w + 14;
        }
    });
    frame
}

fn paint_status(state: &Rc<RefCell<UiState>>, frame: &mut Frame) {
    let (ox, oy, w, h) = (frame.x(), frame.y(), frame.w(), frame.h());
    draw::draw_rect_fill(ox, oy, w, h, theme::PANEL);
    draw::set_draw_color(theme::BORDER);
    draw::draw_line(ox, oy, ox + w, oy);

    let Ok(st) = state.try_borrow() else {
        return;
    };
    let text = status_line(&st);
    let color = if st.toast.is_some() {
        theme::SUCCESS
    } else if st.scanning {
        theme::TEXT
    } else {
        theme::MUTED
    };
    let clipped = sys::truncate_text(&text, w - 16, Font::Helvetica, 11);
    draw::set_font(Font::Helvetica, 11);
    draw::set_draw_color(color);
    draw::draw_text2(
        &clipped,
        ox + 8,
        oy + 2,
        w - 16,
        h - 4,
        Align::Left | Align::Inside,
    );
}

fn paint_rss(state: &Rc<RefCell<UiState>>, frame: &mut Frame) {
    let (ox, oy, w, h) = (frame.x(), frame.y(), frame.w(), frame.h());
    draw::draw_rect_fill(ox, oy, w, h, theme::PANEL);
    draw::set_draw_color(theme::BORDER);
    draw::draw_line(ox, oy, ox + w, oy);

    let Ok(st) = state.try_borrow() else {
        return;
    };
    let (label, color) = if st.rss_kb == 0 {
        ("RSS —".to_string(), theme::MUTED)
    } else if st.rss_kb > RSS_BUDGET_KB {
        (
            format!("RSS {} (!)", sys::format_bytes(st.rss_kb * 1024)),
            theme::DANGER,
        )
    } else {
        (
            format!("RSS {}", sys::format_bytes(st.rss_kb * 1024)),
            theme::MUTED,
        )
    };
    draw::set_font(Font::Helvetica, 10);
    draw::set_draw_color(color);
    draw::draw_text2(
        &label,
        ox + 8,
        oy + 2,
        w - 16,
        h - 4,
        Align::Right | Align::Inside,
    );
}

fn status_line(st: &UiState) -> String {
    if let Some((message, since)) = st.toast.as_ref() {
        if since.elapsed().as_secs_f64() < 6.0 {
            return message.clone();
        }
    }

    if st.scanning {
        let snapshot = st.progress.snapshot();
        return format!(
            "Scanning {}  ·  {} entries  ·  {} folders  ·  {} seen",
            st.root.display(),
            sys::format_count(snapshot.entries),
            sys::format_count(snapshot.dirs),
            sys::format_bytes(snapshot.bytes)
        );
    }

    let Some(tree) = st.tree.as_ref() else {
        return "Ready  ·  choose a folder to scan (Ctrl+O) or scan your home folder (Ctrl+H)"
            .to_string();
    };

    let mut extra = String::new();
    if tree.stats.errors > 0 {
        extra.push_str(&format!(
            "  ·  {} error{}",
            sys::format_count(tree.stats.errors),
            if tree.stats.errors == 1 { "" } else { "s" }
        ));
    }
    if tree.stats.hardlinks_skipped > 0 {
        extra.push_str(&format!(
            "  ·  {} hardlink{} shared",
            sys::format_count(tree.stats.hardlinks_skipped),
            if tree.stats.hardlinks_skipped == 1 {
                ""
            } else {
                "s"
            }
        ));
    }

    format!(
        "{}  ·  {} allocated  ·  {} logical  ·  {} files  ·  {} folders  ·  {}{}",
        tree.root.display(),
        sys::format_bytes(tree.stats.allocated),
        sys::format_bytes(tree.stats.logical),
        sys::format_count(tree.stats.files),
        sys::format_count(tree.stats.dirs),
        sys::format_duration(tree.stats.elapsed_secs),
        extra
    )
}

fn handle_shortcut(state: &Rc<RefCell<UiState>>, search: &mut Input, event: Event) -> bool {
    if event != Event::KeyDown {
        return false;
    }

    let key = flapp::event_key();
    let modifiers = flapp::event_state();
    let ctrl = modifiers.contains(Shortcut::Ctrl) || modifiers.contains(Shortcut::Meta);
    let shift = modifiers.contains(Shortcut::Shift);
    let alt = modifiers.contains(Shortcut::Alt);

    if ctrl {
        let code = key.bits();
        if code == 'c' as i32 || code == 'C' as i32 {
            app::copy_selected_path(state);
            return true;
        }
        if code == 'f' as i32 || code == 'F' as i32 {
            let _ = search.take_focus();
            return true;
        }
    }

    if alt && key == Key::Left {
        app::go_up(state);
        return true;
    }

    match key {
        Key::Up => {
            crate::treemap::move_selection(state, 0.0, -1.0);
            true
        }
        Key::Down => {
            crate::treemap::move_selection(state, 0.0, 1.0);
            true
        }
        Key::Left => {
            crate::treemap::move_selection(state, -1.0, 0.0);
            true
        }
        Key::Right => {
            crate::treemap::move_selection(state, 1.0, 0.0);
            true
        }
        Key::Enter => {
            let selected = state.borrow().selected;
            if let Some(id) = selected {
                app::activate(state, id);
            }
            true
        }
        Key::BackSpace => {
            app::go_up(state);
            true
        }
        Key::Delete => {
            if shift {
                app::delete_selected_permanently(state);
            } else {
                app::trash_selected(state);
            }
            true
        }
        Key::Escape => {
            let (scanning, has_query) = {
                let st = state.borrow();
                (
                    st.scanning,
                    !st.search_query.is_empty() || !search.value().is_empty(),
                )
            };
            if scanning {
                app::cancel_scan(state);
            } else if has_query {
                search.set_value("");
                app::set_search(state, String::new());
            } else {
                crate::treemap::clear_selection(state);
            }
            true
        }
        Key::F5 => {
            app::rescan(state);
            true
        }
        _ => false,
    }
}

pub fn start_timer(state: Rc<RefCell<UiState>>, mut ui: Ui, receiver: flapp::Receiver<Msg>) {
    let mut last_status = String::new();
    let mut last_rss = String::new();
    let mut last_selected: Option<u32> = None;
    let mut last_errors = u64::MAX;
    let mut last_mode = -1;
    let mut last_scanning = false;
    let mut last_up_enabled = true;
    let mut last_actions_enabled = true;

    flapp::add_timeout3(0.1, move |handle| {
        while let Some(message) = receiver.recv() {
            let scan_finished = matches!(message, Msg::ScanDone { .. });
            if let Some(root) = app::handle_message(&state, message) {
                app::start_scan(&state, root);
            }
            if scan_finished {
                sys::trim_allocator();
            }
        }

        let mut rebuild = false;
        let status;
        let rss;
        let dirty;
        let scanning;
        let selected;
        let errors;
        let mode_index;
        let focus_is_root;
        let actions_enabled;

        {
            let mut st = state.borrow_mut();
            if let Some((_, since)) = st.toast.as_ref() {
                if since.elapsed().as_secs_f64() > 6.0 {
                    st.toast = None;
                    st.dirty = true;
                }
            }
            st.rss_kb = sys::read_rss_kb();

            if st.sidebar_dirty {
                st.refresh_sidebar_nodes();
                st.sidebar_dirty = false;
                rebuild = true;
            }

            dirty = st.dirty;
            st.dirty = false;
            scanning = st.scanning;
            selected = st.selected;
            focus_is_root = st.focus == 0;
            errors = st.tree.as_ref().map(|tree| tree.stats.errors).unwrap_or(0);
            actions_enabled = st
                .tree
                .as_ref()
                .zip(st.selected)
                .map(|(tree, id)| id != 0 && !tree.is_small_agg(id))
                .unwrap_or(false);
            mode_index = match st.side_mode {
                SideMode::Files => 0,
                SideMode::Folders => 1,
                SideMode::Search => 2,
            };
            status = status_line(&st);
            rss = if st.rss_kb == 0 {
                "RSS —".to_string()
            } else {
                format!("RSS {}", sys::format_bytes(st.rss_kb * 1024))
            };

            if rebuild {
                panel::rebuild_browser(&mut ui.sidebar, &st);
            }
        }

        if ui.scan.label() != if scanning { "Cancel" } else { "Scan" } {
            ui.scan.set_label(if scanning { "Cancel" } else { "Scan" });
        }
        if scanning != last_scanning {
            ui.scan
                .set_label_color(if scanning { theme::DANGER } else { theme::TEXT });
            last_scanning = scanning;
        }

        if ui.sidebar.choice.value() != mode_index {
            ui.sidebar.choice.set_value(mode_index);
        }

        if last_mode != mode_index || rebuild {
            ui.crumbs.redraw();
            last_mode = mode_index;
        }

        if last_selected != selected || last_actions_enabled != actions_enabled {
            ui.sidebar.set_actions_enabled(actions_enabled);
            ui.sidebar.details.redraw();
            ui.treemap.redraw();
            last_selected = selected;
            last_actions_enabled = actions_enabled;
        }

        if last_up_enabled == focus_is_root {
            if focus_is_root {
                ui.up.deactivate();
            } else {
                ui.up.activate();
            }
            last_up_enabled = !focus_is_root;
        }

        if last_errors != errors {
            ui.errors
                .set_label(&format!("Errors {}", sys::format_count(errors)));
            if errors > 0 {
                ui.errors.activate();
            } else {
                ui.errors.deactivate();
            }
            last_errors = errors;
        }

        if rebuild {
            ui.crumbs.redraw();
            ui.sidebar.details.redraw();
        }

        if dirty {
            ui.window.redraw();
        }
        if scanning {
            ui.treemap.redraw();
        }
        if status != last_status {
            ui.status.redraw();
            last_status = status;
        }
        if rss != last_rss {
            ui.rss.redraw();
            last_rss = rss;
        }

        flapp::repeat_timeout3(0.1, handle);
    });
}
