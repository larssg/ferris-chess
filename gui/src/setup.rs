//! The pre-game screen: pick a side, a time control, a difficulty, and
//! whether the engine may use its opening book.

use crate::game::TimeControl;
use chess::Color;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SideChoice {
    White,
    Black,
    Random,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Preset {
    Bullet,
    Blitz,
    Rapid,
    Custom,
}

impl Preset {
    fn label(self) -> &'static str {
        match self {
            Preset::Bullet => "Bullet 1+0",
            Preset::Blitz => "Blitz 3+2",
            Preset::Rapid => "Rapid 10+5",
            Preset::Custom => "Custom",
        }
    }

    fn minutes_and_increment(self) -> Option<(u32, u32)> {
        match self {
            Preset::Bullet => Some((1, 0)),
            Preset::Blitz => Some((3, 2)),
            Preset::Rapid => Some((10, 5)),
            Preset::Custom => None,
        }
    }
}

/// How strong an opponent the engine should be. Weaker levels cap the search
/// depth, which is how most engines throttle themselves: a shallow search
/// still plays natural-looking moves but overlooks deeper tactics.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Difficulty {
    Easy,
    Medium,
    Hard,
    Full,
}

impl Difficulty {
    pub const ALL: [Difficulty; 4] = [
        Difficulty::Easy,
        Difficulty::Medium,
        Difficulty::Hard,
        Difficulty::Full,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Difficulty::Easy => "Easy",
            Difficulty::Medium => "Medium",
            Difficulty::Hard => "Hard",
            Difficulty::Full => "Full strength",
        }
    }

    /// The search-depth cap for this level; `None` lets the engine search on
    /// its clock budget alone.
    pub fn depth_limit(self) -> Option<u8> {
        match self {
            Difficulty::Easy => Some(1),
            Difficulty::Medium => Some(3),
            Difficulty::Hard => Some(6),
            Difficulty::Full => None,
        }
    }
}

/// Everything the app needs to start a game; `side` may still be Random,
/// which the app resolves with a coin flip at start time.
pub struct GameConfig {
    pub side: SideChoice,
    pub time_control: TimeControl,
    pub difficulty: Difficulty,
    pub use_book: bool,
}

pub struct SetupState {
    side: SideChoice,
    preset: Preset,
    custom_minutes: u32,
    custom_increment: u32,
    difficulty: Difficulty,
    use_book: bool,
}

impl Default for SetupState {
    fn default() -> Self {
        Self {
            side: SideChoice::White,
            preset: Preset::Rapid,
            custom_minutes: 15,
            custom_increment: 10,
            difficulty: Difficulty::Medium,
            use_book: true,
        }
    }
}

