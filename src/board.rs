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
        let king = Piece {
            color,
            kind: PieceKind::King,
        };
        self.squares
            .iter()
            .position(|&p| p == Some(king))
            .map(|sq| sq as u8)
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
    /// Legality is decided the simple way: generate pseudo-legal moves, play each
    /// on a scratch copy, and keep only those that do not leave our own king in
    /// check. This is slower than a pin-aware generator but is easy to trust, and
    /// perft validates it exactly.
    pub fn generate_legal_moves(&self, out: &mut Vec<Move>) {
        out.clear();
        let mut board = self.clone();
        for m in self.pseudo_moves(false) {
            let undo = board.make_move(m);
            if !board.in_check(self.side) {
                out.push(m);
            }
            board.unmake_move(m, undo);
        }
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
        self.pseudo_moves(true)
            .into_iter()
            .filter(|m| {
                let mut b = self.clone();
                b.make_move(*m);
                !b.in_check(self.side)
            })
            .collect()
    }

    /// Generates pseudo-legal moves for the side to move — moves that respect
    /// each piece's movement but may leave the king in check. When
    /// `captures_only` is set (quiescence search), quiet moves are skipped, but
    /// promotions are always emitted because they change material.
    fn pseudo_moves(&self, captures_only: bool) -> Vec<Move> {
        let mut out = Vec::with_capacity(if captures_only { 16 } else { 48 });
        for from in 0..64u8 {
            let Some(piece) = self.squares[from as usize] else {
                continue;
            };
            if piece.color != self.side {
                continue;
            }
            let f = (from % 8) as i8;
            let r = (from / 8) as i8;
            // Records a candidate move, rejecting self-captures and — in
            // captures-only mode — quiet non-promotion moves.
            let mut add = |to: u8, promotion: Option<PieceKind>, ep: bool, castle: bool| {
                let target = self.squares[to as usize];
                if target.map(|p| p.color) == Some(piece.color) {
                    return;
                }
                if !captures_only || target.is_some() || ep || promotion.is_some() {
                    out.push(Move {
                        from,
                        to,
                        promotion,
                        en_passant: ep,
                        castle,
                    })
                }
            };
            match piece.kind {
                PieceKind::Pawn => {
                    // Direction of travel and the ranks where a pawn may still
                    // double-step (`start`) or promote (`last`).
                    let d = if piece.color == Color::White { 1 } else { -1 };
                    let start = if piece.color == Color::White { 1 } else { 6 };
                    let last = if piece.color == Color::White { 7 } else { 0 };
                    let nr = r + d;
                    if (0..8).contains(&nr) {
                        // Single (and double) push onto an empty square.
                        let forward = (nr * 8 + f) as u8;
                        if !captures_only && self.squares[forward as usize].is_none() {
                            if nr == last {
                                // Reaching the last rank fans out into the four
                                // promotion choices.
                                for p in [
                                    PieceKind::Queen,
                                    PieceKind::Rook,
                                    PieceKind::Bishop,
                                    PieceKind::Knight,
                                ] {
                                    add(forward, Some(p), false, false)
                                }
                            } else {
                                add(forward, None, false, false);
                                // From the starting rank, a two-square push is
                                // allowed if the intervening square is also empty.
                                if r == start {
                                    let two = ((r + 2 * d) * 8 + f) as u8;
                                    if self.squares[two as usize].is_none() {
                                        add(two, None, false, false)
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
                            if self.squares[to as usize]
                                .map(|p| p.color == piece.color.other())
                                .unwrap_or(false)
                                || self.en_passant == Some(to)
                            {
                                if nr == last {
                                    for p in [
                                        PieceKind::Queen,
                                        PieceKind::Rook,
                                        PieceKind::Bishop,
                                        PieceKind::Knight,
                                    ] {
                                        add(to, Some(p), false, false)
                                    }
                                } else {
                                    add(to, None, self.en_passant == Some(to), false)
                                }
                            }
                        }
                    }
                }
                PieceKind::Knight => {
                    // The eight L-shaped jumps; `offset` drops any that leave the board.
                    for (df, dr) in [
                        (-2, -1),
                        (-2, 1),
                        (-1, -2),
                        (-1, 2),
                        (1, -2),
                        (1, 2),
                        (2, -1),
                        (2, 1),
                    ] {
                        if let Some(to) = offset(f, r, df, dr) {
                            add(to, None, false, false)
                        }
                    }
                }
                // Sliders share `slide`, differing only in their ray directions.
                PieceKind::Bishop => slide(
                    self,
                    &mut out,
                    from,
                    piece,
                    &[(1, 1), (1, -1), (-1, 1), (-1, -1)],
                    captures_only,
                ),
                PieceKind::Rook => slide(
                    self,
                    &mut out,
                    from,
                    piece,
                    &[(1, 0), (-1, 0), (0, 1), (0, -1)],
                    captures_only,
                ),
                PieceKind::Queen => slide(
                    self,
                    &mut out,
                    from,
                    piece,
                    &[
                        (1, 0),
                        (-1, 0),
                        (0, 1),
                        (0, -1),
                        (1, 1),
                        (1, -1),
                        (-1, 1),
                        (-1, -1),
                    ],
                    captures_only,
                ),
                PieceKind::King => {
                    // One step in each of the eight directions.
                    for (df, dr) in [
                        (-1, -1),
                        (-1, 0),
                        (-1, 1),
                        (0, -1),
                        (0, 1),
                        (1, -1),
                        (1, 0),
                        (1, 1),
                    ] {
                        if let Some(to) = offset(f, r, df, dr) {
                            add(to, None, false, false)
                        }
                    }
                    if !captures_only {
                        // Castling: pick this color's home rank, the enemy color
                        // that would attack through squares, and the king/queen
                        // side rights bits.
                        let (rank, enemy, ks, qs) = if piece.color == Color::White {
                            (0, Color::Black, 1, 2)
                        } else {
                            (7, Color::White, 4, 8)
                        };
                        let base = rank * 8;
                        // The king must be on its home square and not currently in
                        // check; the squares it passes through are checked below.
                        if from == base + 4 && !self.square_attacked(from, enemy) {
                            // Kingside: f- and g-file empty and unattacked, rook present.
                            if self.castling & ks != 0
                                && self.squares[base as usize + 5].is_none()
                                && self.squares[base as usize + 6].is_none()
                                && !self.square_attacked(base + 5, enemy)
                                && !self.square_attacked(base + 6, enemy)
                                && self.squares[base as usize + 7]
                                    == Some(Piece {
                                        color: piece.color,
                                        kind: PieceKind::Rook,
                                    })
                            {
                                add(base + 6, None, false, true)
                            }
                            // Queenside: b-, c-, d-file empty; c- and d-file
                            // unattacked (the king does not pass over b); rook present.
                            if self.castling & qs != 0
                                && self.squares[base as usize + 1].is_none()
                                && self.squares[base as usize + 2].is_none()
                                && self.squares[base as usize + 3].is_none()
                                && !self.square_attacked(base + 3, enemy)
                                && !self.square_attacked(base + 2, enemy)
                                && self.squares[base as usize]
                                    == Some(Piece {
                                        color: piece.color,
                                        kind: PieceKind::Rook,
                                    })
                            {
                                add(base + 2, None, false, true)
                            }
                        }
                    }
                }
            }
        }
        out
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
        self.hash ^= mix64(500 + u64::from(undo.castling));
        self.hash ^= mix64(500 + u64::from(self.castling));
        if let Some(ep) = undo.en_passant {
            self.hash ^= mix64(600 + u64::from(ep % 8));
        }
        if let Some(ep) = self.en_passant {
            self.hash ^= mix64(600 + u64::from(ep % 8));
        }
        self.hash ^= mix64(700);
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
            self.hash ^= mix64(600 + u64::from(ep % 8));
        }
        self.hash ^= mix64(700);
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
    /// adjacent king squares.
    pub fn square_attacked(&self, sq: u8, by: Color) -> bool {
        let f = (sq % 8) as i8;
        let r = (sq / 8) as i8;
        // Knight attackers.
        for (df, dr) in [
            (-2, -1),
            (-2, 1),
            (-1, -2),
            (-1, 2),
            (1, -2),
            (1, 2),
            (2, -1),
            (2, 1),
        ] {
            if offset(f, r, df, dr).is_some_and(|s| {
                self.squares[s as usize]
                    == Some(Piece {
                        color: by,
                        kind: PieceKind::Knight,
                    })
            }) {
                return true;
            }
        }
        // Pawn attackers sit one rank *toward* `by`'s home side of the target.
        let pd = if by == Color::White { -1 } else { 1 };
        for df in [-1, 1] {
            if offset(f, r, df, pd).is_some_and(|s| {
                self.squares[s as usize]
                    == Some(Piece {
                        color: by,
                        kind: PieceKind::Pawn,
                    })
            }) {
                return true;
            }
        }
        // Sliding attackers: orthogonal rays can hold a rook or queen, diagonal
        // rays a bishop or queen. Each ray stops at the first occupied square.
        for (dirs, kinds) in [
            (
                &[(1, 0), (-1, 0), (0, 1), (0, -1)][..],
                &[PieceKind::Rook, PieceKind::Queen][..],
            ),
            (
                &[(1, 1), (1, -1), (-1, 1), (-1, -1)][..],
                &[PieceKind::Bishop, PieceKind::Queen][..],
            ),
        ] {
            for &(df, dr) in dirs {
                let (mut nf, mut nr) = (f + df, r + dr);
                while (0..8).contains(&nf) && (0..8).contains(&nr) {
                    if let Some(p) = self.squares[(nr * 8 + nf) as usize] {
                        if p.color == by && kinds.contains(&p.kind) {
                            return true;
                        }
                        break;
                    }
                    nf += df;
                    nr += dr
                }
            }
        }
        // Enemy king on an adjacent square.
        for df in -1..=1 {
            for dr in -1..=1 {
                if (df != 0 || dr != 0)
                    && offset(f, r, df, dr).is_some_and(|s| {
                        self.squares[s as usize]
                            == Some(Piece {
                                color: by,
                                kind: PieceKind::King,
                            })
                    })
                {
                    return true;
                }
            }
        }
        false
    }
    /// Whether the position is drawn by the 50-move rule or by insufficient
    /// material (bare kings, or king plus a single minor piece). Repetition is
    /// tracked by the search, not here, since it needs the move history.
    pub fn is_draw(&self) -> bool {
        self.halfmove >= 100 || {
            let non_kings: Vec<_> = self
                .squares
                .iter()
                .flatten()
                .filter(|p| p.kind != PieceKind::King)
                .collect();
            non_kings.is_empty()
                || (non_kings.len() == 1
                    && matches!(non_kings[0].kind, PieceKind::Bishop | PieceKind::Knight))
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
                h ^= mix64(
                    sq as u64
                        + 64 * (p.kind as u64 + 8 * if p.color == Color::White { 1 } else { 2 }),
                )
            }
        }
        h ^= mix64(500 + self.castling as u64);
        if let Some(ep) = self.en_passant {
            h ^= mix64(600 + (ep % 8) as u64)
        }
        if self.side == Color::Black {
            h ^= mix64(700)
        }
        h
    }

    /// Recomputes the derived `pieces`/`occupancy` bitboards from the mailbox.
    /// Called after every mutation so the two representations never drift.
    fn rebuild_bitboards(&mut self) {
        self.pieces = [0; 12];
        self.occupancy = [0; 2];
        for (square, piece) in self.squares.iter().copied().enumerate() {
            if let Some(piece) = piece {
                let bit = 1u64 << square;
                self.pieces[piece_index(piece)] |= bit;
                self.occupancy[color_index(piece.color)] |= bit;
            }
        }
    }

    /// Removes a piece from both board representations.
    fn remove_piece(&mut self, square: u8) -> Option<Piece> {
        let piece = self.squares[square as usize].take()?;
        let bit = 1u64 << square;
        self.pieces[piece_index(piece)] &= !bit;
        self.occupancy[color_index(piece.color)] &= !bit;
        self.hash ^= piece_hash(piece, square);
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
    }
}

impl FromStr for Board {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_fen(s)
    }
}
/// Adds `(df, dr)` to file/rank `(f, r)` and returns the resulting square, or
/// `None` if it steps off the board. The bounds check is what makes the
/// per-piece offset tables above safe to apply blindly.
fn offset(f: i8, r: i8, df: i8, dr: i8) -> Option<u8> {
    let (nf, nr) = (f + df, r + dr);
    ((0..8).contains(&nf) && (0..8).contains(&nr)).then_some((nr * 8 + nf) as u8)
}
/// Emits sliding moves for a bishop/rook/queen: walk each direction until the
/// edge, adding quiet moves onto empty squares and a capture onto the first
/// enemy piece, then stop that ray.
fn slide(
    b: &Board,
    out: &mut Vec<Move>,
    from: u8,
    p: Piece,
    dirs: &[(i8, i8)],
    captures_only: bool,
) {
    let (f, r) = ((from % 8) as i8, (from / 8) as i8);
    for &(df, dr) in dirs {
        let (mut nf, mut nr) = (f + df, r + dr);
        while (0..8).contains(&nf) && (0..8).contains(&nr) {
            let to = (nr * 8 + nf) as u8;
            match b.squares[to as usize] {
                None => {
                    if !captures_only {
                        out.push(Move {
                            from,
                            to,
                            ..Move::default()
                        })
                    }
                }
                Some(q) => {
                    if q.color != p.color {
                        out.push(Move {
                            from,
                            to,
                            ..Move::default()
                        })
                    }
                    break;
                }
            }
            nf += df;
            nr += dr
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
fn mix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e3779b97f4a7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

/// The hash feature for one piece on one square.
fn piece_hash(piece: Piece, square: u8) -> u64 {
    mix64(
        u64::from(square)
            + 64 * (piece.kind as u64 + 8 * if piece.color == Color::White { 1 } else { 2 }),
    )
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
const fn piece_index(piece: Piece) -> usize {
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
