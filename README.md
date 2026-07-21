# FerrisChess

[![Deploy web GUI](https://github.com/larssg/ferris-chess/actions/workflows/deploy-gui.yml/badge.svg)](https://github.com/larssg/ferris-chess/actions/workflows/deploy-gui.yml)

A dependency-free UCI chess engine written in Rust, modeled after the sibling
GoChess project. The engine is currently a compact, playable baseline with a
complete legal chess core and classical alpha-beta search.

**▶ Play it in your browser: <https://larssg.github.io/ferris-chess/>** — the web
GUI, built and deployed from `main` on every change by GitHub Actions.

## Features

- Legal move generation including castling, en passant, and all promotions
- Strict FEN parsing and serialization
- Iterative deepening with aspiration windows and principal-variation search
- Quiescence search, check extensions, late-move reductions, killer/history ordering
- Zobrist-style deterministic position keys, repetition and rule-draw detection
- Transposition table and UCI-configurable hash allocation
- UCI `depth`, `movetime`, `nodes`, and clock/increment time controls
- Asynchronous UCI search with responsive `stop`, `ponder`, and `ponderhit`
- Thread-safe library search cancellation through `SearchHandle`
- Explicit single-worker UCI `Threads` compatibility for match runners
- Material, piece-square, and bishop-pair evaluation

## Build and run

```sh
cargo build --release
./target/release/chess
```

The executable speaks UCI on standard input/output:

```text
uci
isready
position startpos moves e2e4 e7e5
go depth 8
quit
```

UCI options are `Hash` (1–4096 MB), `OwnBook`, and `Threads`. `Threads` is
currently fixed at one; multi-worker lazy SMP is a later milestone.

## Graphical interface

The `gui/` crate is an egui front-end for playing against the engine — side
selection, time controls, mouse and keyboard input, clocks, and live search
info. It runs natively and in the browser.

```sh
cargo run --release -p chess-gui        # native window

cd gui && trunk build --release         # web page in gui/dist/
cd gui && trunk serve --release         # or serve it locally
```

The web build needs [trunk](https://trunkrs.dev) and the
`wasm32-unknown-unknown` target (`rustup target add wasm32-unknown-unknown`);
`gui/dist/` is fully static and can be hosted anywhere. This repository
publishes it to GitHub Pages at <https://larssg.github.io/ferris-chess/> via
`.github/workflows/deploy-gui.yml`.
In the browser the search runs in bounded main-thread slices, so the page
stays responsive while the engine thinks.

Piece images are the cburnett set by Colin M.L. Burnett, used under
CC BY-SA 3.0 — see `gui/assets/pieces/COPYING.md`.

## Validation

```sh
cargo test --all-targets
```

Move generation is checked against standard perft totals, including the
initial position through depth 4 and Kiwipete through depth 3.

## Reproducible benchmarks

Build the release binary before collecting numbers:

```sh
cargo build --release
./bench/build-gochess.sh
./target/release/chess bench perft
./target/release/chess bench search 6
./target/release/chess bench bk 6
./bench/fetch-wac.sh
./target/release/chess bench wac 6
./bench/fetch-openings.sh
```

All search modes use fixed depth and fresh 64 MB hash tables, making node
counts, scores, best moves, and PVs deterministic. Wall time and NPS remain
machine-dependent. Suite provenance is recorded in `bench/ATTRIBUTION.md`.

`bench/build-gochess.sh` verifies and builds the sibling checkout at
`../../go/chess`; override it with `GOCHESS_SOURCE` when necessary.
`bench/run-sprt.sh` runs the planned fixed-seed, reversed-color Cute Chess
SPRT and builds the pinned reference automatically. `OPENINGS` must point at
`8moves_v3.pgn` if it is not installed at `bench/data/8moves_v3.pgn`.
`bench/fetch-openings.sh` installs that exact checksum-verified file from the
official Stockfish books repository.
