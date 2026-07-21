use crate::{Board, Color, Move, Piece, PieceKind};

const VALUES: [i32; 6] = [91, 360, 379, 607, 1239, 20_000];
const BISHOP_PAIR: i32 = 48;
const PAWN_SHIELD: i32 = 15;
const OPEN_KING_FILE: i32 = 16;
const CASTLED_KING: i32 = 22;
const ROOK_OPEN_FILE: i32 = 34;
const ROOK_SEMI_OPEN_FILE: i32 = 26;
const ROOK_SEVENTH: i32 = 10;
const ROOK_TRAPS_KING: i32 = 21;
const PASSED_RANK_SQUARED: i32 = 4;
const BACKWARD_PAWN: i32 = 10;
const CONNECTED_PASSED_BASE: i32 = -2;
const CONNECTED_PASSED_RANK: i32 = 6;
const DOUBLED_PAWN: i32 = 8;
const ISOLATED_PAWN: i32 = 13;
const MOP_UP_CORNER: i32 = 14;
const MOP_UP_PROXIMITY: i32 = -5;

const PST: [[i16; 64]; 7] = include!("eval_tables.in");

pub(crate) const fn piece_value(kind: PieceKind) -> i32 {
    VALUES[kind as usize - 1]
}

fn is_endgame(board: &Board) -> bool {
    let count = |color, kind| board.pieces(color, kind).count_ones();
    let wq = count(Color::White, PieceKind::Queen);
    let bq = count(Color::Black, PieceKind::Queen);
    let wm = count(Color::White, PieceKind::Knight) + count(Color::White, PieceKind::Bishop);
    let bm = count(Color::Black, PieceKind::Knight) + count(Color::Black, PieceKind::Bishop);
    let wr = count(Color::White, PieceKind::Rook);
    let br = count(Color::Black, PieceKind::Rook);
    wq == 0 && bq == 0 || wq > 0 && wm <= 1 && wr == 0 || bq > 0 && bm <= 1 && br == 0
}

fn pst(piece: Piece, square: u8, endgame: bool) -> i32 {
    let square = if piece.color == Color::White {
        square
    } else {
        square ^ 56
    } as usize;
    let table = if piece.kind == PieceKind::King && endgame {
        6
    } else {
        piece.kind as usize - 1
    };
    i32::from(PST[table][square])
}

pub fn evaluate(board: &Board) -> i32 {
    let endgame = is_endgame(board);
    let mut score = 0;
    let mut pawns = [Vec::with_capacity(8), Vec::with_capacity(8)];
    let mut bishops = [0; 2];
    for square in 0..64 {
        let Some(piece) = board.piece_at(square) else {
            continue;
        };
        let side = usize::from(piece.color == Color::Black);
        let signed = piece_value(piece.kind) + pst(piece, square, endgame);
        score += if side == 0 { signed } else { -signed };
        if piece.kind == PieceKind::Pawn {
            pawns[side].push(square);
        }
        if piece.kind == PieceKind::Bishop {
            bishops[side] += 1;
        }
    }
    if bishops[0] >= 2 {
        score += BISHOP_PAIR;
    }
    if bishops[1] >= 2 {
        score -= BISHOP_PAIR;
    }
    if !endgame {
        score += king_safety(board, Color::White, &pawns[0]);
        score -= king_safety(board, Color::Black, &pawns[1]);
    }
    score += rooks(board, Color::White) - rooks(board, Color::Black);
    score += pawn_structure(&pawns[0], &pawns[1], Color::White);
    score -= pawn_structure(&pawns[1], &pawns[0], Color::Black);
    if endgame {
        score += mop_up(board, score);
    }
    if board.side == Color::White {
        score
    } else {
        -score
    }
}

