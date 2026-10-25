pub mod layout;

use crate::app::{self, Hover, UiState};
use crate::scan::ScanTree;
use crate::sys;
use crate::theme;
use fltk::{
    app as flapp, draw,
    enums::{Align, Event, Font, FrameType},
    frame::Frame,
    menu::MenuItem,
    prelude::*,
};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

pub const DETAIL_HEIGHT: i32 = 24;

pub struct Treemap {
    pub frame: Frame,
}

impl Treemap {
    pub fn new(x: i32, y: i32, w: i32, h: i32, state: Rc<RefCell<UiState>>) -> Self {
        let mut frame = Frame::new(x, y, w, h, None);
        frame.set_frame(FrameType::NoBox);
        frame.set_color(theme::BG);
        frame.clear_visible_focus();

        let draw_state = state.clone();
        frame.draw(move |widget| paint(&draw_state, widget));

        let handle_state = state.clone();
        frame.handle(move |widget, event| handle(&handle_state, widget, event));

        Self { frame }
    }
}

fltk::widget_extends!(Treemap, Frame, frame);

fn paint(state: &Rc<RefCell<UiState>>, widget: &mut Frame) {
    let (ox, oy, w, h) = (widget.x(), widget.y(), widget.w(), widget.h());
    if w <= 0 || h <= 0 {
        return;
    }

    let Ok(mut st) = state.try_borrow_mut() else {
        return;
    };

    draw::draw_rect_fill(ox, oy, w, h, theme::BG);

    let canvas_h = (h - DETAIL_HEIGHT).max(0);
    st.ensure_layout(w, canvas_h);
    let has_tree = st.tree.is_some();

    if has_tree {
        paint_boxes(&st, ox, oy);
    }

    if st.scanning {
        paint_scan_overlay(&st, ox, oy, w, canvas_h);
    } else if !has_tree {
        paint_empty(&st, ox, oy, w, canvas_h);
    }

    paint_detail(&st, ox, oy + h - DETAIL_HEIGHT, w);
}

fn paint_boxes(st: &UiState, ox: i32, oy: i32) {
    let Some(tree) = st.tree.as_ref() else {
        return;
    };
    let focused = st.focus;
    let selected = st.selected;

    for item in &st.layout.boxes {
        let bx = ox + item.rect.x.round() as i32;
        let by = oy + item.rect.y.round() as i32;
        let bw = item.rect.w.round() as i32;
        let bh = item.rect.h.round() as i32;
        if bw < 1 || bh < 1 {
            continue;
        }

        let is_hovered = match st.hovered {
            Some(Hover::Node(id)) => !item.synthetic && id == item.node,
            Some(Hover::Other { parent, count, .. }) => {
                item.synthetic && parent == item.node && count == item.other_count
            }
            None => false,
        };
        let is_selected = !item.synthetic && selected == Some(item.node);

        let base = box_rgb(st, tree, item.node, item.depth, item.synthetic);
        let mut fill = base;
        if is_selected {
            fill = theme::blend(fill, theme::ACCENT_RGB, 0.22);
        } else if is_hovered {
            fill = theme::blend(fill, (255, 255, 255), 0.16);
        }

        draw::draw_rect_fill(bx, by, bw, bh, theme::rgb_color(fill));

        draw::set_draw_color(theme::rgb_color(theme::blend(fill, (8, 10, 13), 0.55)));
        draw::draw_rect(bx, by, bw, bh);

        if is_selected {
            draw::set_draw_color(theme::ACCENT);
            draw::draw_rect(bx + 1, by + 1, bw - 2, bh - 2);
            draw::draw_rect(bx + 2, by + 2, bw - 4, bh - 4);
        } else if is_hovered {
            draw::set_draw_color(theme::rgb_color((244, 248, 255)));
            draw::draw_rect(bx + 1, by + 1, bw - 2, bh - 2);
        }

        if item.label_height >= 1.0 {
            paint_label(
                st,
                tree,
                item,
                BoxPaint {
                    x: bx,
                    y: by,
                    w: bw,
                    fill,
                },
                selected == Some(item.node),
                focused,
            );
        }
    }
}

