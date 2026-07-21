//! The state of one game in progress: the position, the clocks, the move
//! record, and the result. All rule questions are answered by the `chess`
//! library; this module only strings positions together over time.

use crate::engine_driver::EngineDriver;
use chess::clock::Instant;
use chess::{Board, Color, Move};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct TimeControl {
    pub base: Duration,
    pub increment: Duration,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GameStatus {
    Ongoing,
    /// The winner delivered mate.
    Checkmate(Color),
    Stalemate,
    DrawFiftyMove,
    DrawInsufficient,
    DrawRepetition,
    /// The winner's opponent ran out of time.
    Flagged(Color),
    /// The winner's opponent resigned.
    Resigned(Color),
}

impl GameStatus {
    pub fn is_over(self) -> bool {
        self != GameStatus::Ongoing
    }

    pub fn describe(self, human: Color) -> String {
        let winner_text = |color: Color| {
            if color == human {
                "You win"
            } else {
                "Engine wins"
            }
        };
        match self {
            GameStatus::Ongoing => String::new(),
            GameStatus::Checkmate(winner) => {
                format!("Checkmate — {}", winner_text(winner))
            }
            GameStatus::Stalemate => "Draw by stalemate".into(),
            GameStatus::DrawFiftyMove => "Draw by the fifty-move rule".into(),
            GameStatus::DrawInsufficient => "Draw by insufficient material".into(),
            GameStatus::DrawRepetition => "Draw by threefold repetition".into(),
            GameStatus::Flagged(winner) => {
                format!("Time forfeit — {}", winner_text(winner))
            }
            GameStatus::Resigned(winner) => {
                format!("Resignation — {}", winner_text(winner))
            }
        }
    }
}

pub struct GameState {
    pub board: Board,
    /// The side the human plays; Random is resolved before construction.
    pub human: Color,
    /// UCI move strings in order — the key the opening book looks up.
    pub moves_uci: Vec<String>,
    /// SAN move strings in order, rendered before each move was made.
    pub sans: Vec<String>,
    /// `board.hash` before each move, for threefold-repetition adjudication.
    pub hash_history: Vec<u64>,
    /// Banked remaining time, indexed by [`clock_index`].
    clocks: [Duration; 2],
    pub increment: Duration,
    /// When the side to move started thinking; `None` once the game is over.
    turn_started: Option<Instant>,
    pub status: GameStatus,
    pub use_book: bool,
    pub last_move: Option<Move>,
}

fn clock_index(color: Color) -> usize {
    match color {
        Color::White => 0,
        Color::Black => 1,
    }
}

impl GameState {
    pub fn new(human: Color, time_control: TimeControl, use_book: bool) -> Self {
        Self {
            board: Board::startpos(),
            human,
            moves_uci: Vec::new(),
            sans: Vec::new(),
            hash_history: Vec::new(),
            clocks: [time_control.base; 2],
            increment: time_control.increment,
            turn_started: Some(Instant::now()),
            status: GameStatus::Ongoing,
            use_book,
            last_move: None,
        }
    }

    /// Remaining time on `color`'s clock, live-ticking for the side to move.
    pub fn remaining(&self, color: Color) -> Duration {
        let banked = self.clocks[clock_index(color)];
        match self.turn_started {
            Some(started) if color == self.board.side => banked.saturating_sub(started.elapsed()),
            _ => banked,
        }
    }

    /// The single choke point through which every move — human or engine —
    /// enters the game. Keeps the SAN/UCI/hash records, the clocks, and the
    /// engine's repetition history in lockstep.
    pub fn play_move(&mut self, chess_move: Move, engine: &mut EngineDriver) {
        if self.status.is_over() {
            return;
        }
        let mover = clock_index(self.board.side);
        if let Some(started) = self.turn_started {
            self.clocks[mover] = self.clocks[mover].saturating_sub(started.elapsed());
        }
        self.clocks[mover] += self.increment;
        self.sans.push(self.board.san(chess_move));
        self.hash_history.push(self.board.hash);
        engine.append_hash(self.board.hash);
        self.moves_uci.push(chess_move.to_string());
        self.board.make_move(chess_move);
        self.last_move = Some(chess_move);
        self.status = self.derive_status();
        self.turn_started = (!self.status.is_over()).then(Instant::now);
    }

    /// Ends the game by flag fall if the side to move has run out of time.
    /// Returns true when the flag fell on this call.
    pub fn check_flag(&mut self) -> bool {
        if self.status.is_over() || self.remaining(self.board.side) > Duration::ZERO {
            return false;
        }
        self.status = GameStatus::Flagged(self.board.side.other());
        self.turn_started = None;
        true
    }

    pub fn resign(&mut self) {
        if !self.status.is_over() {
            self.status = GameStatus::Resigned(self.human.other());
            self.turn_started = None;
        }
    }

    fn derive_status(&self) -> GameStatus {
        if self.board.legal_moves().is_empty() {
            return if self.board.in_check(self.board.side) {
                GameStatus::Checkmate(self.board.side.other())
            } else {
                GameStatus::Stalemate
            };
        }
        // `Board::is_draw` bundles the fifty-move rule with the engine's
        // (deliberately basic) insufficient-material test; split them so the
        // result text can say which applied.
        if self.board.halfmove >= 100 {
            return GameStatus::DrawFiftyMove;
        }
        if self.board.is_draw() {
            return GameStatus::DrawInsufficient;
        }
        let repetitions = 1 + self
            .hash_history
            .iter()
            .filter(|hash| **hash == self.board.hash)
            .count();
        if repetitions >= 3 {
            return GameStatus::DrawRepetition;
        }
        GameStatus::Ongoing
    }
}
