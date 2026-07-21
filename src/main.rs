//! UCI front-end: translates the text protocol on stdin/stdout into calls on the
//! `chess` library. The engine runs on a worker thread so the main loop can keep
//! reading commands (notably `stop` and `ponderhit`) while a search is running.

use chess::{Board, Color, Engine, SearchHandle, SearchLimits, book::book_move};
use std::{
    io::{self, BufRead},
    thread::{self, JoinHandle},
    time::Duration,
};

const UCI_OPTIONS: &str = "option name Hash type spin default 64 min 1 max 4096\n\
option name Threads type spin default 1 min 1 max 1\n\
option name OwnBook type check default true";

/// Splits `setoption name X [value Y]` without baking option names into the
/// protocol parser. UCI option names may contain spaces, so token positions are
/// more reliable than assuming the name is always the second word.
fn parse_setoption(parts: &[&str]) -> Option<(String, String)> {
    let name = parts.iter().position(|part| *part == "name")?;
    let value = parts.iter().position(|part| *part == "value")?;
    (name < value).then(|| {
        (
            parts[name + 1..value].join(" ").to_ascii_lowercase(),
            parts[value + 1..].join(" "),
        )
    })
}

/// FerrisChess is deliberately single-threaded until lazy SMP is implemented.
/// Clamp a syntactically valid request to the only supported worker count.
fn parse_thread_count(value: &str) -> Option<usize> {
    value
        .parse::<usize>()
        .ok()
        .filter(|threads| *threads > 0)
        .map(|_| 1)
}

/// A search in flight on a worker thread. The thread owns the [`Engine`] and
/// hands it back when it finishes; `control` is our clone of its stop/ponder handle.
struct RunningSearch {
    control: SearchHandle,
    worker: JoinHandle<Engine>,
}

/// Stops any running search and reclaims the engine the worker was holding. The
/// engine is threaded in and out this way (rather than shared) so it stays
/// single-owner and its TT/history persist across searches.
fn stop_search(running: &mut Option<RunningSearch>, engine: &mut Option<Engine>) {
    if let Some(search) = running.take() {
        search.control.stop();
        if let Ok(returned) = search.worker.join() {
            *engine = Some(returned);
        }
    }
}

/// Produces the iterative-deepening target and absolute deadline. Sudden-death
/// controls assume roughly 30 moves remain and allow slightly more emergency
/// headroom than an explicit moves-to-go control.
fn time_budget(
    remaining: Duration,
    increment: Duration,
    moves_to_go: Option<u32>,
) -> (Duration, Duration) {
    let overhead = Duration::from_millis(30);
    let safe = remaining
        .checked_sub(overhead)
        .unwrap_or(remaining / 4)
        .max(remaining / 4);
    let moves = moves_to_go.unwrap_or(30).clamp(1, 30);
    let mut soft = safe / moves + increment * 3 / 4;
    let mut hard = soft * if moves_to_go.is_some() { 4 } else { 5 };
    hard = hard.min(safe / 2);
    soft = soft.min(hard);
    hard = hard.max(Duration::from_millis(50));
    soft = soft.max(Duration::from_millis(30)).min(hard);
    (soft, hard)
}

fn has_starting_pieces(board: &Board) -> bool {
    let start = Board::startpos();
    board.side == Color::White
        && (0..64).all(|square| board.piece_at(square) == start.piece_at(square))
}

