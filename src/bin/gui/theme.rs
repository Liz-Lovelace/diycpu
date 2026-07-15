//! Light theme palette and visual constants. Every color in the app lives
//! here so the look stays coherent.

use femtovg::Color;

fn hex(rgb: u32) -> Color {
    Color::rgb(
        ((rgb >> 16) & 0xff) as u8,
        ((rgb >> 8) & 0xff) as u8,
        (rgb & 0xff) as u8,
    )
}

// ── Canvas ──
pub fn canvas_bg() -> Color {
    hex(0xFAFAF7)
}
pub fn grid_line() -> Color {
    Color::rgba(40, 44, 52, 14)
}
pub fn grid_line_major() -> Color {
    Color::rgba(40, 44, 52, 26)
}

// ── Components ──
pub fn chip_fill() -> Color {
    hex(0xFFFFFF)
}
pub fn chip_border() -> Color {
    hex(0x3E4450)
}
pub fn chip_label() -> Color {
    hex(0x2B303B)
}
pub fn chip_shadow() -> Color {
    Color::rgba(40, 44, 52, 22)
}
pub fn port_label() -> Color {
    hex(0x7A8090)
}

// ── Ports ──
pub fn port_ring() -> Color {
    hex(0x3E4450)
}
pub fn port_fill_input() -> Color {
    hex(0xFFFFFF)
}
pub fn port_fill_output() -> Color {
    hex(0x3E4450)
}
pub fn port_hover() -> Color {
    accent()
}

// ── Wires ──
pub fn wire() -> Color {
    hex(0x6A7280)
}
pub fn wire_selected() -> Color {
    accent()
}
pub fn wire_on() -> Color {
    hex(0xE39B00) // amber: energized
}
pub fn wire_off() -> Color {
    hex(0xBFC4CC)
}
pub fn wire_preview() -> Color {
    Color::rgba(0x2F, 0x6F, 0xDE, 200)
}
pub fn wire_invalid() -> Color {
    hex(0xD64545)
}

// ── Selection / accents ──
pub fn accent() -> Color {
    hex(0x2F6FDE)
}
pub fn accent_soft() -> Color {
    Color::rgba(0x2F, 0x6F, 0xDE, 28)
}
pub fn ghost_invalid() -> Color {
    Color::rgba(0xD6, 0x45, 0x45, 90)
}
pub fn danger() -> Color {
    hex(0xD64545)
}

// ── Values (run mode) ──
pub fn value_on() -> Color {
    hex(0xE39B00)
}
pub fn value_off() -> Color {
    hex(0x9AA0AB)
}
pub fn badge_on_bg() -> Color {
    hex(0xFFF3D6)
}
pub fn badge_off_bg() -> Color {
    hex(0xEDEFF2)
}
pub fn value_digit_on() -> Color {
    hex(0xB87A00) // dark amber, readable on the tinted body
}
pub fn value_digit_off() -> Color {
    hex(0x8A909C)
}

// ── Abstraction interface pins ──
pub fn pin_plate_input_bg() -> Color {
    hex(0xE3EDFD) // faint blue: signals flowing in
}
pub fn pin_plate_output_bg() -> Color {
    hex(0xFDF3DC) // faint amber: signals flowing out
}
pub fn pin_plate_border() -> Color {
    hex(0xB9C2D0)
}
pub fn pin_label() -> Color {
    hex(0x3E4450)
}

// ── Notes ──
pub fn note_bg() -> Color {
    hex(0xFFF7D6)
}
pub fn note_border() -> Color {
    hex(0xE3D9A8)
}
pub fn note_text() -> Color {
    hex(0x5C5638)
}

// ── UI chrome ──
pub fn panel_bg() -> Color {
    hex(0xF3F4F1)
}
pub fn panel_border() -> Color {
    hex(0xDDDFDA)
}
pub fn panel_item_hover() -> Color {
    hex(0xE9EBE6)
}
pub fn panel_item_armed() -> Color {
    accent_soft()
}
pub fn text() -> Color {
    hex(0x2B303B)
}
pub fn text_dim() -> Color {
    hex(0x8A909C)
}
pub fn text_disabled() -> Color {
    hex(0xC4C8CF)
}
pub fn button_bg() -> Color {
    hex(0xFFFFFF)
}
pub fn button_border() -> Color {
    hex(0xCFD3D9)
}
pub fn button_hover_bg() -> Color {
    hex(0xEEF2FB)
}
pub fn run_active_bg() -> Color {
    hex(0xFFF3D6)
}
pub fn run_active_border() -> Color {
    hex(0xE39B00)
}
pub fn status_error() -> Color {
    danger()
}
pub fn status_info() -> Color {
    hex(0x5A6270)
}

// ── Layout constants (pixels) ──
pub const PALETTE_WIDTH: f32 = 190.0;
pub const TOPBAR_HEIGHT: f32 = 42.0;
pub const STATUSBAR_HEIGHT: f32 = 26.0;

/// Unlit segments of the segment displays: barely-there on the paper.
pub fn segment_off() -> Color {
    Color::rgb(242, 240, 236)
}
