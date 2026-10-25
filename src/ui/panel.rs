use super::widgets;
use crate::app::{self, SideMode, UiState};
use crate::sys;
use crate::theme;
use fltk::{
    browser::HoldBrowser,
    button::Button,
    draw,
    enums::{Align, Font, FrameType},
    frame::Frame,
    group::Flex,
    menu::Choice,
    prelude::*,
};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone)]
pub struct Sidebar {
    pub flex: Flex,
    pub choice: Choice,
    pub browser: HoldBrowser,
    pub details: Frame,
    pub open: Button,
    pub reveal: Button,
    pub copy: Button,
    pub trash: Button,
    pub delete: Button,
}

impl Sidebar {
    pub fn set_actions_enabled(&mut self, enabled: bool) {
        for button in [
            &mut self.open,
            &mut self.reveal,
            &mut self.copy,
            &mut self.trash,
            &mut self.delete,
        ] {
            if enabled {
                button.activate();
            } else {
                button.deactivate();
            }
        }
    }
}

pub fn build(state: &Rc<RefCell<UiState>>) -> Sidebar {
    let mut flex = Flex::default().column();
    flex.set_pad(6);
    flex.set_margins(4, 4, 4, 4);

    let mut choice = Choice::default();
    choice.add_choice("Largest files|Largest folders|Search results");
    choice.set_frame(FrameType::DownBox);
    choice.set_color(theme::PANEL_ALT);
    choice.set_text_color(theme::TEXT);
    choice.set_text_size(11);
    {
        let state = state.clone();
        choice.set_callback(move |choice| {
            let mode = match choice.value() {
                1 => SideMode::Folders,
                2 => SideMode::Search,
                _ => SideMode::Files,
            };
            app::set_side_mode(&state, mode);
        });
    }
    flex.fixed(&choice, 26);

    let mut browser = HoldBrowser::default();
    browser.set_column_char('\t');
    browser.set_column_widths(&[78, 240]);
    browser.set_frame(FrameType::FlatBox);
    browser.set_color(theme::PANEL);
    browser.set_selection_color(theme::ACCENT_DIM);
    browser.set_text_size(11);
    {
        let state = state.clone();
        browser.set_callback(move |browser| {
            let row = browser.value();
            if row > 0 {
                app::navigate_sidebar_row(&state, (row - 1) as usize);
            }
        });
    }

    let mut details = Frame::default();
    details.set_frame(FrameType::NoBox);
    details.set_color(theme::PANEL_ALT);
    {
        let state = state.clone();
        details.draw(move |frame| paint_details(&state, frame));
    }
    flex.fixed(&details, 66);

    let mut row1 = Flex::default().row();
    row1.set_pad(4);
    let mut open = widgets::flat_button("Open", false);
    let mut reveal = widgets::flat_button("Reveal", false);
    let mut copy = widgets::flat_button("Copy path", false);
    {
        let state = state.clone();
        open.set_callback(move |_| app::open_selected(&state));
    }
    {
        let state = state.clone();
        reveal.set_callback(move |_| app::reveal_selected(&state));
    }
    {
        let state = state.clone();
        copy.set_callback(move |_| app::copy_selected_path(&state));
    }
    // Three equal columns. Every button must be fixed into the row: an
    // un-fixed widget keeps a stale geometry and paints as a stray rectangle.
    let actions_w = 330;
    let gap = 4 * 2;
    let action_w = (actions_w - gap) / 3;
    row1.fixed(&open, action_w);
    row1.fixed(&reveal, action_w);
    row1.fixed(&copy, action_w);
    row1.end();
    flex.fixed(&row1, 28);

    let mut row2 = Flex::default().row();
    row2.set_pad(4);
    let mut trash = widgets::flat_button("Trash", true);
    let mut delete = widgets::flat_button("Delete", true);
    {
        let state = state.clone();
        trash.set_callback(move |_| app::trash_selected(&state));
    }
    {
        let state = state.clone();
        delete.set_callback(move |_| app::delete_selected_permanently(&state));
    }
    row2.fixed(&trash, (actions_w - gap) / 2);
    row2.fixed(&delete, (actions_w - gap) / 2);
    row2.end();
    flex.fixed(&row2, 28);

    flex.end();

    Sidebar {
        flex,
        choice,
        browser,
        details,
        open,
        reveal,
        copy,
        trash,
        delete,
    }
}

pub fn rebuild_browser(sidebar: &mut Sidebar, st: &UiState) {
    sidebar.browser.clear();
    let Some(tree) = st.tree.as_ref() else {
        sidebar.browser.redraw();
        return;
    };

    for id in &st.sidebar_nodes {
        let size = tree.size(*id, st.use_alloc);
        let relative = sys::sanitize_label(&tree.relative_path_of(*id));
        let name = if relative.is_empty() {
            sys::sanitize_label(tree.name(*id))
        } else {
            relative
        };
        sidebar
            .browser
            .add(&format!("{}\t{}", sys::format_bytes(size), name));
    }

    if let Some(selected) = st.selected {
        if let Some(index) = st.sidebar_nodes.iter().position(|id| *id == selected) {
            sidebar.browser.select(index as i32 + 1);
        }
    }
    sidebar.browser.redraw();
}