fn box_rgb(_st: &UiState, tree: &ScanTree, node: u32, depth: u8, synthetic: bool) -> (u8, u8, u8) {
    if synthetic {
        return theme::OTHER_RGB;
    }
    if tree.is_small_agg(node) {
        return theme::SMALL_FILES_RGB;
    }
    if tree.is_dir(node) {
        // Tint the depth grey toward whatever the folder mostly contains, so
        // a folder of video reads warm and a source tree reads cyan while the
        // depth ramp still tells nesting levels apart. Neutral folders (no
        // clear majority) keep the plain grey.
        let base = theme::dir_rgb(depth);
        return match tree.dominant_category(node) {
            Some(category) => theme::blend(base, theme::category_rgb(category), 0.45),
            None => base,
        };
    }
    theme::category_rgb(tree.category(node))
}

/// Where a box was painted, so the label can be drawn inside it.
struct BoxPaint {
    x: i32,
    y: i32,
    w: i32,
    fill: (u8, u8, u8),
}

fn paint_label(
    st: &UiState,
    tree: &ScanTree,
    item: &layout::BoxRect,
    paint: BoxPaint,
    selected: bool,
    focus: u32,
) {
    let BoxPaint {
        x: bx,
        y: by,
        w: bw,
        fill,
    } = paint;
    let strip_h = (item.label_height.round() as i32).min(item.rect.h as i32);
    if strip_h <= 0 || bw < 24 {
        return;
    }

    let strip = theme::blend(fill, (10, 12, 16), if selected { 0.62 } else { 0.55 });
    draw::draw_rect_fill(bx, by, bw, strip_h, theme::rgb_color(strip));

    let (name, size_text) = if item.synthetic {
        (
            format!("other {} items", sys::format_count(item.other_count as u64)),
            sys::format_bytes(item.size),
        )
    } else if tree.is_small_agg(item.node) {
        (name_for(st, tree, item.node), sys::format_bytes(item.size))
    } else {
        let name = name_for(st, tree, item.node);
        (name, sys::format_bytes(item.size))
    };

    let text_color = theme::text_on(strip);
    let inset = 4;
    let text_w = (bw - inset * 2).max(8);

    if strip_h >= 24 {
        draw::set_draw_color(text_color);
        let label = sys::truncate_text(&name, text_w, Font::Helvetica, 12);
        draw::set_font(Font::Helvetica, 12);
        draw::draw_text2(
            &label,
            bx + inset,
            by + 2,
            text_w,
            14,
            Align::Left | Align::Inside,
        );
        draw::set_font(Font::Helvetica, 10);
        draw::set_draw_color(theme::rgb_color(theme::blend(strip, (255, 255, 255), 0.55)));
        let sub = sys::truncate_text(
            &format!("{size_text}  ·  {}", kind_label(tree, item.node)),
            text_w,
            Font::Helvetica,
            10,
        );
        draw::draw_text2(
            &sub,
            bx + inset,
            by + 15,
            text_w,
            12,
            Align::Left | Align::Inside,
        );
    } else {
        let label =
            sys::truncate_text(&format!("{name}  {size_text}"), text_w, Font::Helvetica, 11);
        draw::set_font(Font::Helvetica, 11);
        draw::set_draw_color(text_color);
        draw::draw_text2(
            &label,
            bx + inset,
            by + 1,
            text_w,
            strip_h - 2,
            Align::Left | Align::Inside,
        );
    }

    if item.node == focus {
        draw::set_draw_color(theme::rgb_color(theme::blend(fill, theme::ACCENT_RGB, 0.7)));
        draw::draw_line(bx, by, bx, by + strip_h);
    }
}

fn name_for(_st: &UiState, tree: &ScanTree, node: u32) -> String {
    if tree.is_small_agg(node) {
        format!(
            "small files ({})",
            sys::format_count(tree.node(node).files as u64)
        )
    } else {
        let name = sys::sanitize_label(tree.name(node));
        if tree.is_other_fs(node) {
            format!("{name} [mount]")
        } else if tree.is_symlink(node) {
            format!("{name} [link]")
        } else {
            name
        }
    }
}

fn kind_label(tree: &ScanTree, node: u32) -> &'static str {
    if tree.is_small_agg(node) {
        "aggregated"
    } else if tree.is_dir(node) {
        "folder"
    } else {
        "file"
    }
}