impl SetupState {
    /// Draws the setup form; returns a config once Start is clicked.
    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<GameConfig> {
        let mut start = None;
        let ink = egui::Color32::from_rgb(42, 47, 43);
        let muted = egui::Color32::from_rgb(112, 116, 108);
        let green = egui::Color32::from_rgb(65, 111, 87);

        ui.vertical_centered(|ui| {
            ui.add_space(30.0);
            ui.label(
                egui::RichText::new("FERRISCHESS")
                    .size(12.0)
                    .strong()
                    .color(green),
            );
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new("Your next game awaits")
                    .size(30.0)
                    .strong()
                    .color(ink),
            );
            ui.label(
                egui::RichText::new("Choose your match settings and take a seat at the board.")
                    .size(14.0)
                    .color(muted),
            );
            ui.add_space(22.0);

            let card_width = ui.available_width().min(500.0);
            egui::Frame::new()
                .fill(egui::Color32::from_rgb(253, 252, 249))
                .stroke(egui::Stroke::new(
                    1.0,
                    egui::Color32::from_rgb(219, 216, 207),
                ))
                .corner_radius(14.0)
                .inner_margin(egui::Margin::symmetric(28, 24))
                .show(ui, |ui| {
                    ui.set_width(card_width - 56.0);
                    section_heading(ui, "YOUR SIDE", "Which pieces would you like?");
                    ui.add_space(10.0);
                    ui.columns(3, |columns| {
                        choice_button(&mut columns[0], &mut self.side, SideChoice::White, "White");
                        choice_button(&mut columns[1], &mut self.side, SideChoice::Black, "Black");
                        choice_button(
                            &mut columns[2],
                            &mut self.side,
                            SideChoice::Random,
                            "Surprise me",
                        );
                    });
                    ui.add_space(22.0);

                    section_heading(ui, "TIME CONTROL", "How quickly should the game unfold?");
                    ui.add_space(10.0);
                    ui.columns(4, |columns| {
                        for (column, preset) in columns.iter_mut().zip([
                            Preset::Bullet,
                            Preset::Blitz,
                            Preset::Rapid,
                            Preset::Custom,
                        ]) {
                            let selected = self.preset == preset;
                            if option_button(column, selected, preset.label()).clicked() {
                                self.preset = preset;
                            }
                        }
                    });
                    if self.preset == Preset::Custom {
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("Minutes").color(muted));
                            ui.add(egui::DragValue::new(&mut self.custom_minutes).range(1..=180));
                            ui.add_space(12.0);
                            ui.label(egui::RichText::new("Increment").color(muted));
                            ui.add(egui::DragValue::new(&mut self.custom_increment).range(0..=60));
                            ui.label(egui::RichText::new("seconds").color(muted));
                        });
                    }
                    ui.add_space(22.0);

                    section_heading(ui, "OPPONENT", "Choose how far ahead FerrisChess looks.");
                    ui.add_space(10.0);
                    ui.columns(4, |columns| {
                        for (column, level) in columns.iter_mut().zip(Difficulty::ALL) {
                            let selected = self.difficulty == level;
                            if option_button(column, selected, level.label()).clicked() {
                                self.difficulty = level;
                            }
                        }
                    });
                    ui.add_space(20.0);

                    egui::Frame::new()
                        .fill(egui::Color32::from_rgb(244, 247, 242))
                        .corner_radius(8.0)
                        .inner_margin(egui::Margin::symmetric(12, 9))
                        .show(ui, |ui| {
                            ui.checkbox(
                                &mut self.use_book,
                                "Use the opening book for natural early play",
                            );
                        });
                    ui.add_space(20.0);

                    let button = egui::Button::new(
                        egui::RichText::new("Start game")
                            .size(16.0)
                            .strong()
                            .color(egui::Color32::WHITE),
                    )
                    .fill(green)
                    .stroke(egui::Stroke::NONE)
                    .corner_radius(8.0);
                    if ui.add_sized([ui.available_width(), 44.0], button).clicked() {
                        let (minutes, increment) = self
                            .preset
                            .minutes_and_increment()
                            .unwrap_or((self.custom_minutes, self.custom_increment));
                        start = Some(GameConfig {
                            side: self.side,
                            time_control: TimeControl {
                                base: Duration::from_secs(u64::from(minutes) * 60),
                                increment: Duration::from_secs(u64::from(increment)),
                            },
                            difficulty: self.difficulty,
                            use_book: self.use_book,
                        });
                    }
                });
            ui.add_space(24.0);
        });
        start
    }
}

fn section_heading(ui: &mut egui::Ui, title: &str, description: &str) {
    ui.label(
        egui::RichText::new(title)
            .size(12.0)
            .strong()
            .color(egui::Color32::from_rgb(65, 111, 87)),
    );
    ui.label(
        egui::RichText::new(description)
            .size(13.0)
            .color(egui::Color32::from_rgb(112, 116, 108)),
    );
}

fn choice_button<T: PartialEq + Copy>(ui: &mut egui::Ui, value: &mut T, choice: T, label: &str) {
    if option_button(ui, *value == choice, label).clicked() {
        *value = choice;
    }
}

fn option_button(ui: &mut egui::Ui, selected: bool, label: &str) -> egui::Response {
    let (fill, stroke, text) = if selected {
        (
            egui::Color32::from_rgb(226, 237, 229),
            egui::Color32::from_rgb(65, 111, 87),
            egui::Color32::from_rgb(45, 83, 63),
        )
    } else {
        (
            egui::Color32::from_rgb(248, 247, 243),
            egui::Color32::from_rgb(218, 216, 208),
            egui::Color32::from_rgb(65, 68, 64),
        )
    };
    let button = egui::Button::new(egui::RichText::new(label).size(14.0).color(text))
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, stroke))
        .corner_radius(7.0);
    ui.add_sized([ui.available_width(), 38.0], button)
}

/// Resolves a Random side choice with the low bit of egui's frame clock at
/// the instant Start was clicked — sub-microsecond human timing is a fair
/// enough coin, needs no rand dependency, and works on wasm.
pub fn resolve_side(choice: SideChoice, ctx: &egui::Context) -> Color {
    match choice {
        SideChoice::White => Color::White,
        SideChoice::Black => Color::Black,
        SideChoice::Random => {
            if ctx.input(|i| i.time).to_bits() & 1 == 0 {
                Color::White
            } else {
                Color::Black
            }
        }
    }
}
