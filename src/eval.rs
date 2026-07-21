//! Static evaluation: scoring a position without searching.
//!
//! The values and tables here are ported from the GoChess reference so that the
//! two engines evaluate identically (see `PLAN.md`). Everything is computed from
//! White's point of view first and negated for Black at the very end; the term
//! functions themselves take a color and return White-positive contributions.
//!
//! This module also hosts [`static_exchange_eval`] (SEE), which the search uses
//! to score captures without playing them out on the board.

use crate::{Board, Color, Move, Piece, PieceKind};

// Piece values in centipawns (pawn..king). The king's nominal 20,000 keeps it
// from ever being "won" in SEE arithmetic.
const VALUES: [i32; 6] = [91, 360, 379, 607, 1239, 20_000];
// Positional term weights, all in centipawns. Tuned to match the reference.
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

/// Piece-square tables, one 64-entry table per piece kind plus a seventh table
/// (index 6) for the king in the endgame. Indexed from White's perspective;
/// Black squares are mirrored via `square ^ 56` in [`pst`].
const PST: [[i16; 64]; 7] = include!("eval_tables.in");

/// The centipawn value of a piece kind.
pub(crate) const fn piece_value(kind: PieceKind) -> i32 {
    VALUES[kind as usize - 1]
}

/// Whether to use endgame evaluation, using the reference's discrete rule: no
/// queens on either side, or a lone queen backed by at most one minor and no
/// rook. This switches the king's piece-square table and enables mop-up.
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

/// Looks up the piece-square bonus for a piece on a square. Black is mirrored to
/// White's orientation so a single set of tables serves both colors.
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

/// The full static evaluation, in centipawns from the side-to-move's point of
/// view (positive = the side to move is better).
///
/// A single pass over the board accumulates material plus piece-square bonuses
/// and, along the way, collects each side's pawn squares and bishop count for
/// the structural terms that follow. The running `score` is White-positive
/// throughout and only flipped to side-relative at the return.
pub fn evaluate(board: &Board) -> i32 {
    let endgame = is_endgame(board);
    let mut score = 0;
    // Pawn squares per side, gathered here so pawn-structure and king-safety
    // terms don't each have to re-scan the board.
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
    // The bishop pair is worth a small bonus for its coordinated coverage.
    if bishops[0] >= 2 {
        score += BISHOP_PAIR;
    }
    if bishops[1] >= 2 {
        score -= BISHOP_PAIR;
    }
    // King safety matters in the middlegame; in the endgame the king is an
    // active piece and the mop-up term takes over instead.
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
    // Flip to the side-to-move's perspective for the negamax search.
    if board.side == Color::White {
        score
    } else {
        -score
    }
}

/// Scores one side's pawn structure (as a White-positive value): passed-pawn
/// bonuses growing with advancement, connected-passer bonuses, and penalties for
/// backward, doubled, and isolated pawns. `pawns` is the side being scored,
/// `opponents` the enemy pawns used to detect passers and backward pawns.
fn pawn_structure(pawns: &[u8], opponents: &[u8], color: Color) -> i32 {
    // Pawns per file, for the doubled/isolated checks at the end.
    let mut files = [0i32; 8];
    for &sq in pawns {
        files[(sq % 8) as usize] += 1;
    }
    let mut score = 0;
    let mut passed = Vec::with_capacity(8);
    for &sq in pawns {
        let file = i32::from(sq % 8);
        let rank = i32::from(sq / 8);
        // "Ahead" means closer to the enemy back rank, which depends on color.
        let is_ahead = |other: u8| {
            if color == Color::White {
                i32::from(other / 8) > rank
            } else {
                i32::from(other / 8) < rank
            }
        };
        // Passed: no enemy pawn on this or an adjacent file lies ahead to stop it.
        let is_passed = !opponents
            .iter()
            .any(|&other| (i32::from(other % 8) - file).abs() <= 1 && is_ahead(other));
        if is_passed {
            // Reward grows quadratically as the pawn nears promotion.
            let relative_rank = if color == Color::White {
                rank
            } else {
                7 - rank
            };
            score += relative_rank * relative_rank * PASSED_RANK_SQUARED;
            passed.push(sq);
        }
        // Supported: a friendly pawn on an adjacent file, not further advanced.
        let supported = pawns.iter().any(|&other| {
            (i32::from(other % 8) - file).abs() == 1
                && if color == Color::White {
                    other / 8 <= sq / 8
                } else {
                    other / 8 >= sq / 8
                }
        });
        // Backward: an unsupported, non-passed pawn whose advance square is
        // guarded by an enemy pawn, so it cannot safely move forward.
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
    // Connected passers: two passed pawns on adjacent files defend each other's
    // advance. The bonus scales with how far the rear of the pair has advanced.
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
        // Doubled: every pawn beyond the first on a file is penalized.
        if files[file] > 1 {
            score -= (files[file] - 1) * DOUBLED_PAWN;
        }
        // Isolated: pawns on a file with no friendly pawns on either neighbor.
        if files[file] > 0
            && (file == 0 || files[file - 1] == 0)
            && (file == 7 || files[file + 1] == 0)
        {
            score -= files[file] * ISOLATED_PAWN;
        }
    }
    score
}

