//! The two chess clocks in the side panel.

use crate::game::GameState;
use std::time::Duration;

pub fn show_clocks(ui: &mut egui::Ui, game: &GameState) {
    let engine_side = game.human.other();
    for (label, color) in [("Engine", engine_side), ("You", game.human)] {
        let remaining = game.remaining(color);
        let ticking = !game.status.is_over() && game.board.side == color;
        let text = egui::RichText::new(format!("{label:<7}{}", format_clock(remaining)))
            .monospace()
            .size(22.0);
        ui.label(if ticking { text.strong() } else { text.weak() });
    }
}

/// `m:ss` normally, tenths once a flag fall gets close.
pub fn format_clock(remaining: Duration) -> String {
    if remaining < Duration::from_secs(20) {
        format!("{:.1}", remaining.as_secs_f64())
    } else {
        let total = remaining.as_secs();
        format!("{}:{:02}", total / 60, total % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_formatting_switches_to_tenths_when_low() {
        assert_eq!(format_clock(Duration::from_secs(300)), "5:00");
        assert_eq!(format_clock(Duration::from_secs(61)), "1:01");
        assert_eq!(format_clock(Duration::from_millis(12_340)), "12.3");
        assert_eq!(format_clock(Duration::ZERO), "0.0");
    }
}
