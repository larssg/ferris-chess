//! The small built-in opening repertoire ported from the pinned GoChess
//! reference. Keys are complete UCI move histories; values are acceptable
//! replies. Keeping lookup history-based mirrors the reference and avoids
//! treating transpositions as book positions accidentally.

use crate::{Board, Move};
use std::sync::atomic::{AtomicUsize, Ordering};

static CHOICE: AtomicUsize = AtomicUsize::new(0);

/// Returns a legal book reply, cycling through equivalent reference choices.
/// Cycling keeps the dependency-free engine varied while tests remain
/// deterministic within a process.
pub fn book_move(board: &Board, history: &[String]) -> Option<Move> {
    let key = history.join(" ");
    let replies = BOOK.iter().find(|(position, _)| *position == key)?.1;
    let start = CHOICE.fetch_add(1, Ordering::Relaxed) % replies.len();
    (0..replies.len())
        .find_map(|offset| board.parse_move(replies[(start + offset) % replies.len()]))
}

const BOOK: &[(&str, &[&str])] = &[
    ("", &["e2e4", "d2d4", "c2c4", "g1f3"]),
    ("e2e4", &["e7e5", "c7c5", "e7e6", "c7c6", "d7d5"]),
    ("e2e4 e7e5", &["g1f3", "f1c4", "b1c3", "f2f4"]),
    ("e2e4 e7e5 g1f3", &["b8c6", "g8f6", "d7d6"]),
    ("e2e4 e7e5 g1f3 b8c6", &["f1b5", "f1c4", "d2d4", "b1c3"]),
    ("e2e4 e7e5 g1f3 b8c6 f1b5", &["a7a6", "g8f6", "f7f5"]),
    ("e2e4 e7e5 g1f3 b8c6 f1b5 a7a6", &["b5a4", "b5c6"]),
    (
        "e2e4 e7e5 g1f3 b8c6 f1b5 a7a6 b5a4",
        &["g8f6", "d7d6", "b7b5"],
    ),
    ("e2e4 e7e5 g1f3 b8c6 f1b5 a7a6 b5a4 g8f6", &["e1g1", "d2d3"]),
    (
        "e2e4 e7e5 g1f3 b8c6 f1b5 a7a6 b5a4 g8f6 e1g1",
        &["f8e7", "b7b5", "f6e4"],
    ),
    ("e2e4 e7e5 g1f3 b8c6 f1b5 g8f6", &["e1g1", "d2d3"]),
    ("e2e4 e7e5 g1f3 b8c6 f1c4", &["f8c5", "g8f6"]),
    ("e2e4 e7e5 g1f3 b8c6 f1c4 f8c5", &["c2c3", "d2d3", "b2b4"]),
    ("e2e4 e7e5 g1f3 b8c6 f1c4 g8f6", &["d2d3", "d2d4", "b1c3"]),
    ("e2e4 e7e5 g1f3 b8c6 d2d4", &["e5d4"]),
    ("e2e4 e7e5 g1f3 b8c6 d2d4 e5d4", &["f3d4", "c2c3"]),
    ("e2e4 e7e5 g1f3 g8f6", &["f3e5", "b1c3", "d2d4"]),
    ("e2e4 e7e5 g1f3 g8f6 f3e5", &["d7d6"]),
    ("e2e4 e7e5 g1f3 g8f6 f3e5 d7d6", &["e5f3", "e5c4"]),
    ("e2e4 e7e5 f2f4", &["e5f4", "f8c5", "d7d5"]),
    ("e2e4 e7e5 f2f4 e5f4", &["g1f3", "f1c4"]),
    ("e2e4 c7c5", &["g1f3", "b1c3", "c2c3", "d2d4"]),
    ("e2e4 c7c5 g1f3", &["d7d6", "b8c6", "e7e6"]),
    ("e2e4 c7c5 g1f3 d7d6", &["d2d4"]),
    ("e2e4 c7c5 g1f3 d7d6 d2d4", &["c5d4"]),
    ("e2e4 c7c5 g1f3 d7d6 d2d4 c5d4", &["f3d4"]),
    ("e2e4 c7c5 g1f3 d7d6 d2d4 c5d4 f3d4", &["g8f6", "b8c6"]),
    ("e2e4 c7c5 g1f3 d7d6 d2d4 c5d4 f3d4 g8f6", &["b1c3"]),
    (
        "e2e4 c7c5 g1f3 d7d6 d2d4 c5d4 f3d4 g8f6 b1c3",
        &["a7a6", "b8c6", "e7e6", "g7g6"],
    ),
    (
        "e2e4 c7c5 g1f3 d7d6 d2d4 c5d4 f3d4 g8f6 b1c3 a7a6",
        &["f1e2", "f2f3", "c1e3", "f1c4"],
    ),
    (
        "e2e4 c7c5 g1f3 d7d6 d2d4 c5d4 f3d4 g8f6 b1c3 g7g6",
        &["f1e2", "f2f3", "c1e3"],
    ),
    (
        "e2e4 c7c5 g1f3 d7d6 d2d4 c5d4 f3d4 g8f6 b1c3 e7e6",
        &["f1e2", "g2g4", "c1e3"],
    ),
    (
        "e2e4 c7c5 g1f3 d7d6 d2d4 c5d4 f3d4 g8f6 b1c3 b8c6",
        &["f1c4", "c1e3", "f1e2"],
    ),
    ("e2e4 c7c5 g1f3 b8c6", &["d2d4", "f1b5"]),
    ("e2e4 c7c5 g1f3 e7e6", &["d2d4", "b1c3"]),
    ("e2e4 c7c5 b1c3", &["b8c6", "d7d6", "e7e6"]),
    ("e2e4 e7e6", &["d2d4"]),
    ("e2e4 e7e6 d2d4", &["d7d5"]),
    ("e2e4 e7e6 d2d4 d7d5", &["b1c3", "b1d2", "e4e5", "e4d5"]),
    ("e2e4 e7e6 d2d4 d7d5 b1c3", &["g8f6", "f8b4", "d5e4"]),
    ("e2e4 e7e6 d2d4 d7d5 b1c3 g8f6", &["c1g5", "e4e5"]),
    ("e2e4 e7e6 d2d4 d7d5 b1c3 f8b4", &["e4e5", "a2a3"]),
    ("e2e4 e7e6 d2d4 d7d5 b1d2", &["g8f6", "c7c5"]),
    ("e2e4 e7e6 d2d4 d7d5 e4e5", &["c7c5"]),
    ("e2e4 e7e6 d2d4 d7d5 e4e5 c7c5", &["c2c3", "g1f3"]),
    ("e2e4 c7c6", &["d2d4", "b1c3", "g1f3"]),
    ("e2e4 c7c6 d2d4", &["d7d5"]),
    ("e2e4 c7c6 d2d4 d7d5", &["b1c3", "b1d2", "e4e5", "e4d5"]),
    ("e2e4 c7c6 d2d4 d7d5 b1c3", &["d5e4"]),
    ("e2e4 c7c6 d2d4 d7d5 b1c3 d5e4", &["c3e4"]),
    ("e2e4 c7c6 d2d4 d7d5 b1c3 d5e4 c3e4", &["c8f5", "g8f6"]),
    ("e2e4 c7c6 d2d4 d7d5 e4d5", &["c6d5"]),
    ("e2e4 c7c6 d2d4 d7d5 e4e5", &["c8f5"]),
    ("e2e4 d7d5", &["e4d5"]),
    ("e2e4 d7d5 e4d5", &["d8d5", "g8f6"]),
    ("e2e4 d7d5 e4d5 d8d5", &["b1c3"]),
    ("e2e4 d7d5 e4d5 d8d5 b1c3", &["d5a5", "d5d6"]),
    ("d2d4", &["d7d5", "g8f6", "e7e6", "f7f5"]),
    ("d2d4 d7d5", &["c2c4", "g1f3", "c1f4"]),
    ("d2d4 d7d5 c2c4", &["e7e6", "c7c6", "d5c4"]),
    ("d2d4 d7d5 c2c4 e7e6", &["b1c3", "g1f3"]),
    ("d2d4 d7d5 c2c4 e7e6 b1c3", &["g8f6", "c7c6", "f8e7"]),
    ("d2d4 d7d5 c2c4 e7e6 b1c3 g8f6", &["c1g5", "g1f3", "c4d5"]),
    (
        "d2d4 d7d5 c2c4 e7e6 b1c3 g8f6 c1g5",
        &["f8e7", "b8d7", "h7h6"],
    ),
    ("d2d4 d7d5 c2c4 e7e6 g1f3", &["g8f6", "c7c6"]),
    ("d2d4 d7d5 c2c4 e7e6 g1f3 g8f6", &["b1c3", "c1g5"]),
    ("d2d4 d7d5 c2c4 c7c6", &["g1f3", "b1c3"]),
    ("d2d4 d7d5 c2c4 c7c6 g1f3", &["g8f6", "e7e6"]),
    ("d2d4 d7d5 c2c4 c7c6 g1f3 g8f6", &["b1c3", "e2e3"]),
    ("d2d4 d7d5 c2c4 c7c6 b1c3", &["g8f6", "d5c4", "e7e6"]),
    ("d2d4 d7d5 c2c4 d5c4", &["g1f3", "e2e3", "e2e4"]),
    ("d2d4 d7d5 c2c4 d5c4 g1f3", &["g8f6", "e7e6"]),
    ("d2d4 d7d5 c2c4 d5c4 e2e4", &["e7e5", "g8f6"]),
    ("d2d4 g8f6", &["c2c4", "g1f3", "c1f4"]),
    ("d2d4 g8f6 c2c4", &["e7e6", "g7g6", "c7c5", "e7e5"]),
    ("d2d4 g8f6 c2c4 e7e6", &["b1c3", "g1f3", "g2g3"]),
    ("d2d4 g8f6 c2c4 e7e6 b1c3", &["f8b4", "d7d5"]),
    (
        "d2d4 g8f6 c2c4 e7e6 b1c3 f8b4",
        &["d1c2", "e2e3", "c1g5", "g1f3"],
    ),
    ("d2d4 g8f6 c2c4 e7e6 g1f3", &["b7b6", "d7d5", "f8b4"]),
    ("d2d4 g8f6 c2c4 e7e6 g1f3 b7b6", &["g2g3", "b1c3", "e2e3"]),
    ("d2d4 g8f6 c2c4 g7g6", &["b1c3", "g1f3"]),
    ("d2d4 g8f6 c2c4 g7g6 b1c3", &["f8g7", "d7d5"]),
    ("d2d4 g8f6 c2c4 g7g6 b1c3 f8g7", &["e2e4", "g1f3"]),
    ("d2d4 g8f6 c2c4 g7g6 b1c3 f8g7 e2e4", &["d7d6", "e8g8"]),
    (
        "d2d4 g8f6 c2c4 g7g6 b1c3 f8g7 e2e4 d7d6",
        &["g1f3", "f2f3", "f1e2"],
    ),
    ("d2d4 g8f6 c2c4 g7g6 b1c3 f8g7 e2e4 d7d6 g1f3", &["e8g8"]),
    (
        "d2d4 g8f6 c2c4 g7g6 b1c3 f8g7 e2e4 d7d6 g1f3 e8g8",
        &["f1e2"],
    ),
    ("d2d4 g8f6 c2c4 g7g6 b1c3 d7d5", &["c4d5", "g1f3"]),
    ("d2d4 g8f6 c2c4 g7g6 b1c3 d7d5 c4d5", &["f6d5"]),
    ("d2d4 g8f6 c2c4 g7g6 b1c3 d7d5 c4d5 f6d5", &["e2e4"]),
    ("d2d4 g8f6 c2c4 c7c5", &["d4d5"]),
    ("d2d4 g8f6 c2c4 c7c5 d4d5", &["e7e6", "d7d6"]),
    ("d2d4 g8f6 c2c4 c7c5 d4d5 e7e6", &["b1c3"]),
    ("d2d4 g8f6 c2c4 e7e6 g1f3 f8b4", &["c1d2", "b1d2"]),
    ("d2d4 f7f5", &["g2g3", "c2c4", "g1f3"]),
    ("d2d4 f7f5 g2g3", &["g8f6", "e7e6"]),
    ("d2d4 f7f5 c2c4", &["g8f6", "e7e6"]),
    ("d2d4 d7d5 c1f4", &["g8f6", "c7c5", "e7e6"]),
    ("d2d4 d7d5 c1f4 g8f6", &["e2e3", "g1f3"]),
    ("c2c4", &["e7e5", "g8f6", "c7c5", "e7e6"]),
    ("c2c4 e7e5", &["b1c3", "g2g3"]),
    ("c2c4 e7e5 b1c3", &["g8f6", "b8c6"]),
    ("c2c4 g8f6", &["b1c3", "g2g3", "d2d4"]),
    ("c2c4 c7c5", &["g1f3", "b1c3"]),
    ("g1f3", &["d7d5", "g8f6", "c7c5"]),
    ("g1f3 d7d5", &["g2g3", "c2c4", "d2d4"]),
    ("g1f3 d7d5 g2g3", &["g8f6", "c7c5"]),
    ("g1f3 g8f6", &["g2g3", "c2c4", "d2d4"]),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_repertoire_is_complete_and_legal_on_main_lines() {
        assert_eq!(BOOK.len(), 108);
        let lines = [
            ["e2e4", "e7e5", "g1f3", "b8c6"].as_slice(),
            ["d2d4", "d7d5", "c2c4", "e7e6"].as_slice(),
            ["c2c4", "e7e5"].as_slice(),
        ];
        for line in lines {
            let mut board = Board::startpos();
            let mut history = Vec::new();
            for notation in line {
                let chess_move = board.parse_move(notation).unwrap();
                board.make_move(chess_move);
                history.push((*notation).to_owned());
            }
            assert!(book_move(&board, &history).is_some());
        }
    }
}
