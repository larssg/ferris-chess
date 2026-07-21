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
