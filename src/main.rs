use chess::{Board, Color, Engine, SearchLimits};
use std::{
    io::{self, BufRead},
    time::Duration,
};
fn main() {
    let mut board = Board::startpos();
    let mut engine = Engine::new();
    let stdin = io::stdin();
    for line in stdin.lock().lines().map_while(Result::ok) {
        let parts: Vec<_> = line.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }
        match parts[0] {
            "uci" => println!(
                "id name FerrisChess 0.1\nid author LarsSG and OpenAI\noption name Hash type spin default 64 min 1 max 4096\nuciok"
            ),
            "isready" => println!("readyok"),
            "ucinewgame" => {
                board = Board::startpos();
                engine.clear()
            }
            "setoption" => {
                if let (Some(name), Some(value)) = (
                    parts.iter().position(|p| *p == "name"),
                    parts.iter().position(|p| *p == "value"),
                ) && parts
                    .get(name + 1)
                    .is_some_and(|p| p.eq_ignore_ascii_case("hash"))
                    && let Some(mb) = parts.get(value + 1).and_then(|v| v.parse::<usize>().ok())
                {
                    engine.set_hash_mb(mb.clamp(1, 4096));
                }
            }
            "position" => {
                let mut i = 1;
                if parts.get(i) == Some(&"startpos") {
                    board = Board::startpos();
                    i += 1
                } else if parts.get(i) == Some(&"fen") {
                    i += 1;
                    let end = parts[i..]
                        .iter()
                        .position(|x| *x == "moves")
                        .map_or(parts.len(), |n| i + n);
                    if let Ok(b) = Board::from_fen(&parts[i..end].join(" ")) {
                        board = b
                    }
                    i = end
                }
                if parts.get(i) == Some(&"moves") {
                    for s in &parts[i + 1..] {
                        if let Some(m) = board.parse_move(s) {
                            board.make_move(m);
                        }
                    }
                }
            }
            "go" => {
                let mut limits = SearchLimits::default();
                let mut wtime: Option<u64> = None;
                let mut btime: Option<u64> = None;
                let mut winc = 0u64;
                let mut binc = 0u64;
                let mut moves_to_go = 30u64;
                let mut i = 1;
                while i < parts.len() {
                    match parts[i] {
                        "depth" => {
                            i += 1;
                            limits.depth = parts.get(i).and_then(|s| s.parse().ok()).unwrap_or(8)
                        }
                        "movetime" => {
                            i += 1;
                            limits.time = Duration::from_millis(
                                parts.get(i).and_then(|s| s.parse().ok()).unwrap_or(5000),
                            )
                        }
                        "nodes" => {
                            i += 1;
                            limits.nodes = parts.get(i).and_then(|s| s.parse().ok())
                        }
                        "mate" => {
                            i += 1;
                            limits.mate = parts.get(i).and_then(|s| s.parse().ok())
                        }
                        "ponder" => limits.ponder = true,
                        "infinite" => limits.infinite = true,
                        "searchmoves" => {
                            i += 1;
                            while i < parts.len()
                                && !matches!(
                                    parts[i],
                                    "depth"
                                        | "movetime"
                                        | "nodes"
                                        | "mate"
                                        | "wtime"
                                        | "btime"
                                        | "winc"
                                        | "binc"
                                        | "movestogo"
                                        | "ponder"
                                        | "infinite"
                                )
                            {
                                if let Some(chess_move) = board.parse_move(parts[i]) {
                                    limits.root_moves.push(chess_move);
                                }
                                i += 1;
                            }
                            i = i.saturating_sub(1);
                        }
                        "wtime" => {
                            i += 1;
                            wtime = parts.get(i).and_then(|s| s.parse().ok());
                        }
                        "btime" => {
                            i += 1;
                            btime = parts.get(i).and_then(|s| s.parse().ok());
                        }
                        "winc" => {
                            i += 1;
                            winc = parts.get(i).and_then(|s| s.parse().ok()).unwrap_or(0);
                        }
                        "binc" => {
                            i += 1;
                            binc = parts.get(i).and_then(|s| s.parse().ok()).unwrap_or(0);
                        }
                        "movestogo" => {
                            i += 1;
                            moves_to_go = parts
                                .get(i)
                                .and_then(|s| s.parse().ok())
                                .unwrap_or(30)
                                .max(1);
                        }
                        _ => {}
                    }
                    i += 1
                }
                if !parts.contains(&"movetime")
                    && let Some(left) = if board.side == Color::White {
                        wtime
                    } else {
                        btime
                    }
                {
                    let inc = if board.side == Color::White {
                        winc
                    } else {
                        binc
                    };
                    let safe = left.saturating_sub(30);
                    let target = safe / moves_to_go + inc * 3 / 4;
                    limits.time = Duration::from_millis(target.clamp(20, (safe / 2).max(20)));
                }
                limits.white_time = wtime.map(Duration::from_millis);
                limits.black_time = btime.map(Duration::from_millis);
                limits.white_increment = Duration::from_millis(winc);
                limits.black_increment = Duration::from_millis(binc);
                limits.moves_to_go = Some(moves_to_go as u32);
                let r = engine.search(&board, limits, |r| {
                    println!(
                        "info depth {} seldepth {} score cp {} nodes {} time {} nps {} hashfull {} pv {}",
                        r.depth,
                        r.seldepth,
                        r.score,
                        r.nodes,
                        r.elapsed.as_millis(),
                        r.nps,
                        r.hashfull,
                        r.pv.iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(" ")
                    )
                });
                println!(
                    "bestmove {}",
                    r.best_move.map_or("0000".into(), |m| m.to_string())
                )
            }
            "d" => println!("{}", board.to_fen()),
            "quit" => break,
            _ => {}
        }
    }
}
