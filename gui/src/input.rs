//! Typed move entry: a one-line box accepting SAN ("Nf3") or UCI ("g1f3").

use chess::{Board, Move};

#[derive(Default)]
pub struct MoveInput {
    text: String,
    rejected: bool,
}

impl MoveInput {
    /// Shows the entry box. Returns the parsed move on submit, plus whether
    /// the box has keyboard focus (so the board's arrow-key cursor can yield).
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        board: &Board,
        enabled: bool,
    ) -> (Option<Move>, bool) {
        let mut played = None;
        let edit = egui::TextEdit::singleline(&mut self.text)
            .hint_text("e4, Nf3, e7e8q…")
            .desired_width(f32::INFINITY);
        let response = ui.add_enabled(enabled, edit);
        if response.changed() {
            self.rejected = false;
        }
        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let entered = self.text.trim();
            if !entered.is_empty() {
                // SAN first: it covers what players type by hand; the UCI
                // form is the fallback so "e2e4" works too.
                played = board
                    .parse_san(entered)
                    .or_else(|| board.parse_move(entered));
                if played.is_some() {
                    self.text.clear();
                    self.rejected = false;
                } else {
                    self.rejected = true;
                }
            }
            // Keep the box ready for the next move without re-clicking.
            response.request_focus();
        }
        if self.rejected {
            ui.colored_label(ui.visuals().error_fg_color, "Not a legal move");
        }
        (played, response.has_focus())
    }
}
