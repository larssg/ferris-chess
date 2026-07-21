//! The board: painting (squares, highlights, pieces, hints) and pointer
//! interaction (click-click and drag-and-drop). Moves are never assembled by
//! hand — the widget builds a UCI string and lets `Board::parse_move` resolve
//! the castle/en-passant flags, exactly like the UCI front-end does.

use crate::pieces::piece_source;
use chess::board::square_name;
use chess::{Board, Move, Piece, PieceKind};
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};

const LIGHT_SQUARE: Color32 = Color32::from_rgb(240, 217, 181);
const DARK_SQUARE: Color32 = Color32::from_rgb(181, 136, 99);
const LAST_MOVE_TINT: Color32 = Color32::from_rgba_premultiplied(120, 150, 15, 90);
const SELECTED_TINT: Color32 = Color32::from_rgba_premultiplied(20, 100, 40, 90);
const CHECK_TINT: Color32 = Color32::from_rgba_premultiplied(150, 20, 20, 90);
const HINT_COLOR: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 45);
const CURSOR_COLOR: Color32 = Color32::from_rgb(70, 130, 200);

const PROMOTION_CHOICES: [PieceKind; 4] = [
    PieceKind::Queen,
    PieceKind::Rook,
    PieceKind::Bishop,
    PieceKind::Knight,
];

pub struct BoardUi {
    pub flipped: bool,
    selected: Option<u8>,
    dragging: bool,
    /// A from/to pair waiting on the promotion picker.
    pending_promotion: Option<(u8, u8)>,
    /// The arrow-key square cursor; appears on first arrow press.
    cursor: Option<u8>,
    /// Which of [`PROMOTION_CHOICES`] the keyboard has highlighted.
    promo_index: usize,
}

impl BoardUi {
    pub fn new(flipped: bool) -> Self {
        Self {
            flipped,
            selected: None,
            dragging: false,
            pending_promotion: None,
            cursor: None,
            promo_index: 0,
        }
    }

    pub fn clear_interaction(&mut self) {
        self.selected = None;
        self.dragging = false;
        self.pending_promotion = None;
        self.cursor = None;
        self.promo_index = 0;
    }

