//! The rules of chess: board representation, move generation, and make/unmake.
//!
//! Squares are numbered 0..64 in little-endian rank-file order: `a1 = 0`,
//! `h1 = 7`, `a8 = 56`, `h8 = 63`. Thus `square % 8` is the file (0=a) and
//! `square / 8` is the rank (0=rank 1). Flipping a square vertically — used to
//! mirror White/Black piece-square tables — is `square ^ 56`.

use std::{fmt, str::FromStr};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Color {
    White,
    Black,
}
impl Color {
    /// The opposing color. Used constantly to flip perspective.
    pub const fn other(self) -> Self {
        match self {
            Self::White => Self::Black,
            Self::Black => Self::White,
        }
    }
}

/// The six piece kinds. Discriminants start at 1 so that `kind as usize - 1`
/// indexes the six-entry value and piece-square tables, while leaving 0 free as
/// a sentinel elsewhere (e.g. packed transposition-table moves).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
#[repr(u8)]
pub enum PieceKind {
    Pawn = 1,
    Knight,
    Bishop,
    Rook,
    Queen,
    King,
}
impl PieceKind {
    /// The piece's lowercase letter, as used in FEN and UCI notation.
    pub const fn letter(self) -> char {
        match self {
            Self::Pawn => 'p',
            Self::Knight => 'n',
            Self::Bishop => 'b',
            Self::Rook => 'r',
            Self::Queen => 'q',
            Self::King => 'k',
        }
    }

    /// The piece named by a FEN/UCI letter, in either case (`'N'` or `'n'`).
    pub fn from_letter(letter: char) -> Option<Self> {
        Some(match letter.to_ascii_lowercase() {
            'p' => Self::Pawn,
            'n' => Self::Knight,
            'b' => Self::Bishop,
            'r' => Self::Rook,
            'q' => Self::Queen,
            'k' => Self::King,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Piece {
    pub color: Color,
    pub kind: PieceKind,
}

/// A single move. `en_passant` and `castle` flag the two moves whose effect on
/// the board is not captured by `from`/`to` alone — they move or remove a
/// second piece — so make/unmake can reproduce them exactly.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct Move {
    pub from: u8,
    pub to: u8,
    pub promotion: Option<PieceKind>,
    pub en_passant: bool,
    pub castle: bool,
}

impl fmt::Display for Move {
    /// Renders long algebraic (UCI) notation, e.g. `e2e4` or `e7e8q`. A null
    /// move (`from == to`) prints as the UCI sentinel `0000`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.from == self.to {
            return f.write_str("0000");
        }
        write!(f, "{}{}", square_name(self.from), square_name(self.to))?;
        if let Some(p) = self.promotion {
            write!(f, "{}", p.letter())?;
        }
        Ok(())
    }
}

/// A full chess position.
///
/// The position is stored two ways that are always kept in sync: `squares` is
/// the authoritative mailbox (one slot per square) that move generation reads,
/// while `pieces`/`occupancy` are derived bitboards that evaluation and SEE use
/// for fast set queries. `hash` is a Zobrist-style key of the whole state, used
/// for the transposition table and repetition detection.
///
/// `castling` is a 4-bit mask: `1=K, 2=Q, 4=k, 8=q`. `en_passant` holds the
/// square a pawn may capture *onto*, not the captured pawn's square.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Board {
    squares: [Option<Piece>; 64],
    /// One bitboard per (color, kind), indexed by [`piece_index`].
    pieces: [u64; 12],
    /// Per-color occupancy, indexed by [`color_index`].
    occupancy: [u64; 2],
    /// Running White-positive material-plus-piece-square total, maintained as
    /// pieces are placed and removed so evaluation need not walk every piece.
    material_pst: i32,
    pub side: Color,
    pub castling: u8,
    pub en_passant: Option<u8>,
    pub halfmove: u16,
    pub fullmove: u16,
    pub hash: u64,
}

/// Everything needed to reverse a [`Board::make_move`]. State that a move can
/// silently change (castling rights, en-passant, clocks, hash) is snapshotted
/// wholesale rather than recomputed, which keeps `unmake_move` exact and cheap.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Undo {
    captured: Option<(u8, Piece)>,
    moved: Piece,
    castling: u8,
    en_passant: Option<u8>,
    halfmove: u16,
    fullmove: u16,
    hash: u64,
}

/// Precomputed facts for judging move legality without playing moves out; see
/// [`Board::legality_context`].
struct LegalityContext {
    /// Our king's square; `None` legalizes everything (no king, no check).
    king: Option<u8>,
    /// Enemy pieces currently giving check.
    checkers: u64,
    /// Our pieces that are absolutely pinned to the king.
    pinned: u64,
    /// Squares a non-king move must land on: everything when not in check, the
    /// capture-or-block squares in single check, nothing in double check.
    evasions: u64,
}

/// The counterpart of [`Undo`] for a null move (see [`Board::make_null_move`]),
/// which only toggles the side to move and clears en-passant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NullUndo {
    side: Color,
    en_passant: Option<u8>,
    hash: u64,
}

impl Default for Board {
    fn default() -> Self {
        Self::startpos()
    }
}

impl Board {
    /// The standard chess starting position.
    pub fn startpos() -> Self {
        Self::from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap()
    }

    /// Parses a FEN string into a board, deriving the bitboards and hash.
    ///
    /// Validation is strict: the placement must describe exactly eight ranks of
    /// eight squares, and the side/castling/en-passant fields must be
    /// well-formed. The halfmove and fullmove fields are optional (defaulting to
    /// `0` and `1`) so that four-field EPD positions parse.
    pub fn from_fen(fen: &str) -> Result<Self, String> {
        let p: Vec<_> = fen.split_whitespace().collect();
        if p.len() < 4 {
            return Err("FEN requires at least four fields".into());
        }
        // Field 1: piece placement, given rank 8 first (top of the board) down
        // to rank 1. We therefore map FEN rank index `ri` to board rank `7 - ri`.
        let ranks: Vec<_> = p[0].split('/').collect();
        if ranks.len() != 8 {
            return Err("FEN must contain eight ranks".into());
        }
        let mut squares = [None; 64];
        for (ri, rank) in ranks.iter().enumerate() {
            let mut file = 0usize;
            for ch in rank.chars() {
                if let Some(n) = ch.to_digit(10) {
                    // A digit is a run of empty squares.
                    if !(1..=8).contains(&n) {
                        return Err("invalid empty run".into());
                    }
                    file += n as usize;
                } else {
                    if file >= 8 {
                        return Err("rank is too long".into());
                    }
                    let color = if ch.is_ascii_uppercase() {
                        Color::White
                    } else {
                        Color::Black
                    };
                    let kind =
                        PieceKind::from_letter(ch).ok_or_else(|| format!("invalid piece {ch}"))?;
                    squares[(7 - ri) * 8 + file] = Some(Piece { color, kind });
                    file += 1;
                }
            }
            if file != 8 {
                return Err("each FEN rank must describe eight squares".into());
            }
        }
        // Field 2: side to move.
        let side = match p[1] {
            "w" => Color::White,
            "b" => Color::Black,
            _ => return Err("invalid side to move".into()),
        };
        // Field 3: castling rights, packed into the 4-bit mask described on `Board`.
        let mut castling = 0;
        if p[2] != "-" {
            for c in p[2].chars() {
                castling |= match c {
                    'K' => 1,
                    'Q' => 2,
                    'k' => 4,
                    'q' => 8,
                    _ => return Err("invalid castling rights".into()),
                };
            }
        }
        // Field 4: en-passant target square (the square a pawn would capture onto).
        let en_passant = if p[3] == "-" {
            None
        } else {
            Some(parse_square(p[3]).ok_or("invalid en-passant square")?)
        };
        // Fields 5-6: halfmove clock and fullmove number, both optional.
        let halfmove = p
            .get(4)
            .unwrap_or(&"0")
            .parse()
            .map_err(|_| "invalid halfmove clock")?;
        let fullmove = p
            .get(5)
            .unwrap_or(&"1")
            .parse()
            .map_err(|_| "invalid fullmove number")?;
        let mut b = Self {
            squares,
            pieces: [0; 12],
            occupancy: [0; 2],
            material_pst: 0,
            side,
            castling,
            en_passant,
            halfmove,
            fullmove,
            hash: 0,
        };
        // The mailbox is authoritative; the bitboards and hash are derived from it.
        b.rebuild_bitboards();
        b.hash = b.compute_hash();
        Ok(b)
    }