fn pawn_structure(pawns: &[u8], opponents: &[u8], color: Color) -> i32 {
    let mut files = [0i32; 8];
    for &sq in pawns {
        files[(sq % 8) as usize] += 1;
    }
    let mut score = 0;
    let mut passed = Vec::with_capacity(8);
    for &sq in pawns {
        let file = i32::from(sq % 8);
        let rank = i32::from(sq / 8);
        let is_ahead = |other: u8| {
            if color == Color::White {
                i32::from(other / 8) > rank
            } else {
                i32::from(other / 8) < rank
            }
        };
        let is_passed = !opponents
            .iter()
            .any(|&other| (i32::from(other % 8) - file).abs() <= 1 && is_ahead(other));
        if is_passed {
            let relative_rank = if color == Color::White {
                rank
            } else {
                7 - rank
            };
            score += relative_rank * relative_rank * PASSED_RANK_SQUARED;
            passed.push(sq);
        }
        let supported = pawns.iter().any(|&other| {
            (i32::from(other % 8) - file).abs() == 1
                && if color == Color::White {
                    other / 8 <= sq / 8
                } else {
                    other / 8 >= sq / 8
                }
        });
        if !supported && !is_passed {
            let stop = if color == Color::White {
                rank + 1
            } else {
                rank - 1
            };
            let backward = opponents.iter().any(|&other| {
                (i32::from(other % 8) - file).abs() == 1
                    && if color == Color::White {
                        i32::from(other / 8) == stop + 1
                    } else {
                        i32::from(other / 8) == stop - 1
                    }
            });
            if backward {
                score -= BACKWARD_PAWN;
            }
        }
    }
    for (i, &a) in passed.iter().enumerate() {
        for &b in &passed[i + 1..] {
            if (i32::from(a % 8) - i32::from(b % 8)).abs() == 1 {
                let ar = if color == Color::White {
                    a / 8
                } else {
                    7 - a / 8
                };
                let br = if color == Color::White {
                    b / 8
                } else {
                    7 - b / 8
                };
                score += CONNECTED_PASSED_BASE + i32::from(ar.min(br)) * CONNECTED_PASSED_RANK;
            }
        }
    }
    for file in 0..8 {
        if files[file] > 1 {
            score -= (files[file] - 1) * DOUBLED_PAWN;
        }
        if files[file] > 0
            && (file == 0 || files[file - 1] == 0)
            && (file == 7 || files[file + 1] == 0)
        {
            score -= files[file] * ISOLATED_PAWN;
        }
    }
    score
}

fn king_safety(board: &Board, color: Color, pawns: &[u8]) -> i32 {
    let Some(king) = (0..64).find(|&sq| {
        board.piece_at(sq)
            == Some(Piece {
                color,
                kind: PieceKind::King,
            })
    }) else {
        return 0;
    };
    let file = i32::from(king % 8);
    let rank = king / 8;
    let mut score = 0;
    for df in -1..=1 {
        let f = file + df;
        if !(0..8).contains(&f) {
            continue;
        }
        let shielded = pawns.iter().any(|&sq| {
            sq % 8 == f as u8
                && if color == Color::White {
                    sq / 8 >= rank
                } else {
                    sq / 8 <= rank
                }
        });
        score += if shielded {
            PAWN_SHIELD
        } else {
            -OPEN_KING_FILE
        };
    }
    if rank == if color == Color::White { 0 } else { 7 } && !(3..=5).contains(&file) {
        score += CASTLED_KING;
    }
    score
}

fn rooks(board: &Board, color: Color) -> i32 {
    let mut score = 0;
    for sq in 0..64 {
        if board.piece_at(sq)
            != Some(Piece {
                color,
                kind: PieceKind::Rook,
            })
        {
            continue;
        }
        let own = board.pieces(color, PieceKind::Pawn) & (0x0101_0101_0101_0101 << (sq % 8));
        let enemy =
            board.pieces(color.other(), PieceKind::Pawn) & (0x0101_0101_0101_0101 << (sq % 8));
        if own == 0 {
            score += if enemy == 0 {
                ROOK_OPEN_FILE
            } else {
                ROOK_SEMI_OPEN_FILE
            };
        }
        let seventh = if color == Color::White { 6 } else { 1 };
        if sq / 8 == seventh {
            score += ROOK_SEVENTH;
            if (0..64).any(|ksq| {
                board.piece_at(ksq)
                    == Some(Piece {
                        color: color.other(),
                        kind: PieceKind::King,
                    })
                    && ksq / 8 == if color == Color::White { 7 } else { 0 }
            }) {
                score += ROOK_TRAPS_KING;
            }
        }
    }
    score
}

fn mop_up(board: &Board, material: i32) -> i32 {
    if material.abs() < 300 {
        return 0;
    }
    let winner = if material > 0 {
        Color::White
    } else {
        Color::Black
    };
    let king = |color| {
        (0..64).find(|&sq| {
            board.piece_at(sq)
                == Some(Piece {
                    color,
                    kind: PieceKind::King,
                })
        })
    };
    let (Some(wk), Some(lk)) = (king(winner), king(winner.other())) else {
        return 0;
    };
    let (lx, ly) = (i32::from(lk % 8), i32::from(lk / 8));
    let center = (lx - if lx > 3 { 4 } else { 3 }).abs() + (ly - if ly > 3 { 4 } else { 3 }).abs();
    let distance = (i32::from(wk % 8) - lx).abs() + (i32::from(wk / 8) - ly).abs();
    let score = center * MOP_UP_CORNER + (14 - distance) * MOP_UP_PROXIMITY;
    if winner == Color::White {
        score
    } else {
        -score
    }
}