fn paint_detail(st: &UiState, ox: i32, oy: i32, w: i32) {
    draw::draw_rect_fill(ox, oy, w, DETAIL_HEIGHT, theme::PANEL_ALT);
    draw::set_draw_color(theme::BORDER);
    draw::draw_line(ox, oy, ox + w, oy);

    let text = detail_text(st);
    if text.is_empty() || w < 20 {
        return;
    }
    let clipped = sys::truncate_text(&text, w - 16, Font::Helvetica, 11);
    draw::set_font(Font::Helvetica, 11);
    draw::set_draw_color(if st.hovered.is_some() {
        theme::TEXT
    } else {
        theme::MUTED
    });
    draw::draw_text2(
        &clipped,
        ox + 8,
        oy + 2,
        w - 16,
        DETAIL_HEIGHT - 4,
        Align::Left | Align::Inside,
    );
}

fn detail_text(st: &UiState) -> String {
    let Some(tree) = st.tree.as_ref() else {
        return String::new();
    };

    if let Some(hovered) = st.hovered {
        return match hovered {
            Hover::Node(id) => describe_node(st, tree, id),
            Hover::Other {
                parent,
                count,
                size,
            } => format!(
                "{} more items  ·  {}  ·  not drawn at this zoom  ·  {}",
                sys::format_count(count as u64),
                sys::format_bytes(size),
                tree.path_of(parent).display()
            ),
        };
    }

    if let Some(id) = st.selected {
        if id != 0 {
            return describe_node(st, tree, id);
        }
    }

    let focus = st.focus;
    let size = tree.size(focus, st.use_alloc);
    let files = tree.node(focus).files;
    format!(
        "{}  ·  {}  ·  {} files  ·  Double-click a folder to drill in  ·  Right-click for actions",
        tree.path_of(focus).display(),
        sys::format_bytes(size),
        sys::format_count(files as u64)
    )
}

fn describe_node(st: &UiState, tree: &ScanTree, id: u32) -> String {
    let size = tree.size(id, st.use_alloc);
    let parent = tree.parent(id).unwrap_or(id);
    let parent_size = tree.size(parent, st.use_alloc);
    let percent = if parent_size > 0 {
        size as f64 / parent_size as f64 * 100.0
    } else {
        0.0
    };

    let mut extras = String::new();
    if tree.is_other_fs(id) {
        extras.push_str("  ·  other filesystem");
    } else if tree.is_symlink(id) {
        extras.push_str("  ·  symlink");
    }
    if tree.is_small_agg(id) {
        extras.push_str(&format!(
            "  ·  {} files",
            sys::format_count(tree.node(id).files as u64)
        ));
    }

    format!(
        "{}  ·  {}  ·  {:.1}% of parent  ·  {}{}  ·  {}",
        name_for(st, tree, id),
        sys::format_bytes(size),
        percent,
        kind_label(tree, id),
        extras,
        tree.path_of(id).display()
    )
}

fn paint_empty(st: &UiState, ox: i32, oy: i32, w: i32, h: i32) {
    if w < 80 || h < 60 {
        return;
    }
    let center_y = oy + h / 2 - 36;

    draw::set_font(Font::Helvetica, 20);
    draw::set_draw_color(theme::TEXT);
    draw::draw_text2(
        "No folder scanned",
        ox,
        center_y,
        w,
        26,
        Align::Center | Align::Inside,
    );

    draw::set_font(Font::Helvetica, 12);
    draw::set_draw_color(theme::MUTED);
    draw::draw_text2(
        "Use File > Choose Folder (Ctrl+O) or Scan Home to start.",
        ox,
        center_y + 30,
        w,
        18,
        Align::Center | Align::Inside,
    );

    if let Some(fatal) = st.fatal.as_deref() {
        let text = sys::truncate_text(fatal, w - 80, Font::Helvetica, 11);
        draw::set_draw_color(theme::DANGER);
        draw::draw_text2(
            &text,
            ox + 40,
            center_y + 56,
            w - 80,
            18,
            Align::Center | Align::Inside,
        );
    }
}

