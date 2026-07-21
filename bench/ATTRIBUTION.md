# Benchmark data

- `data/bratko-kopec.epd` is the 24-position Bratko–Kopec suite mirrored by
  [python-chess](https://github.com/niklasf/python-chess/blob/b73da0dc5310ff4ead0c9c5606fb07199448f194/examples/bratko_kopec/bratko-kopec.epd).
- `fetch-wac.sh` installs the exact `wac.epd` embedded in pinned GoChess commit
  `c3da983fff608ded2c1c59687c030aeba6998f8a` and verifies its SHA-256 digest.
  The suite contains 300 Win at Chess positions and remains subject to its
  upstream provenance.
- `fetch-openings.sh` installs `8moves_v3.pgn` from the
  [official Stockfish books repository](https://github.com/official-stockfish/books),
  which distributes its opening books under CC0 1.0, and verifies both the
  archive and extracted-file SHA-256 digests.

The tactical suites are diagnostics. They are not substitutes for games or
the SPRT acceptance test.
