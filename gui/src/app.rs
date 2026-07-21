//! Top-level application: the setup screen, the game loop glue between the
//! UI and the engine worker, and the surrounding panels.

use crate::board_widget::BoardUi;
use crate::engine_driver::{EngineDriver, EngineEvent};
use crate::game::GameState;
use crate::setup::{GameConfig, SetupState, resolve_side};
use chess::{Color, SearchLimits, SearchResult, book, time_budget};
use std::time::Duration;

pub struct ChessApp {
    setup: SetupState,
    game: Option<GameState>,
    /// The config of the running game, kept so Rematch can replay it.
    config: Option<GameConfig>,
    engine: EngineDriver,
    board_ui: BoardUi,
    /// Latest search progress plus the side the engine was thinking for,
    /// needed to show scores from White's point of view.
    latest_info: Option<(Color, SearchResult)>,
}

impl ChessApp {
    pub fn new() -> Self {
        Self {
            setup: SetupState::default(),
            game: None,
            config: None,
            engine: EngineDriver::new(),
            board_ui: BoardUi::new(false),
            latest_info: None,
        }
    }

    fn start_game(&mut self, config: GameConfig, ctx: &egui::Context) {
        let human = resolve_side(config.side, ctx);
        self.engine.new_game();
        self.game = Some(GameState::new(human, config.time_control, config.use_book));
        self.config = Some(config);
        self.board_ui = BoardUi::new(human == Color::Black);
        self.latest_info = None;
    }

    fn show_setup(&mut self, ui: &mut egui::Ui) {
        let mut start = None;
        egui::CentralPanel::default().show(ui, |ui| {
            start = self.setup.show(ui);
        });
        if let Some(config) = start {
            let ctx = ui.ctx().clone();
            self.start_game(config, &ctx);
        }
    }

    fn show_game(&mut self, ui: &mut egui::Ui) {
        let Self {
            game,
            engine,
            board_ui,
            latest_info,
            ..
        } = self;
        let Some(game) = game.as_mut() else { return };

        for event in engine.poll() {
            match event {
                EngineEvent::Info(result) => {
                    let pov = game.human.other();
                    *latest_info = Some((pov, result));
                }
                EngineEvent::BestMove(Some(chess_move)) => game.play_move(chess_move, engine),
                // Only reachable if the game ended while the search wound
                // down; the stale-generation filter already drops that case.
                EngineEvent::BestMove(None) => {}
            }
        }
        if game.check_flag() {
            engine.abort();
        }
        start_engine_turn_if_due(game, engine);

        let mut back_to_setup = false;
        let mut rematch = false;

        egui::Panel::right("side_panel")
            .min_size(240.0)
            .show(ui, |ui| {
                ui.add_space(8.0);
                show_clocks(ui, game);
                ui.separator();
                show_turn_banner(ui, game, engine.searching());
                show_search_info(ui, latest_info.as_ref());
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("New game").clicked() {
                        back_to_setup = true;
                    }
                    if !game.status.is_over() && ui.button("Resign").clicked() {
                        game.resign();
                        engine.abort();
                    }
                    if ui.button("Flip board").clicked() {
                        board_ui.flipped = !board_ui.flipped;
                    }
                });
            });

        egui::CentralPanel::default().show(ui, |ui| {
            let interactive = !game.status.is_over() && game.board.side == game.human;
            ui.centered_and_justified(|ui| {
                if let Some(chess_move) =
                    board_ui.show(ui, &game.board, game.last_move, interactive)
                {
                    game.play_move(chess_move, engine);
                }
            });
        });

        if game.status.is_over() {
            egui::Window::new("Game over")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ui.ctx(), |ui| {
                    ui.label(game.status.describe(game.human));
                    ui.horizontal(|ui| {
                        if ui.button("Rematch").clicked() {
                            rematch = true;
                        }
                        if ui.button("New game").clicked() {
                            back_to_setup = true;
                        }
                    });
                });
        } else {
            // Keep clocks ticking and search progress flowing even when the
            // user isn't generating input events.
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }

        if back_to_setup {
            self.engine.abort();
            self.game = None;
        } else if rematch && let Some(config) = self.config.take() {
            let ctx = ui.ctx().clone();
            self.start_game(config, &ctx);
        }
    }
}

impl eframe::App for ChessApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.game.is_some() {
            self.show_game(ui);
        } else {
            self.show_setup(ui);
        }
    }
}

/// Kicks off the engine's reply — book first, otherwise a clock-budgeted
/// search — once it is the engine's turn and nothing is already running.
fn start_engine_turn_if_due(game: &mut GameState, engine: &mut EngineDriver) {
    if game.status.is_over() || game.board.side == game.human || engine.searching() {
        return;
    }
    if game.use_book
        && let Some(book_reply) = book::book_move(&game.board, &game.moves_uci)
    {
        game.play_move(book_reply, engine);
        return;
    }
    let engine_side = game.board.side;
    let (soft, hard) = time_budget(game.remaining(engine_side), game.increment, None);
    let limits = SearchLimits {
        soft_time: Some(soft),
        time: hard,
        ..Default::default()
    };
    engine.search(game.board.clone(), limits);
}

fn show_clocks(ui: &mut egui::Ui, game: &GameState) {
    let engine_side = game.human.other();
    for (label, color) in [("Engine", engine_side), ("You", game.human)] {
        let remaining = game.remaining(color);
        let ticking = !game.status.is_over() && game.board.side == color;
        let text = egui::RichText::new(format!("{label}  {}", format_clock(remaining)))
            .monospace()
            .size(20.0);
        ui.label(if ticking { text.strong() } else { text.weak() });
    }
}

fn show_turn_banner(ui: &mut egui::Ui, game: &GameState, searching: bool) {
    let text = if game.status.is_over() {
        game.status.describe(game.human)
    } else if game.board.side == game.human {
        "Your move".into()
    } else if searching {
        "Engine is thinking…".into()
    } else {
        "Engine to move".into()
    };
    ui.label(egui::RichText::new(text).heading());
}

fn show_search_info(ui: &mut egui::Ui, latest: Option<&(Color, SearchResult)>) {
    let Some((pov, info)) = latest else { return };
    let white_score = if *pov == Color::White {
        info.score
    } else {
        -info.score
    };
    let score_text = match info.score_type {
        chess::ScoreType::Centipawn => format!("{:+.2}", f64::from(white_score) / 100.0),
        chess::ScoreType::Mate => format!("mate {}", white_score),
    };
    ui.label(format!(
        "depth {}  score {score_text}  nodes {}",
        info.depth, info.nodes
    ));
}

fn format_clock(remaining: Duration) -> String {
    let total = remaining.as_secs();
    format!("{}:{:02}", total / 60, total % 60)
}