pub fn static_exchange_eval(board: &Board, chess_move: Move) -> i32 {
    let captured = if chess_move.en_passant {
        Some(PieceKind::Pawn)
    } else {
        board.piece_at(chess_move.to).map(|p| p.kind)
    };
    let Some(captured) = captured else { return 0 };
    let mut squares = [None; 64];
    for sq in 0..64 {
        squares[sq as usize] = board.piece_at(sq);
    }
    let mut gain = [0; 32];
    gain[0] = piece_value(captured);
    let mut attacker_value = piece_value(squares[chess_move.from as usize].unwrap().kind);
    squares[chess_move.from as usize] = None;
    let mut side = board.side.other();
    let mut depth = 0usize;
    while depth + 1 < gain.len() {
        let Some((attacker, from)) = least_attacker(&squares, chess_move.to, side) else {
            break;
        };
        depth += 1;
        gain[depth] = attacker_value - gain[depth - 1];
        attacker_value = piece_value(attacker.kind);
        squares[from as usize] = None;
        if (-gain[depth - 1]).max(gain[depth]) < 0 {
            break;
        }
        side = side.other();
    }
    while depth > 0 {
        gain[depth - 1] = -(-gain[depth - 1]).max(gain[depth]);
        depth -= 1;
    }
    gain[0]
}

fn least_attacker(squares: &[Option<Piece>; 64], target: u8, color: Color) -> Option<(Piece, u8)> {
    let (tx, ty) = (i32::from(target % 8), i32::from(target / 8));
    let at = |x: i32, y: i32| {
        if (0..8).contains(&x) && (0..8).contains(&y) {
            squares[(y * 8 + x) as usize]
        } else {
            None
        }
    };
    let pawn_y = ty + if color == Color::White { -1 } else { 1 };
    for x in [tx - 1, tx + 1] {
        if let Some(
            p @ Piece {
                color: c,
                kind: PieceKind::Pawn,
            },
        ) = at(x, pawn_y)
            && c == color
        {
            return Some((p, (pawn_y * 8 + x) as u8));
        }
    }
    for (dx, dy) in [
        (-2, -1),
        (-2, 1),
        (-1, -2),
        (-1, 2),
        (1, -2),
        (1, 2),
        (2, -1),
        (2, 1),
    ] {
        if let Some(
            p @ Piece {
                color: c,
                kind: PieceKind::Knight,
            },
        ) = at(tx + dx, ty + dy)
            && c == color
        {
            return Some((p, ((ty + dy) * 8 + tx + dx) as u8));
        }
    }
    for (dirs, kinds) in [
        (
            &[(-1, -1), (-1, 1), (1, -1), (1, 1)][..],
            &[PieceKind::Bishop, PieceKind::Queen][..],
        ),
        (
            &[(-1, 0), (1, 0), (0, -1), (0, 1)][..],
            &[PieceKind::Rook, PieceKind::Queen][..],
        ),
    ] {
        for &(dx, dy) in dirs {
            for distance in 1..8 {
                let (x, y) = (tx + dx * distance, ty + dy * distance);
                if !(0..8).contains(&x) || !(0..8).contains(&y) {
                    break;
                }
                if let Some(p) = at(x, y) {
                    if p.color == color && kinds.contains(&p.kind) {
                        return Some((p, (y * 8 + x) as u8));
                    }
                    break;
                }
            }
        }
    }
    for y in ty - 1..=ty + 1 {
        for x in tx - 1..=tx + 1 {
            if let Some(
                p @ Piece {
                    color: c,
                    kind: PieceKind::King,
                },
            ) = at(x, y)
                && c == color
            {
                return Some((p, (y * 8 + x) as u8));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_evaluation_fixtures() {
        for (fen, expected) in [
            (
                "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
                0,
            ),
            ("7k/8/8/8/8/8/4Q3/K7 w - - 0 1", 1311),
            ("7k/8/8/8/8/8/4Q3/K7 b - - 0 1", -1311),
        ] {
            assert_eq!(evaluate(&Board::from_fen(fen).unwrap()), expected, "{fen}");
        }
    }

    #[test]
    fn see_matches_reference_exchange_model() {
        let board = Board::from_fen("4k3/8/4p3/3p4/2B5/8/8/4K3 w - - 0 1").unwrap();
        assert_eq!(
            static_exchange_eval(&board, board.parse_move("c4d5").unwrap()),
            -288
        );
        let board = Board::from_fen("4k3/8/8/3q4/2B5/8/8/4K3 w - - 0 1").unwrap();
        assert_eq!(
            static_exchange_eval(&board, board.parse_move("c4d5").unwrap()),
            1239
        );
    }
}
