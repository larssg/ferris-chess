use std::{fmt, str::FromStr};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Color {
    White,
    Black,
}
impl Color {
    pub const fn other(self) -> Self {
        match self {
            Self::White => Self::Black,
            Self::Black => Self::White,
        }
    }
}

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Piece {
    pub color: Color,
    pub kind: PieceKind,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct Move {
    pub from: u8,
    pub to: u8,
    pub promotion: Option<PieceKind>,
    pub en_passant: bool,
    pub castle: bool,
}

impl fmt::Display for Move {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.from == self.to {
            return f.write_str("0000");
        }
        write!(f, "{}{}", square_name(self.from), square_name(self.to))?;
        if let Some(p) = self.promotion {
            f.write_str(match p {
                PieceKind::Queen => "q",
                PieceKind::Rook => "r",
                PieceKind::Bishop => "b",
                PieceKind::Knight => "n",
                _ => "",
            })?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Board {
    squares: [Option<Piece>; 64],
    pieces: [u64; 12],
    occupancy: [u64; 2],
    pub side: Color,
    pub castling: u8,
    pub en_passant: Option<u8>,
    pub halfmove: u16,
    pub fullmove: u16,
    pub hash: u64,
}

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

impl Default for Board {
    fn default() -> Self {
        Self::startpos()
    }
}

impl Board {
    pub fn startpos() -> Self {
        Self::from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap()
    }

    pub fn from_fen(fen: &str) -> Result<Self, String> {
        let p: Vec<_> = fen.split_whitespace().collect();
        if p.len() < 4 {
            return Err("FEN requires at least four fields".into());
        }
        let ranks: Vec<_> = p[0].split('/').collect();
        if ranks.len() != 8 {
            return Err("FEN must contain eight ranks".into());
        }
        let mut squares = [None; 64];
        for (ri, rank) in ranks.iter().enumerate() {
            let mut file = 0usize;
            for ch in rank.chars() {
                if let Some(n) = ch.to_digit(10) {
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
                    let kind = match ch.to_ascii_lowercase() {
                        'p' => PieceKind::Pawn,
                        'n' => PieceKind::Knight,
                        'b' => PieceKind::Bishop,
                        'r' => PieceKind::Rook,
                        'q' => PieceKind::Queen,
                        'k' => PieceKind::King,
                        _ => return Err(format!("invalid piece {ch}")),
                    };
                    squares[(7 - ri) * 8 + file] = Some(Piece { color, kind });
                    file += 1;
                }
            }
            if file != 8 {
                return Err("each FEN rank must describe eight squares".into());
            }
        }
        let side = match p[1] {
            "w" => Color::White,
            "b" => Color::Black,
            _ => return Err("invalid side to move".into()),
        };
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
        let en_passant = if p[3] == "-" {
            None
        } else {
            Some(parse_square(p[3]).ok_or("invalid en-passant square")?)
        };
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
        b.rebuild_bitboards();
        b.hash = b.compute_hash();
        Ok(b)
    }

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
                        let mut c = match p.kind {
                            PieceKind::Pawn => 'p',
                            PieceKind::Knight => 'n',
                            PieceKind::Bishop => 'b',
                            PieceKind::Rook => 'r',
                            PieceKind::Queen => 'q',
                            PieceKind::King => 'k',
                        };
                        if p.color == Color::White {
                            c = c.to_ascii_uppercase()
                        }
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

    pub fn parse_move(&self, text: &str) -> Option<Move> {
        let t = text.trim().to_ascii_lowercase();
        if t.len() < 4 {
            return None;
        }
        let from = parse_square(&t[0..2])?;
        let to = parse_square(&t[2..4])?;
        let promo = t.as_bytes().get(4).and_then(|c| match c {
            b'q' => Some(PieceKind::Queen),
            b'r' => Some(PieceKind::Rook),
            b'b' => Some(PieceKind::Bishop),
            b'n' => Some(PieceKind::Knight),
            _ => None,
        });
        self.legal_moves().into_iter().find(|m| {
            m.from == from
                && m.to == to
                && (m.promotion == promo
                    || (promo.is_none() && m.promotion == Some(PieceKind::Queen)))
        })
    }

    pub fn piece_at(&self, square: u8) -> Option<Piece> {
        self.squares.get(square as usize).copied().flatten()
    }

    pub fn pieces(&self, color: Color, kind: PieceKind) -> u64 {
        self.pieces[piece_index(Piece { color, kind })]
    }

    pub fn occupancy(&self, color: Color) -> u64 {
        self.occupancy[color_index(color)]
    }

    pub fn occupied(&self) -> u64 {
        self.occupancy[0] | self.occupancy[1]
    }

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
    pub fn legal_moves(&self) -> Vec<Move> {
        let mut moves = Vec::with_capacity(48);
        self.generate_legal_moves(&mut moves);
        moves
    }
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
                    let d = if piece.color == Color::White { 1 } else { -1 };
                    let start = if piece.color == Color::White { 1 } else { 6 };
                    let last = if piece.color == Color::White { 7 } else { 0 };
                    let nr = r + d;
                    if (0..8).contains(&nr) {
                        let forward = (nr * 8 + f) as u8;
                        if !captures_only && self.squares[forward as usize].is_none() {
                            if nr == last {
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
                                if r == start {
                                    let two = ((r + 2 * d) * 8 + f) as u8;
                                    if self.squares[two as usize].is_none() {
                                        add(two, None, false, false)
                                    }
                                }
                            }
                        }
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
                        let (rank, enemy, ks, qs) = if piece.color == Color::White {
                            (0, Color::Black, 1, 2)
                        } else {
                            (7, Color::White, 4, 8)
                        };
                        let base = rank * 8;
                        if from == base + 4 && !self.square_attacked(from, enemy) {
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

    pub fn make_move(&mut self, m: Move) -> Undo {
        let piece = self.squares[m.from as usize].unwrap();
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
        self.en_passant = None;
        if m.en_passant {
            let sq = if piece.color == Color::White {
                m.to - 8
            } else {
                m.to + 8
            };
            self.squares[sq as usize] = None
        }
        self.squares[m.from as usize] = None;
        self.squares[m.to as usize] = Some(Piece {
            color: piece.color,
            kind: m.promotion.unwrap_or(piece.kind),
        });
        if m.castle {
            let (rf, rt) = if m.to > m.from {
                (m.from + 3, m.from + 1)
            } else {
                (m.from - 4, m.from - 1)
            };
            self.squares[rt as usize] = self.squares[rf as usize];
            self.squares[rf as usize] = None
        }
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
        if piece.kind == PieceKind::Pawn && (m.to as i16 - m.from as i16).abs() == 16 {
            self.en_passant = Some(if piece.color == Color::White {
                m.from + 8
            } else {
                m.from - 8
            })
        }
        self.halfmove = if piece.kind == PieceKind::Pawn || capture {
            0
        } else {
            self.halfmove + 1
        };
        if self.side == Color::Black {
            self.fullmove += 1
        }
        self.side = self.side.other();
        self.rebuild_bitboards();
        self.hash = self.compute_hash();
        undo
    }

    pub fn unmake_move(&mut self, m: Move, undo: Undo) {
        self.side = self.side.other();
        self.squares[m.from as usize] = Some(undo.moved);
        self.squares[m.to as usize] = None;
        if m.castle {
            let (rf, rt) = if m.to > m.from {
                (m.from + 3, m.from + 1)
            } else {
                (m.from - 4, m.from - 1)
            };
            self.squares[rf as usize] = self.squares[rt as usize];
            self.squares[rt as usize] = None;
        }
        if let Some((square, piece)) = undo.captured {
            self.squares[square as usize] = Some(piece);
        }
        self.castling = undo.castling;
        self.en_passant = undo.en_passant;
        self.halfmove = undo.halfmove;
        self.fullmove = undo.fullmove;
        self.hash = undo.hash;
        self.rebuild_bitboards();
    }

    pub fn in_check(&self, color: Color) -> bool {
        self.squares
            .iter()
            .position(|p| {
                *p == Some(Piece {
                    color,
                    kind: PieceKind::King,
                })
            })
            .is_some_and(|sq| self.square_attacked(sq as u8, color.other()))
    }
    pub fn square_attacked(&self, sq: u8, by: Color) -> bool {
        let f = (sq % 8) as i8;
        let r = (sq / 8) as i8;
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
}

impl FromStr for Board {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_fen(s)
    }
}
fn offset(f: i8, r: i8, df: i8, dr: i8) -> Option<u8> {
    let (nf, nr) = (f + df, r + dr);
    ((0..8).contains(&nf) && (0..8).contains(&nr)).then_some((nr * 8 + nf) as u8)
}
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
pub fn parse_square(s: &str) -> Option<u8> {
    let b = s.as_bytes();
    if b.len() != 2 || !(b'a'..=b'h').contains(&b[0]) || !(b'1'..=b'8').contains(&b[1]) {
        None
    } else {
        Some((b[1] - b'1') * 8 + b[0] - b'a')
    }
}
pub fn square_name(s: u8) -> String {
    format!("{}{}", (b'a' + s % 8) as char, (b'1' + s / 8) as char)
}
fn mix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e3779b97f4a7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

const fn color_index(color: Color) -> usize {
    match color {
        Color::White => 0,
        Color::Black => 1,
    }
}

const fn piece_index(piece: Piece) -> usize {
    color_index(piece.color) * 6 + piece.kind as usize - 1
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fen_roundtrip() {
        let b = Board::startpos();
        assert_eq!(
            b.to_fen(),
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"
        )
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
}
