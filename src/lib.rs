//! FerrisChess: a dependency-free UCI chess engine.
//!
//! The crate is organized as four cooperating layers:
//!
//! - [`board`] owns the rules of chess: the position, legal move generation,
//!   make/unmake, FEN, and attack detection.
//! - [`eval`] turns a static position into a single centipawn score, and scores
//!   individual captures via static exchange evaluation (SEE).
//! - [`engine`] drives the alpha-beta search that repeatedly consults `board`
//!   and `eval` to choose a move, plus the transposition table backing it.
//! - [`benchmark`] provides the deterministic `bench` subcommands used to
//!   compare against the GoChess reference.
//!
//! `main.rs` is a thin UCI adapter on top of these; the layers here are a
//! reusable library so the same search can be embedded and driven in-process.

pub mod benchmark;
pub mod board;
pub mod engine;
pub mod eval;

pub use board::{Board, Color, Move, Piece, PieceKind, Undo};
pub use engine::{Engine, ScoreType, SearchHandle, SearchLimits, SearchResult, TerminationReason};
pub use eval::{evaluate, static_exchange_eval};
