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
        ui.vertical_centered(|ui| {
            ui.add_space(24.0);
            ui.heading("FerrisChess");
            ui.add_space(24.0);

            ui.group(|ui| {
                ui.set_width(320.0);
                ui.label("Play as");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.side, SideChoice::White, "White");
                    ui.selectable_value(&mut self.side, SideChoice::Black, "Black");
                    ui.selectable_value(&mut self.side, SideChoice::Random, "Random");
                });
                ui.add_space(12.0);

                ui.label("Time control");
                ui.horizontal(|ui| {
                    for preset in [Preset::Bullet, Preset::Blitz, Preset::Rapid, Preset::Custom] {
                        ui.selectable_value(&mut self.preset, preset, preset.label());
                    }
                });
                if self.preset == Preset::Custom {
                    ui.horizontal(|ui| {
                        ui.label("Minutes:");
                        ui.add(egui::DragValue::new(&mut self.custom_minutes).range(1..=180));
                        ui.label("Increment (s):");
                        ui.add(egui::DragValue::new(&mut self.custom_increment).range(0..=60));
                    });
                }
                ui.add_space(12.0);

                ui.label("Difficulty");
                ui.horizontal(|ui| {
                    for level in Difficulty::ALL {
                        ui.selectable_value(&mut self.difficulty, level, level.label());
                    }
                });
                ui.add_space(12.0);

                ui.checkbox(&mut self.use_book, "Engine uses its opening book");
                ui.add_space(12.0);

                if ui
                    .add_sized([120.0, 32.0], egui::Button::new("Start game"))
                    .clicked()
                {
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
        });
        start
    }
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