    /// Serializes the position back to FEN. Inverse of [`Board::from_fen`].
    pub fn to_fen(&self) -> String {
        let mut placement = String::new();
        for rank in (0..8).rev() {
            if rank < 7 {
                placement.push('/')
            }
            let mut empty = 0;
            for file in 0..8 {
                match self.squares[rank * 8 + file] {
                    None => empty += 1,
                    Some(p) => {
                        if empty > 0 {
                            placement.push(char::from_digit(empty, 10).unwrap());
                            empty = 0
                        }
                        let c = if p.color == Color::White {
                            p.kind.letter().to_ascii_uppercase()
                        } else {
                            p.kind.letter()
                        };
                        placement.push(c)
                    }
                }
            }
            if empty > 0 {
                placement.push(char::from_digit(empty, 10).unwrap())
            }
        }
        let mut rights = String::new();
        for (bit, c) in [(1, 'K'), (2, 'Q'), (4, 'k'), (8, 'q')] {
            if self.castling & bit != 0 {
                rights.push(c)
            }
        }
        if rights.is_empty() {
            rights.push('-')
        }
        format!(
            "{placement} {} {rights} {} {} {}",
            if self.side == Color::White { "w" } else { "b" },
            self.en_passant
                .map(square_name)
                .unwrap_or_else(|| "-".into()),
            self.halfmove,
            self.fullmove
        )
    }

    /// Resolves a long-algebraic (UCI) move string against the legal moves of
    /// this position, returning the fully-populated [`Move`] (with its castle,
    /// en-passant, and promotion flags) or `None` if it is not legal here.
    ///
    /// Matching against legal moves is what lets a bare `e1g1` be recognized as
    /// a castle and a promotion default to a queen when no piece letter is given.
    pub fn parse_move(&self, text: &str) -> Option<Move> {
        let t = text.trim().to_ascii_lowercase();
        if t.len() < 4 {
            return None;
        }
        let from = parse_square(&t[0..2])?;
        let to = parse_square(&t[2..4])?;
        let promo = t
            .chars()
            .nth(4)
            .and_then(PieceKind::from_letter)
            .filter(|kind| !matches!(kind, PieceKind::Pawn | PieceKind::King));
        self.legal_moves().into_iter().find(|m| {
            m.from == from
                && m.to == to
                && (m.promotion == promo
                    // A promotion with no letter defaults to a queen.
                    || (promo.is_none() && m.promotion == Some(PieceKind::Queen)))
        })
    }

    /// Resolves a move written in standard algebraic notation (SAN), e.g. `Nf3`
    /// or `exd5`. Check/mate/annotation suffixes are ignored, and `0-0` is
    /// accepted as an alias for `O-O`. Used when reading EPD test suites.
    pub fn parse_san(&self, text: &str) -> Option<Move> {
        let target = text
            .trim()
            .trim_end_matches(['+', '#', '!', '?'])
            .replace('0', "O");
        self.legal_moves()
            .into_iter()
            .find(|chess_move| self.san_without_check(*chess_move) == target)
    }

    /// Renders a move as SAN *without* the trailing `+`/`#` check markers, which
    /// is enough for [`parse_san`](Self::parse_san) to match against (it strips
    /// those markers from its input too). The bulk of the work is
    /// disambiguation: when another piece of the same kind could also reach the
    /// destination, SAN adds the origin file, rank, or both.
    fn san_without_check(&self, chess_move: Move) -> String {
        if chess_move.castle {
            return if chess_move.to > chess_move.from {
                "O-O".into()
            } else {
                "O-O-O".into()
            };
        }
        let piece = self.piece_at(chess_move.from).unwrap();
        let capture = chess_move.en_passant || self.piece_at(chess_move.to).is_some();
        let mut san = String::new();
        if piece.kind != PieceKind::Pawn {
            san.push(piece.kind.letter().to_ascii_uppercase());
            // Other pieces of the same kind that also attack the target square
            // force a disambiguating file and/or rank on the origin.
            let alternatives: Vec<_> = self
                .legal_moves()
                .into_iter()
                .filter(|candidate| {
                    candidate.to == chess_move.to
                        && candidate.from != chess_move.from
                        && self.piece_at(candidate.from).map(|other| other.kind) == Some(piece.kind)
                })
                .collect();
            if !alternatives.is_empty() {
                let same_file = alternatives
                    .iter()
                    .any(|candidate| candidate.from % 8 == chess_move.from % 8);
                let same_rank = alternatives
                    .iter()
                    .any(|candidate| candidate.from / 8 == chess_move.from / 8);
                // Prefer the file; fall back to rank; use both only if neither
                // alone is unique.
                if !same_file {
                    san.push((b'a' + chess_move.from % 8) as char);
                } else if !same_rank {
                    san.push((b'1' + chess_move.from / 8) as char);
                } else {
                    san.push((b'a' + chess_move.from % 8) as char);
                    san.push((b'1' + chess_move.from / 8) as char);
                }
            }
        } else if capture {
            // A pawn capture names its origin file, e.g. the `e` in `exd5`.
            san.push((b'a' + chess_move.from % 8) as char);
        }
        if capture {
            san.push('x');
        }
        san.push_str(&square_name(chess_move.to));
        if let Some(promotion) = chess_move.promotion {
            san.push('=');
            san.push(promotion.letter().to_ascii_uppercase());
        }
        san
    }

    /// The piece on `square`, or `None` if it is empty or out of range.
    pub fn piece_at(&self, square: u8) -> Option<Piece> {
        self.squares.get(square as usize).copied().flatten()
    }

    /// The square of `color`'s king, or `None` if it is absent (e.g. mid-analysis positions).
    pub fn king_square(&self, color: Color) -> Option<u8> {
        let kings = self.pieces(color, PieceKind::King);
        (kings != 0).then(|| kings.trailing_zeros() as u8)
    }

    /// The bitboard of pieces of a given color and kind.
    pub fn pieces(&self, color: Color, kind: PieceKind) -> u64 {
        self.pieces[piece_index(Piece { color, kind })]
    }

    /// The bitboard of every square occupied by `color`.
    pub fn occupancy(&self, color: Color) -> u64 {
        self.occupancy[color_index(color)]
    }

    /// The bitboard of every occupied square, regardless of color.
    pub fn occupied(&self) -> u64 {
        self.occupancy[0] | self.occupancy[1]
    }

    /// Generates the fully legal moves into `out`, reusing the caller's buffer to
    /// avoid an allocation on the search's hot path.
    ///
    /// Legality filtering is pin-aware: checkers and absolutely pinned pieces
    /// are computed once, after which each pseudo-legal move is judged with a
    /// few bit tests instead of being played and reversed (see
    /// [`retain_legal`](Self::retain_legal)). Perft validates the filter
    /// exactly, and a differential test checks it against the played-out
    /// definition of legality.
    pub fn generate_legal_moves(&self, out: &mut Vec<Move>) {
        out.clear();
        self.generate_pseudo_moves(out, false);
        self.retain_legal(out);
    }
    /// Convenience wrapper around [`generate_legal_moves`](Self::generate_legal_moves)
    /// that allocates a fresh vector.
    pub fn legal_moves(&self) -> Vec<Move> {
        let mut moves = Vec::with_capacity(48);
        self.generate_legal_moves(&mut moves);
        moves
    }
    /// The legal capturing moves only, used to drive the quiescence search.
    pub fn legal_captures(&self) -> Vec<Move> {
        let mut moves = Vec::with_capacity(16);
        self.generate_legal_captures(&mut moves);
        moves
    }