/// Scores the safety of `color`'s king (White-positive): a bonus per adjacent
/// file that has a sheltering pawn, a penalty per open file beside the king, and
/// a bonus for a king tucked into a castled corner.
fn king_safety(board: &Board, color: Color, pawns: &[u8]) -> i32 {
    let Some(king) = board.king_square(color) else {
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
    // A king on its back rank and off the center files is treated as castled.
    if rank == if color == Color::White { 0 } else { 7 } && !(3..=5).contains(&file) {
        score += CASTLED_KING;
    }
    score
}

/// Scores `color`'s rook placement (White-positive): bonuses for rooks on open
/// or half-open files, on the enemy's second rank, and doubly so when they trap
/// the enemy king on its back rank. The file mask `0x0101...01 << file` selects
/// one whole file.
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
        // No friendly pawn on the file: open (no pawns at all) or half-open.
        if own == 0 {
            score += if enemy == 0 {
                ROOK_OPEN_FILE
            } else {
                ROOK_SEMI_OPEN_FILE
            };
        }
        // A rook on the enemy's second rank (the "seventh" from its own side).
        let seventh = if color == Color::White { 6 } else { 1 };
        if sq / 8 == seventh {
            score += ROOK_SEVENTH;
            // Extra credit when the enemy king is stuck on the back rank behind it.
            let back_rank = if color == Color::White { 7 } else { 0 };
            if board
                .king_square(color.other())
                .is_some_and(|ksq| ksq / 8 == back_rank)
            {
                score += ROOK_TRAPS_KING;
            }
        }
    }
    score
}

/// Endgame "mop-up": when one side is clearly ahead, reward driving the losing
/// king toward a corner and bringing the winning king close, which is what
/// actually converts a won endgame. `material` is the White-positive score so
/// far; its sign identifies the winning side. Returns a White-positive value.
fn mop_up(board: &Board, material: i32) -> i32 {
    // Only engage with a decisive material edge; otherwise it is noise.
    if material.abs() < 300 {
        return 0;
    }
    let winner = if material > 0 {
        Color::White
    } else {
        Color::Black
    };
    let (Some(wk), Some(lk)) = (board.king_square(winner), board.king_square(winner.other()))
    else {
        return 0;
    };
    let (lx, ly) = (i32::from(lk % 8), i32::from(lk / 8));
    // How far the losing king is from the center (larger = nearer a corner)...
    let center = (lx - if lx > 3 { 4 } else { 3 }).abs() + (ly - if ly > 3 { 4 } else { 3 }).abs();
    // ...and how close the winning king has approached (Manhattan distance).
    let distance = (i32::from(wk % 8) - lx).abs() + (i32::from(wk / 8) - ly).abs();
    let score = center * MOP_UP_CORNER + (14 - distance) * MOP_UP_PROXIMITY;
    if winner == Color::White {
        score
    } else {
        -score
    }
}