fn paint_scan_overlay(st: &UiState, ox: i32, oy: i32, w: i32, h: i32) {
    if w < 200 || h < 120 {
        return;
    }
    let panel_w = (w - 80).clamp(220, 520);
    let panel_h = 138;
    let px = ox + (w - panel_w) / 2;
    let py = oy + (h - panel_h) / 2;

    draw::draw_rect_fill(px, py, panel_w, panel_h, theme::PANEL);
    draw::set_draw_color(theme::BORDER);
    draw::draw_rect(px, py, panel_w, panel_h);
    draw::draw_rect(px + 1, py + 1, panel_w - 2, panel_h - 2);

    draw::set_font(Font::Helvetica, 15);
    draw::set_draw_color(theme::TEXT);
    draw::draw_text2(
        "Scanning disk…",
        px + 20,
        py + 12,
        panel_w - 40,
        20,
        Align::Left | Align::Inside,
    );

    let snapshot = st.progress.snapshot();
    draw::set_font(Font::Helvetica, 11);
    draw::set_draw_color(theme::MUTED);
    let current = if snapshot.current.is_empty() {
        "Reading directory tree".to_string()
    } else {
        snapshot.current
    };
    let line = sys::truncate_text(&current, panel_w - 40, Font::Helvetica, 11);
    draw::draw_text2(
        &line,
        px + 20,
        py + 36,
        panel_w - 40,
        16,
        Align::Left | Align::Inside,
    );

    let stats = format!(
        "{} entries  ·  {} folders  ·  {} seen",
        sys::format_count(snapshot.entries),
        sys::format_count(snapshot.dirs),
        sys::format_bytes(snapshot.bytes)
    );
    draw::draw_text2(
        &stats,
        px + 20,
        py + 54,
        panel_w - 40,
        16,
        Align::Left | Align::Inside,
    );

    let bar_x = px + 20;
    let bar_y = py + 80;
    let bar_w = panel_w - 40;
    let bar_h = 8;
    draw::draw_rect_fill(bar_x, bar_y, bar_w, bar_h, theme::LINE);
    let phase = (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        % 1400) as f32
        / 1400.0;
    let chunk = (bar_w as f32 * 0.42) as i32;
    let travel = (bar_w - chunk).max(0) as f32;
    draw::draw_rect_fill(
        bar_x + (phase * travel) as i32,
        bar_y,
        chunk,
        bar_h,
        theme::ACCENT,
    );

    draw::set_draw_color(theme::MUTED);
    draw::set_font(Font::Helvetica, 11);
    draw::draw_text2(
        "Press Esc or use the toolbar button to cancel.",
        px + 20,
        py + 96,
        panel_w - 40,
        16,
        Align::Left | Align::Inside,
    );
}

fn local_coords(widget: &Frame) -> (f32, f32) {
    let (mx, my) = flapp::event_coords();
    ((mx - widget.x()) as f32, (my - widget.y()) as f32)
}

fn handle(state: &Rc<RefCell<UiState>>, widget: &mut Frame, event: Event) -> bool {
    match event {
        Event::Push => {
            let (lx, ly) = local_coords(widget);
            let hit = {
                let st = state.borrow();
                st.layout
                    .hit_test(lx, ly)
                    .map(|index| st.layout.boxes[index])
            };
            let Some(item) = hit else {
                let changed = {
                    let mut st = state.borrow_mut();
                    let changed = st.selected.is_some();
                    st.selected = None;
                    if changed {
                        st.dirty = true;
                    }
                    changed
                };
                if changed {
                    widget.redraw();
                }
                return true;
            };

            if flapp::event_mouse_button() == flapp::MouseButton::Right {
                show_context_menu(state, item, widget);
                return true;
            }

            if flapp::event_clicks() {
                if item.synthetic {
                    app::reveal_node(state, item.node);
                } else {
                    app::activate(state, item.node);
                }
                widget.redraw();
                return true;
            }

            if !item.synthetic {
                let mut st = state.borrow_mut();
                st.selected = Some(item.node);
                st.dirty = true;
                drop(st);
                widget.redraw();
            }
            true
        }
        Event::Move => {
            let (lx, ly) = local_coords(widget);
            let next = {
                let st = state.borrow();
                st.layout.hit_test(lx, ly).map(|index| {
                    let item = &st.layout.boxes[index];
                    if item.synthetic {
                        Hover::Other {
                            parent: item.node,
                            count: item.other_count,
                            size: item.size,
                        }
                    } else {
                        Hover::Node(item.node)
                    }
                })
            };
            let changed = {
                let mut st = state.borrow_mut();
                let changed = st.hovered != next;
                st.hovered = next;
                if changed {
                    st.dirty = true;
                }
                changed
            };
            if changed {
                widget.redraw();
            }
            true
        }
        Event::Leave => {
            let changed = {
                let mut st = state.borrow_mut();
                let changed = st.hovered.is_some();
                st.hovered = None;
                changed
            };
            if changed {
                widget.redraw();
            }
            true
        }
        Event::Enter => true,
        _ => false,
    }
}

