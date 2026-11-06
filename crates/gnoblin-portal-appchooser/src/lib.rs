//! The AppChooser dialog: a filter, the application list, a remember checkbox
//! and the Cancel and Choose buttons.
//!
//! The model ([`AppChooser`]) decides what can be chosen. This crate draws it
//! with egui's stock widgets and reports the result.

use egui::{Key, ScrollArea, TextEdit, Ui};
use gnoblin_portal_core::model::{AppChooser, DialogError, Settled};
use gnoblin_portal_core::ui::{DialogUi, error_text};
use gnoblin_types::Tick;

/// An AppChooser on screen. Owns the model and the state only the view needs.
pub struct AppChooserUi {
    chooser: AppChooser,
    /// The desktop file id of the highlighted application.
    selected: Option<String>,
    remember: bool,
    /// The last refused choice, shown above the list until the next action.
    error: Option<DialogError>,
}

impl AppChooserUi {
    /// A view of `chooser`, with the last choice highlighted when it is offered.
    #[must_use]
    pub fn new(chooser: AppChooser) -> Self {
        let selected = chooser.last_choice().map(str::to_owned);
        Self {
            chooser,
            selected,
            remember: false,
            error: None,
        }
    }

    /// Chooses `desktop_file_id` through the model.
    fn choose(&mut self, desktop_file_id: &str) -> Option<Settled> {
        match self.chooser.choose(desktop_file_id, self.remember) {
            Ok(grant) => {
                self.error = None;
                Some(Settled::Granted(grant))
            }
            Err(refused) => {
                self.error = Some(refused);
                None
            }
        }
    }
}

impl DialogUi for AppChooserUi {
    fn show(&mut self, ui: &mut Ui, _now: Tick) -> Option<Settled> {
        if ui.input(|input| input.key_pressed(Key::Escape)) {
            return Some(Settled::Cancelled);
        }
        let enter = ui.input(|input| input.key_pressed(Key::Enter));
        let mut settled = None;

        ui.heading(format!("Choose an application for {}", self.chooser.content_type()));
        let mut text = self.chooser.filter_text().to_owned();
        if ui.add(TextEdit::singleline(&mut text).hint_text("Filter")).changed() {
            self.chooser.set_filter_text(text);
        }
        if let Some(refused) = self.error {
            ui.colored_label(ui.visuals().error_fg_color, error_text(refused));
        }

        // Rows are copied out so the scroll closure can borrow nothing from the model.
        let rows: Vec<(String, String)> = self
            .chooser
            .visible()
            .iter()
            .map(|choice| (choice.desktop_file_id.clone(), choice.name.clone()))
            .collect();
        // An empty list would otherwise look broken, so say why nothing is shown.
        if rows.is_empty() {
            ui.weak("No matching applications");
        }
        let highlighted = self.selected.clone();
        let row_height = ui.spacing().interact_size.y;
        let mut clicked = None;
        let mut double_clicked = None;
        ScrollArea::vertical()
            .max_height(row_height * 8.0)
            .auto_shrink([false, true])
            .show_rows(ui, row_height, rows.len(), |ui, range| {
                for (id, name) in &rows[range] {
                    let is_selected = highlighted.as_deref() == Some(id.as_str());
                    let response = ui.selectable_label(is_selected, name);
                    if response.double_clicked() {
                        double_clicked = Some(id.clone());
                    } else if response.clicked() {
                        clicked = Some(id.clone());
                    }
                }
            });
        if let Some(id) = clicked {
            self.selected = Some(id);
        }
        if let Some(id) = double_clicked {
            self.selected = Some(id.clone());
            settled = self.choose(&id);
        }

        ui.checkbox(&mut self.remember, "Remember this choice");
        ui.horizontal(|ui| {
            if ui.button("Cancel").clicked() {
                settled = Some(Settled::Cancelled);
            }
            let selected = self.selected.clone();
            let choose = ui.add_enabled(selected.is_some(), egui::Button::new("Choose"));
            if settled.is_none()
                && let Some(id) = selected
                && (choose.clicked() || enter)
            {
                settled = self.choose(&id);
            }
        });
        settled
    }
}