    /// Generates legal captures into a reusable caller-owned buffer.
    pub(crate) fn generate_legal_captures(&self, out: &mut Vec<Move>) {
        out.clear();
        self.generate_pseudo_moves(out, true);
        self.retain_legal(out);
    }

    /// Drops the pseudo-legal moves in `out` that would leave our king in
    /// check, preserving order. Almost every move is judged by bit tests
    /// against the precomputed [`LegalityContext`]; only en passant falls back
    /// to playing the move out.
    fn retain_legal(&self, out: &mut Vec<Move>) {
        let context = self.legality_context();
        let mut scratch = None;
        out.retain(|&m| self.move_is_legal(&context, m, &mut scratch));
    }

    /// The per-position facts the legality filter needs: our king, the enemy
    /// pieces currently checking it, our absolutely pinned pieces, and where a
    /// non-king move must land to answer a check.
    fn legality_context(&self) -> LegalityContext {
        let enemy = self.side.other();
        // No king (possible in synthetic FENs) means nothing can be in check,
        // matching the played-out definition of legality.
        let Some(king) = self.king_square(self.side) else {
            return LegalityContext {
                king: None,
                checkers: 0,
                pinned: 0,
                evasions: u64::MAX,
            };
        };
        let occupied = self.occupied();
        let queens = self.pieces(enemy, PieceKind::Queen);
        let orthogonal_sliders = self.pieces(enemy, PieceKind::Rook) | queens;
        let diagonal_sliders = self.pieces(enemy, PieceKind::Bishop) | queens;
        let pawn_attackers = match enemy {
            Color::White => WHITE_PAWN_ATTACKERS[king as usize],
            Color::Black => BLACK_PAWN_ATTACKERS[king as usize],
        };
        let checkers = (KNIGHT_ATTACKS[king as usize] & self.pieces(enemy, PieceKind::Knight))
            | (pawn_attackers & self.pieces(enemy, PieceKind::Pawn))
            | (KING_ATTACKS[king as usize] & self.pieces(enemy, PieceKind::King))
            | (rook_attacks(king, occupied) & orthogonal_sliders)
            | (bishop_attacks(king, occupied) & diagonal_sliders);
        // A piece is pinned when it is the first blocker on a ray from the
        // king and the next blocker beyond it is an enemy slider of that
        // ray's kind.
        let own = self.occupancy[color_index(self.side)];
        let mut pinned = 0;
        for (rays, sliders) in [
            (&ORTHOGONAL_RAYS, orthogonal_sliders),
            (&DIAGONAL_RAYS, diagonal_sliders),
        ] {
            for ray in rays {
                if let Some(first) = ray.first_blocker(king, occupied)
                    && own & (1u64 << first) != 0
                    && let Some(second) = ray.first_blocker(first, occupied)
                    && sliders & (1u64 << second) != 0
                {
                    pinned |= 1u64 << first;
                }
            }
        }
        let evasions = match checkers.count_ones() {
            // Not in check: land anywhere.
            0 => u64::MAX,
            // Single check: capture the checker, or block a slider's ray.
            1 => checkers | BETWEEN[king as usize][checkers.trailing_zeros() as usize],
            // Double check: no single non-king move can answer both.
            _ => 0,
        };
        LegalityContext {
            king: Some(king),
            checkers,
            pinned,
            evasions,
        }
    }

    /// Whether one pseudo-legal move leaves our king safe. `scratch` lazily
    /// holds a clone of `self` for the en-passant fallback.
    fn move_is_legal(
        &self,
        context: &LegalityContext,
        m: Move,
        scratch: &mut Option<Board>,
    ) -> bool {
        let Some(king) = context.king else {
            return true;
        };
        if m.en_passant {
            // En passant clears two squares of one rank at once — the one case
            // the ray reasoning below does not cover (a rank ray through both
            // pawns may uncover the king). Play it out instead; it is rare.
            let board = scratch.get_or_insert_with(|| self.clone());
            let undo = board.make_move(m);
            let legal = !board.in_check(self.side);
            board.unmake_move(m, undo);
            return legal;
        }
        if m.from == king {
            // Castling proved the king's start, path, and destination safe
            // during generation.
            if m.castle {
                return true;
            }
            // Lift the king so a checker's ray extends through its old square.
            let occupied = self.occupied() ^ (1u64 << m.from);
            return !self.square_attacked_with(m.to, self.side.other(), occupied);
        }
        if context.pinned & (1u64 << m.from) != 0 {
            // A pinned piece may only slide along its pin line, and while in
            // check it cannot help at all: the check ray and the pin line only
            // meet at the king's own square.
            return context.checkers == 0
                && LINE[king as usize][m.from as usize] & (1u64 << m.to) != 0;
        }
        context.evasions & (1u64 << m.to) != 0
    }

    /// Generates pseudo-legal moves for the side to move — moves that respect
    /// each piece's movement but may leave the king in check. When
    /// `captures_only` is set (quiescence search), quiet moves are skipped, but
    /// capture-promotions are always emitted because they change material.
    ///
    /// Everything is driven by the occupancy bitboards and the precomputed
    /// target/ray tables; the mailbox is read once per piece for its kind. The
    /// emission order deliberately reproduces the original square-by-square
    /// scan (pieces by ascending square, targets in fixed offset/ray order),
    /// because downstream tie-breaks turn generation order into search-visible
    /// behavior.
    fn generate_pseudo_moves(&self, out: &mut Vec<Move>, captures_only: bool) {
        let own = self.occupancy[color_index(self.side)];
        let enemy = self.occupancy[color_index(self.side.other())];
        let occupied = own | enemy;
        for from in bit_squares(own) {
            let piece = self.squares[from as usize].expect("occupancy bit implies a piece");
            match piece.kind {
                PieceKind::Pawn => {
                    let f = (from % 8) as i8;
                    let r = (from / 8) as i8;
                    // Direction of travel and the ranks where a pawn may still
                    // double-step (`start`) or promote (`last`).
                    let (d, start, last) = if piece.color == Color::White {
                        (1, 1, 7)
                    } else {
                        (-1, 6, 0)
                    };
                    // A FEN can place a pawn on its own last rank; skip it
                    // rather than step off the board.
                    let nr = r + d;
                    if !(0..8).contains(&nr) {
                        continue;
                    }
                    // Single (and double) push onto an empty square.
                    let forward = (nr * 8 + f) as u8;
                    if !captures_only && occupied & (1u64 << forward) == 0 {
                        if nr == last {
                            // Reaching the last rank fans out into the four
                            // promotion choices.
                            for p in [
                                PieceKind::Queen,
                                PieceKind::Rook,
                                PieceKind::Bishop,
                                PieceKind::Knight,
                            ] {
                                out.push(Move {
                                    from,
                                    to: forward,
                                    promotion: Some(p),
                                    ..Move::default()
                                })
                            }
                        } else {
                            out.push(Move {
                                from,
                                to: forward,
                                ..Move::default()
                            });
                            // From the starting rank, a two-square push is
                            // allowed if the intervening square is also empty.
                            if r == start {
                                let two = ((r + 2 * d) * 8 + f) as u8;
                                if occupied & (1u64 << two) == 0 {
                                    out.push(Move {
                                        from,
                                        to: two,
                                        ..Move::default()
                                    })
                                }
                            }
                        }
                    }
                    // Diagonal captures, including en passant onto the
                    // recorded target square.
                    for df in [-1, 1] {
                        let nf = f + df;
                        if !(0..8).contains(&nf) {
                            continue;
                        }
                        let to = (nr * 8 + nf) as u8;
                        let is_en_passant = self.en_passant == Some(to);
                        if enemy & (1u64 << to) != 0 || is_en_passant {
                            if nr == last {
                                for p in [
                                    PieceKind::Queen,
                                    PieceKind::Rook,
                                    PieceKind::Bishop,
                                    PieceKind::Knight,
                                ] {
                                    out.push(Move {
                                        from,
                                        to,
                                        promotion: Some(p),
                                        ..Move::default()
                                    })
                                }
                            } else {
                                out.push(Move {
                                    from,
                                    to,
                                    en_passant: is_en_passant,
                                    ..Move::default()
                                })
                            }
                        }
                    }
                }
                PieceKind::Knight => {
                    leaper_moves(out, from, &KNIGHT_TARGETS, own, enemy, captures_only)
                }
                PieceKind::Bishop => {
                    slider_moves(out, from, &BISHOP_MOVE_RAYS, own, occupied, captures_only)
                }
                PieceKind::Rook => {
                    slider_moves(out, from, &ROOK_MOVE_RAYS, own, occupied, captures_only)
                }
                PieceKind::Queen => {
                    slider_moves(out, from, &QUEEN_MOVE_RAYS, own, occupied, captures_only)
                }
                PieceKind::King => {
                    leaper_moves(out, from, &KING_TARGETS, own, enemy, captures_only);
                    if !captures_only {
                        // Castling: pick this color's home rank, the enemy color
                        // that would attack through squares, and the king/queen
                        // side rights bits.
                        let (rank, enemy_color, ks, qs) = if piece.color == Color::White {
                            (0, Color::Black, 1, 2)
                        } else {
                            (7, Color::White, 4, 8)
                        };
                        let base = rank * 8;
                        let rooks = self.pieces(piece.color, PieceKind::Rook);
                        let empty = |square: u8| occupied & (1u64 << square) == 0;
                        // The king must be on its home square and not currently in
                        // check; the squares it passes through are checked below.
                        if from == base + 4 && !self.square_attacked(from, enemy_color) {
                            // Kingside: f- and g-file empty and unattacked, rook present.
                            if self.castling & ks != 0
                                && empty(base + 5)
                                && empty(base + 6)
                                && !self.square_attacked(base + 5, enemy_color)
                                && !self.square_attacked(base + 6, enemy_color)
                                && rooks & (1u64 << (base + 7)) != 0
                            {
                                out.push(Move {
                                    from,
                                    to: base + 6,
                                    castle: true,
                                    ..Move::default()
                                })
                            }
                            // Queenside: b-, c-, d-file empty; c- and d-file
                            // unattacked (the king does not pass over b); rook present.
                            if self.castling & qs != 0
                                && empty(base + 1)
                                && empty(base + 2)
                                && empty(base + 3)
                                && !self.square_attacked(base + 3, enemy_color)
                                && !self.square_attacked(base + 2, enemy_color)
                                && rooks & (1u64 << base) != 0
                            {
                                out.push(Move {
                                    from,
                                    to: base + 2,
                                    castle: true,
                                    ..Move::default()
                                })
                            }
                        }
                    }
                }
            }
        }
    }

