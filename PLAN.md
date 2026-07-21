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

The first throughput batch replaced full mailbox-to-bitboard rebuilds and
full-position hash recomputation after every move with incremental updates.
The depth-6 search benchmark remains deterministic at 92,572 nodes with
identical scores, best moves, and PVs; on the development machine its sampled
runtime fell from 316 ms to 175 ms (roughly 293k to 529k NPS).
Evaluation now walks piece bitboards directly and uses bitboards for pawn
structure and king-safety inputs, removing its temporary pawn allocations and
mailbox/rook rescans. The same benchmark fell further to 155 ms (roughly 597k
NPS) without changing its deterministic output.
Search now reuses one move buffer per ply for alpha-beta and quiescence, filters
pseudo-legal moves in place, and records failed quiets without a heap vector.
Three depth-6 samples completed in 147--152 ms (up to roughly 630k NPS), again
with identical deterministic output.
Move ordering now caches its score by packed move before sorting, so expensive
SEE is evaluated once per capture instead of repeatedly by the sort comparator.
The 164,847-node depth-7 benchmark fell from 278 ms to 221--240 ms (up to
roughly 746k NPS) with identical scores, best moves, PVs, and node counts.
A macOS `sample` profile then exposed per-node heap allocation in insufficient-
material detection. Replacing its mailbox collection with bitboard counts
reduced the 514,650-node depth-9 benchmark from 716 ms to 676--697 ms (up to
roughly 761k NPS), again without changing deterministic output.
Bitboard king lookup, precomputed knight/king attack masks, and bitboard-backed
slider blocker tests then reduced the same depth-9 benchmark to 434--444 ms
(up to roughly 1.19M NPS), preserving all deterministic results and perft.
Per-file pawn bitboards and rank masks replaced repeated pawn-set walks in the
profile's next-largest leaf hotspot. Depth 9 fell further to 415--427 ms (up to
roughly 1.24M NPS) with exact evaluation and search parity preserved.
Compile-time piece, castling, en-passant, and side hash tables then removed
SplitMix arithmetic from make/unmake. Depth 9 fell to 400--407 ms (up to
roughly 1.29M NPS) while retaining the engine's exact hash identities.
A fresh profile ranked static exchange evaluation first and `square_attacked`
fourth; both still walked mailbox rays square by square. Compile-time ray
tables now cut each slider ray at its first blocker with one bit scan, SEE
tracks the exchange on an occupancy bitboard instead of copying the mailbox,
and its least-attacker probe replicates the reference's exact tie-break scan
order so exchange sequences stay reproducible. Depth 9 fell to 309--317 ms
(up to roughly 1.67M NPS) with identical node counts, scores, best moves, and
PVs; the profile's next-largest costs are evaluation, `make_move`, and
mailbox move generation.
Evaluation's per-piece material and piece-square walk then became a running
board accumulator maintained by `place_piece`/`remove_piece`, with the endgame
king-table switch applied as a per-king correction and a drift test comparing
the accumulator against from-scratch rebuilds. Depth 9 fell to 294--301 ms
(up to roughly 1.75M NPS). An exact direct-mapped pawn-structure cache — keyed
and fully verified by both pawn bitboards, owned by the engine — then removed
most remaining pawn-term work. Depth 9 fell to 269--284 ms (up to roughly
1.91M NPS), always with identical node counts, scores, best moves, and PVs.
The profile now ranks `make_move`, mailbox `generate_pseudo_moves`,
`square_attacked` legality filtering, and slider move emission as the largest
remaining costs.
Move generation then moved onto the bitboards: occupancy bits drive the piece
scan, compile-time target lists serve the leapers, and the slider rays reuse
the blocker-cut ray tables, iterated in each ray's direction of travel. The
emission order deliberately reproduces the old square-by-square scan, because
unstable sorts turn generation order into search-visible behavior; identity
was verified by diffing old-versus-new deterministic outputs for depth-11
search (1,831,720 nodes), all 24 BK positions at depth 6, and all 300 WAC
positions at depth 5. Depth 9 fell to 233--248 ms (up to roughly 2.21M NPS)
with perft exact.
The legality filter then became pin-aware: checkers and absolutely pinned
pieces are computed once per generation from the ray tables (with new
compile-time between/line tables), after which each pseudo-legal move is
judged by bit tests — king moves against king-lifted occupancy, castling
proven during generation, and only en passant still played out. A
differential test holds the filter equal to the played-out definition across
en-passant pins, double checks, and deterministic random walks. Depth 9 fell
to 161--169 ms (up to roughly 3.20M NPS) and perft to 481 ms (41.5M NPS),
with the same byte-identical outputs across depth-11 search, BK depth 6, and
WAC depth 5.

## Next checkpoint

The first non-inferiority SPRT failed at its lower boundary after 35 games
(1–24–10, estimated -273.7 +/- 116.9 Elo, LLR -3.01). The match was
protocol-clean, but GoChess generally searched roughly three plies deeper at
similar move times. Six throughput batches (see the checkpoint narrative
above) then raised deterministic fixed-depth throughput from roughly 1.29M to
3.20M NPS without changing any search result.

The second SPRT, run 2026-07-21 under the identical protocol, reached its
*upper* boundary after 75 games: 41–9–25 (71.6%), estimated +158.4 +/- 68.2
Elo, LLR 2.96 against bounds -2.94/+2.94 — H1 accepted, no crashes, illegal
moves, or time forfeits. The `elo0=0 elo1=20` gate is passed; details in
`bench/BASELINE.md`.

With parity exceeded, remaining work moves to the superiority experiments and
finalization milestones below. Keep every experiment in an isolated commit and
retain only changes that hold up in their own match results.

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