fn show_context_menu(state: &Rc<RefCell<UiState>>, item: layout::BoxRect, widget: &mut Frame) {
    if !item.synthetic {
        let mut st = state.borrow_mut();
        st.selected = Some(item.node);
        st.dirty = true;
        drop(st);
        widget.redraw();
    }

    let menu = if item.synthetic {
        MenuItem::new(&["Reveal in file manager", "Copy folder path"])
    } else {
        MenuItem::new(&[
            "Open",
            "Reveal in file manager",
            "Copy path",
            "Move to Trash",
            "Delete permanently",
        ])
    };

    let (mx, my) = flapp::event_coords();
    let Some(choice) = menu.popup(mx, my) else {
        return;
    };
    let Some(label) = choice.label() else {
        return;
    };

    match label.as_str() {
        "Open" => app::open_selected(state),
        "Reveal in file manager" => {
            if item.synthetic {
                app::reveal_node(state, item.node);
            } else {
                app::reveal_selected(state);
            }
        }
        "Copy path" => app::copy_selected_path(state),
        "Copy folder path" => {
            if let Some(tree) = state.borrow().tree.as_ref() {
                let path = tree.path_of(item.node);
                crate::fsops::copy_path(&path);
                state
                    .borrow_mut()
                    .set_toast("Folder path copied to clipboard");
            }
        }
        "Move to Trash" => app::trash_selected(state),
        "Delete permanently" => app::delete_selected_permanently(state),
        _ => {}
    }
    widget.redraw();
}

pub fn move_selection(state: &Rc<RefCell<UiState>>, dx: f32, dy: f32) {
    let mut st = state.borrow_mut();
    let Some(current) = st.selected else {
        if let Some(item) = st.layout.boxes.iter().find(|item| !item.synthetic) {
            st.selected = Some(item.node);
            st.dirty = true;
        }
        return;
    };

    let center = |item: &layout::BoxRect| {
        (
            item.rect.x + item.rect.w * 0.5,
            item.rect.y + item.rect.h * 0.5,
        )
    };

    let Some((cx, cy)) = st
        .layout
        .boxes
        .iter()
        .find(|item| !item.synthetic && item.node == current)
        .map(center)
    else {
        return;
    };

    let mut best: Option<(bool, f32, u32)> = None;
    {
        let Some(tree) = st.tree.as_ref() else {
            return;
        };
        let mut ancestors = Vec::new();
        let mut cursor = current;
        while let Some(parent) = tree.parent(cursor) {
            ancestors.push(parent);
            cursor = parent;
        }
        let parent = tree.parent(current);

        for item in st.layout.boxes.iter().filter(|item| !item.synthetic) {
            if item.node == current || ancestors.contains(&item.node) {
                continue;
            }
            let (bx, by) = center(item);
            let vx = bx - cx;
            let vy = by - cy;
            let along = vx * dx + vy * dy;
            if along <= 4.0 {
                continue;
            }
            let across = (vx * -dy + vy * dx).abs();
            let score = along + across * 1.6;
            let sibling = tree.parent(item.node) == parent;
            let better = match best {
                None => true,
                Some((best_sibling, best_score, _)) => match (sibling, best_sibling) {
                    (true, false) => true,
                    (false, true) => false,
                    _ => score < best_score,
                },
            };
            if better {
                best = Some((sibling, score, item.node));
            }
        }
    }

    if let Some((_, _, node)) = best {
        st.selected = Some(node);
        st.dirty = true;
    }
}

pub fn clear_selection(state: &Rc<RefCell<UiState>>) {
    let mut st = state.borrow_mut();
    st.selected = None;
    st.hovered = None;
    st.dirty = true;
}
