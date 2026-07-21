use crate::{Board, Color, Move, PieceKind};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
const INF: i32 = 100_000;
const MATE: i32 = 90_000;
#[derive(Clone, Copy, Debug)]
pub struct SearchLimits {
    pub depth: u8,
    pub time: Duration,
    pub nodes: Option<u64>,
}
impl Default for SearchLimits {
    fn default() -> Self {
        Self {
            depth: 64,
            time: Duration::from_secs(5),
            nodes: None,
        }
    }
}
#[derive(Clone, Debug)]
pub struct SearchResult {
    pub best_move: Option<Move>,
    pub score: i32,
    pub depth: u8,
    pub nodes: u64,
    pub elapsed: Duration,
    pub pv: Vec<Move>,
}
#[derive(Clone, Copy)]
struct Entry {
    depth: u8,
    score: i32,
    bound: u8,
    best: Move,
}
pub struct Engine {
    tt: HashMap<u64, Entry>,
    history: [[i32; 64]; 64],
    killers: [[Move; 2]; 64],
    hashes: Vec<u64>,
    nodes: u64,
    start: Instant,
    limit: Duration,
    node_limit: Option<u64>,
    stopped: bool,
}
impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}
impl Engine {
    pub fn new() -> Self {
        Self {
            tt: HashMap::with_capacity(1 << 20),
            history: [[0; 64]; 64],
            killers: [[Move::default(); 2]; 64],
            hashes: Vec::new(),
            nodes: 0,
            start: Instant::now(),
            limit: Duration::ZERO,
            node_limit: None,
            stopped: false,
        }
    }
    pub fn clear(&mut self) {
        self.tt.clear();
        self.hashes.clear()
    }
    pub fn set_hash_mb(&mut self, mb: usize) {
        // A HashMap entry has allocator/control-byte overhead; using 32 bytes
        // per entry keeps the requested UCI budget conservative.
        let entries = (mb.max(1) * 1024 * 1024 / 32).max(1024);
        self.tt = HashMap::with_capacity(entries);
    }
    pub fn search(
        &mut self,
        b: &Board,
        limits: SearchLimits,
        mut info: impl FnMut(&SearchResult),
    ) -> SearchResult {
        self.nodes = 0;
        self.start = Instant::now();
        self.limit = limits.time;
        self.node_limit = limits.nodes;
        self.stopped = false;
        let mut result = SearchResult {
            best_move: None,
            score: 0,
            depth: 0,
            nodes: 0,
            elapsed: Duration::ZERO,
            pv: vec![],
        };
        let legal = b.legal_moves();
        if legal.is_empty() {
            return result;
        }
        result.best_move = Some(legal[0]);
        for depth in 1..=limits.depth {
            let (mut alpha, mut beta) = if depth > 3 {
                (result.score - 40, result.score + 40)
            } else {
                (-INF, INF)
            };
            loop {
                let (score, best) = self.root(b, depth, alpha, beta);
                if self.stopped {
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
            result.pv = self.extract_pv(b, depth);
            info(&result);
            if result.score.abs() > MATE - 64 || self.start.elapsed() >= self.limit / 2 {
                break;
            }
        }
        result
    }
    fn root(&mut self, b: &Board, depth: u8, mut alpha: i32, beta: i32) -> (i32, Option<Move>) {
        let mut moves = b.legal_moves();
        self.order(b, &mut moves, 0);
        let mut best = None;
        for m in moves {
            let mut child = b.clone();
            child.make_move(m);
            self.hashes.push(b.hash);
            let score = -self.negamax(&child, depth - 1, -beta, -alpha, 1, true);
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
            self.tt.insert(
                b.hash,
                Entry {
                    depth,
                    score: alpha,
                    bound: 0,
                    best: best_move,
                },
            );
        }
        (alpha, best)
    }
    fn negamax(
        &mut self,
        b: &Board,
        depth: u8,
        mut alpha: i32,
        beta: i32,
        ply: u8,
        pv: bool,
    ) -> i32 {
        self.nodes += 1;
        if ply >= 63 {
            return evaluate(b);
        }
        if self.nodes & 2047 == 0
            && (self.start.elapsed() >= self.limit
                || self.node_limit.is_some_and(|n| self.nodes >= n))
        {
            self.stopped = true;
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
        let tt = self.tt.get(&b.hash).copied();
        if let Some(e) = tt {
            if e.depth >= depth {
                match e.bound {
                    0 => return e.score,
                    1 if e.score >= beta => return e.score,
                    2 if e.score <= alpha => return e.score,
                    _ => {}
                }
            }
        }
        let mut moves = b.legal_moves();
        if moves.is_empty() {
            return if in_check { -MATE + ply as i32 } else { 0 };
        }
        self.order(b, &mut moves, ply as usize);
        if let Some(e) = tt {
            if let Some(i) = moves.iter().position(|m| {
                m.from == e.best.from && m.to == e.best.to && m.promotion == e.best.promotion
            }) {
                moves.swap(0, i)
            }
        }
        let mut best = Move::default();
        let mut best_score = -INF;
        for (i, m) in moves.into_iter().enumerate() {
            let capture = b.squares[m.to as usize].is_some() || m.en_passant;
            let mut child = b.clone();
            child.make_move(m);
            self.hashes.push(b.hash);
            let reduction = if depth >= 3 && i >= 4 && !pv && !capture && !in_check {
                1
            } else {
                0
            };
            let mut score = -self.negamax(
                &child,
                depth - 1 - reduction,
                -alpha - 1,
                -alpha,
                ply + 1,
                false,
            );
            if score > alpha && (reduction > 0 || pv) {
                score = -self.negamax(&child, depth - 1, -beta, -alpha, ply + 1, pv)
            }
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
            2
        } else if best_score >= beta {
            1
        } else {
            0
        };
        self.tt.insert(
            b.hash,
            Entry {
                depth,
                score: best_score,
                bound,
                best,
            },
        );
        best_score
    }
    fn qsearch(&mut self, b: &Board, mut alpha: i32, beta: i32, ply: u8) -> i32 {
        self.nodes += 1;
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
                let gain = b.squares[m.to as usize].map_or(100, |p| value(p.kind));
                if stand + gain + 200 < alpha {
                    continue;
                }
            }
            let mut c = b.clone();
            c.make_move(m);
            let score = -self.qsearch(&c, -beta, -alpha, ply + 1);
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
            std::cmp::Reverse(if let Some(p) = b.squares[m.to as usize] {
                100_000 + 10 * value(p.kind) - value(b.squares[m.from as usize].unwrap().kind)
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
            let Some(e) = self.tt.get(&b.hash) else { break };
            let Some(m) = b.legal_moves().into_iter().find(|m| {
                m.from == e.best.from && m.to == e.best.to && m.promotion == e.best.promotion
            }) else {
                break;
            };
            pv.push(m);
            b.make_move(m)
        }
        pv
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
    for (sq, p) in b.squares.iter().enumerate() {
        let Some(p) = p else { continue };
        let idx = if p.color == Color::White { sq } else { sq ^ 56 };
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
}