    /// Applies a move and returns the [`Undo`] token that reverses it.
    ///
    /// The steps mirror the rules that a plain "lift from `from`, drop on `to`"
    /// would miss: remove an en-passant victim, promote, shuffle the castling
    /// rook, revoke castling rights when a king or rook square is touched, set a
    /// new en-passant target after a double push, and update the clocks. The
    /// mailbox, bitboards, and hash are updated together.
    pub fn make_move(&mut self, m: Move) -> Undo {
        let piece = self.squares[m.from as usize].unwrap();
        // An en-passant capture removes a pawn that sits beside `to`, not on it.
        let captured_square = if m.en_passant {
            if piece.color == Color::White {
                m.to - 8
            } else {
                m.to + 8
            }
        } else {
            m.to
        };
        let undo = Undo {
            captured: self.squares[captured_square as usize].map(|p| (captured_square, p)),
            moved: piece,
            castling: self.castling,
            en_passant: self.en_passant,
            halfmove: self.halfmove,
            fullmove: self.fullmove,
            hash: self.hash,
        };
        let capture = self.squares[m.to as usize].is_some() || m.en_passant;
        // Any prior en-passant right expires; a new one may be set below.
        self.en_passant = None;
        if m.en_passant {
            let sq = if piece.color == Color::White {
                m.to - 8
            } else {
                m.to + 8
            };
            self.remove_piece(sq);
        }
        // Move the piece, applying a promotion by swapping in the new kind.
        self.remove_piece(m.from);
        self.remove_piece(m.to);
        self.place_piece(
            m.to,
            Piece {
                color: piece.color,
                kind: m.promotion.unwrap_or(piece.kind),
            },
        );
        if m.castle {
            // The king has already moved two squares; bring the rook to the far
            // side of it. Kingside jumps three squares, queenside four.
            let (rf, rt) = if m.to > m.from {
                (m.from + 3, m.from + 1)
            } else {
                (m.from - 4, m.from - 1)
            };
            let rook = self.remove_piece(rf).expect("castling rook must exist");
            self.place_piece(rt, rook);
        }
        // Moving a king or rook off its home square revokes the matching rights;
        // capturing a rook on its home square revokes the opponent's.
        match m.from {
            4 => self.castling &= !3,
            60 => self.castling &= !12,
            0 => self.castling &= !2,
            7 => self.castling &= !1,
            56 => self.castling &= !8,
            63 => self.castling &= !4,
            _ => {}
        }
        match m.to {
            0 => self.castling &= !2,
            7 => self.castling &= !1,
            56 => self.castling &= !8,
            63 => self.castling &= !4,
            _ => {}
        }
        // A two-square pawn push exposes the square it skipped to en passant.
        if piece.kind == PieceKind::Pawn && (m.to as i16 - m.from as i16).abs() == 16 {
            self.en_passant = Some(if piece.color == Color::White {
                m.from + 8
            } else {
                m.from - 8
            })
        }
        // The halfmove clock (for the 50-move rule) resets on a pawn move or
        // capture and otherwise counts up.
        self.halfmove = if piece.kind == PieceKind::Pawn || capture {
            0
        } else {
            self.halfmove + 1
        };
        if self.side == Color::Black {
            self.fullmove += 1
        }
        self.side = self.side.other();
        // Piece keys were maintained while moving the pieces above. Finish the
        // incremental hash update with the non-piece state that changed.
        self.hash ^= CASTLING_HASHES[undo.castling as usize];
        self.hash ^= CASTLING_HASHES[self.castling as usize];
        if let Some(ep) = undo.en_passant {
            self.hash ^= EN_PASSANT_HASHES[(ep % 8) as usize];
        }
        if let Some(ep) = self.en_passant {
            self.hash ^= EN_PASSANT_HASHES[(ep % 8) as usize];
        }
        self.hash ^= SIDE_HASH;
        undo
    }

    /// Reverses a [`make_move`](Self::make_move) using its [`Undo`] token,
    /// restoring the position bit-for-bit. Snapshotted fields (rights, clocks,
    /// hash) are copied straight back; only the piece movement is replayed in
    /// reverse.
    pub fn unmake_move(&mut self, m: Move, undo: Undo) {
        self.side = self.side.other();
        self.remove_piece(m.to);
        self.place_piece(m.from, undo.moved);
        if m.castle {
            let (rf, rt) = if m.to > m.from {
                (m.from + 3, m.from + 1)
            } else {
                (m.from - 4, m.from - 1)
            };
            let rook = self
                .remove_piece(rt)
                .expect("castling rook must exist while unmaking");
            self.place_piece(rf, rook);
        }
        // Restore any captured piece (its square differs from `to` for en passant).
        if let Some((square, piece)) = undo.captured {
            self.place_piece(square, piece);
        }
        self.castling = undo.castling;
        self.en_passant = undo.en_passant;
        self.halfmove = undo.halfmove;
        self.fullmove = undo.fullmove;
        self.hash = undo.hash;
    }

    /// Plays a null move: pass the turn without moving. Used by null-move
    /// pruning in the search to test whether the side to move is so far ahead
    /// that even doing nothing holds above beta.
    pub(crate) fn make_null_move(&mut self) -> NullUndo {
        let undo = NullUndo {
            side: self.side,
            en_passant: self.en_passant,
            hash: self.hash,
        };
        self.side = self.side.other();
        self.en_passant = None;
        if let Some(ep) = undo.en_passant {
            self.hash ^= EN_PASSANT_HASHES[(ep % 8) as usize];
        }
        self.hash ^= SIDE_HASH;
        undo
    }

