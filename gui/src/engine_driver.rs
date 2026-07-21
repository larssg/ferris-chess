//! Asynchronous facade over the blocking engine search.
//!
//! The UI talks exclusively in [`EngineCmd`]/[`EngineEvent`] messages and a
//! per-frame [`EngineDriver::poll`], so this module is the whole
//! wasm-portability seam: the native implementation below runs one long-lived
//! worker thread that owns the [`Engine`]; a web build swaps in a web-worker
//! transport with the same surface.
//!
//! Every search carries a generation number. Cancelling (`abort`) bumps the
//! live generation, so events from a search that was stopped for a flag fall
//! or a new game are recognised as stale and dropped instead of playing a
//! ghost move.

use chess::{Board, Engine, Move, SearchHandle, SearchLimits, SearchResult};
use std::sync::mpsc::{Receiver, Sender, channel};

enum EngineCmd {
    NewGame,
    AppendHash(u64),
    // Boxed because a Board dwarfs the other variants.
    Search(Box<SearchJob>),
}

struct SearchJob {
    board: Board,
    limits: SearchLimits,
    handle: SearchHandle,
    generation: u64,
}

enum WorkerEvent {
    Info(u64, SearchResult),
    BestMove(u64, Option<Move>),
}

/// What `poll` hands the UI once stale generations are filtered out.
pub enum EngineEvent {
    Info(SearchResult),
    BestMove(Option<Move>),
}

pub struct EngineDriver {
    commands: Sender<EngineCmd>,
    events: Receiver<WorkerEvent>,
    handle: Option<SearchHandle>,
    generation: u64,
}

impl EngineDriver {
    pub fn new() -> Self {
        let (commands, command_rx) = channel();
        let (event_tx, events) = channel();
        // The worker owns the Engine for the app's lifetime and exits when the
        // command sender is dropped. Constructing the Engine there keeps the
        // 64 MB transposition-table allocation off the first frame.
        std::thread::spawn(move || worker(command_rx, event_tx));
        Self {
            commands,
            events,
            handle: None,
            generation: 0,
        }
    }

    /// Clears the transposition table and repetition history for a new game.
    pub fn new_game(&mut self) {
        self.abort();
        let _ = self.commands.send(EngineCmd::NewGame);
    }

    /// Feeds the engine's repetition detector; call with `board.hash` before
    /// every move actually played, mirroring the UCI front-end.
    pub fn append_hash(&mut self, hash: u64) {
        let _ = self.commands.send(EngineCmd::AppendHash(hash));
    }

    pub fn search(&mut self, board: Board, limits: SearchLimits) {
        self.abort();
        self.generation += 1;
        let handle = SearchHandle::new(limits.ponder);
        self.handle = Some(handle.clone());
        let _ = self.commands.send(EngineCmd::Search(Box::new(SearchJob {
            board,
            limits,
            handle,
            generation: self.generation,
        })));
    }

    /// Cancels any search in flight; its remaining events become stale.
    pub fn abort(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.stop();
            self.generation += 1;
        }
    }

    pub fn searching(&self) -> bool {
        self.handle.is_some()
    }

    /// Drains events from the worker, dropping those of cancelled searches.
    pub fn poll(&mut self) -> Vec<EngineEvent> {
        let mut live = Vec::new();
        for event in self.events.try_iter() {
            match event {
                WorkerEvent::Info(generation, result) if generation == self.generation => {
                    live.push(EngineEvent::Info(result));
                }
                WorkerEvent::BestMove(generation, best) if generation == self.generation => {
                    self.handle = None;
                    live.push(EngineEvent::BestMove(best));
                }
                _ => {}
            }
        }
        live
    }
}

fn worker(commands: Receiver<EngineCmd>, events: Sender<WorkerEvent>) {
    let mut engine = Engine::new();
    while let Ok(command) = commands.recv() {
        match command {
            EngineCmd::NewGame => {
                engine.clear();
                engine.reset_game_history();
            }
            EngineCmd::AppendHash(hash) => engine.append_game_hash(hash),
            EngineCmd::Search(job) => {
                let SearchJob {
                    board,
                    limits,
                    handle,
                    generation,
                } = *job;
                let progress = events.clone();
                let result = engine.search_with_handle(&board, limits, handle, |update| {
                    let _ = progress.send(WorkerEvent::Info(generation, update.clone()));
                });
                let _ = events.send(WorkerEvent::BestMove(generation, result.best_move));
            }
        }
    }
}
