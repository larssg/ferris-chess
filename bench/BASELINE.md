# Pinned baseline

Reference: GoChess `c3da983fff608ded2c1c59687c030aeba6998f8a`, built from
`../../go/chess` with Go 1.26.5. Both engines used 64 MB transposition tables.

## Fixed-depth search

Collected on 2026-07-21. Node counts, scores, moves, and PVs are deterministic;
timing is diagnostic and machine-dependent.

| Position | Engine | Depth | Score | Nodes | Best move | PV |
|---|---|---:|---:|---:|---|---|
| Kiwipete | GoChess | 4 | +45 | 1,841 | `e2a6` | `e2a6 b4c3 d2c3 h3g2` |
| Kiwipete | FerrisChess | 4 | +90 | 407,531 | `e2a6` | `e2a6 e6d5 c3d5 b6d5` |
| Middlegame | GoChess | 4 | +34 | 4,118 | `f3e5` | `f3e5 a8c8 e5c6 c8c6` |
| Middlegame | FerrisChess | 4 | +35 | 128,890 | `c2b3` | `c2b3 d8c7 c1d2 c5c4` |

The Kiwipete best move agrees, but the node gap is roughly 221×. The
middlegame node gap is roughly 31× and the best moves differ. This makes
search pruning and move ordering the highest-priority parity work.

## Tactical diagnostics

- GoChess BK, 1 second per position: 11/24 in 17.483 seconds.
- FerrisChess BK, fixed depth 3: 6/24 over 10,354,891 nodes.
- FerrisChess WAC, fixed depth 1: 34/300 over 8,586,444 nodes.

These use different stopping conditions and are recorded as independent
baselines, not as a direct strength comparison.

## Evaluation and SEE parity checkpoint

After porting the pinned GoChess classical evaluator and static exchange
evaluation, FerrisChess produced this release depth-4 search benchmark:

| Position | Nodes | Score | Best move |
|---|---:|---:|---|
| Start position | 9,333 | 0 | `g1f3` |
| Kiwipete | 36,995 | 45 | `e2a6` |
| Middlegame | 31,370 | 34 | `f3e5` |
| Total | 77,698 | — | — |

The run completed in 230 ms at 337,817 NPS on the same development machine.
Compared with the original FerrisChess baseline, total nodes fell by about
85.5%; the middlegame best move now matches the pinned GoChess result. These
figures remain diagnostic because the search algorithms are not yet at parity.

## TT evaluation cache and PVS checkpoint

Packing TT scores to 16 bits made room for a cached static evaluation without
changing the four-entry cluster's 64-byte size. Together with reference-style
aspiration widening and root PVS, the release depth-4 benchmark became:

| Position | Nodes | Score | Best move |
|---|---:|---:|---|
| Start position | 4,552 | 0 | `g1f3` |
| Kiwipete | 21,573 | 45 | `e2a6` |
| Middlegame | 21,070 | 34 | `f3e5` |
| Total | 47,195 | — | — |

The run completed in 159 ms at 296,823 NPS. Nodes fell about 39.3% from the
evaluation/SEE checkpoint while every recorded score, best move, and PV stayed
unchanged.

## Null-move and reverse-futility checkpoint

Reference-style adaptive null-move pruning and reverse futility pruning
produced this release depth-4 benchmark:

| Position | Nodes | Score | Best move |
|---|---:|---:|---|
| Start position | 4,198 | 0 | `g1f3` |
| Kiwipete | 17,092 | 45 | `e2a6` |
| Middlegame | 18,146 | 34 | `f3e5` |
| Total | 39,436 | — | — |

The run completed in 98 ms at 402,408 NPS. Nodes fell about 16.4% from the TT
evaluation cache/PVS checkpoint with scores, best moves, and PVs unchanged.

## Shallow pruning and LMR checkpoint

Razoring, futility pruning, and late-move pruning first reduced the release
depth-4 total to 17,967 nodes. Adding reference-style logarithmic late-move
reductions and internal iterative reduction produced:

| Position | Nodes | Score | Best move |
|---|---:|---:|---|
| Start position | 1,449 | 0 | `g1f3` |
| Kiwipete | 9,516 | 45 | `e2a6` |
| Middlegame | 6,185 | 34 | `f3e5` |
| Total | 17,150 | — | — |

The final LMR/IIR step reduced nodes by 4.5% with all depth-4 scores, best
moves, and PVs unchanged. Relative to the null-move/reverse-futility checkpoint,
the combined shallow-pruning and reduction work cut the node count by 56.5%.

## First non-inferiority SPRT

Run on 2026-07-21 with Cute Chess 1.4.0, the checksum-verified Stockfish
`8moves_v3.pgn`, a fixed seed, reversed colors, one thread per engine, 64 MB
hash, and 10+0.1 time control. The sequential test reached its lower boundary
after 35 games:

```text
FerrisChess vs GoChess: 1 win, 24 losses, 10 draws
Elo difference: -273.7 +/- 116.9
SPRT: llr -3.01, lower bound -2.94, upper bound 2.94
Result: H0 accepted; the +0 to +20 Elo non-inferiority gate failed
```

FerrisChess scored 25.0% as White and 8.8% as Black. The PGN showed no engine
crashes, illegal moves, or time forfeits. GoChess generally reached roughly
three more plies at similar move times, making search throughput and avoidable
per-node work the next measured parity target.
