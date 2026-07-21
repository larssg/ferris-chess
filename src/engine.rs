//! The search: iterative-deepening alpha-beta (negamax with principal-variation
//! search) plus the transposition table backing it.
//!
//! Scores are in centipawns from the side-to-move's point of view (negamax
//! convention: a node negates its child's score). A checkmate is scored near
//! `MATE`, offset by the ply at which it is delivered so that shorter mates
//! score higher. `INF` sits above every real score and is used as the initial
//! alpha/beta bound.

use crate::eval::piece_value;
use crate::eval::{PawnCache, evaluate_cached};
use crate::{Board, Move, PieceKind, static_exchange_eval};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
const INF: i32 = 100_000;
/// Scores at or above `MATE - 64` denote a forced mate; the 64-ply band leaves
/// room to encode "mate in N" without colliding with ordinary evaluations.
const MATE: i32 = 90_000;
/// Reverse futility pruning applies only in shallow nodes...
const RFP_MAX_DEPTH: u8 = 6;
/// ...and requires the static eval to clear beta by this much per remaining ply.
const RFP_MARGIN: i32 = 80;
/// Razoring replaces a shallow search with quiescence when the static score is
/// already well below alpha. The margin grows slightly with remaining depth.
const RAZOR_MAX_DEPTH: u8 = 3;
const RAZOR_BASE_MARGIN: i32 = 300;
const RAZOR_DEPTH_MARGIN: i32 = 60;
/// Shallow quiet-move pruning margins, indexed by remaining depth.
const FUTILITY_MARGIN: [i32; 3] = [0, 200, 500];
/// Number of legal moves searched before late quiet moves may be discarded,
/// indexed by remaining depth.
const LMP_THRESHOLD: [usize; 6] = [0, 4, 6, 10, 15, 21];
const LMR_FULL_MOVES: usize = 4;
const LMR_MIN_DEPTH: u8 = 3;
const HISTORY_MAX: i32 = 16_384;

/// Reference logarithmic LMR formula. Inputs are clamped because reductions
/// beyond the search's 64-ply horizon are never useful.
fn lmr_reduction(depth: u8, move_count: usize, pv: bool) -> u8 {
    let depth = depth.min(63);
    let move_count = move_count.min(63);
    let base = (0.75 + f64::from(depth).ln() * (move_count as f64).ln() / 2.25) as i32;
    (base - i32::from(pv)).max(1).min(i32::from(depth) - 1) as u8
}

/// Decides whether iterative deepening should start another depth. A changed
/// best move or a meaningful score drop gets 50% more thinking time, but the
/// target remains below the hard deadline so recursion has room to finish.
#[derive(Clone, Copy)]
struct Iteration {
    best: Option<Move>,
    score: i32,
}

fn should_stop_iterating(
    depth: u8,
    elapsed: Duration,
    base_soft: Duration,
    hard: Duration,
    current: Iteration,
    previous: Iteration,
) -> bool {
    let unstable =
        depth > 1 && (current.best != previous.best || previous.score - current.score >= 30);
    let effective = if unstable {
        base_soft + base_soft / 2
    } else {
        base_soft
    }
    .min(hard * 4 / 5);
    elapsed > effective
}
/// The stopping conditions for a search, mirroring the UCI `go` parameters. A
/// search ends when any active limit is reached; the clock fields feed the time
/// budget computed by the UCI adapter, and `root_moves` restricts the search to
/// a `searchmoves` subset.
#[derive(Clone, Debug)]
pub struct SearchLimits {
    pub depth: u8,
    /// Absolute deadline. Search recursion never runs beyond this budget.
    pub time: Duration,
    /// Target for starting another iteration. Instability may extend it.
    pub soft_time: Option<Duration>,
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
            soft_time: None,
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
/// Whether a reported score is an ordinary centipawn value or a distance to
/// mate, which the UCI layer prints differently (`score cp` vs `score mate`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScoreType {
    Centipawn,
    Mate,
}

/// Why a search stopped — surfaced for diagnostics and UCI reporting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminationReason {
    Depth,
    Time,
    Nodes,
    Mate,
    NoLegalMoves,
    Cancelled,
}

/// A cloneable, thread-safe handle for controlling a running search from
/// another thread. The UCI worker thread owns the search; the main thread holds
/// a clone to deliver `stop` and `ponderhit`. Both flags are shared via `Arc`
/// so either side sees the other's writes.
#[derive(Clone, Debug)]
pub struct SearchHandle {
    stop: Arc<AtomicBool>,
    pondering: Arc<AtomicBool>,
}

impl SearchHandle {
    /// Creates a handle. `pondering` starts true for a `go ponder` search, which
    /// then searches without spending its clock until a `ponderhit` arrives.
    pub fn new(pondering: bool) -> Self {
        Self {
            stop: Arc::new(AtomicBool::new(false)),
            pondering: Arc::new(AtomicBool::new(pondering)),
        }
    }

    /// Requests that the search abort as soon as it next checks in.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// Signals that the pondered move was played: the search converts to a
    /// normal timed search from this moment.
    pub fn ponderhit(&self) {
        self.pondering.store(false, Ordering::Relaxed);
    }

