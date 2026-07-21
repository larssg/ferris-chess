//! Top-level application: the setup screen, the game loop glue between the
//! UI and the engine worker, and the surrounding panels.

use crate::board_widget::BoardUi;
use crate::clock::show_clocks;
use crate::engine_driver::{EngineDriver, EngineEvent};
use crate::game::GameState;
use crate::input::MoveInput;
use crate::movelist::show_move_list;
use crate::setup::{GameConfig, SetupState, resolve_side};
use chess::{Color, ScoreType, SearchLimits, SearchResult, book, time_budget};
use std::time::Duration;

pub struct ChessApp {
    setup: SetupState,
    game: Option<GameState>,
    /// The config of the running game, kept so Rematch can replay it.
    config: Option<GameConfig>,
    engine: EngineDriver,
    board_ui: BoardUi,
    move_input: MoveInput,
    latest_info: Option<SearchDisplay>,
}

/// Search progress pre-rendered for display. Built the moment an info event
/// arrives, while the board still holds the position that was searched —
/// afterwards the PV could no longer be replayed into SAN.
struct SearchDisplay {
    summary: String,
    pv: String,
    /// White's expected share of the point, for the eval bar.
    white_share: f32,
}

impl ChessApp {
    pub fn new() -> Self {
        Self {
            setup: SetupState::default(),
            game: None,
            config: None,
            engine: EngineDriver::new(),
            board_ui: BoardUi::new(false),
            move_input: MoveInput::default(),
            latest_info: None,
        }
    }

    fn start_game(&mut self, config: GameConfig, ctx: &egui::Context) {
        let human = resolve_side(config.side, ctx);
        self.engine.new_game();
        self.game = Some(GameState::new(human, config.time_control, config.use_book));
        self.config = Some(config);
        self.board_ui = BoardUi::new(human == Color::Black);
        self.move_input = MoveInput::default();
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
            move_input,
            latest_info,
            ..
        } = self;
        let Some(game) = game.as_mut() else { return };

        for event in engine.poll() {
            match event {
                EngineEvent::Info(result) => {
                    *latest_info = Some(build_search_display(game, &result));
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
        let mut input_focused = false;

        egui::Panel::right("side_panel")
            .min_size(260.0)
            .show(ui, |ui| {
                ui.add_space(8.0);
                show_clocks(ui, game);
                ui.separator();
                show_turn_banner(ui, game, engine.searching());
                if let Some(info) = latest_info.as_ref() {
                    ui.label(egui::RichText::new(&info.summary).monospace().small());
                    ui.add(egui::Label::new(egui::RichText::new(&info.pv).weak().small()).wrap());
                }
                ui.separator();
                let human_turn = !game.status.is_over() && game.board.side == game.human;
                let (typed, focused) = move_input.show(ui, &game.board, human_turn);
                input_focused = focused;
                if let Some(chess_move) = typed {
                    game.play_move(chess_move, engine);
                }
                ui.add_space(4.0);
                show_move_list(ui, &game.sans);
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
                ui.add_space(8.0);
            });

        let white_share = latest_info.as_ref().map_or(0.5, |info| info.white_share);
        egui::CentralPanel::default().show(ui, |ui| {
            let interactive = !game.status.is_over() && game.board.side == game.human;
            ui.horizontal_centered(|ui| {
                let bar_width = 16.0;
                let bar_height = (ui
                    .available_height()
                    .min(ui.available_width() - bar_width - ui.spacing().item_spacing.x)
                    / 8.0)
                    .floor()
                    .max(24.0)
                    * 8.0;
                draw_eval_bar(ui, egui::Vec2::new(bar_width, bar_height), white_share);
                if let Some(chess_move) =
                    board_ui.show(ui, &game.board, game.last_move, interactive, !input_focused)
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
                    ui.label(egui::RichText::new(game.status.describe(game.human)).heading());
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Rematch").clicked() {
                            rematch = true;
                        }
                        if ui.button("New game").clicked() {
                            back_to_setup = true;
                        }
                    });
                });
        } else if engine.searching() {
            // Repaint immediately while thinking: on wasm the driver searches
            // in per-frame slices, so idle gaps would waste the engine's clock.
            ui.ctx().request_repaint();
        } else {
            // Keep clocks ticking even when the user isn't generating input.
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

/// Renders one search info event into display strings. Scores arrive from
/// the side-to-move's perspective and are flipped to White's point of view.
fn build_search_display(game: &GameState, info: &SearchResult) -> SearchDisplay {
    let white_score = if game.board.side == Color::White {
        info.score
    } else {
        -info.score
    };
    let (score_text, white_share) = match info.score_type {
        ScoreType::Centipawn => (
            format!("{:+.2}", f64::from(white_score) / 100.0),
            expected_score(white_score),
        ),
        ScoreType::Mate => {
            let moves_to_mate = (chess::engine::MATE - info.score.abs() + 1) / 2;
            let signed = if white_score > 0 {
                moves_to_mate
            } else {
                -moves_to_mate
            };
            (
                format!("#{signed}"),
                if white_score > 0 { 1.0 } else { 0.0 },
            )
        }
    };
    let summary = format!(
        "d{}/{}  {score_text}  {}kn  {}kn/s",
        info.depth,
        info.seldepth,
        info.nodes / 1000,
        info.nps / 1000,
    );
    SearchDisplay {
        summary,
        pv: pv_to_san(game, &info.pv),
        white_share,
    }
}

/// The logistic curve commonly used to map centipawns to an expected score.
fn expected_score(white_centipawns: i32) -> f32 {
    1.0 / (1.0 + 10f32.powf(-white_centipawns as f32 / 400.0))
}

/// Replays the PV on a scratch board to render it in SAN. Stops early if a
/// stale transposition-table tail turns out not to be legal.
fn pv_to_san(game: &GameState, pv: &[chess::Move]) -> String {
    let mut board = game.board.clone();
    let mut rendered = Vec::with_capacity(pv.len());
    for chess_move in pv {
        let Some(verified) = board.parse_move(&chess_move.to_string()) else {
            break;
        };
        rendered.push(board.san(verified));
        board.make_move(verified);
    }
    rendered.join(" ")
}

fn draw_eval_bar(ui: &mut egui::Ui, size: egui::Vec2, white_share: f32) {
    let (response, painter) = ui.allocate_painter(size, egui::Sense::hover());
    let rect = response.rect;
    painter.rect_filled(rect, 3.0, egui::Color32::from_gray(35));
    let share = white_share.clamp(0.0, 1.0);
    let white_rect = egui::Rect::from_min_max(
        egui::pos2(rect.left(), rect.bottom() - rect.height() * share),
        rect.right_bottom(),
    );
    painter.rect_filled(white_rect, 3.0, egui::Color32::from_gray(230));
}
