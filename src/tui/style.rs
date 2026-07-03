//! Hand-rolled palette and render helpers.
//!
//! Deliberately not the default ratatui look. The goal is "looks
//! like a quiet document", not "looks like htop". Single accent
//! color (ember/orange), paper background, ink foreground.

use ratatui::style::{Color, Modifier, Style};

pub const PAPER: Color = Color::Rgb(0xf3, 0xe9, 0xd2); // warm cream
pub const INK: Color = Color::Rgb(0x1f, 0x1c, 0x18); // near-black
pub const INK_DIM: Color = Color::Rgb(0x6b, 0x5e, 0x4d); // faded ink
pub const EMBER: Color = Color::Rgb(0xc8, 0x4a, 0x2b); // accent
pub const MOSS: Color = Color::Rgb(0x6a, 0x7a, 0x3a); // on / ok
pub const ASH: Color = Color::Rgb(0x8a, 0x7a, 0x66); // off / muted

pub fn title_style() -> Style {
    Style::default()
        .fg(INK)
        .bg(PAPER)
        .add_modifier(Modifier::BOLD)
}

pub fn body_style() -> Style {
    Style::default().fg(INK).bg(PAPER)
}

pub fn body_dim_style() -> Style {
    Style::default().fg(INK_DIM).bg(PAPER)
}

pub fn selected_style() -> Style {
    Style::default().fg(PAPER).bg(EMBER).add_modifier(Modifier::BOLD)
}

pub fn selected_dim_style() -> Style {
    Style::default().fg(PAPER).bg(INK_DIM)
}

pub fn on_indicator_style() -> Style {
    Style::default().fg(MOSS).bg(PAPER).add_modifier(Modifier::BOLD)
}

pub fn off_indicator_style() -> Style {
    Style::default().fg(ASH).bg(PAPER).add_modifier(Modifier::BOLD)
}

pub fn help_style() -> Style {
    Style::default().fg(INK_DIM).bg(PAPER)
}

pub fn error_style() -> Style {
    Style::default().fg(EMBER).bg(PAPER).add_modifier(Modifier::BOLD)
}