    pub fn is_stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
}

/// The outcome of a search, carrying both the chosen move and the statistics
/// UCI reports in `info` lines (depth, node counts, PV, and so on).
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
// What a stored score represents relative to the window it was searched in.
// EXACT is a true value; LOWER/UPPER are beta/alpha cutoffs, usable only when
// the current window makes the bound conclusive.
const BOUND_EXACT: u8 = 0;
const BOUND_LOWER: u8 = 1;
const BOUND_UPPER: u8 = 2;

/// One packed transposition-table record. Kept to 16 bytes so a four-slot
/// cluster is exactly one 64-byte cache line. `depth` doubles as an occupancy
/// flag: a stored entry always has `depth >= 1`, so `depth == 0` means empty.
/// `flags` packs the generation (high 6 bits) and the bound (low 2 bits).
#[derive(Clone, Copy, Default)]
#[repr(C)]
struct Slot {
    key: u64,
    score: i16,
    best: u16,
    static_eval: i16,
    depth: u8,
    flags: u8,
}

/// A cache-line-aligned bucket of four slots that share a table index; probing
/// and replacement operate within the cluster. See the `tt_cluster_is_one_cache_line`
/// test for the size/alignment guarantees this relies on.
#[derive(Clone, Copy, Default)]
#[repr(C, align(64))]
struct Cluster {
    slots: [Slot; 4],
}

/// A decoded transposition-table hit, with mate scores already adjusted back to
/// the current ply and depth restored to its true value.
#[derive(Clone, Copy, Debug)]
struct Entry {
    depth: u8,
    score: i32,
    bound: u8,
    best: Move,
    static_eval: Option<i16>,
}

/// Per-node context threaded through the recursion: whether this is a
/// principal-variation node (searched with a full window) and whether a null
/// move is still permitted here (it is disabled right after playing one).
#[derive(Clone, Copy)]
struct NodeState {
    pv: bool,
    allow_null: bool,
}

/// A fixed-size, cluster-bucketed transposition table. `generation` is bumped
/// each search so that stale entries can be preferred for replacement.
struct TranspositionTable {
    clusters: Vec<Cluster>,
    generation: u8,
}

impl TranspositionTable {
    /// Allocates a table sized to roughly `mb` megabytes, rounded down to a
    /// whole number of clusters (at least one).
    fn with_mb(mb: usize) -> Self {
        let bytes = mb.max(1).saturating_mul(1024 * 1024);
        let count = (bytes / size_of::<Cluster>()).max(1);
        Self {
            clusters: vec![Cluster::default(); count],
            generation: 0,
        }
    }

    /// Wipes the table (used by `ucinewgame`).
    fn clear(&mut self) {
        self.clusters.fill(Cluster::default());
        self.generation = 0;
    }

    /// Advances the generation counter at the start of each search, wrapping
    /// within the 6 bits it shares with the bound in `flags`. Entries from older
    /// generations become cheaper to evict.
    fn new_search(&mut self) {
        self.generation = self.generation.wrapping_add(1) & 0x3f;
    }

    /// Probes the cluster for `key`. The stored score is un-normalized from its
    /// ply-independent form back to the caller's `ply`, and `static_eval`'s
    /// `i16::MIN` sentinel is decoded to `None` (meaning "not cached").
    ///
    /// `depth` is stored biased by one (so 0 can mean empty) and decoded here.
    fn get(&self, key: u64, ply: u8) -> Option<Entry> {
        self.clusters[key as usize % self.clusters.len()]
            .slots
            .iter()
            .find(|slot| slot.key == key && slot.depth != 0)
            .map(|slot| Entry {
                depth: slot.depth - 1,
                score: score_from_tt(unpack_tt_score(slot.score), ply),
                bound: slot.flags & 3,
                best: unpack_move(slot.best),
                static_eval: (slot.static_eval != i16::MIN).then_some(slot.static_eval),
            })
    }

