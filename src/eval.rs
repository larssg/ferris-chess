//! Static evaluation: scoring a position without searching.
//!
//! The values and tables here are ported from the GoChess reference so that the
//! two engines evaluate identically (see `PLAN.md`). Everything is computed from
//! White's point of view first and negated for Black at the very end; the term
//! functions themselves take a color and return White-positive contributions.
//!
//! This module also hosts [`static_exchange_eval`] (SEE), which the search uses
//! to score captures without playing them out on the board.

use crate::board::bit_squares;
use crate::{Board, Color, Move, PieceKind, board};

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

/// Material plus middlegame piece-square value for every piece/square slot of
/// the `Board::pieces` array, signed White-positive with Black squares
/// pre-mirrored. The board keeps a running sum of these entries as pieces are
/// placed and removed, so [`evaluate`] starts from that total instead of
/// walking every piece; the endgame king-table switch is applied there as a
/// per-king correction on top.
pub(crate) const MATERIAL_PST: [[i32; 64]; 12] = build_material_pst();

const fn build_material_pst() -> [[i32; 64]; 12] {
    let mut table = [[0; 64]; 12];
    let mut slot = 0;
    while slot < 12 {
        // Slots 0..6 are White, 6..12 Black, ordered by `PieceKind` within.
        let kind = slot % 6;
        let white = slot < 6;
        let mut square = 0;
        while square < 64 {
            let oriented = if white { square } else { square ^ 56 };
            let value = VALUES[kind] + PST[kind][oriented] as i32;
            table[slot][square] = if white { value } else { -value };
            square += 1;
        }
        slot += 1;
    }
    table
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

/// The full static evaluation, in centipawns from the side-to-move's point of
/// view (positive = the side to move is better).
///
/// Piece bitboards drive both the base score and the structural terms, avoiding
/// mailbox rescans and temporary pawn lists. The running `score` is
/// White-positive throughout and only flipped to side-relative at the return.
pub fn evaluate(board: &Board) -> i32 {
    evaluate_with(board, None)
}

/// [`evaluate`] with the pawn-structure term served from `cache` when its two
/// pawn bitboards match. Hits verify the full key, so the result is
/// bit-identical to an uncached evaluation.
pub(crate) fn evaluate_cached(board: &Board, cache: &mut PawnCache) -> i32 {
    evaluate_with(board, Some(cache))
}

/// A direct-mapped cache for the combined White-positive pawn-structure term,
/// which depends only on the two pawn bitboards. Pawn formations persist
/// across most of a search subtree, so nearly every node hits. The zeroed
/// initial entries are themselves exact: an empty pawn set scores zero.
pub(crate) struct PawnCache {
    entries: Vec<PawnCacheEntry>,
}

#[derive(Clone, Copy)]
struct PawnCacheEntry {
    white_pawns: u64,
    black_pawns: u64,
    score: i32,
}

impl PawnCache {
    const ENTRIES: usize = 1 << 14;

    pub(crate) fn new() -> Self {
        Self {
            entries: vec![
                PawnCacheEntry {
                    white_pawns: 0,
                    black_pawns: 0,
                    score: 0,
                };
                Self::ENTRIES
            ],
        }
    }

    /// The combined pawn-structure score for the given pawn sets, computed on
    /// a miss and remembered for next time.
    fn pawn_score(&mut self, white_pawns: u64, black_pawns: u64) -> i32 {
        let index =
            board::mix64(white_pawns ^ board::mix64(black_pawns)) as usize & (Self::ENTRIES - 1);
        let entry = &mut self.entries[index];
        if (entry.white_pawns, entry.black_pawns) != (white_pawns, black_pawns) {
            *entry = PawnCacheEntry {
                white_pawns,
                black_pawns,
                score: pawn_structure(white_pawns, black_pawns, Color::White)
                    - pawn_structure(black_pawns, white_pawns, Color::Black),
            };
        }
        entry.score
    }
}

fn evaluate_with(board: &Board, cache: Option<&mut PawnCache>) -> i32 {
    let endgame = is_endgame(board);
    // Material and piece-square values are maintained incrementally by
    // make/unmake; only the endgame king-table switch is applied here.
    let mut score = board.material_pst();
    if endgame {
        for color in [Color::White, Color::Black] {
            for square in bit_squares(board.pieces(color, PieceKind::King)) {
                let oriented = (if color == Color::White {
                    square
                } else {
                    square ^ 56
                }) as usize;
                let swap = i32::from(PST[6][oriented]) - i32::from(PST[5][oriented]);
                score += if color == Color::White { swap } else { -swap };
            }
        }
    }
    let pawns = [
        board.pieces(Color::White, PieceKind::Pawn),
        board.pieces(Color::Black, PieceKind::Pawn),
    ];
    // The bishop pair is worth a small bonus for its coordinated coverage.
    if board.pieces(Color::White, PieceKind::Bishop).count_ones() >= 2 {
        score += BISHOP_PAIR;
    }
    if board.pieces(Color::Black, PieceKind::Bishop).count_ones() >= 2 {
        score -= BISHOP_PAIR;
    }
    // King safety matters in the middlegame; in the endgame the king is an
    // active piece and the mop-up term takes over instead.
    if !endgame {
        score += king_safety(board, Color::White, &pawns[0]);
        score -= king_safety(board, Color::Black, &pawns[1]);
    }
    score += rooks(board, Color::White) - rooks(board, Color::Black);
    score += match cache {
        Some(cache) => cache.pawn_score(pawns[0], pawns[1]),
        None => {
            pawn_structure(pawns[0], pawns[1], Color::White)
                - pawn_structure(pawns[1], pawns[0], Color::Black)
        }
    };
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
fn pawn_structure(pawns: u64, opponents: u64, color: Color) -> i32 {
    // Per-file bitboards make the structural questions below constant-time for
    // each pawn instead of repeatedly walking both pawn sets.
    let mut files = [0i32; 8];
    let mut pawn_files = [0u64; 8];
    let mut opponent_files = [0u64; 8];
    for file in 0..8 {
        let mask = 0x0101_0101_0101_0101u64 << file;
        pawn_files[file] = pawns & mask;
        opponent_files[file] = opponents & mask;
        files[file] = pawn_files[file].count_ones() as i32;
    }
    let mut score = 0;
    let mut passed = 0u64;
    for sq in bit_squares(pawns) {
        let file = i32::from(sq % 8);
        let rank = i32::from(sq / 8);
        // Passed: no enemy pawn on this or an adjacent file lies ahead to stop it.
        let first_file = (file - 1).max(0) as usize;
        let last_file = (file + 1).min(7) as usize;
        let opposing_files = opponent_files[first_file..=last_file]
            .iter()
            .copied()
            .fold(0, |combined, bits| combined | bits);
        let adjacent_opposing = if file > 0 {
            opponent_files[file as usize - 1]
        } else {
            0
        } | if file < 7 {
            opponent_files[file as usize + 1]
        } else {
            0
        };
        let ahead = if color == Color::White {
            ranks_above(rank as u8)
        } else {
            ranks_below(rank as u8)
        };
        let is_passed = opposing_files & ahead == 0;
        if is_passed {
            // Reward grows quadratically as the pawn nears promotion.
            let relative_rank = if color == Color::White {
                rank
            } else {
                7 - rank
            };
            score += relative_rank * relative_rank * PASSED_RANK_SQUARED;
            passed |= 1u64 << sq;
        }
        // Supported: a friendly pawn on an adjacent file, not further advanced.
        let adjacent_friendly = if file > 0 {
            pawn_files[file as usize - 1]
        } else {
            0
        } | if file < 7 {
            pawn_files[file as usize + 1]
        } else {
            0
        };
        let support_ranks = if color == Color::White {
            !ranks_above(rank as u8)
        } else {
            !ranks_below(rank as u8)
        };
        let supported = adjacent_friendly & support_ranks != 0;
        // Backward: an unsupported, non-passed pawn whose advance square is
        // guarded by an enemy pawn, so it cannot safely move forward.
        if !supported && !is_passed {
            let attacking_rank = if color == Color::White {
                rank + 2
            } else {
                rank - 2
            };
            let backward = (0..8).contains(&attacking_rank)
                && adjacent_opposing & (0xffu64 << (attacking_rank * 8)) != 0;
            if backward {
                score -= BACKWARD_PAWN;
            }
        }
    }
    // Connected passers: two passed pawns on adjacent files defend each other's
    // advance. The bonus scales with how far the rear of the pair has advanced.
    for a in bit_squares(passed) {
        for b in bit_squares(passed).filter(|&b| b > a) {
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

const fn ranks_above(rank: u8) -> u64 {
    if rank == 7 {
        0
    } else {
        u64::MAX << ((rank + 1) * 8)
    }
}

const fn ranks_below(rank: u8) -> u64 {
    if rank == 0 {
        0
    } else {
        (1u64 << (rank * 8)) - 1
    }
}

/// Scores the safety of `color`'s king (White-positive): a bonus per adjacent
/// file that has a sheltering pawn, a penalty per open file beside the king, and
/// a bonus for a king tucked into a castled corner.
fn king_safety(board: &Board, color: Color, pawns: &u64) -> i32 {
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
        let shielded = bit_squares(*pawns).any(|sq| {
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
    for sq in bit_squares(board.pieces(color, PieceKind::Rook)) {
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
    // Track the exchange on a scratch occupancy bitboard; attackers are removed
    // as they strike. Only the moving piece leaves up front — like the
    // reference, an en-passant victim stays put and keeps blocking its file.
    let mut occupied = board.occupied() & !(1u64 << chess_move.from);
    let mut gain = [0; 32];
    // First capture: we win the target piece; the mover becomes the next victim.
    gain[0] = piece_value(captured);
    let mut attacker_value = piece_value(board.piece_at(chess_move.from).unwrap().kind);
    let mut side = board.side.other();
    let mut depth = 0usize;
    while depth + 1 < gain.len() {
        // The defender recaptures with its least valuable attacker.
        let Some((attacker, from)) = least_attacker(board, occupied, chess_move.to, side) else {
            break;
        };
        depth += 1;
        // Speculative material balance if this recapture is made.
        gain[depth] = attacker_value - gain[depth - 1];
        attacker_value = piece_value(attacker);
        occupied &= !(1u64 << from);
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

/// Finds `color`'s least valuable piece that attacks `target` among the pieces
/// still present in `occupied`, returning its kind and square. Attackers are
/// tried cheapest first (pawn, knight, diagonal sliders, orthogonal sliders,
/// king). Which of several equal attackers strikes first decides which rays
/// open up next, so ties are broken in the reference's exact scan order to
/// keep exchange sequences — and search node counts — reproducible.
fn least_attacker(
    board: &Board,
    occupied: u64,
    target: u8,
    color: Color,
) -> Option<(PieceKind, u8)> {
    // Pawn: sits one rank behind the target (from the attacker's viewpoint).
    // The lower-file candidate has the lower bit index, so a plain bit scan
    // matches the reference's left-to-right preference.
    let pawn_attackers = match color {
        Color::White => board::WHITE_PAWN_ATTACKERS[target as usize],
        Color::Black => board::BLACK_PAWN_ATTACKERS[target as usize],
    };
    let pawns = pawn_attackers & board.pieces(color, PieceKind::Pawn) & occupied;
    if pawns != 0 {
        return Some((PieceKind::Pawn, pawns.trailing_zeros() as u8));
    }
    // Knight. With a single knight attacker (the usual case) a bit scan
    // suffices; several tie, and the reference's offset order picks one.
    let knights =
        board::KNIGHT_ATTACKS[target as usize] & board.pieces(color, PieceKind::Knight) & occupied;
    if knights != 0 {
        if knights & (knights - 1) == 0 {
            return Some((PieceKind::Knight, knights.trailing_zeros() as u8));
        }
        let (tx, ty) = ((target % 8) as i8, (target / 8) as i8);
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
            let (x, y) = (tx + dx, ty + dy);
            if (0..8).contains(&x) && (0..8).contains(&y) && knights & (1u64 << (y * 8 + x)) != 0 {
                return Some((PieceKind::Knight, (y * 8 + x) as u8));
            }
        }
    }
    // Sliders: diagonal rays for bishops/queens, orthogonal for rooks/queens.
    // Each ray's first blocker either is such a slider or shuts the ray.
    let queens = board.pieces(color, PieceKind::Queen);
    for (rays, slider, sliders) in [
        (
            &board::DIAGONAL_RAYS,
            PieceKind::Bishop,
            board.pieces(color, PieceKind::Bishop) | queens,
        ),
        (
            &board::ORTHOGONAL_RAYS,
            PieceKind::Rook,
            board.pieces(color, PieceKind::Rook) | queens,
        ),
    ] {
        for ray in rays {
            let Some(blocker) = ray.first_blocker(target, occupied) else {
                continue;
            };
            let bit = 1u64 << blocker;
            if sliders & bit != 0 {
                let kind = if queens & bit != 0 {
                    PieceKind::Queen
                } else {
                    slider
                };
                return Some((kind, blocker));
            }
        }
    }
    // King (the most valuable, hence last-resort, attacker; one per side, so
    // there is never a tie to break).
    let king =
        board::KING_ATTACKS[target as usize] & board.pieces(color, PieceKind::King) & occupied;
    if king != 0 {
        return Some((PieceKind::King, king.trailing_zeros() as u8));
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
