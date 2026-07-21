//! Asynchronous facade over the blocking engine search.
//!
//! The UI talks exclusively through this module's small surface — `search`,
//! `abort`, `searching`, and a per-frame `poll` — so the two platform
//! implementations can differ completely behind it:
//!
//! - **Native** runs one long-lived worker thread that owns the [`Engine`].
//!   Every search carries a generation number; cancelling (`abort`) bumps the
//!   live generation so events from a search that was stopped for a flag fall
//!   or a new game are recognised as stale and dropped instead of playing a
//!   ghost move.
//! - **Wasm** has no blocking thread to spare, so `poll` runs the search in
//!   bounded main-thread slices (see the `web` module below).

use chess::{Move, SearchResult};

/// What `poll` hands the UI.
pub enum EngineEvent {
    Info(SearchResult),
    BestMove(Option<Move>),
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::EngineDriver;
#[cfg(target_arch = "wasm32")]
pub use web::EngineDriver;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::EngineEvent;
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
            // The worker owns the Engine for the app's lifetime and exits when
            // the command sender is dropped. Constructing the Engine there
            // keeps the 64 MB transposition-table allocation off the first
            // frame.
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

        /// Feeds the engine's repetition detector; call with `board.hash`
        /// before every move actually played, mirroring the UCI front-end.
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
}

#[cfg(target_arch = "wasm32")]
mod web {
    //! Browser implementation: the search runs on the main thread in bounded
    //! slices. Each `poll` re-runs the search with a node cap sized to hit
    //! [`TARGET_SLICE`]; because the transposition table stays warm between
    //! slices, every restart replays the already-searched depths almost for
    //! free and spends its budget going one deeper. The UI therefore stalls
    //! at most one slice at a time instead of a whole think. A dedicated web
    //! worker would not even help without `SharedArrayBuffer` (unavailable on
    //! plain static hosts): there would be no way to interrupt it mid-search.

    use super::EngineEvent;
    use chess::clock::Instant;
    use chess::{Board, Engine, SearchLimits, SearchResult, TerminationReason};
    use std::time::Duration;

    /// How long one main-thread slice may block the UI.
    const TARGET_SLICE: Duration = Duration::from_millis(70);
    /// First-slice node budget; later slices adapt to measured speed.
    const INITIAL_NODES_PER_SLICE: u64 = 40_000;

    struct SliceJob {
        board: Board,
        limits: SearchLimits,
        started: Instant,
        best_so_far: Option<SearchResult>,
        reported_depth: u8,
    }

    pub struct EngineDriver {
        engine: Engine,
        job: Option<SliceJob>,
        nodes_per_slice: u64,
    }

    impl EngineDriver {
        pub fn new() -> Self {
            Self {
                engine: Engine::new(),
                job: None,
                nodes_per_slice: INITIAL_NODES_PER_SLICE,
            }
        }

        pub fn new_game(&mut self) {
            self.job = None;
            self.engine.clear();
            self.engine.reset_game_history();
        }

        pub fn append_hash(&mut self, hash: u64) {
            self.engine.append_game_hash(hash);
        }

        pub fn search(&mut self, board: Board, limits: SearchLimits) {
            self.job = Some(SliceJob {
                board,
                limits,
                started: Instant::now(),
                best_so_far: None,
                reported_depth: 0,
            });
        }

        pub fn abort(&mut self) {
            self.job = None;
        }

        pub fn searching(&self) -> bool {
            self.job.is_some()
        }

        /// Runs at most one slice and reports progress, exactly like the
        /// native driver's channel drain.
        pub fn poll(&mut self) -> Vec<EngineEvent> {
            let Some(job) = self.job.as_mut() else {
                return Vec::new();
            };
            let mut events = Vec::new();
            let elapsed = job.started.elapsed();
            let soft = job.limits.soft_time.unwrap_or(job.limits.time / 2);
            let mut finished = elapsed >= soft.min(job.limits.time) && job.best_so_far.is_some();

            if !finished {
                let mut slice_limits = job.limits.clone();
                slice_limits.nodes = Some(self.nodes_per_slice);
                slice_limits.time = job.limits.time.saturating_sub(elapsed);
                slice_limits.soft_time = Some(soft.saturating_sub(elapsed));
                let slice_started = Instant::now();
                let result = self.engine.search(&job.board, slice_limits, |_| {});
                let slice_millis = slice_started.elapsed().as_millis().max(1) as u64;
                self.nodes_per_slice = (result.nodes.max(1) * TARGET_SLICE.as_millis() as u64
                    / slice_millis)
                    .clamp(20_000, 4_000_000);

                if result.depth > job.reported_depth {
                    job.reported_depth = result.depth;
                    events.push(EngineEvent::Info(result.clone()));
                }
                // Node-capped slices end with `Nodes`; anything else means
                // the search ran out of something real and cannot go deeper.
                finished = !matches!(result.termination, TerminationReason::Nodes)
                    || job.started.elapsed() >= soft;
                job.best_so_far = Some(result);
            }

            if finished {
                let best = job.best_so_far.as_ref().and_then(|result| result.best_move);
                events.push(EngineEvent::BestMove(best));
                self.job = None;
            }
            events
        }
    }
}