fn paint_details(state: &Rc<RefCell<UiState>>, frame: &mut Frame) {
    let (ox, oy, w, h) = (frame.x(), frame.y(), frame.w(), frame.h());
    draw::draw_rect_fill(ox, oy, w, h, theme::PANEL_ALT);
    draw::set_draw_color(theme::BORDER);
    draw::draw_line(ox, oy, ox + w, oy);

    let Ok(st) = state.try_borrow() else {
        return;
    };
    let Some(tree) = st.tree.as_ref() else {
        draw::set_font(Font::Helvetica, 11);
        draw::set_draw_color(theme::MUTED);
        draw::draw_text2(
            "No scan data",
            ox + 8,
            oy + 4,
            w - 16,
            h - 8,
            Align::Left | Align::Inside,
        );
        return;
    };

    let inset = 8;
    let text_w = (w - inset * 2).max(10);

    let Some(id) = st.selected.filter(|id| *id != 0) else {
        let focus = st.focus;
        let name = sys::truncate_text(
            &sys::sanitize_label(tree.name(focus)),
            text_w,
            Font::Helvetica,
            12,
        );
        draw::set_font(Font::Helvetica, 12);
        draw::set_draw_color(theme::TEXT);
        draw::draw_text2(
            &name,
            ox + inset,
            oy + 6,
            text_w,
            16,
            Align::Left | Align::Inside,
        );

        let summary = format!(
            "{}  ·  {} files  ·  nothing selected",
            sys::format_bytes(tree.size(focus, st.use_alloc)),
            sys::format_count(tree.node(focus).files as u64)
        );
        draw::set_font(Font::Helvetica, 10);
        draw::set_draw_color(theme::MUTED);
        let summary = sys::truncate_text(&summary, text_w, Font::Helvetica, 10);
        draw::draw_text2(
            &summary,
            ox + inset,
            oy + 24,
            text_w,
            14,
            Align::Left | Align::Inside,
        );
        let path = sys::truncate_text(
            &tree.path_of(focus).display().to_string(),
            text_w,
            Font::Helvetica,
            10,
        );
        draw::draw_text2(
            &path,
            ox + inset,
            oy + 39,
            text_w,
            14,
            Align::Left | Align::Inside,
        );
        return;
    };

    if tree.is_small_agg(id) {
        let name = format!(
            "small files ({})",
            sys::format_count(tree.node(id).files as u64)
        );
        draw::set_font(Font::Helvetica, 12);
        draw::set_draw_color(theme::TEXT);
        draw::draw_text2(
            &name,
            ox + inset,
            oy + 6,
            text_w,
            16,
            Align::Left | Align::Inside,
        );
        let summary = format!(
            "{}  ·  aggregated; reveal from the file manager",
            sys::format_bytes(tree.size(id, st.use_alloc))
        );
        draw::set_font(Font::Helvetica, 10);
        draw::set_draw_color(theme::MUTED);
        let summary = sys::truncate_text(&summary, text_w, Font::Helvetica, 10);
        draw::draw_text2(
            &summary,
            ox + inset,
            oy + 24,
            text_w,
            14,
            Align::Left | Align::Inside,
        );
        let parent = tree.parent(id).unwrap_or(id);
        let path = sys::truncate_text(
            &tree.path_of(parent).display().to_string(),
            text_w,
            Font::Helvetica,
            10,
        );
        draw::draw_text2(
            &path,
            ox + inset,
            oy + 39,
            text_w,
            14,
            Align::Left | Align::Inside,
        );
        return;
    }

    let name = sys::truncate_text(
        &sys::sanitize_label(tree.name(id)),
        text_w,
        Font::Helvetica,
        12,
    );
    draw::set_font(Font::Helvetica, 12);
    draw::set_draw_color(theme::TEXT);
    draw::draw_text2(
        &name,
        ox + inset,
        oy + 6,
        text_w,
        16,
        Align::Left | Align::Inside,
    );

    let size = tree.size(id, st.use_alloc);
    let parent = tree.parent(id).unwrap_or(id);
    let parent_size = tree.size(parent, st.use_alloc);
    let percent = if parent_size > 0 {
        size as f64 / parent_size as f64 * 100.0
    } else {
        0.0
    };
    let kind = if tree.is_dir(id) { "folder" } else { "file" };
    let summary = format!(
        "{}  ·  {:.1}% of parent  ·  {}",
        sys::format_bytes(size),
        percent,
        kind
    );
    draw::set_font(Font::Helvetica, 10);
    draw::set_draw_color(theme::MUTED);
    let summary = sys::truncate_text(&summary, text_w, Font::Helvetica, 10);
    draw::draw_text2(
        &summary,
        ox + inset,
        oy + 24,
        text_w,
        14,
        Align::Left | Align::Inside,
    );

    let path = sys::truncate_text(
        &tree.path_of(id).display().to_string(),
        text_w,
        Font::Helvetica,
        10,
    );
    draw::draw_text2(
        &path,
        ox + inset,
        oy + 39,
        text_w,
        14,
        Align::Left | Align::Inside,
    );
}