    /// Reverses a [`make_null_move`](Self::make_null_move). No pieces moved, so
    /// only the side, en-passant square, and hash need restoring.
    pub(crate) fn unmake_null_move(&mut self, undo: NullUndo) {
        self.side = undo.side;
        self.en_passant = undo.en_passant;
        self.hash = undo.hash;
    }

    /// Whether `color`'s king is currently attacked.
    pub fn in_check(&self, color: Color) -> bool {
        self.king_square(color)
            .is_some_and(|sq| self.square_attacked(sq, color.other()))
    }
    /// Whether `by` attacks `sq`. Rather than scan every enemy piece, this looks
    /// *outward from the target square* along each attack geometry and asks
    /// whether the right kind of enemy piece sits at the other end — knight
    /// jumps, pawn diagonals, sliding rays (blocked by the first piece hit), and
    /// adjacent king squares. Every geometry is a precomputed mask, so each
    /// check is a handful of bit operations.
    pub fn square_attacked(&self, sq: u8, by: Color) -> bool {
        self.square_attacked_with(sq, by, self.occupied())
    }
    /// [`square_attacked`](Self::square_attacked) against an explicit blocker
    /// set. The legality filter lifts the moving king out of `occupied` so a
    /// sliding checker's ray extends through the square the king vacates.
    fn square_attacked_with(&self, sq: u8, by: Color, occupied: u64) -> bool {
        if KNIGHT_ATTACKS[sq as usize] & self.pieces(by, PieceKind::Knight) != 0 {
            return true;
        }
        // Pawn attackers sit one rank *toward* `by`'s home side of the target.
        let pawn_attackers = match by {
            Color::White => WHITE_PAWN_ATTACKERS[sq as usize],
            Color::Black => BLACK_PAWN_ATTACKERS[sq as usize],
        };
        if pawn_attackers & self.pieces(by, PieceKind::Pawn) != 0 {
            return true;
        }
        // Sliding attackers: orthogonal rays can hold a rook or queen, diagonal
        // rays a bishop or queen. Each ray stops at the first occupied square.
        let queens = self.pieces(by, PieceKind::Queen);
        if rook_attacks(sq, occupied) & (self.pieces(by, PieceKind::Rook) | queens) != 0 {
            return true;
        }
        if bishop_attacks(sq, occupied) & (self.pieces(by, PieceKind::Bishop) | queens) != 0 {
            return true;
        }
        KING_ATTACKS[sq as usize] & self.pieces(by, PieceKind::King) != 0
    }
    /// Whether the position is drawn by the 50-move rule or by insufficient
    /// material (bare kings, or king plus a single minor piece). Repetition is
    /// tracked by the search, not here, since it needs the move history.
    pub fn is_draw(&self) -> bool {
        self.halfmove >= 100 || {
            let kings = self.pieces(Color::White, PieceKind::King)
                | self.pieces(Color::Black, PieceKind::King);
            let non_kings = self.occupied() & !kings;
            non_kings == 0
                || (non_kings.count_ones() == 1
                    && non_kings
                        & (self.pieces(Color::White, PieceKind::Bishop)
                            | self.pieces(Color::Black, PieceKind::Bishop)
                            | self.pieces(Color::White, PieceKind::Knight)
                            | self.pieces(Color::Black, PieceKind::Knight))
                        != 0)
        }
    }
    /// Counts the leaf nodes reachable in exactly `depth` plies — the standard
    /// move-generation correctness test. Exact totals for known positions are
    /// asserted in the tests and the `bench perft` suite.
    pub fn perft(&self, depth: u8) -> u64 {
        if depth == 0 {
            return 1;
        }
        self.legal_moves()
            .into_iter()
            .map(|m| {
                let mut b = self.clone();
                b.make_move(m);
                b.perft(depth - 1)
            })
            .sum()
    }
    /// Computes the position's hash from scratch by mixing a distinct key for
    /// every piece/square, plus the castling rights, en-passant file, and side
    /// to move. It plays the role of a Zobrist key but derives its per-feature
    /// keys on the fly via [`mix64`] instead of storing a random table.
    fn compute_hash(&self) -> u64 {
        let mut h = 0x9e3779b97f4a7c15u64;
        for (sq, p) in self.squares.iter().enumerate() {
            if let Some(p) = p {
                h ^= piece_hash(*p, sq as u8)
            }
        }
        h ^= CASTLING_HASHES[self.castling as usize];
        if let Some(ep) = self.en_passant {
            h ^= EN_PASSANT_HASHES[(ep % 8) as usize]
        }
        if self.side == Color::Black {
            h ^= SIDE_HASH
        }
        h
    }

    /// Recomputes the derived `pieces`/`occupancy` bitboards and the material
    /// accumulator from the mailbox. Called after every wholesale rebuild (FEN
    /// parsing) so the representations never drift.
    fn rebuild_bitboards(&mut self) {
        self.pieces = [0; 12];
        self.occupancy = [0; 2];
        self.material_pst = 0;
        for (square, piece) in self.squares.iter().copied().enumerate() {
            if let Some(piece) = piece {
                let bit = 1u64 << square;
                self.pieces[piece_index(piece)] |= bit;
                self.occupancy[color_index(piece.color)] |= bit;
                self.material_pst += crate::eval::MATERIAL_PST[piece_index(piece)][square];
            }
        }
    }

    /// The running White-positive material-plus-piece-square total. Exact:
    /// every placement and removal adjusts it, and unmake replays them in
    /// reverse, so it always equals a from-scratch recomputation.
    pub(crate) fn material_pst(&self) -> i32 {
        self.material_pst
    }

    /// Removes a piece from both board representations.
    fn remove_piece(&mut self, square: u8) -> Option<Piece> {
        let piece = self.squares[square as usize].take()?;
        let bit = 1u64 << square;
        self.pieces[piece_index(piece)] &= !bit;
        self.occupancy[color_index(piece.color)] &= !bit;
        self.hash ^= piece_hash(piece, square);
        self.material_pst -= crate::eval::MATERIAL_PST[piece_index(piece)][square as usize];
        Some(piece)
    }

    /// Places a piece in both board representations.
    fn place_piece(&mut self, square: u8, piece: Piece) {
        debug_assert!(self.squares[square as usize].is_none());
        let bit = 1u64 << square;
        self.squares[square as usize] = Some(piece);
        self.pieces[piece_index(piece)] |= bit;
        self.occupancy[color_index(piece.color)] |= bit;
        self.hash ^= piece_hash(piece, square);
        self.material_pst += crate::eval::MATERIAL_PST[piece_index(piece)][square as usize];
    }
}

impl FromStr for Board {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_fen(s)
    }
}
/// Iterates the set squares of a bitboard from a1 toward h8.
pub(crate) fn bit_squares(mut bits: u64) -> impl Iterator<Item = u8> {
    std::iter::from_fn(move || {
        if bits == 0 {
            return None;
        }
        let square = bits.trailing_zeros() as u8;
        bits &= bits - 1;
        Some(square)
    })
}

/// The knight's and king's step offsets. Their order fixes the order moves are
/// emitted in, which downstream tie-breaks (unstable sorts on equal ordering
/// scores) turn into search-visible behavior — so it must not change.
const KNIGHT_OFFSETS: [(i8, i8); 8] = [
    (-2, -1),
    (-2, 1),
    (-1, -2),
    (-1, 2),
    (1, -2),
    (1, 2),
    (2, -1),
    (2, 1),
];
const KING_OFFSETS: [(i8, i8); 8] = [
    (-1, -1),
    (-1, 0),
    (-1, 1),
    (0, -1),
    (0, 1),
    (1, -1),
    (1, 0),
    (1, 1),
];

pub(crate) const KNIGHT_ATTACKS: [u64; 64] = attack_table(&KNIGHT_OFFSETS);
pub(crate) const KING_ATTACKS: [u64; 64] = attack_table(&KING_OFFSETS);