    /// Inserts or updates the record for `key`.
    ///
    /// Replacement first reuses a matching or empty slot; otherwise it evicts
    /// the slot with the lowest "worth", scoring depth minus an age penalty so
    /// that shallow and stale entries go first. A deep existing entry is kept
    /// over a much shallower non-exact one, but its missing static eval is still
    /// backfilled. Mate scores are normalized to be ply-independent on the way in.
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
        // Keep a substantially deeper existing entry; only salvage its static eval.
        if old.key == key && old.depth > entry.depth.saturating_add(4) && entry.bound != BOUND_EXACT
        {
            if old.static_eval == i16::MIN
                && let Some(static_eval) = entry.static_eval
            {
                slots[replace].static_eval = static_eval;
            }
            return;
        }
        // Preserve a previously cached static eval when this store lacks one.
        let static_eval = entry
            .static_eval
            .or_else(|| (old.key == key && old.static_eval != i16::MIN).then_some(old.static_eval));
        slots[replace] = Slot {
            key,
            score: pack_tt_score(score_to_tt(entry.score, ply)),
            best: pack_move(entry.best),
            static_eval: static_eval.unwrap_or(i16::MIN),
            depth: entry.depth.saturating_add(1),
            flags: generation << 2 | entry.bound,
        };
    }

    /// Estimates table fullness in permille (0..1000) for the UCI `hashfull`
    /// field, sampling the first 250 clusters and counting current-generation
    /// entries.
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
/// A reusable search engine. It owns the transposition table and the move-
/// ordering heuristics (history and killer tables) so they persist across the
/// `go` commands of a game; per-search bookkeeping (node count, clocks, stop
/// state) is reset at the start of every search.
pub struct Engine {
    tt: TranspositionTable,
    /// `history[from][to]`: a running bonus for quiet moves that caused cutoffs,
    /// used as a move-ordering tiebreak.
    history: [[i32; 64]; 64],
    /// Two killer moves per ply: quiet moves that recently produced a cutoff at
    /// the same depth and are tried early elsewhere.
    killers: [[Move; 2]; 64],
    /// The quiet reply that most recently refuted each preceding move.
    counter_moves: [[Move; 64]; 64],
    /// Moves on the current search line, used to find countermoves.
    move_stack: [Move; 64],
    /// One reusable legal-move buffer per ply. Recursive calls use distinct
    /// slots, so move generation does not allocate at each search node.
    move_buffers: [Vec<Move>; 64],
    /// Scratch space keyed by packed move, so sorting computes expensive SEE
    /// ordering scores once per move rather than once per comparison.
    order_scores: Box<[i32]>,
    /// Stack of position hashes along the current line, for repetition detection.
    hashes: Vec<u64>,
    /// Exact cache for the pawn-structure evaluation term, which depends only
    /// on the pawn bitboards and so survives across moves and searches.
    pawn_cache: PawnCache,
    nodes: u64,
    start: Instant,
    limit: Duration,
    node_limit: Option<u64>,
    stopped: bool,
    seldepth: u8,
    termination: TerminationReason,
    control: SearchHandle,
    pondering: bool,
    /// False during `infinite`/ponder searches, which ignore the clock.
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
            counter_moves: [[Move::default(); 64]; 64],
            move_stack: [Move::default(); 64],
            move_buffers: std::array::from_fn(|_| Vec::with_capacity(48)),
            order_scores: vec![0; 1 << 15].into_boxed_slice(),
            hashes: Vec::new(),
            pawn_cache: PawnCache::new(),
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
    /// Resets game-spanning state for a fresh game (`ucinewgame`).
    pub fn clear(&mut self) {
        self.tt.clear();
        self.hashes.clear()
    }
    /// Starts a new real-game history, normally for a UCI `position` command.
    pub fn reset_game_history(&mut self) {
        self.hashes.clear();
    }
    /// Records the position immediately before a real game move is played.
    pub fn append_game_hash(&mut self, hash: u64) {
        self.hashes.push(hash);
    }
    /// Reports whether the current position has appeared twice previously.
    pub fn is_threefold_repetition(&self, b: &Board) -> bool {
        self.repetition_count(b) >= 2
    }

    fn repetition_count(&self, b: &Board) -> usize {
        self.hashes
            .iter()
            .rev()
            .take(b.halfmove as usize)
            .skip(1)
            .step_by(2)
            .filter(|hash| **hash == b.hash)
            .count()
    }
    /// Reallocates the transposition table to the given size (`setoption Hash`).
    pub fn set_hash_mb(&mut self, mb: usize) {
        self.tt = TranspositionTable::with_mb(mb);
    }
    /// Searches with a freshly created control handle. Convenient for callers
    /// (tests, benchmarks) that do not need external cancellation.
    pub fn search(
        &mut self,
        b: &Board,
        limits: SearchLimits,
        info: impl FnMut(&SearchResult),
    ) -> SearchResult {
        let control = SearchHandle::new(limits.ponder);
        self.search_with_handle(b, limits, control, info)
    }

