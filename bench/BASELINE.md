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