/// Per-square target lists for the leapers, in offset order, padded with the
/// `NO_TARGET` sentinel. Move generation walks these instead of re-deriving
/// file/rank arithmetic and bounds checks for every move.
const NO_TARGET: u8 = 64;
const KNIGHT_TARGETS: [[u8; 8]; 64] = target_table(&KNIGHT_OFFSETS);
const KING_TARGETS: [[u8; 8]; 64] = target_table(&KING_OFFSETS);

/// Builds per-square on-board target squares in offset order at compile time.
const fn target_table(offsets: &[(i8, i8)]) -> [[u8; 8]; 64] {
    let mut table = [[NO_TARGET; 8]; 64];
    let mut square = 0;
    while square < 64 {
        let file = (square % 8) as i8;
        let rank = (square / 8) as i8;
        let mut count = 0;
        let mut index = 0;
        while index < offsets.len() {
            let target_file = file + offsets[index].0;
            let target_rank = rank + offsets[index].1;
            if target_file >= 0 && target_file < 8 && target_rank >= 0 && target_rank < 8 {
                table[square][count] = (target_rank * 8 + target_file) as u8;
                count += 1;
            }
            index += 1;
        }
        square += 1;
    }
    table
}

/// Squares holding a pawn of the named color that would attack the indexed
/// square: white pawns attack from one rank below, black pawns from one above.
pub(crate) const WHITE_PAWN_ATTACKERS: [u64; 64] = attack_table(&[(-1, -1), (1, -1)]);
pub(crate) const BLACK_PAWN_ATTACKERS: [u64; 64] = attack_table(&[(-1, 1), (1, 1)]);

/// Builds target-centric leaper masks at compile time.
const fn attack_table(offsets: &[(i8, i8)]) -> [u64; 64] {
    let mut table = [0; 64];
    let mut square = 0;
    while square < 64 {
        let file = (square % 8) as i8;
        let rank = (square / 8) as i8;
        let mut index = 0;
        while index < offsets.len() {
            let target_file = file + offsets[index].0;
            let target_rank = rank + offsets[index].1;
            if target_file >= 0 && target_file < 8 && target_rank >= 0 && target_rank < 8 {
                table[square] |= 1u64 << (target_rank * 8 + target_file);
            }
            index += 1;
        }
        square += 1;
    }
    table
}
/// One sliding direction: the per-square masks of every square from there to
/// the board edge, plus whether stepping along it increases square indices.
/// That sign decides which end of a blocker set is the *first* blocker — the
/// lowest bit when ascending, the highest when descending — so a single bit
/// scan replaces walking the ray square by square.
pub(crate) struct Ray {
    table: &'static [u64; 64],
    ascending: bool,
}

impl Ray {
    /// The first occupied square along this ray from `from`, if any.
    pub(crate) fn first_blocker(&self, from: u8, occupied: u64) -> Option<u8> {
        let blockers = self.table[from as usize] & occupied;
        if blockers == 0 {
            None
        } else if self.ascending {
            Some(blockers.trailing_zeros() as u8)
        } else {
            Some((63 - blockers.leading_zeros()) as u8)
        }
    }
    /// Every square a slider on `from` attacks along this ray: the whole ray,
    /// cut just past its first blocker (captures include the blocker itself).
    fn attacks(&self, from: u8, occupied: u64) -> u64 {
        let ray = self.table[from as usize];
        match self.first_blocker(from, occupied) {
            None => ray,
            // The blocker's own ray holds exactly the squares behind it.
            Some(blocker) => ray ^ self.table[blocker as usize],
        }
    }
}

const RAY_NORTH: [u64; 64] = ray_table(0, 1);
const RAY_SOUTH: [u64; 64] = ray_table(0, -1);
const RAY_EAST: [u64; 64] = ray_table(1, 0);
const RAY_WEST: [u64; 64] = ray_table(-1, 0);
const RAY_NORTH_EAST: [u64; 64] = ray_table(1, 1);
const RAY_NORTH_WEST: [u64; 64] = ray_table(-1, 1);
const RAY_SOUTH_EAST: [u64; 64] = ray_table(1, -1);
const RAY_SOUTH_WEST: [u64; 64] = ray_table(-1, -1);

const NORTH: Ray = Ray {
    table: &RAY_NORTH,
    ascending: true,
};
const SOUTH: Ray = Ray {
    table: &RAY_SOUTH,
    ascending: false,
};
const EAST: Ray = Ray {
    table: &RAY_EAST,
    ascending: true,
};
const WEST: Ray = Ray {
    table: &RAY_WEST,
    ascending: false,
};
const NORTH_EAST: Ray = Ray {
    table: &RAY_NORTH_EAST,
    ascending: true,
};
const NORTH_WEST: Ray = Ray {
    table: &RAY_NORTH_WEST,
    ascending: true,
};
const SOUTH_EAST: Ray = Ray {
    table: &RAY_SOUTH_EAST,
    ascending: false,
};
const SOUTH_WEST: Ray = Ray {
    table: &RAY_SOUTH_WEST,
    ascending: false,
};

/// Bishop and rook directions in two fixed orders. Static exchange evaluation
/// probes rays one by one and keeps the first attacker it finds; move
/// generation emits each ray's moves in sequence. Both orders match the
/// historical square-by-square scans so exchange sequences and move lists —
/// and therefore search node counts — stay reproducible.
pub(crate) const DIAGONAL_RAYS: [Ray; 4] = [SOUTH_WEST, NORTH_WEST, SOUTH_EAST, NORTH_EAST];
pub(crate) const ORTHOGONAL_RAYS: [Ray; 4] = [WEST, EAST, SOUTH, NORTH];
const BISHOP_MOVE_RAYS: [Ray; 4] = [NORTH_EAST, SOUTH_EAST, NORTH_WEST, SOUTH_WEST];
const ROOK_MOVE_RAYS: [Ray; 4] = [EAST, WEST, NORTH, SOUTH];
const QUEEN_MOVE_RAYS: [Ray; 8] = [
    EAST, WEST, NORTH, SOUTH, NORTH_EAST, SOUTH_EAST, NORTH_WEST, SOUTH_WEST,
];

/// `BETWEEN[a][b]`: the squares strictly between `a` and `b` when they share a
/// rank, file, or diagonal; empty otherwise. Blocking a slider's check means
/// landing in `BETWEEN[king][checker]`.
static BETWEEN: [[u64; 64]; 64] = build_between();

/// `LINE[a][b]`: every square of the full rank, file, or diagonal through `a`
/// and `b` (including both), or empty if they are not aligned. A piece pinned
/// to the king may only move along `LINE[king][pin]`.
static LINE: [[u64; 64]; 64] = build_line();

const ALL_DIRECTIONS: [(i8, i8); 8] = [
    (0, 1),
    (0, -1),
    (1, 0),
    (-1, 0),
    (1, 1),
    (1, -1),
    (-1, 1),
    (-1, -1),
];

const fn build_between() -> [[u64; 64]; 64] {
    let mut between = [[0u64; 64]; 64];
    let mut a = 0;
    while a < 64 {
        let mut dir = 0;
        while dir < 8 {
            let (df, dr) = ALL_DIRECTIONS[dir];
            // Walk outward from `a`; the squares passed so far are exactly the
            // ones strictly between `a` and the square just reached.
            let mut passed = 0u64;
            let mut file = (a % 8) as i8 + df;
            let mut rank = (a / 8) as i8 + dr;
            while file >= 0 && file < 8 && rank >= 0 && rank < 8 {
                let b = (rank * 8 + file) as usize;
                between[a][b] = passed;
                passed |= 1u64 << b;
                file += df;
                rank += dr;
            }
            dir += 1;
        }
        a += 1;
    }
    between
}

