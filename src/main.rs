use chess::{Board, Engine, SearchLimits};
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
                            board.make_move(m)
                        }
                    }
                }
            }
            "go" => {
                let mut limits = SearchLimits::default();
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
                        _ => {}
                    }
                    i += 1
                }
                let r = engine.search(&board, limits, |r| {
                    println!(
                        "info depth {} score cp {} nodes {} time {} nps {} pv {}",
                        r.depth,
                        r.score,
                        r.nodes,
                        r.elapsed.as_millis(),
                        r.nodes * 1000 / (r.elapsed.as_millis() as u64).max(1),
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