    /// Draws the board and handles input; returns a move the human played.
    /// `keys_enabled` is false while the typed-move box owns the keyboard.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        board: &Board,
        last_move: Option<Move>,
        interactive: bool,
        keys_enabled: bool,
    ) -> Option<Move> {
        // Whole pixels per square keep the SVG rasterizer from re-rendering
        // on sub-pixel size wobble.
        let square = (ui.available_size().min_elem() / 8.0).floor().max(24.0);
        let (response, painter) =
            ui.allocate_painter(Vec2::splat(square * 8.0), Sense::click_and_drag());
        let origin = response.rect;

        if !interactive {
            self.clear_interaction();
        }
        let mut played = self.handle_pointer(&response, board, origin, square, interactive);
        if played.is_none() && interactive && keys_enabled {
            played = played.or_else(|| self.handle_keys(ui, board));
        }

        // Paint order: squares, tints, hints, pieces; the dragged piece last
        // so it floats above everything.
        for sq in 0..64u8 {
            let rect = self.rect_of(sq, origin, square);
            let light = (sq % 8 + sq / 8) % 2 == 1;
            painter.rect_filled(rect, 0.0, if light { LIGHT_SQUARE } else { DARK_SQUARE });
        }
        if let Some(m) = last_move {
            for sq in [m.from, m.to] {
                painter.rect_filled(self.rect_of(sq, origin, square), 0.0, LAST_MOVE_TINT);
            }
        }
        if board.in_check(board.side)
            && let Some(king) = board.king_square(board.side)
        {
            painter.rect_filled(self.rect_of(king, origin, square), 0.0, CHECK_TINT);
        }
        if let Some(selected) = self.selected {
            painter.rect_filled(self.rect_of(selected, origin, square), 0.0, SELECTED_TINT);
        }
        self.paint_coordinates(&painter, origin, square);
        for sq in 0..64u8 {
            if self.dragging && self.selected == Some(sq) {
                continue; // this one rides the pointer below
            }
            if let Some(piece) = board.piece_at(sq) {
                let rect = self.rect_of(sq, origin, square).shrink(square * 0.03);
                egui::Image::new(piece_source(piece)).paint_at(ui, rect);
            }
        }
        if let Some(selected) = self.selected {
            self.paint_move_hints(&painter, board, selected, origin, square);
        }
        if let Some(cursor) = self.cursor
            && interactive
        {
            let width = square * 0.055;
            painter.rect_stroke(
                self.rect_of(cursor, origin, square).shrink(width / 2.0),
                0.0,
                Stroke::new(width, CURSOR_COLOR),
                egui::StrokeKind::Inside,
            );
        }
        if self.dragging
            && let (Some(from), Some(pos)) = (self.selected, response.interact_pointer_pos())
            && let Some(piece) = board.piece_at(from)
        {
            let size = Vec2::splat(square * 1.1);
            let rect = Rect::from_center_size(pos, size);
            egui::Image::new(piece_source(piece)).paint_at(ui, rect);
        }

        if played.is_none() {
            played = self.promotion_picker(ui, board, keys_enabled);
        }
        played
    }

    /// Arrow keys move a square cursor (screen-relative, so flipping the
    /// board flips the arrows too); Enter picks up and drops, Esc clears.
    fn handle_keys(&mut self, ui: &egui::Ui, board: &Board) -> Option<Move> {
        use egui::{Key, Modifiers};
        if self.pending_promotion.is_some() {
            return None; // the picker owns the keyboard for now
        }
        let (up, down, left, right, enter, escape) = ui.input_mut(|input| {
            (
                input.consume_key(Modifiers::NONE, Key::ArrowUp),
                input.consume_key(Modifiers::NONE, Key::ArrowDown),
                input.consume_key(Modifiers::NONE, Key::ArrowLeft),
                input.consume_key(Modifiers::NONE, Key::ArrowRight),
                input.consume_key(Modifiers::NONE, Key::Enter),
                input.consume_key(Modifiers::NONE, Key::Escape),
            )
        });
        if escape {
            self.selected = None;
            return None;
        }
        let step_file = i32::from(right) - i32::from(left);
        let step_rank = i32::from(up) - i32::from(down);
        if step_file != 0 || step_rank != 0 {
            let from = self
                .cursor
                .or_else(|| board.king_square(board.side))
                .unwrap_or(28);
            let (df, dr) = if self.flipped {
                (-step_file, -step_rank)
            } else {
                (step_file, step_rank)
            };
            let file = (i32::from(from % 8) + df).clamp(0, 7);
            let rank = (i32::from(from / 8) + dr).clamp(0, 7);
            // The first arrow press only summons the cursor.
            self.cursor = Some(if self.cursor.is_some() {
                (rank * 8 + file) as u8
            } else {
                from
            });
        }
        if enter && let Some(sq) = self.cursor {
            let own = board
                .piece_at(sq)
                .is_some_and(|piece| piece.color == board.side);
            match self.selected {
                Some(from) if from != sq && !own => return self.resolve_target(board, from, sq),
                Some(from) if from == sq => self.selected = None,
                _ if own => self.selected = Some(sq),
                _ => {}
            }
        }
        None
    }

    fn handle_pointer(
        &mut self,
        response: &egui::Response,
        board: &Board,
        origin: Rect,
        square: f32,
        interactive: bool,
    ) -> Option<Move> {
        if !interactive || self.pending_promotion.is_some() {
            self.dragging = false;
            return None;
        }
        let pointer_square = response
            .interact_pointer_pos()
            .and_then(|pos| self.square_at(pos, origin, square));
        let own_piece = |sq: u8| board.piece_at(sq).is_some_and(|p| p.color == board.side);

        if response.drag_started()
            && let Some(sq) = pointer_square
            && own_piece(sq)
        {
            self.selected = Some(sq);
            self.dragging = true;
            return None;
        }
        if response.drag_stopped() && self.dragging {
            self.dragging = false;
            if let (Some(from), Some(to)) = (self.selected, pointer_square)
                && from != to
            {
                return self.resolve_target(board, from, to);
            }
            return None;
        }
        if response.clicked()
            && let Some(sq) = pointer_square
        {
            match self.selected {
                Some(from) if from != sq && !own_piece(sq) => {
                    let played = self.resolve_target(board, from, sq);
                    if played.is_none() && self.pending_promotion.is_none() {
                        self.selected = None;
                    }
                    return played;
                }
                Some(from) if from == sq => self.selected = None,
                _ if own_piece(sq) => self.selected = Some(sq),
                _ => self.selected = None,
            }
        }
        None
    }

    /// Turns a from/to pair into the legal move it names, or parks it in the
    /// promotion picker when several promotions match.
    fn resolve_target(&mut self, board: &Board, from: u8, to: u8) -> Option<Move> {
        let mut candidates = board
            .legal_moves()
            .into_iter()
            .filter(|m| m.from == from && m.to == to);
        let first = candidates.next()?;
        if first.promotion.is_some() {
            self.pending_promotion = Some((from, to));
            return None;
        }
        self.selected = None;
        Some(first)
    }

    fn promotion_picker(
        &mut self,
        ui: &mut egui::Ui,
        board: &Board,
        keys_enabled: bool,
    ) -> Option<Move> {
        let (from, to) = self.pending_promotion?;
        let mut played = None;
        let mut cancelled = false;
        if keys_enabled {
            use egui::{Key, Modifiers};
            let (left, right, enter, escape) = ui.input_mut(|input| {
                (
                    input.consume_key(Modifiers::NONE, Key::ArrowLeft),
                    input.consume_key(Modifiers::NONE, Key::ArrowRight),
                    input.consume_key(Modifiers::NONE, Key::Enter),
                    input.consume_key(Modifiers::NONE, Key::Escape),
                )
            });
            if left {
                self.promo_index = self.promo_index.saturating_sub(1);
            }
            if right {
                self.promo_index = (self.promo_index + 1).min(PROMOTION_CHOICES.len() - 1);
            }
            if enter {
                let letter = PROMOTION_CHOICES[self.promo_index].letter();
                let uci = format!("{}{}{}", square_name(from), square_name(to), letter);
                played = board.parse_move(&uci);
            }
            cancelled = escape;
        }
        egui::Window::new("promotion_picker")
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ui.ctx(), |ui| {
                ui.label("Promote to");
                ui.horizontal(|ui| {
                    for (index, kind) in PROMOTION_CHOICES.into_iter().enumerate() {
                        let piece = Piece {
                            color: board.side,
                            kind,
                        };
                        let image = egui::Image::new(piece_source(piece))
                            .fit_to_exact_size(Vec2::splat(48.0));
                        let button = egui::Button::image(image).selected(index == self.promo_index);
                        if ui.add(button).clicked() {
                            let uci = format!(
                                "{}{}{}",
                                square_name(from),
                                square_name(to),
                                kind.letter()
                            );
                            played = board.parse_move(&uci);
                        }
                    }
                });
                cancelled |= ui.button("Cancel").clicked();
            });
        if played.is_some() || cancelled {
            self.pending_promotion = None;
            self.selected = None;
            self.promo_index = 0;
        }
        played
    }

    fn paint_move_hints(
        &self,
        painter: &egui::Painter,
        board: &Board,
        from: u8,
        origin: Rect,
        square: f32,
    ) {
        for m in board.legal_moves() {
            if m.from != from {
                continue;
            }
            let center = self.rect_of(m.to, origin, square).center();
            let capture = m.en_passant || board.piece_at(m.to).is_some();
            if capture {
                painter.circle_stroke(
                    center,
                    square * 0.44,
                    Stroke::new(square * 0.07, HINT_COLOR),
                );
            } else {
                painter.circle_filled(center, square * 0.16, HINT_COLOR);
            }
        }
    }

    fn paint_coordinates(&self, painter: &egui::Painter, origin: Rect, square: f32) {
        let font = FontId::proportional(square * 0.2);
        for screen_file in 0..8u8 {
            let file = if self.flipped {
                7 - screen_file
            } else {
                screen_file
            };
            let sq_bottom = if self.flipped { 56 + file } else { file };
            let rect = self.rect_of(sq_bottom, origin, square);
            let color = square_label_color(sq_bottom);
            painter.text(
                rect.right_bottom() + Vec2::new(-2.0, -1.0),
                Align2::RIGHT_BOTTOM,
                ((b'a' + file) as char).to_string(),
                font.clone(),
                color,
            );
        }
        for screen_rank in 0..8u8 {
            let rank = if self.flipped {
                screen_rank
            } else {
                7 - screen_rank
            };
            let sq_left = if self.flipped { rank * 8 + 7 } else { rank * 8 };
            let rect = self.rect_of(sq_left, origin, square);
            let color = square_label_color(sq_left);
            painter.text(
                rect.left_top() + Vec2::new(2.0, 1.0),
                Align2::LEFT_TOP,
                (rank + 1).to_string(),
                font.clone(),
                color,
            );
        }
    }

    fn square_at(&self, pos: Pos2, origin: Rect, square: f32) -> Option<u8> {
        if !origin.contains(pos) {
            return None;
        }
        let col = (((pos.x - origin.left()) / square) as u8).min(7);
        let row = (((pos.y - origin.top()) / square) as u8).min(7);
        let file = if self.flipped { 7 - col } else { col };
        let rank = if self.flipped { row } else { 7 - row };
        Some(rank * 8 + file)
    }

    fn rect_of(&self, sq: u8, origin: Rect, square: f32) -> Rect {
        let file = sq % 8;
        let rank = sq / 8;
        let col = if self.flipped { 7 - file } else { file };
        let row = if self.flipped { rank } else { 7 - rank };
        Rect::from_min_size(
            origin.min + Vec2::new(f32::from(col) * square, f32::from(row) * square),
            Vec2::splat(square),
        )
    }
}

/// Coordinate labels sit on the square itself, so use the opposite square
/// color for contrast.
fn square_label_color(sq: u8) -> Color32 {
    if (sq % 8 + sq / 8) % 2 == 1 {
        DARK_SQUARE
    } else {
        LIGHT_SQUARE
    }
}