const fn build_line() -> [[u64; 64]; 64] {
    let mut line = [[0u64; 64]; 64];
    let mut a = 0;
    while a < 64 {
        // Each direction pair spans one full line through `a`.
        let mut dir = 0;
        while dir < 8 {
            let (df, dr) = ALL_DIRECTIONS[dir];
            let full = ray_mask(a, df, dr) | ray_mask(a, -df, -dr) | 1u64 << a;
            let mut file = (a % 8) as i8 + df;
            let mut rank = (a / 8) as i8 + dr;
            while file >= 0 && file < 8 && rank >= 0 && rank < 8 {
                line[a][(rank * 8 + file) as usize] = full;
                file += df;
                rank += dr;
            }
            dir += 1;
        }
        a += 1;
    }
    line
}

/// The ray mask from `square` toward one direction, to the board edge.
const fn ray_mask(square: usize, df: i8, dr: i8) -> u64 {
    let mut mask = 0u64;
    let mut file = (square % 8) as i8 + df;
    let mut rank = (square / 8) as i8 + dr;
    while file >= 0 && file < 8 && rank >= 0 && rank < 8 {
        mask |= 1u64 << (rank * 8 + file);
        file += df;
        rank += dr;
    }
    mask
}

/// Builds one direction's ray masks at compile time.
const fn ray_table(df: i8, dr: i8) -> [u64; 64] {
    let mut table = [0u64; 64];
    let mut square = 0;
    while square < 64 {
        table[square] = ray_mask(square, df, dr);
        square += 1;
    }
    table
}

/// Every square a rook on `sq` attacks given `occupied` blockers.
pub(crate) fn rook_attacks(sq: u8, occupied: u64) -> u64 {
    let [w, e, s, n] = &ORTHOGONAL_RAYS;
    w.attacks(sq, occupied)
        | e.attacks(sq, occupied)
        | s.attacks(sq, occupied)
        | n.attacks(sq, occupied)
}
/// Every square a bishop on `sq` attacks given `occupied` blockers.
pub(crate) fn bishop_attacks(sq: u8, occupied: u64) -> u64 {
    let [sw, nw, se, ne] = &DIAGONAL_RAYS;
    sw.attacks(sq, occupied)
        | nw.attacks(sq, occupied)
        | se.attacks(sq, occupied)
        | ne.attacks(sq, occupied)
}

/// Emits knight or king steps from the precomputed target list, skipping own
/// pieces and — in captures-only mode — empty squares. The list's offset order
/// is preserved verbatim.
fn leaper_moves(
    out: &mut Vec<Move>,
    from: u8,
    targets: &[[u8; 8]; 64],
    own: u64,
    enemy: u64,
    captures_only: bool,
) {
    for &to in &targets[from as usize] {
        if to == NO_TARGET {
            break;
        }
        let bit = 1u64 << to;
        if own & bit != 0 || (captures_only && enemy & bit == 0) {
            continue;
        }
        out.push(Move {
            from,
            to,
            ..Move::default()
        })
    }
}

/// Emits sliding moves for a bishop/rook/queen: each ray, cut at its first
/// blocker by the precomputed tables, yields quiet moves and a capture of an
/// enemy blocker. Iterating each ray's bits in its direction of travel
/// reproduces the near-to-far order of the original square-by-square walk.
fn slider_moves(
    out: &mut Vec<Move>,
    from: u8,
    rays: &[Ray],
    own: u64,
    occupied: u64,
    captures_only: bool,
) {
    let enemy = occupied & !own;
    for ray in rays {
        let mut reachable = ray.attacks(from, occupied) & !own;
        if captures_only {
            reachable &= enemy;
        }
        while reachable != 0 {
            let to = if ray.ascending {
                reachable.trailing_zeros() as u8
            } else {
                (63 - reachable.leading_zeros()) as u8
            };
            reachable &= !(1u64 << to);
            out.push(Move {
                from,
                to,
                ..Move::default()
            })
        }
    }
}
/// Parses a square name like `e4` into its 0..64 index, or `None` if malformed.
pub fn parse_square(s: &str) -> Option<u8> {
    let b = s.as_bytes();
    if b.len() != 2 || !(b'a'..=b'h').contains(&b[0]) || !(b'1'..=b'8').contains(&b[1]) {
        None
    } else {
        Some((b[1] - b'1') * 8 + b[0] - b'a')
    }
}
/// Formats a square index as its name, e.g. `28 -> "e4"`. Inverse of [`parse_square`].
pub fn square_name(s: u8) -> String {
    format!("{}{}", (b'a' + s % 8) as char, (b'1' + s / 8) as char)
}
/// SplitMix64 finalizer: scrambles an integer into a well-distributed 64-bit
/// value. Used to derive per-feature hash keys in [`Board::compute_hash`].
pub(crate) const fn mix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e3779b97f4a7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

/// The hash feature for one piece on one square.
fn piece_hash(piece: Piece, square: u8) -> u64 {
    PIECE_HASHES[piece_index(piece) * 64 + square as usize]
}

const PIECE_HASHES: [u64; 12 * 64] = build_piece_hashes();
const CASTLING_HASHES: [u64; 16] = build_hashes::<16>(500);
const EN_PASSANT_HASHES: [u64; 8] = build_hashes::<8>(600);
const SIDE_HASH: u64 = mix64(700);

const fn build_piece_hashes() -> [u64; 12 * 64] {
    let mut hashes = [0; 12 * 64];
    let mut piece = 0;
    while piece < 12 {
        let color_key = if piece < 6 { 1 } else { 2 };
        let kind = piece % 6 + 1;
        let mut square = 0;
        while square < 64 {
            hashes[piece * 64 + square] = mix64(square as u64 + 64 * (kind as u64 + 8 * color_key));
            square += 1;
        }
        piece += 1;
    }
    hashes
}

const fn build_hashes<const N: usize>(base: u64) -> [u64; N] {
    let mut hashes = [0; N];
    let mut index = 0;
    while index < N {
        hashes[index] = mix64(base + index as u64);
        index += 1;
    }
    hashes
}

/// Maps a color to the 0/1 index used by the `occupancy` array.
const fn color_index(color: Color) -> usize {
    match color {
        Color::White => 0,
        Color::Black => 1,
    }
}