fn main() {
    // First, give the benchmark harness a chance to handle `bench ...` args.
    let args: Vec<_> = std::env::args().skip(1).collect();
    match chess::benchmark::run(&args) {
        Ok(true) => return,
        Ok(false) => {}
        Err(error) => {
            eprintln!("benchmark error: {error}");
            std::process::exit(2);
        }
    }
    // `engine` is `Some` when idle and `None` while the worker holds it.
    let mut board = Board::startpos();
    let mut move_history = Vec::<String>::new();
    let mut own_book = true;
    let mut engine = Some(Engine::new());
    let mut running: Option<RunningSearch> = None;
    let stdin = io::stdin();
    for line in stdin.lock().lines().map_while(Result::ok) {
        let parts: Vec<_> = line.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }
        // `stop` and `ponderhit` are handled up front because they must reach a
        // search that is currently running, not wait for the engine to be idle.
        if parts[0] == "stop" {
            stop_search(&mut running, &mut engine);
            continue;
        }
        if parts[0] == "ponderhit" {
            if let Some(search) = &running {
                search.control.ponderhit();
            }
            continue;
        }
        match parts[0] {
            "uci" => println!(
                "id name FerrisChess 0.1\nid author LarsSG and OpenAI\n{UCI_OPTIONS}\nuciok"
            ),
            "isready" => println!("readyok"),
            "ucinewgame" => {
                stop_search(&mut running, &mut engine);
                board = Board::startpos();
                move_history.clear();
                engine.as_mut().unwrap().clear()
            }
            // Parse advertised options; unknown options are silently ignored
            // as required by UCI.
            "setoption" => {
                stop_search(&mut running, &mut engine);
                if let Some((option, setting)) = parse_setoption(&parts) {
                    match option.as_str() {
                        "hash" => {
                            if let Ok(mb) = setting.parse::<usize>() {
                                engine.as_mut().unwrap().set_hash_mb(mb.clamp(1, 4096));
                            }
                        }
                        // The option is accepted for GUI and match-runner
                        // compatibility, but one remains the only worker count.
                        "threads" => {
                            let _ = parse_thread_count(&setting);
                        }
                        "ownbook" => own_book = setting.eq_ignore_ascii_case("true"),
                        _ => {}
                    }
                }
            }
            // `position [startpos | fen <FEN>] [moves <m>...]`: set the board,
            // then replay any listed moves from it.
            "position" => {
                stop_search(&mut running, &mut engine);
                engine.as_mut().unwrap().reset_game_history();
                move_history.clear();
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
                            engine.as_mut().unwrap().append_game_hash(board.hash);
                            board.make_move(m);
                            move_history.push(m.to_string());
                        }
                    }
                }
            }
            // `go ...`: parse the search limits, derive a time budget from the
            // clock if no explicit `movetime` was given, then launch the worker.
            "go" => {
                stop_search(&mut running, &mut engine);
                let mut limits = SearchLimits::default();
                let mut wtime: Option<u64> = None;
                let mut btime: Option<u64> = None;
                let mut winc = 0u64;
                let mut binc = 0u64;
                let mut moves_to_go: Option<u32> = None;
                let mut i = 1;
                while i < parts.len() {
                    match parts[i] {
                        "depth" => {
                            i += 1;
                            limits.depth = parts.get(i).and_then(|s| s.parse().ok()).unwrap_or(8)
                        }
                        "movetime" => {
                            i += 1;
                            let move_time = Duration::from_millis(
                                parts.get(i).and_then(|s| s.parse().ok()).unwrap_or(5000),
                            );
                            limits.time = move_time;
                            limits.soft_time = Some(move_time);
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
                                .map(|moves: u32| moves.max(1));
                        }
                        _ => {}
                    }
                    i += 1
                }
                // Budget the move from the side-to-move's remaining clock: a
                // share of the time (split over the expected moves-to-go) plus
                // most of the increment, kept below half the clock and above a
                // small floor, with a 30 ms safety margin against flagging.
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
                    let (soft, hard) = time_budget(
                        Duration::from_millis(left),
                        Duration::from_millis(inc),
                        moves_to_go,
                    );
                    limits.soft_time = Some(soft);
                    limits.time = hard;
                }
                limits.white_time = wtime.map(Duration::from_millis);
                limits.black_time = btime.map(Duration::from_millis);
                limits.white_increment = Duration::from_millis(winc);
                limits.black_increment = Duration::from_millis(binc);
                limits.moves_to_go = moves_to_go;
                // Book lookup precedes search, but never overrides an explicit
                // root restriction. The reply is parsed on the current board so
                // malformed repertoire data can never produce an illegal move.
                if own_book
                    && limits.root_moves.is_empty()
                    && (!move_history.is_empty() || has_starting_pieces(&board))
                    && let Some(chess_move) = book_move(&board, &move_history)
                {
                    println!("bestmove {chess_move}");
                    continue;
                }
                // Move the engine onto a worker thread; it prints `info` lines as
                // it searches and a final `bestmove`, then returns the engine.
                let control = SearchHandle::new(limits.ponder);
                let worker_control = control.clone();
                let search_board = board.clone();
                let mut search_engine = engine.take().unwrap();
                let worker = thread::spawn(move || {
                    let result = search_engine.search_with_handle(
                        &search_board,
                        limits,
                        worker_control,
                        |result| {
                            println!(
                                "info depth {} seldepth {} score cp {} nodes {} time {} nps {} hashfull {} pv {}",
                                result.depth,
                                result.seldepth,
                                result.score,
                                result.nodes,
                                result.elapsed.as_millis(),
                                result.nps,
                                result.hashfull,
                                result
                                    .pv
                                    .iter()
                                    .map(ToString::to_string)
                                    .collect::<Vec<_>>()
                                    .join(" ")
                            )
                        },
                    );
                    println!(
                        "bestmove {}",
                        result.best_move.map_or("0000".into(), |m| m.to_string())
                    );
                    search_engine
                });
                running = Some(RunningSearch { control, worker });
            }
            // `d`: a debug aid that dumps the current position as FEN.
            "d" => println!("{}", board.to_fen()),
            "quit" => {
                stop_search(&mut running, &mut engine);
                break;
            }
            _ => {}
        }
    }
    // Stdin closed: make sure no worker is left running.
    stop_search(&mut running, &mut engine);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn increment_and_moves_to_go_increase_the_soft_budget() {
        let remaining = Duration::from_secs(300);
        let (sudden_death, _) = time_budget(remaining, Duration::ZERO, None);
        let (with_increment, _) = time_budget(remaining, Duration::from_secs(3), None);
        let (five_moves, _) = time_budget(remaining, Duration::ZERO, Some(5));
        assert!(with_increment > sudden_death);
        assert!(five_moves > sudden_death);
    }

    #[test]
    fn time_budget_has_safe_floors_and_a_bounded_hard_limit() {
        let (soft, hard) = time_budget(Duration::from_millis(100), Duration::ZERO, None);
        assert!(soft > Duration::ZERO);
        assert!(soft <= hard);
        assert_eq!(hard, Duration::from_millis(50));

        let remaining = Duration::from_secs(300);
        let (_, hard) = time_budget(remaining, Duration::ZERO, None);
        assert!(hard <= remaining / 2);
    }

    #[test]
    fn setoption_parser_handles_names_and_values() {
        assert_eq!(
            parse_setoption(&["setoption", "name", "Own", "Book", "value", "true"]),
            Some(("own book".into(), "true".into()))
        );
        assert_eq!(parse_setoption(&["setoption", "name", "Hash"]), None);
    }

    #[test]
    fn threads_is_fixed_to_one_worker() {
        assert_eq!(parse_thread_count("1"), Some(1));
        assert_eq!(parse_thread_count("32"), Some(1));
        assert_eq!(parse_thread_count("0"), None);
        assert_eq!(parse_thread_count("many"), None);
        assert!(UCI_OPTIONS.contains("option name Threads type spin default 1 min 1 max 1"));
    }
}