/// Static exchange evaluation (SEE): the net material change if both sides keep
/// capturing on `chess_move`'s destination square with their cheapest available
/// attacker. Positive means the initiating side comes out ahead. The search
/// uses it to prune and order captures without actually playing the sequence.
///
/// The algorithm plays out the capture chain on a scratch board, recording the
/// swing `gain[d]` at each step, then folds the sequence back with a minimax
/// (`gain[d-1] = -max(-gain[d-1], gain[d])`): each side, at its turn, will only
/// continue capturing if doing so does not worsen its own outcome.
pub fn static_exchange_eval(board: &Board, chess_move: Move) -> i32 {
    // What sits on the target square (a pawn, for en passant).
    let captured = if chess_move.en_passant {
        Some(PieceKind::Pawn)
    } else {
        board.piece_at(chess_move.to).map(|p| p.kind)
    };
    let Some(captured) = captured else { return 0 };
    // Work on a mutable copy of the mailbox; attackers are removed as they strike.
    let mut squares = [None; 64];
    for sq in 0..64 {
        squares[sq as usize] = board.piece_at(sq);
    }
    let mut gain = [0; 32];
    // First capture: we win the target piece; the mover becomes the next victim.
    gain[0] = piece_value(captured);
    let mut attacker_value = piece_value(squares[chess_move.from as usize].unwrap().kind);
    squares[chess_move.from as usize] = None;
    let mut side = board.side.other();
    let mut depth = 0usize;
    while depth + 1 < gain.len() {
        // The defender recaptures with its least valuable attacker.
        let Some((attacker, from)) = least_attacker(&squares, chess_move.to, side) else {
            break;
        };
        depth += 1;
        // Speculative material balance if this recapture is made.
        gain[depth] = attacker_value - gain[depth - 1];
        attacker_value = piece_value(attacker.kind);
        squares[from as usize] = None;
        // If the side to move here is already losing either way, it stops.
        if (-gain[depth - 1]).max(gain[depth]) < 0 {
            break;
        }
        side = side.other();
    }
    // Minimax the speculative gains back to the root of the exchange.
    while depth > 0 {
        gain[depth - 1] = -(-gain[depth - 1]).max(gain[depth]);
        depth -= 1;
    }
    gain[0]
}

/// Finds `color`'s least valuable piece that attacks `target` on the given
/// scratch board, returning it and its square. Attackers are tried cheapest
/// first (pawn, knight, sliders, king) so SEE recaptures in the correct order.
/// Sliding attackers see through nothing — the first piece on a ray blocks it.
fn least_attacker(squares: &[Option<Piece>; 64], target: u8, color: Color) -> Option<(Piece, u8)> {
    let (tx, ty) = (i32::from(target % 8), i32::from(target / 8));
    let at = |x: i32, y: i32| {
        if (0..8).contains(&x) && (0..8).contains(&y) {
            squares[(y * 8 + x) as usize]
        } else {
            None
        }
    };
    // Pawn: sits one rank behind the target (from the attacker's viewpoint).
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
    // Knight.
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
    // Sliders: diagonal rays for bishops/queens, orthogonal for rooks/queens.
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
    // King (the most valuable, hence last-resort, attacker).
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
            ("4k3/8/8/r4pPK/8/8/8/8 w - f6 0 1", -597),
            ("4k3/1P6/8/8/8/8/6p1/4K3 w - - 0 1", 44),
            ("4k3/8/3p4/3P4/2P5/8/8/4K3 w - - 0 1", 121),
            ("6k1/5ppp/8/8/8/8/5PPP/4R1K1 w - - 0 1", 687),
            (
                "r2q1rk1/pp2bppp/2n1pn2/2pp4/3P4/2PBPN2/PPQ2PPP/R1B2RK1 w - - 2 10",
                36,
            ),
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
        let board = Board::from_fen("4k3/8/4p3/3p4/4P3/8/8/4K3 w - - 0 1").unwrap();
        assert_eq!(
            static_exchange_eval(&board, board.parse_move("e4d5").unwrap()),
            0
        );
    }
}