/// Maps a piece to its 0..12 slot in the `pieces` array: white pieces occupy
/// slots 0..6, black 6..12, ordered by [`PieceKind`].
pub(crate) const fn piece_index(piece: Piece) -> usize {
    color_index(piece.color) * 6 + piece.kind as usize - 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legal_move_names(fen: &str) -> Vec<String> {
        let mut moves = Board::from_fen(fen)
            .unwrap()
            .legal_moves()
            .into_iter()
            .map(|chess_move| chess_move.to_string())
            .collect::<Vec<_>>();
        moves.sort_unstable();
        moves
    }

    #[test]
    fn fen_roundtrip() {
        let b = Board::startpos();
        assert_eq!(
            b.to_fen(),
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"
        )
    }

    #[test]
    fn draw_detection_covers_clocks_and_insufficient_material() {
        assert!(
            Board::from_fen("4k3/8/8/8/8/8/8/4K3 w - - 0 1")
                .unwrap()
                .is_draw()
        );
        assert!(
            Board::from_fen("4k3/8/8/8/8/8/8/3BK3 w - - 0 1")
                .unwrap()
                .is_draw()
        );
        assert!(
            Board::from_fen("4k3/8/8/8/8/8/8/3NK3 w - - 0 1")
                .unwrap()
                .is_draw()
        );
        assert!(
            !Board::from_fen("4k3/8/8/8/8/8/8/3RK3 w - - 0 1")
                .unwrap()
                .is_draw()
        );
        assert!(
            !Board::from_fen("4k3/8/8/8/8/8/8/2BBK3 w - - 0 1")
                .unwrap()
                .is_draw()
        );
        assert!(
            Board::from_fen("4k3/8/8/8/8/8/8/3RK3 w - - 100 1")
                .unwrap()
                .is_draw()
        );
    }

    #[test]
    fn incremental_hash_matches_full_recomputation() {
        for fen in [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "4k3/1P6/8/3pP3/8/8/6p1/4K3 w - d6 0 1",
        ] {
            let original = Board::from_fen(fen).unwrap();
            for chess_move in original.legal_moves() {
                let mut board = original.clone();
                let undo = board.make_move(chess_move);
                assert_eq!(board.hash, board.compute_hash(), "{fen}: {chess_move}");
                board.unmake_move(chess_move, undo);
                assert_eq!(board, original, "{fen}: {chess_move}");
            }

            let mut board = original.clone();
            let undo = board.make_null_move();
            assert_eq!(board.hash, board.compute_hash(), "{fen}: null move");
            board.unmake_null_move(undo);
            assert_eq!(board, original, "{fen}: null move");
        }
    }

    #[test]
    fn standard_perft() {
        let b = Board::startpos();
        assert_eq!(b.perft(1), 20);
        assert_eq!(b.perft(2), 400);
        assert_eq!(b.perft(3), 8902);
        assert_eq!(b.perft(4), 197281)
    }

    #[test]
    fn legal_moves_match_reference_rule_edge_fixtures() {
        assert_eq!(
            legal_move_names("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1"),
            [
                "a1a2", "a1a3", "a1a4", "a1a5", "a1a6", "a1a7", "a1a8", "a1b1", "a1c1", "a1d1",
                "e1c1", "e1d1", "e1d2", "e1e2", "e1f1", "e1f2", "e1g1", "h1f1", "h1g1", "h1h2",
                "h1h3", "h1h4", "h1h5", "h1h6", "h1h7", "h1h8",
            ]
        );
        // g5xf6 e.p. would remove both blockers from the fifth rank and expose
        // the white king to the rook on a5, so the reference rejects it.
        assert_eq!(
            legal_move_names("4k3/8/8/r4pPK/8/8/8/8 w - f6 0 1"),
            ["g5g6", "h5g6", "h5h4", "h5h6"]
        );
        assert_eq!(
            legal_move_names("4k3/1P6/8/8/8/8/6p1/4K3 w - - 0 1"),
            [
                "b7b8b", "b7b8n", "b7b8q", "b7b8r", "e1d1", "e1d2", "e1e2", "e1f2",
            ]
        );
    }
    #[test]
    fn kiwipete() {
        let b =
            Board::from_fen("r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1")
                .unwrap();
        assert_eq!(b.perft(1), 48);
        assert_eq!(b.perft(2), 2039);
        assert_eq!(b.perft(3), 97862)
    }
    #[test]
    fn special_moves() {
        let mut b = Board::startpos();
        for s in ["e2e4", "a7a6", "e4e5", "d7d5", "e5d6"] {
            let m = b.parse_move(s).unwrap();
            b.make_move(m);
        }
        assert!(b.squares[35].is_none());
        assert!(b.squares[43].is_some())
    }

    #[test]
    fn make_unmake_restores_every_root_move() {
        for fen in [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 7 19",
            "4k3/P6p/8/3pP3/8/8/7P/4K3 w - d6 31 72",
        ] {
            let original = Board::from_fen(fen).unwrap();
            let mut board = original.clone();
            for chess_move in original.legal_moves() {
                let undo = board.make_move(chess_move);
                board.unmake_move(chess_move, undo);
                assert_eq!(board, original, "failed to restore {chess_move} from {fen}");
            }
        }
    }

    /// Legality by definition: play each pseudo-legal move and keep it only if
    /// our king is not left in check. The pin-aware filter must match this
    /// exactly.
    fn played_out_legal_moves(board: &Board, captures_only: bool) -> Vec<Move> {
        let mut moves = Vec::new();
        board.generate_pseudo_moves(&mut moves, captures_only);
        let mut scratch = board.clone();
        moves.retain(|&m| {
            let undo = scratch.make_move(m);
            let legal = !scratch.in_check(board.side);
            scratch.unmake_move(m, undo);
            legal
        });
        moves
    }

    #[test]
    fn pin_aware_filter_matches_played_out_legality() {
        let fens = [
            // Ordinary opening and the tactical Kiwipete standard.
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 7 19",
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R b KQkq - 7 19",
            // En passant capture legal: no pin behind it.
            "8/8/8/8/k2Pp3/8/8/4K3 b - d3 0 1",
            // En passant capture illegal: both pawns leave the fourth rank and
            // uncover the king to the rook.
            "8/8/8/8/k2Pp2R/8/8/4K3 b - d3 0 1",
            // The Chess Programming Wiki's en-passant/pin tangles.
            "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
            "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 b - - 0 1",
            "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
            "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
            // Double check: only king moves answer.
            "4k3/8/8/8/7b/5n2/8/4K3 w - - 0 1",
            // Checked by a slider: block, capture, or step off the ray.
            "4k3/8/8/8/8/8/8/4R2K b - - 0 1",
            // A full complement of pins on both wings.
            "3r4/8/8/b2q4/8/2NPB3/3K4/8 w - - 0 1",
        ];
        for fen in fens {
            // Walk a deterministic pseudo-random game from each start, holding
            // the filters equal at every position along the way.
            let mut board = Board::from_fen(fen).unwrap();
            let mut seed = 0x9e3779b97f4a7c15u64;
            for ply in 0..40 {
                let legal = board.legal_moves();
                assert_eq!(
                    legal,
                    played_out_legal_moves(&board, false),
                    "legal moves diverged at ply {ply} of {fen} ({})",
                    board.to_fen()
                );
                assert_eq!(
                    board.legal_captures(),
                    played_out_legal_moves(&board, true),
                    "captures diverged at ply {ply} of {fen} ({})",
                    board.to_fen()
                );
                if legal.is_empty() {
                    break;
                }
                seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                board.make_move(legal[(seed >> 33) as usize % legal.len()]);
            }
        }
    }

    #[test]
    fn material_accumulator_matches_recomputation_after_every_root_move() {
        // Make+unmake symmetry alone could hide an update that is wrong the
        // same way in both directions, so compare against a from-scratch
        // rebuild after each move. The positions cover captures, castling,
        // en passant, and promotion.
        for fen in [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 7 19",
            "4k3/P6p/8/3pP3/8/8/7P/4K3 w - d6 31 72",
        ] {
            let mut board = Board::from_fen(fen).unwrap();
            for chess_move in board.legal_moves() {
                let undo = board.make_move(chess_move);
                let rebuilt = Board::from_fen(&board.to_fen()).unwrap();
                assert_eq!(
                    board.material_pst(),
                    rebuilt.material_pst(),
                    "accumulator drifted after {chess_move} from {fen}"
                );
                board.unmake_move(chess_move, undo);
            }
        }
    }

    #[test]
    fn null_move_restores_side_en_passant_and_hash() {
        let mut board =
            Board::from_fen("rnbqkbnr/pppppppp/8/8/4Pp2/8/PPPP2PP/RNBQKBNR w KQkq f6 0 3").unwrap();
        let original = board.clone();
        let undo = board.make_null_move();
        assert_eq!(board.side, Color::Black);
        assert_eq!(board.en_passant, None);
        assert_ne!(board.hash, original.hash);
        assert_eq!(board.halfmove, original.halfmove);
        assert_eq!(board.fullmove, original.fullmove);
        board.unmake_null_move(undo);
        assert_eq!(board, original);
    }

    #[test]
    fn bitboards_match_square_inspection() {
        let board = Board::startpos();
        assert_eq!(board.occupancy(Color::White).count_ones(), 16);
        assert_eq!(board.occupancy(Color::Black).count_ones(), 16);
        assert_eq!(
            board.pieces(Color::White, PieceKind::Pawn),
            0x0000_0000_0000_ff00
        );
        assert_eq!(board.occupied(), 0xffff_0000_0000_ffff);
    }

    #[test]
    fn parses_standard_algebraic_moves() {
        let board = Board::startpos();
        assert_eq!(board.parse_san("Nf3"), board.parse_move("g1f3"));
        assert_eq!(board.parse_san("e4"), board.parse_move("e2e4"));

        let tactical =
            Board::from_fen("2rr3k/pp3pp1/1nnqbN1p/3pN3/2pP4/2P3Q1/PPB4P/R4RK1 w - - 0 1").unwrap();
        assert_eq!(tactical.parse_san("Qg6"), tactical.parse_move("g3g6"));
    }
}
