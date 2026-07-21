//! Scrollable SAN move history, shown in numbered White/Black pairs.

pub fn show_move_list(ui: &mut egui::Ui, sans: &[String]) {
    egui::ScrollArea::vertical()
        .stick_to_bottom(true)
        .max_height(ui.available_height() - 48.0)
        .show(ui, |ui| {
            egui::Grid::new("move_list")
                .striped(true)
                .min_col_width(48.0)
                .show(ui, |ui| {
                    for (index, pair) in sans.chunks(2).enumerate() {
                        ui.label(format!("{}.", index + 1));
                        ui.label(&pair[0]);
                        if let Some(black) = pair.get(1) {
                            ui.label(black);
                        }
                        ui.end_row();
                    }
                });
        });
}
