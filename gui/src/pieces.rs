//! The embedded cburnett piece set (see `assets/pieces/COPYING.md` for
//! attribution). egui caches the rasterized texture per (uri, size), so
//! handing out `ImageSource`s per frame is cheap.

use chess::{Color, Piece, PieceKind};

pub fn piece_source(piece: Piece) -> egui::ImageSource<'static> {
    use Color::*;
    use PieceKind::*;
    match (piece.color, piece.kind) {
        (White, Pawn) => egui::include_image!("../assets/pieces/wP.svg"),
        (White, Knight) => egui::include_image!("../assets/pieces/wN.svg"),
        (White, Bishop) => egui::include_image!("../assets/pieces/wB.svg"),
        (White, Rook) => egui::include_image!("../assets/pieces/wR.svg"),
        (White, Queen) => egui::include_image!("../assets/pieces/wQ.svg"),
        (White, King) => egui::include_image!("../assets/pieces/wK.svg"),
        (Black, Pawn) => egui::include_image!("../assets/pieces/bP.svg"),
        (Black, Knight) => egui::include_image!("../assets/pieces/bN.svg"),
        (Black, Bishop) => egui::include_image!("../assets/pieces/bB.svg"),
        (Black, Rook) => egui::include_image!("../assets/pieces/bR.svg"),
        (Black, Queen) => egui::include_image!("../assets/pieces/bQ.svg"),
        (Black, King) => egui::include_image!("../assets/pieces/bK.svg"),
    }
}
