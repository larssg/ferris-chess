# FerrisChess — agent guide

A dependency-free UCI chess engine in Rust, built to reach parity with (and then
exceed) the sibling **GoChess** engine. The end goal is passing a Cute Chess
SPRT against GoChess; see `PLAN.md` for the full roadmap and `bench/BASELINE.md`
for measured results.

## Guiding principle

**Write code as a readable narrative.** Prefer names, structure, and comments
that let a reader follow the intent top to bottom. Comments should explain
*why*, not restate the code.

## Quality loop — run before every commit

This is mandatory. Do not commit formatting failures, Clippy warnings, or
failing tests.

```sh
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets
```

The `--workspace` flag matters: the root package is the only default member
(so `cargo build --release` stays engine-only for the bench scripts), and
without it the `gui/` crate is silently skipped.

Finish milestones with `cargo build --release` and a UCI smoke test. Keep
stable checkpoints in separate commits.

## Layout

The workspace has two members: the engine at the root (a reusable library,
`src/lib.rs`, with a thin UCI binary on top) and `gui/`, an egui/eframe
front-end (crate `chess-gui`, run with `cargo run -p chess-gui`).

| File | Role |
|---|---|
| `src/board.rs` | Rules of chess: position, move generation, make/unmake, FEN, attack detection, hashing. |
| `src/eval.rs` | Static evaluation (material, PST, pawn/king/rook terms, mop-up) and static exchange evaluation (SEE). |
| `src/engine.rs` | Iterative-deepening alpha-beta search (negamax + PVS), the transposition table, and `time_budget`. |
| `src/benchmark.rs` | Deterministic `bench` subcommands for reproducible comparison against GoChess. |
| `src/main.rs` | UCI front-end; runs the search on a worker thread. |
| `src/clock.rs` | `Instant` alias that swaps in `web-time` on wasm32 (the engine's only conditional dependency). |
| `src/eval_tables.in` | Piece-square tables `include!`d by `eval.rs` (the active tables). |
| `src/pst.in` | An earlier PST set; **not referenced by any code**. |
| `gui/src/app.rs` | GUI state machine: setup screen vs. game, panels, engine-turn scheduling. |
| `gui/src/engine_driver.rs` | Command/event facade over a worker thread that owns the `Engine`; the wasm seam (a web worker slots in behind the same surface). |
| `gui/src/board_widget.rs` | Board painting plus mouse (click/drag) and arrow-key-cursor interaction. |

The engine library must keep compiling for wasm32
(`cargo check --lib --target wasm32-unknown-unknown`); use
`crate::clock::Instant`, never `std::time::Instant`, inside the library.

## Conventions worth knowing

- **Square indexing** is little-endian rank-file: `a1 = 0`, `h1 = 7`, `a8 = 56`,
  `h8 = 63`. So `sq % 8` is the file and `sq / 8` is the rank; `sq ^ 56` flips a
  square vertically (used to mirror White/Black PSTs).
- **Scores** are centipawns from the side-to-move's perspective (negamax). Mates
  are scored near `MATE` (90_000) offset by ply, so shorter mates score higher.
  `INF` (100_000) bounds every real score.
- **`Board` keeps two synced representations**: the `squares` mailbox is
  authoritative for move generation; the `pieces`/`occupancy` bitboards are
  derived (rebuilt after every mutation) and used by evaluation/SEE.
- **make/unmake, not clone-per-child**: the search plays moves on one board and
  reverses them with the returned `Undo`. `make_unmake_restores_every_root_move`
  guards this exactness — keep it passing.
- **Evaluation parity is the point**: constants and tables in `eval.rs` are
  ported from GoChess. Don't "improve" them casually; parity is validated by
  fixtures and the differential benchmarks. Superiority experiments come *after*
  the SPRT non-inferiority gate and belong in isolated commits.

## Benchmarks

All `bench` modes use fixed depth and a fresh 64 MB hash, so node counts,
scores, best moves, and PVs are deterministic (only wall time/NPS vary).

```sh
cargo build --release
./target/release/chess bench perft         # move-gen correctness (asserts totals)
./target/release/chess bench search 6      # search node counts
./target/release/chess bench bk 6          # Bratko-Kopec (diagnostic only)
./target/release/chess bench wac 6         # Win at Chess (diagnostic only)
```

`bench/build-gochess.sh` builds the pinned reference; `bench/run-sprt.sh` runs
the match gate. Treat BK/WAC and NPS as diagnostics, not Elo evidence.

## UCI

The binary speaks UCI on stdin/stdout. The only option is `Hash` (1–4096 MB).
Search runs on a worker thread so `stop`/`ponderhit` stay responsive. A non-UCI
`d` command dumps the current position as FEN.
