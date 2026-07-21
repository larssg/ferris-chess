# FerrisChess

A dependency-free UCI chess engine written in Rust, modeled after the sibling
GoChess project. The engine is currently a compact, playable baseline with a
complete legal chess core and classical alpha-beta search.

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
./target/release/chess bench perft
./target/release/chess bench search 6
./target/release/chess bench bk 6
./bench/fetch-wac.sh
./target/release/chess bench wac 6
```

All search modes use fixed depth and fresh 64 MB hash tables, making node
counts, scores, best moves, and PVs deterministic. Wall time and NPS remain
machine-dependent. Suite provenance is recorded in `bench/ATTRIBUTION.md`.

`bench/run-sprt.sh` runs the planned fixed-seed, reversed-color Cute Chess
SPRT. It requires `GOCHESS_BIN` to point at GoChess commit
`c3da983fff608ded2c1c59687c030aeba6998f8a` and `OPENINGS` to point at
`8moves_v3.pgn`. The GoChess repository location is intentionally not guessed;
this checkout contains neither the reference source nor metadata identifying
its upstream URL.
