use crate::{Board, Engine, Move, SearchLimits};
use std::{fs, path::Path, time::Instant};

const PERFT: &[(&str, &str, u8, u64)] = &[
    (
        "startpos",
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
        5,
        4_865_609,
    ),
    (
        "kiwipete",
        "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        4,
        4_085_603,
    ),
    (
        "position-3",
        "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
        6,
        11_030_083,
    ),
];

const SEARCH: &[(&str, &str)] = &[
    (
        "startpos",
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
    ),
    (
        "kiwipete",
        "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
    ),
    (
        "middlegame",
        "r2q1rk1/pp2bppp/2n1pn2/2pp4/3P4/2PBPN2/PPQ2PPP/R1B2RK1 w - - 2 10",
    ),
];

pub fn run(args: &[String]) -> Result<bool, String> {
    if args.first().map(String::as_str) != Some("bench") {
        return Ok(false);
    }
    match args.get(1).map(String::as_str).unwrap_or("search") {
        "perft" => bench_perft()?,
        "search" => bench_search(parse_depth(args.get(2))?)?,
        "bk" => bench_epd(
            Path::new("bench/data/bratko-kopec.epd"),
            parse_depth(args.get(2))?,
        )?,
        "wac" => bench_epd(Path::new("bench/data/wac.epd"), parse_depth(args.get(2))?)?,
        "epd" => {
            let path = args.get(2).ok_or("usage: chess bench epd FILE [DEPTH]")?;
            bench_epd(Path::new(path), parse_depth(args.get(3))?)?
        }
        other => return Err(format!("unknown benchmark {other}")),
    }
    Ok(true)
}

fn parse_depth(value: Option<&String>) -> Result<u8, String> {
    value.map_or(Ok(6), |depth| {
        depth.parse().map_err(|_| format!("invalid depth {depth}"))
    })
}

fn bench_perft() -> Result<(), String> {
    let started = Instant::now();
    let mut total = 0u64;
    for &(name, fen, depth, expected) in PERFT {
        let nodes = Board::from_fen(fen)?.perft(depth);
        if nodes != expected {
            return Err(format!("{name}: expected {expected}, got {nodes}"));
        }
        total += nodes;
        println!("perft {name} depth {depth} nodes {nodes}");
    }
    let elapsed = started.elapsed();
    println!(
        "perft total nodes {total} time_ms {} nps {}",
        elapsed.as_millis(),
        total * 1000 / (elapsed.as_millis() as u64).max(1)
    );
    Ok(())
}

fn bench_search(depth: u8) -> Result<(), String> {
    let mut total_nodes = 0u64;
    let started = Instant::now();
    for &(name, fen) in SEARCH {
        let board = Board::from_fen(fen)?;
        let mut engine = Engine::new();
        let result = engine.search(
            &board,
            SearchLimits {
                depth,
                infinite: true,
                ..SearchLimits::default()
            },
            |_| {},
        );
        total_nodes += result.nodes;
        println!(
            "search {name} depth {} score {} nodes {} bestmove {} pv {}",
            result.depth,
            result.score,
            result.nodes,
            result.best_move.map_or("0000".into(), |m| m.to_string()),
            format_pv(&result.pv)
        );
    }
    let elapsed = started.elapsed();
    println!(
        "search total nodes {total_nodes} time_ms {} nps {}",
        elapsed.as_millis(),
        total_nodes * 1000 / (elapsed.as_millis() as u64).max(1)
    );
    Ok(())
}

fn bench_epd(path: &Path, depth: u8) -> Result<(), String> {
    let input = fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut solved = 0usize;
    let mut tested = 0usize;
    let mut total_nodes = 0u64;
    for (line_number, line) in input.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('%') {
            continue;
        }
        let (board, expected, id) = parse_epd(line)
            .map_err(|error| format!("{}:{}: {error}", path.display(), line_number + 1))?;
        let mut engine = Engine::new();
        let result = engine.search(
            &board,
            SearchLimits {
                depth,
                infinite: true,
                ..SearchLimits::default()
            },
            |_| {},
        );
        let hit = result
            .best_move
            .is_some_and(|best| expected.contains(&best));
        solved += usize::from(hit);
        tested += 1;
        total_nodes += result.nodes;
        println!(
            "epd {id} {} bestmove {} nodes {}",
            if hit { "ok" } else { "miss" },
            result.best_move.map_or("0000".into(), |m| m.to_string()),
            result.nodes
        );
    }
    println!("epd score {solved}/{tested} depth {depth} nodes {total_nodes}");
    Ok(())
}

fn parse_epd(line: &str) -> Result<(Board, Vec<Move>, String), String> {
    let fields: Vec<_> = line.split_whitespace().collect();
    if fields.len() < 6 {
        return Err("EPD requires a position and bm operation".into());
    }
    let board = Board::from_fen(&format!(
        "{} {} {} {} 0 1",
        fields[0], fields[1], fields[2], fields[3]
    ))?;
    let bm = line.find(" bm ").ok_or("missing bm operation")? + 4;
    let best_text = line[bm..].split(';').next().ok_or("empty bm operation")?;
    let expected: Vec<_> = best_text
        .split_whitespace()
        .filter_map(|text| board.parse_move(text).or_else(|| board.parse_san(text)))
        .collect();
    if expected.is_empty() {
        return Err(format!("no recognized best moves in {best_text:?}"));
    }
    let id = line
        .find(" id ")
        .and_then(|start| line[start + 4..].split(';').next())
        .unwrap_or("unnamed")
        .trim_matches([' ', '"'])
        .to_owned();
    Ok((board, expected, id))
}

fn format_pv(pv: &[Move]) -> String {
    pv.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_epd_best_moves() {
        let (_, moves, id) = parse_epd(
            "2rr3k/pp3pp1/1nnqbN1p/3pN3/2pP4/2P3Q1/PPB4P/R4RK1 w - - bm Qg6; id \"WAC.001\";",
        )
        .unwrap();
        assert_eq!(moves[0].to_string(), "g3g6");
        assert_eq!(id, "WAC.001");
    }
}
