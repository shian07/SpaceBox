use crate::app::{self, UiState};
use crate::sys;
use crate::theme;
use fltk::{
    app as flapp, draw,
    enums::{Align, Event, Font, FrameType},
    frame::Frame,
    prelude::*,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Hit rectangle and node id for one breadcrumb chip.
type ChipRects = Rc<RefCell<Vec<(i32, i32, i32, u32)>>>;
/// Index of the chip under the pointer, if any.
type HoverCell = Rc<Cell<Option<usize>>>;

pub struct Breadcrumbs {
    pub frame: Frame,
}

impl Breadcrumbs {
    pub fn new(state: Rc<RefCell<UiState>>) -> Self {
        let mut frame = Frame::default();
        frame.set_frame(FrameType::NoBox);
        frame.set_color(theme::BG);

        let rects: ChipRects = Rc::new(RefCell::new(Vec::new()));
        let hovered: HoverCell = Rc::new(Cell::new(None));

        {
            let state = state.clone();
            let rects = rects.clone();
            let hovered = hovered.clone();
            frame.draw(move |frame| {
                paint(&state, frame, &rects, &hovered);
            });
        }

        {
            let state = state.clone();
            let rects = rects.clone();
            let hovered = hovered.clone();
            frame.handle(move |frame, event| match event {
                Event::Push => {
                    let (mx, my) = flapp::event_coords();
                    let lx = mx - frame.x();
                    let ly = my - frame.y();
                    let hit = rects
                        .borrow()
                        .iter()
                        .find(|(x, y, w, _)| lx >= *x && lx < x + w && ly >= *y && ly < *y + 22)
                        .map(|(_, _, _, node)| *node);
                    if let Some(node) = hit {
                        app::navigate_to(&state, node);
                        frame.redraw();
                    }
                    true
                }
                Event::Move => {
                    let (mx, my) = flapp::event_coords();
                    let lx = mx - frame.x();
                    let ly = my - frame.y();
                    let next = rects.borrow().iter().position(|(x, y, w, _)| {
                        lx >= *x && lx < x + w && ly >= *y && ly < *y + 22
                    });
                    if hovered.get() != next {
                        hovered.set(next);
                        frame.redraw();
                    }
                    true
                }
                Event::Leave => {
                    if hovered.get().is_some() {
                        hovered.set(None);
                        frame.redraw();
                    }
                    true
                }
                _ => false,
            });
        }

        Self { frame }
    }
}

fltk::widget_extends!(Breadcrumbs, Frame, frame);

struct Chip {
    node: Option<u32>,
    name: String,
}

fn chips(st: &UiState) -> Vec<Chip> {
    let Some(tree) = st.tree.as_ref() else {
        let name = st
            .root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| st.root.to_string_lossy().into_owned());
        return vec![Chip { node: None, name }];
    };

    let segments = tree.segments(st.focus);
    if segments.is_empty() {
        return Vec::new();
    }

    let mut chips = Vec::new();
    if segments.len() <= 5 {
        for (node, name) in segments {
            chips.push(Chip {
                node: Some(node),
                name,
            });
        }
    } else {
        chips.push(Chip {
            node: Some(segments[0].0),
            name: segments[0].1.clone(),
        });
        chips.push(Chip {
            node: None,
            name: "…".to_string(),
        });
        for (node, name) in segments.iter().skip(segments.len() - 3) {
            chips.push(Chip {
                node: Some(*node),
                name: name.clone(),
            });
        }
    }
    chips
}

fn paint(state: &Rc<RefCell<UiState>>, frame: &Frame, rects: &ChipRects, hovered: &HoverCell) {
    let (ox, oy, w, h) = (frame.x(), frame.y(), frame.w(), frame.h());
    draw::draw_rect_fill(ox, oy, w, h, theme::BG);

    let Ok(st) = state.try_borrow() else {
        return;
    };
    let mut rects_out = rects.borrow_mut();
    rects_out.clear();

    let list = chips(&st);
    if list.is_empty() {
        return;
    }

    let mut x = ox + 2;
    let y = oy + 2;
    let chip_h = 22;
    let right = ox + w - 2;
    let hover = hovered.get();

    for (index, chip) in list.iter().enumerate() {
        let display = sys::truncate_text(&chip.name, 170, Font::Helvetica, 11);
        draw::set_font(Font::Helvetica, 11);
        let text_w = draw::measure(&display, false).0;
        let chip_w = text_w + 16;
        if x + chip_w > right {
            break;
        }

        let active = chip.node == Some(st.focus);
        let is_hovered = hover == Some(index) && chip.node.is_some();
        let background = if active {
            theme::ACCENT_DIM
        } else if is_hovered {
            theme::PANEL_ALT
        } else {
            theme::PANEL
        };
        draw::draw_rect_fill(x, y, chip_w, chip_h, background);
        if is_hovered && !active {
            draw::set_draw_color(theme::BORDER);
            draw::draw_rect(x, y, chip_w, chip_h);
        }

        draw::set_draw_color(if active {
            theme::TEXT
        } else if chip.node.is_some() {
            theme::MUTED
        } else {
            theme::rgb_color(theme::blend(theme::MUTED_RGB, theme::BG_RGB, 0.4))
        });
        draw::draw_text2(
            &display,
            x + 8,
            y,
            chip_w - 16,
            chip_h,
            Align::Left | Align::Inside,
        );

        if let Some(node) = chip.node {
            rects_out.push((x - ox, y - oy, chip_w, node));
        }
        x += chip_w;

        if index + 1 < list.len() {
            draw::set_draw_color(theme::rgb_color(theme::blend(
                theme::MUTED_RGB,
                theme::BG_RGB,
                0.35,
            )));
            draw::set_font(Font::Helvetica, 11);
            draw::draw_text2("›", x, y, 14, chip_h, Align::Center | Align::Inside);
            x += 14;
        }
    }
}
