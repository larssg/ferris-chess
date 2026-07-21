# FerrisChess parity and superiority plan

## Objective

Replace the original cloning-based Rust engine with a Rust-native bitboard
engine, reproduce the behavior and strength of GoChess, then retain measured
improvements until FerrisChess passes a Cute Chess SPRT against GoChess with
`H0=0 Elo` and `H1=+20 Elo`.

The pinned reference is GoChess commit
`c3da983fff608ded2c1c59687c030aeba6998f8a`, located by default at
`../../go/chess`.

## Required quality loop

After every coherent edit batch and before every commit, run:

```sh
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```

Do not commit formatting failures, Clippy warnings, or failing tests. Finish
milestones with `cargo build --release` and a UCI smoke test. Keep stable
checkpoints in separate commits.

## Current status

The worktree was clean after the documentation checkpoint below. Completed
commits, in order:

| Commit | Checkpoint |
|---|---|
| `bf6c690` | Implement legal chess core and UCI search engine |
| `5ae1426` | Add UCI time management and search regression coverage |
| `65b6d64` | Add bitboard state and exact make/unmake |
| `a942dce` | Replace `HashMap` with clustered transposition table |
| `aa59dc6` | Extend search limits and UCI search metadata |
| `1bb6bb0` | Run UCI search asynchronously |
| `d248fe7` | Add reproducible benchmark and SPRT harness |
| `d5290b5` | Pin and measure GoChess reference baseline |

Implemented public interfaces and behavior include:

- Twelve piece bitboards and per-color occupancy masks.
- Private square storage with `piece_at`, `pieces`, and occupancy inspection.
- `Board::make_move` returns an exact `Undo` and `Board::unmake_move` restores
  the complete position.
- Caller-provided `generate_legal_moves` plus the `legal_moves` convenience
  wrapper.
- Search recursion uses make/unmake instead of cloning every child.
- Four-entry, 64-byte transposition-table clusters with packed moves,
  generations, depth-aware replacement, and mate normalization.
- Extended `SearchLimits` clocks, increments, moves-to-go, nodes, mate,
  ponder, infinite mode, and root move restrictions.
- Extended `SearchResult` score type, seldepth, nodes, NPS, hash fullness, PV,
  and termination reason.
- Public thread-safe `SearchHandle` cancellation and ponder-hit control.
- Asynchronous UCI `go`, responsive `stop`, `ponder`, and `ponderhit`.
- Deterministic `bench perft`, `bench search`, `bench bk`, `bench wac`, and
  arbitrary EPD modes.
- Pinned GoChess build helper, exact BK/WAC inputs, and fixed SPRT harness.
- GoChess-tuned material and piece-square values with its endgame king switch.
- Separately testable bishop-pair, pawn-structure, king-safety, rook-placement,
  and endgame mop-up evaluation terms.
- Static exchange evaluation used for capture ordering and quiescence pruning.
- Static evaluations cached in the 64-byte clustered transposition table.
- One-sided 50/200/full aspiration widening and root principal-variation
  search behavior matching the pinned reference.
- Reverse futility pruning through depth 6 with reference margins.
- Adaptive null-move pruning with pawn-only, check, PV, and consecutive-null
  guards plus exact null-state restoration.
- Razoring through depth 3, shallow quiet-move futility pruning, and late-move
  pruning through depth 5 with reference margins and thresholds.
- Logarithmic late-move reductions with full-depth verification and internal
  iterative reduction at deep PV nodes without a transposition-table move.
- Bounded history gravity with failed-quiet maluses, distinct killers, and
  countermove ordering tied to the preceding move.
- Same-side repetition scanning across real-game and search history, plus
  adaptive soft/hard time budgets that respond to search instability.
- The complete 108-position GoChess opening repertoire with legal-move
  validation, move-history lookup, `OwnBook`, and `searchmoves` bypass.
- Explicit fixed-one-worker `Threads` UCI semantics and SPRT configuration.
- Pinned GoChess fixtures for exact rule-edge legal moves, numeric evaluation
  and SEE results, and fixed-depth search scores and best moves.

At the current worktree checkpoint, all 30 Rust tests, warning-free Clippy,
release builds, GoChess tests, and UCI smoke tests passed.

## Recorded baseline

Full details are in `bench/BASELINE.md`. Important results:

- Validated perft benchmark: 19,981,295 total nodes across start position,
  Kiwipete, and Chess Programming Wiki position 3.
- Kiwipete depth 4: GoChess 1,841 nodes, FerrisChess 407,531 nodes; both choose
  `e2a6`. FerrisChess searches roughly 221 times as many nodes.
- Middlegame depth 4: GoChess 4,118 nodes and `f3e5`; FerrisChess 128,890 nodes
  and `c2b3`. FerrisChess searches roughly 31 times as many nodes.
- GoChess BK at one second per position: 11/24.
- FerrisChess BK at fixed depth 3: 6/24 over 10,354,891 nodes.
- FerrisChess WAC at fixed depth 1: 34/300 over 8,586,444 nodes.