    /// The main entry point: iterative deepening from depth 1 up to the limit,
    /// invoking `info` after each completed depth. `control` allows another
    /// thread to `stop` the search or convert a ponder into a real search.
    ///
    /// Returns the best move found so far even when interrupted, so a `stop`
    /// always yields a legal reply.
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
        let base_soft = limits.soft_time.unwrap_or(limits.time / 2);
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
        // Compute the legal root moves once, honoring any `searchmoves` filter.
        let mut legal = b.legal_moves();
        if !limits.root_moves.is_empty() {
            legal.retain(|chess_move| limits.root_moves.contains(chess_move));
        }
        if legal.is_empty() {
            result.termination = TerminationReason::NoLegalMoves;
            return result;
        }
        // Seed a fallback so an immediate stop still returns a legal move.
        result.best_move = Some(legal[0]);
        let mut board = b.clone();
        let mut previous_best = None;
        let mut previous_score = 0;
        for depth in 1..=limits.depth {
            // Aspiration windows: after the first depth, search a narrow window
            // centered on the previous score, widening on a fail high/low. A
            // correct guess makes the search much cheaper than a full window.
            let (mut alpha, mut beta) = if depth == 1 {
                (-INF, INF)
            } else {
                (result.score - 50, result.score + 50)
            };
            let mut delta = 50;
            loop {
                let (score, best) = self.root(&mut board, depth, alpha, beta, &legal);
                if self.stopped {
                    result.termination = self.termination;
                    return result;
                }
                // Fail low: widen alpha once to ±200, then open it fully.
                if score <= alpha {
                    if delta >= 200 {
                        alpha = -INF;
                        beta = INF;
                    } else {
                        delta = 200;
                        alpha = result.score - delta;
                    }
                    continue;
                }
                // Fail high: symmetric widening of beta.
                if score >= beta {
                    if delta >= 200 {
                        alpha = -INF;
                        beta = INF;
                    } else {
                        delta = 200;
                        beta = result.score + delta;
                    }
                    continue;
                }
                // Score landed inside the window: accept this depth's result.
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
            let stop_for_time = self.time_enabled
                && !self.pondering
                && should_stop_iterating(
                    depth,
                    self.start.elapsed(),
                    base_soft,
                    self.limit,
                    Iteration {
                        best: result.best_move,
                        score: result.score,
                    },
                    Iteration {
                        best: previous_best,
                        score: previous_score,
                    },
                );
            previous_best = result.best_move;
            previous_score = result.score;
            // Stop early once a mate is proven, or once the adaptive soft
            // budget says there is not enough time for another full iteration.
            if result.score.abs() > MATE - 64 || stop_for_time {
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
    /// Searches the root move list at one depth using principal-variation
    /// search: the first (best-ordered) move gets a full window, and the rest a
    /// null-window scout that is re-searched fully only if it unexpectedly
    /// raises alpha. Returns the best score and move.
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
        for (index, m) in moves.into_iter().enumerate() {
            self.hashes.push(b.hash);
            self.move_stack[0] = m;
            let undo = b.make_move(m);
            let mut score = if index == 0 {
                // Full-window search of the presumed best move.
                -self.negamax(
                    b,
                    depth - 1,
                    -beta,
                    -alpha,
                    1,
                    NodeState {
                        pv: true,
                        allow_null: true,
                    },
                )
            } else {
                // Scout the remaining moves with a null window around alpha.
                -self.negamax(
                    b,
                    depth - 1,
                    -alpha - 1,
                    -alpha,
                    1,
                    NodeState {
                        pv: false,
                        allow_null: true,
                    },
                )
            };
            // A scout that beat alpha (but not beta) needs a full re-search to
            // get its true score.
            if index > 0 && score > alpha && score < beta {
                score = -self.negamax(
                    b,
                    depth - 1,
                    -beta,
                    -alpha,
                    1,
                    NodeState {
                        pv: true,
                        allow_null: true,
                    },
                );
            }
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
        // Record the root result so the next iteration and the PV extraction can
        // find the best move immediately.
        if let Some(best_move) = best {
            self.tt.store(
                b.hash,
                Entry {
                    depth,
                    score: alpha,
                    bound: BOUND_EXACT,
                    best: best_move,
                    static_eval: None,
                },
                0,
            );
        }
        (alpha, best)
    }
    /// The recursive negamax search with alpha-beta pruning. Returns the score
    /// of the position from the side-to-move's perspective. `ply` is the
    /// distance from the root (used for mate scoring and killer/repetition
    /// indexing); `depth` is the remaining search depth.
    ///
    /// The body is a sequence of gates applied before the move loop — stop
    /// checks, draw detection, transposition-table cutoff, reverse futility
    /// pruning, null-move pruning — followed by the move loop itself with
    /// late-move reductions and PVS re-searches.
    fn negamax(
        &mut self,
        b: &mut Board,
        depth: u8,
        mut alpha: i32,
        beta: i32,
        ply: u8,
        node: NodeState,
    ) -> i32 {
        self.nodes += 1;
        self.seldepth = self.seldepth.max(ply);
        // Honor an external stop request as soon as it is seen.
        if self.control.is_stopped() {
            self.stopped = true;
            self.termination = TerminationReason::Cancelled;
            return 0;
        }
        // A ponderhit converts this into a timed search; restart the clock now.
        if self.pondering && !self.control.pondering.load(Ordering::Relaxed) {
            self.pondering = false;
            self.start = Instant::now();
        }
        // Hard ceiling on recursion depth; just evaluate statically.
        if ply >= 63 {
            return evaluate_cached(b, &mut self.pawn_cache);
        }
        // Check the clock and node budget periodically rather than every node.
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
        // Draw by rule or by repetition. Only positions since the last
        // irreversible move (bounded by the halfmove clock) can repeat, and only
        // same-side-to-move positions (every second entry) are candidates.
        if b.is_draw() || self.repetition_count(b) > 0 {
            return 0;
        }
        // At the horizon, resolve captures via quiescence search.
        if depth == 0 {
            return self.qsearch(b, alpha, beta, ply);
        }
        // Check extension: search one ply deeper when in check, since forcing
        // lines resolve slowly and are cheap (few legal replies).
        let in_check = b.in_check(b.side);
        let mut depth = depth.saturating_add(in_check as u8);
        let original = alpha;
        // Transposition-table cutoff: a stored result searched at least as deep
        // as needed can end this node outright when its bound is conclusive.
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
        // Static evaluation, reused from the TT when available. Not meaningful
        // in check (the side to move may be losing the king next).
        let static_eval = if in_check {
            None
        } else {
            tt.and_then(|entry| entry.static_eval)
                .or_else(|| i16::try_from(evaluate_cached(b, &mut self.pawn_cache)).ok())
        };
        // Reverse futility pruning: in a shallow non-PV node, if the static eval
        // already exceeds beta by a depth-scaled margin, assume the opponent
        // cannot claw it all back and cut off.
        if !node.pv && !in_check && depth <= RFP_MAX_DEPTH && beta.abs() < MATE - 64 {
            let margin = RFP_MARGIN * i32::from(depth);
            if i32::from(static_eval.unwrap()) - margin >= beta {
                return i32::from(static_eval.unwrap()) - margin;
            }
        }
        // Razoring: when a shallow non-PV position is far below alpha, ask
        // quiescence whether forcing captures can rescue it. If not, avoid the
        // full-width search.
        if !node.pv && !in_check && depth <= RAZOR_MAX_DEPTH {
            let margin = RAZOR_BASE_MARGIN + RAZOR_DEPTH_MARGIN * i32::from(depth);
            if i32::from(static_eval.unwrap()) + margin <= alpha {
                let score = self.qsearch(b, alpha, beta, ply);
                if score <= alpha {
                    return score;
                }
            }
        }
        // Null-move pruning: let the opponent move twice in a row (via a reduced
        // search). If we still hold above beta, the position is so good that the
        // real move will too. Skipped without non-pawn material, where zugzwang
        // makes "passing" misleading.
        if node.allow_null
            && !node.pv
            && !in_check
            && depth >= 3
            && has_non_pawn_material(b, b.side)
        {
            // Reduce the null search more when there is more depth to spare.
            let reduction = if depth > 10 {
                4
            } else if depth > 6 {
                3
            } else {
                2
            };
            self.hashes.push(b.hash);
            self.move_stack[ply as usize] = Move::default();
            let undo = b.make_null_move();
            let mut score = -self.negamax(
                b,
                depth - 1 - reduction,
                -beta,
                -beta + 1,
                ply + 1,
                NodeState {
                    pv: false,
                    allow_null: false,
                },
            );
            b.unmake_null_move(undo);
            self.hashes.pop();
            if self.stopped {
                return 0;
            }
            if score >= beta {
                // Don't return an unproven mate score from a null search.
                if score >= MATE - 64 {
                    score = beta;
                }
                return score;
            }
        }
        // Internal iterative reduction: a deep PV node without a TT move has
        // poor ordering information, so spend one less ply instead of paying
        // for a separate internal iterative-deepening search.
        let missing_tt_move = tt.is_none_or(|entry| entry.best == Move::default());
        if node.pv && depth >= 4 && missing_tt_move {
            depth -= 1;
        }
        // No legal moves: checkmate (scored by ply, so faster mates win) or
        // stalemate (a draw).
        let buffer = &mut self.move_buffers[ply as usize];
        b.generate_legal_moves(buffer);
        let mut moves = std::mem::take(buffer);
        if moves.is_empty() {
            self.move_buffers[ply as usize] = moves;
            return if in_check { -MATE + ply as i32 } else { 0 };
        }
        // Order the moves, then float the transposition-table best move to the
        // very front — it is the single strongest ordering hint available.
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
        let can_futility_prune = !in_check && !node.pv && depth <= 2;
        let can_lmp = !in_check && !node.pv && depth <= 5;
        let mut quiet_moves = [Move::default(); 256];
        let mut quiet_count = 0;
        for (i, m) in moves.iter().copied().enumerate() {
            let capture = b.piece_at(m.to).is_some() || m.en_passant;
            let quiet = !capture && m.promotion.is_none();
            let legal_moves = i + 1;
            // Futility pruning: after one move has established a valid score,
            // skip shallow quiets whose optimistic static bound cannot raise
            // alpha.
            if can_futility_prune
                && legal_moves > 1
                && quiet
                && i32::from(static_eval.unwrap()) + FUTILITY_MARGIN[depth as usize] <= alpha
            {
                continue;
            }
            // Late-move pruning: at low-depth non-PV nodes, sufficiently late
            // quiet moves are unlikely to refute the current bound.
            if can_lmp && legal_moves > LMP_THRESHOLD[depth as usize] && quiet {
                continue;
            }
            if quiet {
                quiet_moves[quiet_count] = m;
                quiet_count += 1;
            }
            self.hashes.push(b.hash);
            self.move_stack[ply as usize] = m;
            let undo = b.make_move(m);
            // Logarithmic late-move reduction: search late quiets at reduced
            // depth with a null window. A result above alpha is verified at
            // full depth before a PV node widens to its full window.
            let reduction =
                if legal_moves > LMR_FULL_MOVES && depth >= LMR_MIN_DEPTH && quiet && !in_check {
                    lmr_reduction(depth, legal_moves, node.pv)
                } else {
                    0
                };
            let mut score;
            if reduction > 0 {
                score = -self.negamax(
                    b,
                    depth - 1 - reduction,
                    -alpha - 1,
                    -alpha,
                    ply + 1,
                    NodeState {
                        pv: false,
                        allow_null: true,
                    },
                );
                if score > alpha {
                    score = -self.negamax(
                        b,
                        depth - 1,
                        -alpha - 1,
                        -alpha,
                        ply + 1,
                        NodeState {
                            pv: false,
                            allow_null: true,
                        },
                    );
                    if score > alpha && node.pv {
                        score = -self.negamax(
                            b,
                            depth - 1,
                            -beta,
                            -alpha,
                            ply + 1,
                            NodeState {
                                pv: true,
                                allow_null: true,
                            },
                        );
                    }
                }
            } else if !node.pv || i > 0 {
                score = -self.negamax(
                    b,
                    depth - 1,
                    -alpha - 1,
                    -alpha,
                    ply + 1,
                    NodeState {
                        pv: false,
                        allow_null: true,
                    },
                );
                if score > alpha && score < beta {
                    score = -self.negamax(
                        b,
                        depth - 1,
                        -beta,
                        -alpha,
                        ply + 1,
                        NodeState {
                            pv: node.pv,
                            allow_null: true,
                        },
                    );
                }
            } else {
                score = -self.negamax(
                    b,
                    depth - 1,
                    -beta,
                    -alpha,
                    ply + 1,
                    NodeState {
                        pv: node.pv,
                        allow_null: true,
                    },
                );
            }
            b.unmake_move(m, undo);
            self.hashes.pop();
            if self.stopped {
                break;
            }
            if score > best_score {
                best_score = score;
                best = m
            }
            if score > alpha {
                alpha = score
            }
            // Beta cutoff: this move is too good, the opponent will avoid it.
            // Reward the quiet move that caused it (history and killers) so it is
            // tried earlier in sibling nodes.
            if alpha >= beta {
                if quiet && (ply as usize) < 64 {
                    if self.killers[ply as usize][0] != m {
                        self.killers[ply as usize][1] = self.killers[ply as usize][0];
                        self.killers[ply as usize][0] = m;
                    }
                    let bonus = i32::from(depth).pow(2);
                    self.update_history(m, bonus);
                    for failed in &quiet_moves[..quiet_count - 1] {
                        self.update_history(*failed, -bonus);
                    }
                    if ply > 0 {
                        let previous = self.move_stack[ply as usize - 1];
                        if previous.from != previous.to {
                            self.counter_moves[previous.from as usize][previous.to as usize] = m;
                        }
                    }
                }
                break;
            }
        }
        self.move_buffers[ply as usize] = moves;
        if self.stopped {
            return 0;
        }
        // Classify the result relative to the original window so the stored
        // bound is interpreted correctly on a later probe.
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
                static_eval,
            },
            ply,
        );
        best_score
    }
    /// Quiescence search: extends the search past the horizon along forcing
    /// moves (captures, and all evasions when in check) so that the static
    /// evaluation is only trusted in "quiet" positions. This is what prevents
    /// the horizon effect of stopping in the middle of a capture sequence.
    fn qsearch(&mut self, b: &mut Board, mut alpha: i32, beta: i32, ply: u8) -> i32 {
        self.nodes += 1;
        self.seldepth = self.seldepth.max(ply);
        if self.control.is_stopped() {
            self.stopped = true;
            self.termination = TerminationReason::Cancelled;
            return 0;
        }
        if ply >= 63 {
            return evaluate_cached(b, &mut self.pawn_cache);
        }
        let in_check = b.in_check(b.side);
        // "Stand pat": the score for making no capture at all. When not in check
        // it establishes a lower bound — the side to move can always decline.
        let stand = self
            .tt
            .get(b.hash, ply)
            .and_then(|entry| entry.static_eval)
            .map_or_else(|| evaluate_cached(b, &mut self.pawn_cache), i32::from);
        if !in_check {
            if stand >= beta {
                return stand;
            }
            if stand > alpha {
                alpha = stand
            }
        }
        // In check every legal move must be considered (it may be the only
        // escape); otherwise only captures are examined.
        let buffer = &mut self.move_buffers[ply as usize];
        if in_check {
            b.generate_legal_moves(buffer);
        } else {
            b.generate_legal_captures(buffer);
        }
        let moves = std::mem::take(buffer);
        if moves.is_empty() && in_check {
            self.move_buffers[ply as usize] = moves;
            return -MATE + ply as i32;
        }
        for m in moves.iter().copied() {
            if !in_check {
                // Skip captures that lose material by SEE...
                if m.promotion.is_none() && static_exchange_eval(b, m) < 0 {
                    continue;
                }
                // ...and delta-prune captures that cannot lift alpha even with a
                // generous margin.
                let gain = b.piece_at(m.to).map_or(100, |p| piece_value(p.kind));
                if stand + gain + 200 < alpha {
                    continue;
                }
            }
            let undo = b.make_move(m);
            let score = -self.qsearch(b, -beta, -alpha, ply + 1);
            b.unmake_move(m, undo);
            if score >= beta {
                self.move_buffers[ply as usize] = moves;
                return score;
            }
            if score > alpha {
                alpha = score
            }
        }
        self.move_buffers[ply as usize] = moves;
        alpha
    }
    /// Sorts moves best-first to maximize alpha-beta cutoffs. The ranking, from
    /// highest: winning/equal captures (by SEE), then losing captures, then
    /// promotions, then the two killer moves, and finally quiet moves by their
    /// history score.
    fn update_history(&mut self, chess_move: Move, bonus: i32) {
        let bonus = bonus.clamp(-HISTORY_MAX, HISTORY_MAX);
        let history = &mut self.history[chess_move.from as usize][chess_move.to as usize];
        *history += bonus - *history * bonus.abs() / HISTORY_MAX;
    }

    fn order(&mut self, b: &Board, moves: &mut [Move], ply: usize) {
        let counter_move = if ply > 0 && ply <= 64 {
            let previous = self.move_stack[ply - 1];
            self.counter_moves[previous.from as usize][previous.to as usize]
        } else {
            Move::default()
        };
        for &chess_move in moves.iter() {
            let score = self.move_order_score(b, chess_move, ply, counter_move);
            self.order_scores[usize::from(pack_move(chess_move))] = score;
        }
        moves.sort_unstable_by_key(|m| {
            std::cmp::Reverse(self.order_scores[usize::from(pack_move(*m))])
        })
    }

    fn move_order_score(&self, b: &Board, chess_move: Move, ply: usize, counter_move: Move) -> i32 {
        let capture = b.piece_at(chess_move.to).is_some() || chess_move.en_passant;
        let mut score = if capture {
            let see = static_exchange_eval(b, chess_move);
            if see >= 0 {
                100_000 + see
            } else {
                50_000 + see
            }
        } else {
            0
        };
        if let Some(promotion) = chess_move.promotion {
            score += 90_000 + piece_value(promotion);
        }
        if !capture && chess_move.promotion.is_none() && ply < 64 {
            score = if chess_move == self.killers[ply][0] {
                80_000
            } else if chess_move == self.killers[ply][1] {
                79_000
            } else if chess_move == counter_move && counter_move != Move::default() {
                78_000
            } else {
                score
            };
        }
        if score == 0 {
            score = self.history[chess_move.from as usize][chess_move.to as usize];
        }
        score
    }
    /// Reconstructs the principal variation by walking the transposition table:
    /// repeatedly follow each position's stored best move. Stops at the first
    /// position with no entry or no matching legal move (e.g. an overwritten slot).
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

/// Packs a move into 16 bits for compact TT storage: 6 bits `from`, 6 bits
/// `to`, 3 bits promotion. The en-passant and castle flags are not stored; they
/// are re-derived when the move is matched against the position's legal moves.
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

/// Whether `color` has any piece other than pawns and the king — the guard for
/// null-move pruning, which is unsound in pawn-only endgames prone to zugzwang.
fn has_non_pawn_material(board: &Board, color: crate::Color) -> bool {
    [
        PieceKind::Knight,
        PieceKind::Bishop,
        PieceKind::Rook,
        PieceKind::Queen,
    ]
    .into_iter()
    .any(|kind| board.pieces(color, kind) != 0)
}

// Mate scores encode the distance to mate, so they must be made relative to the
// current node before storing and back to the caller's ply on retrieval;
// otherwise a mate found deep in the tree would report the wrong distance when
// probed from a different ply. `score_to_tt`/`score_from_tt` handle that ply
// shift; `pack`/`unpack` additionally compress into the slot's i16.
fn score_to_tt(score: i32, ply: u8) -> i32 {
    if score >= MATE - 64 {
        score + i32::from(ply)
    } else if score <= -MATE + 64 {
        score - i32::from(ply)
    } else {
        score
    }
}

fn pack_tt_score(score: i32) -> i16 {
    if score >= MATE - 64 {
        (30_000 + score - (MATE - 64)).min(i32::from(i16::MAX)) as i16
    } else if score <= -MATE + 64 {
        (-30_000 + score + (MATE - 64)).max(i32::from(i16::MIN)) as i16
    } else {
        score.clamp(-29_999, 29_999) as i16
    }
}

fn unpack_tt_score(score: i16) -> i32 {
    let score = i32::from(score);
    if score >= 30_000 {
        MATE - 64 + score - 30_000
    } else if score <= -30_000 {
        -MATE + 64 + score + 30_000
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluate;

    #[test]
    fn repetition_uses_same_side_and_halfmove_window() {
        let mut board = Board::startpos();
        board.halfmove = 10;
        let mut engine = Engine::new();
        engine.hashes = vec![board.hash, 0xdead_beef];
        assert_eq!(engine.repetition_count(&board), 1);
        engine.hashes = vec![board.hash];
        assert_eq!(engine.repetition_count(&board), 0);
        board.halfmove = 1;
        engine.hashes = vec![board.hash, 0xdead_beef];
        assert_eq!(engine.repetition_count(&board), 0);
    }

    #[test]
    fn game_history_detects_threefold_after_two_knight_shuffles() {
        let mut board = Board::startpos();
        let mut engine = Engine::new();
        let shuffle = ["g1f3", "g8f6", "f3g1", "f6g8"];
        for turn in 0..2 {
            for notation in shuffle {
                let chess_move = board.parse_move(notation).unwrap();
                engine.append_game_hash(board.hash);
                board.make_move(chess_move);
            }
            assert_eq!(engine.is_threefold_repetition(&board), turn == 1);
        }
    }

    #[test]
    fn unstable_iteration_extends_the_soft_deadline() {
        let first = Move {
            from: 12,
            to: 28,
            ..Move::default()
        };
        let second = Move {
            from: 6,
            to: 21,
            ..Move::default()
        };
        let soft = Duration::from_millis(100);
        let hard = Duration::from_secs(1);
        assert!(should_stop_iterating(
            3,
            Duration::from_millis(120),
            soft,
            hard,
            Iteration {
                best: Some(first),
                score: 20
            },
            Iteration {
                best: Some(first),
                score: 20
            }
        ));
        assert!(!should_stop_iterating(
            3,
            Duration::from_millis(120),
            soft,
            hard,
            Iteration {
                best: Some(second),
                score: 20
            },
            Iteration {
                best: Some(first),
                score: 20
            }
        ));
        assert!(!should_stop_iterating(
            3,
            Duration::from_millis(120),
            soft,
            hard,
            Iteration {
                best: Some(first),
                score: 20
            },
            Iteration {
                best: Some(first),
                score: 70
            }
        ));
    }

    #[test]
    fn history_updates_are_bounded_and_accept_maluses() {
        let mut engine = Engine::new();
        let chess_move = Move {
            from: 12,
            to: 28,
            ..Move::default()
        };
        engine.update_history(chess_move, 100);
        assert_eq!(engine.history[12][28], 100);
        engine.update_history(chess_move, -100);
        assert!(engine.history[12][28] < 100);
        for _ in 0..1_000 {
            engine.update_history(chess_move, HISTORY_MAX);
        }
        assert_eq!(engine.history[12][28], HISTORY_MAX);
    }

    #[test]
    fn countermove_orders_a_quiet_move_before_history() {
        let board = Board::startpos();
        let previous = Move {
            from: 52,
            to: 36,
            ..Move::default()
        };
        let counter = board.parse_move("c2c4").unwrap();
        let history_move = board.parse_move("d2d4").unwrap();
        let mut engine = Engine::new();
        engine.move_stack[0] = previous;
        engine.counter_moves[previous.from as usize][previous.to as usize] = counter;
        engine.history[history_move.from as usize][history_move.to as usize] = HISTORY_MAX;
        let mut moves = [history_move, counter];
        engine.order(&board, &mut moves, 1);
        assert_eq!(moves[0], counter);
    }

    #[test]
    fn logarithmic_lmr_matches_reference_points() {
        assert_eq!(lmr_reduction(3, 5, false), 1);
        assert_eq!(lmr_reduction(10, 20, false), 3);
        assert_eq!(lmr_reduction(10, 20, true), 2);
        assert_eq!(lmr_reduction(63, 63, false), 8);
        assert_eq!(lmr_reduction(63, 63, true), 7);
    }
    use crate::Color;

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
                static_eval: Some(123),
            },
            7,
        );
        let entry = table.get(42, 3).unwrap();
        assert_eq!(entry.score, MATE - 3);
        assert_eq!(entry.best, chess_move);
        assert_eq!(entry.static_eval, Some(123));

        table.store(
            42,
            Entry {
                depth: 10,
                score: 17,
                bound: BOUND_LOWER,
                best: chess_move,
                static_eval: None,
            },
            0,
        );
        assert_eq!(table.get(42, 0).unwrap().static_eval, Some(123));
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
    fn fixed_depth_results_match_reference_fixtures() {
        for (fen, expected_score, expected_move) in [
            (
                "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
                0,
                "g1f3",
            ),
            (
                "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
                45,
                "e2a6",
            ),
            (
                "r2q1rk1/pp2bppp/2n1pn2/2pp4/3P4/2PBPN2/PPQ2PPP/R1B2RK1 w - - 2 10",
                34,
                "f3e5",
            ),
        ] {
            let board = Board::from_fen(fen).unwrap();
            let mut engine = Engine::new();
            let result = engine.search(
                &board,
                SearchLimits {
                    depth: 4,
                    infinite: true,
                    ..SearchLimits::default()
                },
                |_| {},
            );
            assert_eq!(result.score, expected_score, "{fen}");
            assert_eq!(
                result.best_move.map(|chess_move| chess_move.to_string()),
                Some(expected_move.into()),
                "{fen}"
            );
        }
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
