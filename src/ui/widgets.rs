//! Small widget helpers shared by the toolbar and the sidebar.
//!
//! FLTK buttons only change on press. On desktop a pointer that lands on a
//! target without feedback reads as broken, so every button built here gets a
//! hover state: a subtle lift in surface luminance, reversed on leave.

use crate::theme;
use fltk::{
    button::Button,
    enums::{Event, FrameType},
    prelude::*,
};

/// A flat button with a hover state. `danger` tints the label and the pressed
/// state red for destructive actions.
pub fn flat_button(label: &str, danger: bool) -> Button {
    let mut button = Button::default().with_label(label);
    button.set_frame(FrameType::FlatBox);
    button.set_color(theme::PANEL_ALT);
    button.set_selection_color(if danger {
        theme::DANGER
    } else {
        theme::ACCENT_DIM
    });
    button.set_label_color(if danger { theme::DANGER } else { theme::TEXT });
    button.set_label_size(11);
    attach_hover(&mut button);
    button
}

/// Lift the widget surface while the pointer is over it.
///
/// Redraws only on Enter/Leave transitions, so idle widgets cost nothing. Kept
/// to a 7% white blend: perceptible against `PANEL_ALT`, never a glow.
pub fn attach_hover<W>(widget: &mut W)
where
    W: WidgetBase + Clone,
{
    let rest = widget.color();
    let hover = theme::rgb_color(theme::blend(rest.to_rgb(), (255, 255, 255), 0.07));

    widget.handle(move |widget, event| match event {
        Event::Enter => {
            if widget.active() {
                widget.set_color(hover);
                widget.redraw();
            }
            false
        }
        Event::Leave => {
            widget.set_color(rest);
            widget.redraw();
            false
        }
        _ => false,
    });
}