These results make search pruning, move ordering, and evaluation parity the
next priority. Tactical-suite results are diagnostics, not Elo evidence.

## Current checkpoint

GoChess evaluation and SEE parity is implemented in the current worktree:

- Ported the pinned tuned material and middlegame/endgame piece-square tables.
- Matched the reference's discrete endgame king-table selection. Continuous
  tapered evaluation remains a post-parity experiment.
- Ported bishop pair, pawn structure, king safety, rook placement, and mop-up.
- Ported SEE and use it for good/bad capture ordering and quiescence pruning.
- Added fixed evaluation and exchange fixtures.
- Passed the quality loop, release build, depth-4 benchmark, and UCI smoke test.

The evaluation/SEE depth-4 checkpoint visited 77,698 total nodes. TT static
evaluation caching and aspiration/PVS reduced that to 47,195 nodes; null-move
and reverse futility pruning reduced it again to 39,436 nodes. Razoring,
futility pruning, and late-move pruning reduced the total to 17,967 nodes and
matched GoChess's `f3e5` middlegame best move while preserving the other two
best moves. Logarithmic late-move reductions and internal iterative reduction
reduced the total again to 17,150 nodes. Bounded history updates, failed-quiet
maluses, and countermoves reduced it to 15,067 nodes while preserving all
three scores and best moves. Correct repetition semantics and adaptive time
management preserve that deterministic fixed-depth result.
Opening-book parity does not affect the explicit benchmark search path.

## Next checkpoint

The first non-inferiority SPRT failed at its lower boundary after 35 games
(1–24–10, estimated -273.7 +/- 116.9 Elo, LLR -3.01). The match was
protocol-clean, but GoChess generally searched roughly three plies deeper at
similar move times.

Profile and remove the largest search-throughput costs, starting with board
state rebuilding, evaluation rescans, and move-list allocation. Preserve exact
evaluation, SEE, fixed-depth scores, best moves, and deterministic node counts.

Re-run fixed-depth differential benchmarks after each coherent batch. Require
an SPRT non-inferiority result before superiority experiments.

## Remaining milestones

### GoChess parity

- Match tuned classical evaluation and SEE.
- Match search pruning, reductions, extensions, ordering, and repetition.
- Add opening book parity and `OwnBook` UCI behavior.
- Add `Threads` option semantics; classical single-thread behavior comes first.
- Differential-test legal moves, evaluation terms, SEE, book behavior, and
  fixed-depth searches against the pinned reference.
- Pass Cute Chess SPRT non-inferiority at `elo0=0`.

### Exceed GoChess

Test enhancements in isolated commits and retain only positive match results:

- Incremental evaluation and remaining allocation removal.
- Continuation history and capture history.
- Improving-aware LMP and LMR.
- Singular extensions.
- ProbCut.
- Lazy SMP.
- Deterministic classical-evaluation tuning with training/validation splits.
- Optional bundled NNUE with incremental accumulators only if classical
  improvements cannot clear the superiority gate. Keep classical evaluation
  selectable through UCI.

### Finalization

- Add Linux/macOS CI for formatting, warning-free Clippy, tests, perft,
  release builds, and UCI transcripts.
- Document architecture, UCI options, benchmark reproduction, reference
  commit, NPS, and Elo results.
- Make a clean release/version commit.

## Acceptance tests

- Validate the initial position and standard Chess Programming Wiki perft
  positions through at least one million nodes each.
- Property-test exact make/unmake restoration of FEN, hash, evaluation,
  occupancy, clocks, and legal moves through deterministic randomized games.
- Cover castling through check, rook-capture castling rights, en-passant pins,
  promotions, mate, stalemate, insufficient material, 50-move draws,
  repetition, malformed FEN, and TT mate normalization.
- Test UCI clocks, `searchmoves`, `stop`, ponder, option changes, terminal
  positions, and malformed commands.
- Treat BK/WAC and NPS as diagnostics only.

Final completion gate:

```text
Cute Chess SPRT
elo0=0 elo1=20 alpha=0.05 beta=0.05
10+0.1, one thread, 64 MB hash
fixed seed, 8moves_v3.pgn, reversed colors
reference draw/resign adjudication
```

Use `bench/build-gochess.sh` to verify and build the pinned reference,
`bench/fetch-wac.sh` to install the exact checksum-verified WAC input, and
`bench/fetch-openings.sh` to install the checksum-verified Stockfish opening
file. Then use `bench/run-sprt.sh` for the match gate.

## Scope assumptions

- Scope includes the UCI engine, reusable library, benchmarks, and match
  harness. Web and interactive-console modes are excluded.
- Small audited crates may support CLI, synchronization, tests, and benchmarks;
  core chess logic remains custom.
- Classical evaluation comes first; NNUE is an optional fallback.
- Tablebases are not required.
