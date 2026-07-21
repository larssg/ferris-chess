use crate::{Board, Color, Move, PieceKind};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
const INF: i32 = 100_000;
const MATE: i32 = 90_000;
#[derive(Clone, Debug)]
pub struct SearchLimits {
    pub depth: u8,
    pub time: Duration,
    pub nodes: Option<u64>,
    pub white_time: Option<Duration>,
    pub black_time: Option<Duration>,
    pub white_increment: Duration,
    pub black_increment: Duration,
    pub moves_to_go: Option<u32>,
    pub mate: Option<u8>,
    pub ponder: bool,
    pub infinite: bool,
    pub root_moves: Vec<Move>,
}
impl Default for SearchLimits {
    fn default() -> Self {
        Self {
            depth: 64,
            time: Duration::from_secs(5),
            nodes: None,
            white_time: None,
            black_time: None,
            white_increment: Duration::ZERO,
            black_increment: Duration::ZERO,
            moves_to_go: None,
            mate: None,
            ponder: false,
            infinite: false,
            root_moves: Vec::new(),
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScoreType {
    Centipawn,
    Mate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminationReason {
    Depth,
    Time,
    Nodes,
    Mate,
    NoLegalMoves,
    Cancelled,
}

#[derive(Clone, Debug)]
pub struct SearchHandle {
    stop: Arc<AtomicBool>,
    pondering: Arc<AtomicBool>,
}

impl SearchHandle {
    pub fn new(pondering: bool) -> Self {
        Self {
            stop: Arc::new(AtomicBool::new(false)),
            pondering: Arc::new(AtomicBool::new(pondering)),
        }
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn ponderhit(&self) {
        self.pondering.store(false, Ordering::Relaxed);
    }

    pub fn is_stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
}

#[derive(Clone, Debug)]
pub struct SearchResult {
    pub best_move: Option<Move>,
    pub score: i32,
    pub depth: u8,
    pub seldepth: u8,
    pub nodes: u64,
    pub nps: u64,
    pub hashfull: u16,
    pub elapsed: Duration,
    pub pv: Vec<Move>,
    pub score_type: ScoreType,
    pub termination: TerminationReason,
}
const BOUND_EXACT: u8 = 0;
const BOUND_LOWER: u8 = 1;
const BOUND_UPPER: u8 = 2;

#[derive(Clone, Copy, Default)]
#[repr(C)]
struct Slot {
    key: u64,
    score: i32,
    best: u16,
    depth: u8,
    flags: u8,
}

#[derive(Clone, Copy, Default)]
#[repr(C, align(64))]
struct Cluster {
    slots: [Slot; 4],
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    depth: u8,
    score: i32,
    bound: u8,
    best: Move,
}

struct TranspositionTable {
    clusters: Vec<Cluster>,
    generation: u8,
}

impl TranspositionTable {
    fn with_mb(mb: usize) -> Self {
        let bytes = mb.max(1).saturating_mul(1024 * 1024);
        let count = (bytes / size_of::<Cluster>()).max(1);
        Self {
            clusters: vec![Cluster::default(); count],
            generation: 0,
        }
    }

    fn clear(&mut self) {
        self.clusters.fill(Cluster::default());
        self.generation = 0;
    }

    fn new_search(&mut self) {
        self.generation = self.generation.wrapping_add(1) & 0x3f;
    }

    fn get(&self, key: u64, ply: u8) -> Option<Entry> {
        self.clusters[key as usize % self.clusters.len()]
            .slots
            .iter()
            .find(|slot| slot.key == key && slot.depth != 0)
            .map(|slot| Entry {
                depth: slot.depth - 1,
                score: score_from_tt(slot.score, ply),
                bound: slot.flags & 3,
                best: unpack_move(slot.best),
            })
    }

    fn store(&mut self, key: u64, entry: Entry, ply: u8) {
        let index = key as usize % self.clusters.len();
        let generation = self.generation;
        let slots = &mut self.clusters[index].slots;
        let replace = slots
            .iter()
            .position(|slot| slot.key == key || slot.depth == 0)
            .unwrap_or_else(|| {
                slots
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, slot)| {
                        let age = generation.wrapping_sub(slot.flags >> 2) & 0x3f;
                        i16::from(slot.depth) - i16::from(age) * 4
                    })
                    .map_or(0, |(index, _)| index)
            });
        let old = slots[replace];
        if old.key == key && old.depth > entry.depth.saturating_add(4) && entry.bound != BOUND_EXACT
        {
            return;
        }
        slots[replace] = Slot {
            key,
            score: score_to_tt(entry.score, ply),
            best: pack_move(entry.best),
            depth: entry.depth.saturating_add(1),
            flags: generation << 2 | entry.bound,
        };
    }

    fn hashfull(&self) -> u16 {
        let mut total = 0usize;
        let mut used = 0usize;
        for cluster in self.clusters.iter().take(250) {
            for slot in cluster.slots {
                total += 1;
                used += usize::from(slot.depth != 0 && slot.flags >> 2 == self.generation);
            }
        }
        (used * 1000).checked_div(total).unwrap_or(0) as u16
    }
}
pub struct Engine {
    tt: TranspositionTable,
    history: [[i32; 64]; 64],
    killers: [[Move; 2]; 64],
    hashes: Vec<u64>,
    nodes: u64,
    start: Instant,
    limit: Duration,
    node_limit: Option<u64>,
    stopped: bool,
    seldepth: u8,
    termination: TerminationReason,
    control: SearchHandle,
    pondering: bool,
    time_enabled: bool,
}
impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}
impl Engine {
    pub fn new() -> Self {
        Self {
            tt: TranspositionTable::with_mb(64),
            history: [[0; 64]; 64],
            killers: [[Move::default(); 2]; 64],
            hashes: Vec::new(),
            nodes: 0,
            start: Instant::now(),
            limit: Duration::ZERO,
            node_limit: None,
            stopped: false,
            seldepth: 0,
            termination: TerminationReason::Depth,
            control: SearchHandle::new(false),
            pondering: false,
            time_enabled: true,
        }
    }
    pub fn clear(&mut self) {
        self.tt.clear();
        self.hashes.clear()
    }
    pub fn set_hash_mb(&mut self, mb: usize) {
        self.tt = TranspositionTable::with_mb(mb);
    }
    pub fn search(
        &mut self,
        b: &Board,
        limits: SearchLimits,
        info: impl FnMut(&SearchResult),
    ) -> SearchResult {
        let control = SearchHandle::new(limits.ponder);
        self.search_with_handle(b, limits, control, info)
    }

    pub fn search_with_handle(
        &mut self,
        b: &Board,
        limits: SearchLimits,
        control: SearchHandle,
        mut info: impl FnMut(&SearchResult),
    ) -> SearchResult {
        self.nodes = 0;
        self.start = Instant::now();
        self.limit = limits.time;
        self.node_limit = limits.nodes;
        self.stopped = false;
        self.seldepth = 0;
        self.termination = TerminationReason::Depth;
        self.pondering = limits.ponder;
        self.time_enabled = !limits.infinite;
        self.control = control;
        self.tt.new_search();
        let mut result = SearchResult {
            best_move: None,
            score: 0,
            depth: 0,
            seldepth: 0,
            nodes: 0,
            nps: 0,
            hashfull: 0,
            elapsed: Duration::ZERO,
            pv: vec![],
            score_type: ScoreType::Centipawn,
            termination: TerminationReason::Depth,
        };
        let mut legal = b.legal_moves();
        if !limits.root_moves.is_empty() {
            legal.retain(|chess_move| limits.root_moves.contains(chess_move));
        }
        if legal.is_empty() {
            result.termination = TerminationReason::NoLegalMoves;
            return result;
        }
        result.best_move = Some(legal[0]);
        let mut board = b.clone();
        for depth in 1..=limits.depth {
            let (mut alpha, mut beta) = if depth > 3 {
                (result.score - 40, result.score + 40)
            } else {
                (-INF, INF)
            };
            loop {
                let (score, best) = self.root(&mut board, depth, alpha, beta, &legal);
                if self.stopped {
                    result.termination = self.termination;
                    return result;
                }
                if score <= alpha {
                    alpha = -INF;
                    continue;
                }
                if score >= beta {
                    beta = INF;
                    continue;
                }
                result.score = score;
                result.best_move = best;
                break;
            }
            result.depth = depth;
            result.nodes = self.nodes;
            result.elapsed = self.start.elapsed();
            result.seldepth = self.seldepth;
            result.nps = self.nodes * 1000 / (result.elapsed.as_millis() as u64).max(1);
            result.hashfull = self.tt.hashfull();
            result.pv = self.extract_pv(b, depth);
            result.score_type = if result.score.abs() >= MATE - 64 {
                ScoreType::Mate
            } else {
                ScoreType::Centipawn
            };
            info(&result);
            if result.score.abs() > MATE - 64
                || (self.time_enabled && !self.pondering && self.start.elapsed() >= self.limit / 2)
            {
                result.termination = if result.score.abs() > MATE - 64 {
                    TerminationReason::Mate
                } else {
                    TerminationReason::Time
                };
                break;
            }
        }
        result
    }
    fn root(
        &mut self,
        b: &mut Board,
        depth: u8,
        mut alpha: i32,
        beta: i32,
        root_moves: &[Move],
    ) -> (i32, Option<Move>) {
        let mut moves = b.legal_moves();
        moves.retain(|chess_move| root_moves.contains(chess_move));
        self.order(b, &mut moves, 0);
        let mut best = None;
        for m in moves {
            self.hashes.push(b.hash);
            let undo = b.make_move(m);
            let score = -self.negamax(b, depth - 1, -beta, -alpha, 1, true);
            b.unmake_move(m, undo);
            self.hashes.pop();
            if self.stopped {
                break;
            }
            if score > alpha {
                alpha = score;
                best = Some(m)
            }
        }
        if let Some(best_move) = best {
            self.tt.store(
                b.hash,
                Entry {
                    depth,
                    score: alpha,
                    bound: BOUND_EXACT,
                    best: best_move,
                },
                0,
            );
        }
        (alpha, best)
    }
    fn negamax(
        &mut self,
        b: &mut Board,
        depth: u8,
        mut alpha: i32,
        beta: i32,
        ply: u8,
        pv: bool,
    ) -> i32 {
        self.nodes += 1;
        self.seldepth = self.seldepth.max(ply);
        if self.control.is_stopped() {
            self.stopped = true;
            self.termination = TerminationReason::Cancelled;
            return 0;
        }
        if self.pondering && !self.control.pondering.load(Ordering::Relaxed) {
            self.pondering = false;
            self.start = Instant::now();
        }
        if ply >= 63 {
            return evaluate(b);
        }
        if self.nodes & 2047 == 0
            && ((self.time_enabled && !self.pondering && self.start.elapsed() >= self.limit)
                || self.node_limit.is_some_and(|n| self.nodes >= n))
        {
            self.stopped = true;
            self.termination = if self.node_limit.is_some_and(|n| self.nodes >= n) {
                TerminationReason::Nodes
            } else {
                TerminationReason::Time
            };
            return 0;
        }
        if b.is_draw()
            || self
                .hashes
                .iter()
                .rev()
                .take(b.halfmove as usize)
                .step_by(2)
                .any(|h| *h == b.hash)
        {
            return 0;
        }
        if depth == 0 {
            return self.qsearch(b, alpha, beta, ply);
        }
        let in_check = b.in_check(b.side);
        let depth = depth.saturating_add(in_check as u8);
        let original = alpha;
        let tt = self.tt.get(b.hash, ply);
        if let Some(e) = tt
            && e.depth >= depth
        {
            match e.bound {
                BOUND_EXACT => return e.score,
                BOUND_LOWER if e.score >= beta => return e.score,
                BOUND_UPPER if e.score <= alpha => return e.score,
                _ => {}
            }
        }
        let mut moves = b.legal_moves();
        if moves.is_empty() {
            return if in_check { -MATE + ply as i32 } else { 0 };
        }
        self.order(b, &mut moves, ply as usize);
        if let Some(e) = tt
            && let Some(i) = moves.iter().position(|m| {
                m.from == e.best.from && m.to == e.best.to && m.promotion == e.best.promotion
            })
        {
            moves.swap(0, i)
        }
        let mut best = Move::default();
        let mut best_score = -INF;
        for (i, m) in moves.into_iter().enumerate() {
            let capture = b.piece_at(m.to).is_some() || m.en_passant;
            self.hashes.push(b.hash);
            let undo = b.make_move(m);
            let reduction = if depth >= 3 && i >= 4 && !pv && !capture && !in_check {
                1
            } else {
                0
            };
            let mut score =
                -self.negamax(b, depth - 1 - reduction, -alpha - 1, -alpha, ply + 1, false);
            if score > alpha && (reduction > 0 || pv) {
                score = -self.negamax(b, depth - 1, -beta, -alpha, ply + 1, pv)
            }
            b.unmake_move(m, undo);
            self.hashes.pop();
            if self.stopped {
                return 0;
            }
            if score > best_score {
                best_score = score;
                best = m
            }
            if score > alpha {
                alpha = score
            }
            if alpha >= beta {
                if !capture && (ply as usize) < 64 {
                    self.history[m.from as usize][m.to as usize] += (depth as i32).pow(2);
                    self.killers[ply as usize][1] = self.killers[ply as usize][0];
                    self.killers[ply as usize][0] = m
                }
                break;
            }
        }
        let bound = if best_score <= original {
            BOUND_UPPER
        } else if best_score >= beta {
            BOUND_LOWER
        } else {
            BOUND_EXACT
        };
        self.tt.store(
            b.hash,
            Entry {
                depth,
                score: best_score,
                bound,
                best,
            },
            ply,
        );
        best_score
    }
    fn qsearch(&mut self, b: &mut Board, mut alpha: i32, beta: i32, ply: u8) -> i32 {
        self.nodes += 1;
        self.seldepth = self.seldepth.max(ply);
        if self.control.is_stopped() {
            self.stopped = true;
            self.termination = TerminationReason::Cancelled;
            return 0;
        }
        if ply >= 63 {
            return evaluate(b);
        }
        let in_check = b.in_check(b.side);
        let stand = evaluate(b);
        if !in_check {
            if stand >= beta {
                return stand;
            }
            if stand > alpha {
                alpha = stand
            }
        }
        let moves = if in_check {
            b.legal_moves()
        } else {
            b.legal_captures()
        };
        if moves.is_empty() && in_check {
            return -MATE + ply as i32;
        }
        for m in moves {
            if !in_check {
                let gain = b.piece_at(m.to).map_or(100, |p| value(p.kind));
                if stand + gain + 200 < alpha {
                    continue;
                }
            }
            let undo = b.make_move(m);
            let score = -self.qsearch(b, -beta, -alpha, ply + 1);
            b.unmake_move(m, undo);
            if score >= beta {
                return score;
            }
            if score > alpha {
                alpha = score
            }
        }
        alpha
    }
    fn order(&self, b: &Board, moves: &mut [Move], ply: usize) {
        moves.sort_unstable_by_key(|m| {
            std::cmp::Reverse(if let Some(p) = b.piece_at(m.to) {
                100_000 + 10 * value(p.kind) - value(b.piece_at(m.from).unwrap().kind)
            } else if m.promotion.is_some() {
                90_000
            } else if ply < 64 && *m == self.killers[ply][0] {
                80_000
            } else if ply < 64 && *m == self.killers[ply][1] {
                70_000
            } else {
                self.history[m.from as usize][m.to as usize]
            })
        })
    }
    fn extract_pv(&self, b: &Board, depth: u8) -> Vec<Move> {
        let mut b = b.clone();
        let mut pv = vec![];
        for _ in 0..depth {
            let Some(e) = self.tt.get(b.hash, pv.len() as u8) else {
                break;
            };
            let Some(m) = b.legal_moves().into_iter().find(|m| {
                m.from == e.best.from && m.to == e.best.to && m.promotion == e.best.promotion
            }) else {
                break;
            };
            pv.push(m);
            b.make_move(m);
        }
        pv
    }
}

fn pack_move(chess_move: Move) -> u16 {
    let promotion = match chess_move.promotion {
        None => 0,
        Some(PieceKind::Knight) => 1,
        Some(PieceKind::Bishop) => 2,
        Some(PieceKind::Rook) => 3,
        Some(PieceKind::Queen) => 4,
        Some(PieceKind::Pawn | PieceKind::King) => 0,
    };
    u16::from(chess_move.from) | u16::from(chess_move.to) << 6 | promotion << 12
}

fn unpack_move(packed: u16) -> Move {
    let promotion = match packed >> 12 & 7 {
        1 => Some(PieceKind::Knight),
        2 => Some(PieceKind::Bishop),
        3 => Some(PieceKind::Rook),
        4 => Some(PieceKind::Queen),
        _ => None,
    };
    Move {
        from: (packed & 63) as u8,
        to: (packed >> 6 & 63) as u8,
        promotion,
        ..Move::default()
    }
}

fn score_to_tt(score: i32, ply: u8) -> i32 {
    if score >= MATE - 64 {
        score + i32::from(ply)
    } else if score <= -MATE + 64 {
        score - i32::from(ply)
    } else {
        score
    }
}

fn score_from_tt(score: i32, ply: u8) -> i32 {
    if score >= MATE - 64 {
        score - i32::from(ply)
    } else if score <= -MATE + 64 {
        score + i32::from(ply)
    } else {
        score
    }
}

fn value(k: PieceKind) -> i32 {
    match k {
        PieceKind::Pawn => 100,
        PieceKind::Knight => 320,
        PieceKind::Bishop => 335,
        PieceKind::Rook => 500,
        PieceKind::Queen => 950,
        PieceKind::King => 20_000,
    }
}
const PST: [[i16; 64]; 6] = include!("pst.in");
pub fn evaluate(b: &Board) -> i32 {
    let mut score = 0;
    let mut bishops = [0; 2];
    for sq in 0..64 {
        let Some(p) = b.piece_at(sq) else { continue };
        let idx = if p.color == Color::White {
            sq as usize
        } else {
            (sq ^ 56) as usize
        };
        let v = value(p.kind) + PST[p.kind as usize - 1][idx] as i32;
        if p.color == Color::White {
            score += v
        } else {
            score -= v
        }
        if p.kind == PieceKind::Bishop {
            bishops[if p.color == Color::White { 0 } else { 1 }] += 1
        }
    }
    if bishops[0] >= 2 {
        score += 40
    }
    if bishops[1] >= 2 {
        score -= 40
    }
    if b.side == Color::White {
        score
    } else {
        -score
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_mate_in_one() {
        let board = Board::from_fen("7k/5Q2/6K1/8/8/8/8/8 w - - 0 1").unwrap();
        let mut engine = Engine::new();
        let result = engine.search(
            &board,
            SearchLimits {
                depth: 3,
                time: Duration::from_secs(1),
                nodes: None,
                ..SearchLimits::default()
            },
            |_| {},
        );
        let mut after = board.clone();
        after.make_move(result.best_move.unwrap());
        assert!(after.legal_moves().is_empty());
        assert!(after.in_check(Color::Black));
        assert!(result.score > MATE - 10);
    }

    #[test]
    fn evaluation_is_side_relative() {
        let white = Board::from_fen("7k/8/8/8/8/8/4Q3/K7 w - - 0 1").unwrap();
        let black = Board::from_fen("7k/8/8/8/8/8/4Q3/K7 b - - 0 1").unwrap();
        assert_eq!(evaluate(&white), -evaluate(&black));
        assert!(evaluate(&white) > 800);
    }

    #[test]
    fn tt_cluster_is_one_cache_line() {
        assert_eq!(size_of::<Slot>(), 16);
        assert_eq!(size_of::<Cluster>(), 64);
        assert_eq!(align_of::<Cluster>(), 64);
    }

    #[test]
    fn tt_preserves_moves_and_normalizes_mates() {
        let chess_move = Move {
            from: 48,
            to: 56,
            promotion: Some(PieceKind::Knight),
            ..Move::default()
        };
        assert_eq!(unpack_move(pack_move(chess_move)), chess_move);

        let mut table = TranspositionTable::with_mb(1);
        table.store(
            42,
            Entry {
                depth: 9,
                score: MATE - 7,
                bound: BOUND_EXACT,
                best: chess_move,
            },
            7,
        );
        let entry = table.get(42, 3).unwrap();
        assert_eq!(entry.score, MATE - 3);
        assert_eq!(entry.best, chess_move);
    }

    #[test]
    fn search_honors_root_move_restrictions() {
        let board = Board::startpos();
        let only_move = board.parse_move("a2a3").unwrap();
        let mut engine = Engine::new();
        let result = engine.search(
            &board,
            SearchLimits {
                depth: 2,
                time: Duration::from_secs(1),
                root_moves: vec![only_move],
                ..SearchLimits::default()
            },
            |_| {},
        );
        assert_eq!(result.best_move, Some(only_move));
        assert!(result.seldepth >= result.depth);
        assert!(result.nps > 0);
    }

    #[test]
    fn search_can_be_cancelled_from_another_thread() {
        let control = SearchHandle::new(false);
        let worker_control = control.clone();
        let worker = std::thread::spawn(move || {
            let mut engine = Engine::new();
            engine.search_with_handle(
                &Board::startpos(),
                SearchLimits {
                    infinite: true,
                    ..SearchLimits::default()
                },
                worker_control,
                |_| {},
            )
        });
        std::thread::sleep(Duration::from_millis(10));
        control.stop();
        let result = worker.join().unwrap();
        assert_eq!(result.termination, TerminationReason::Cancelled);
        assert!(result.best_move.is_some());
    }
}
