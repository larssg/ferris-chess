#!/bin/sh
set -eu

rust_engine=${RUST_ENGINE:-./target/release/chess}
GOCHESS_BIN=${GOCHESS_BIN:-bench/bin/gochess-c3da983}
CUTECHESS_BIN=${CUTECHESS_BIN:-bench/bin/cutechess-cli}
openings=${OPENINGS:-bench/data/8moves_v3.pgn}
games=${GAMES:-10000}

if ! test -x "$GOCHESS_BIN"; then
  GOCHESS_BIN="$GOCHESS_BIN" ./bench/build-gochess.sh
fi

test -x "$rust_engine"
test -x "$GOCHESS_BIN"
if ! test -x "$CUTECHESS_BIN"; then
  CUTECHESS_BIN=$(command -v cutechess-cli)
fi
test -f "$openings"

"$CUTECHESS_BIN" \
  -engine "cmd=$rust_engine" name=FerrisChess proto=uci option.Threads=1 \
  -engine "cmd=$GOCHESS_BIN" arg=uci name=GoChess-c3da983 proto=uci \
  -each tc=10+0.1 option.Hash=64 \
  -openings "file=$openings" format=pgn order=random plies=16 \
  -srand 1 -repeat -games "$games" \
  -sprt elo0=0 elo1=20 alpha=0.05 beta=0.05 \
  -resign movecount=3 score=600 \
  -draw movenumber=40 movecount=8 score=10 \
  -pgnout bench/results/sprt.pgn
