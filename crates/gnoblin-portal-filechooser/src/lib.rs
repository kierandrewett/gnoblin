//! The FileChooser dialog: a directory listing, a save name, a filter and the
//! confirm and cancel buttons.
//!
//! The model ([`FileChooser`]) decides what can be chosen. This crate draws it
//! with egui's stock widgets and reports the result.

use egui::{Key, ScrollArea, TextEdit, Ui};
use gnoblin_portal_core::model::{Confirmation, DialogError, DirectoryListing, FileChooser, FileChooserMode, Settled};
use gnoblin_portal_core::ui::{DialogUi, error_text};
use gnoblin_types::Tick;

/// A FileChooser on screen. Owns the model and the state only the view needs.
pub struct FileChooserUi {
    chooser: FileChooser,
    /// The folder the view asked for and is waiting to see listed.
    request: Option<String>,
    /// The last refused action, shown above the list until the next action.
    error: Option<DialogError>,
    /// Whether the first frame has taken focus for the save name.
    focused: bool,
}

impl FileChooserUi {
    /// A view of `chooser`, which asks for its starting folder to be listed.
    #[must_use]
    pub fn new(chooser: FileChooser) -> Self {
        let request = Some(chooser.folder().to_owned());
        Self {
            chooser,
            request,
            error: None,
            focused: false,
        }
    }

    /// The verb on the confirm button for this mode.
    const fn confirm_label(&self) -> &'static str {
        match self.chooser.mode() {
            FileChooserMode::Open => "Open",
            FileChooserMode::Save if self.chooser.overwrite_pending() => "Replace",
            FileChooserMode::Save => "Save",
            FileChooserMode::Folder => "Select",
        }
    }

    /// Confirms the chooser; an overwrite question stays on screen until answered.
    fn confirm(&mut self) -> Option<Settled> {
        match self.chooser.confirm() {
            Ok(Confirmation::Chosen(grant)) => {
                self.error = None;
                Some(Settled::Granted(grant))
            }
            Ok(Confirmation::OverwriteAsk { .. }) => {
                self.error = None;
                None
            }
            Err(refused) => {
                self.error = Some(refused);
                None
            }
        }
    }

    /// Enters a folder, or selects a file and confirms it.
    fn activate(&mut self, index: usize) -> Option<Settled> {
        let is_dir = self.chooser.entries().get(index).is_some_and(|entry| entry.is_dir);
        if !is_dir {
            if let Err(refused) = self.chooser.select(index, false) {
                self.error = Some(refused);
                return None;
            }
            return self.confirm();
        }
        match self.chooser.enter(index) {
            Ok(folder) => {
                self.error = None;
                self.request = Some(folder);
            }
            Err(refused) => self.error = Some(refused),
        }
        None
    }
}

impl DialogUi for FileChooserUi {
    fn show(&mut self, ui: &mut Ui, _now: Tick) -> Option<Settled> {
        if ui.input(|input| input.key_pressed(Key::Escape)) {
            return Some(Settled::Cancelled);
        }
        let enter = ui.input(|input| input.key_pressed(Key::Enter));
        let mut settled = None;

        ui.heading(self.chooser.folder());
        if let Some(refused) = self.error {
            ui.colored_label(ui.visuals().error_fg_color, error_text(refused));
        }
        if self.chooser.overwrite_pending() {
            ui.label(format!(
                "\u{201c}{}\u{201d} already exists. Replace it?",
                self.chooser.name()
            ));
        }

        // Rows are captured before the scroll area so the closure can mutate the model.
        let visible = self.chooser.visible();
        let selected = self.chooser.selection();
        let rows: Vec<(usize, String, bool)> = visible
            .iter()
            .map(|&index| {
                let entry = &self.chooser.entries()[index];
                let label = if entry.is_dir {
                    format!("{}/", entry.name)
                } else {
                    entry.name.clone()
                };
                (index, label, selected.contains(&index))
            })
            .collect();
        let row_height = ui.spacing().interact_size.y;
        let mut clicked = None;
        let mut double_clicked = None;
        ScrollArea::vertical()
            .max_height(row_height * 8.0)
            .auto_shrink([false, true])
            .show_rows(ui, row_height, rows.len(), |ui, range| {
                for (index, label, is_selected) in &rows[range] {
                    let response = ui.selectable_label(*is_selected, label);
                    if response.double_clicked() {
                        double_clicked = Some(*index);
                    } else if response.clicked() {
                        clicked = Some((*index, ui.input(|input| input.modifiers.ctrl)));
                    }
                }
            });
        if let Some((index, toggle)) = clicked {
            match self.chooser.select(index, toggle) {
                Ok(()) => self.error = None,
                Err(refused) => self.error = Some(refused),
            }
        }
        if let Some(index) = double_clicked {
            settled = self.activate(index);
        }

        let save = self.chooser.mode() == FileChooserMode::Save;
        let choose_filter = self.chooser.filters().len() > 1;
        let mut name_response = None;
        let mut next_filter = None;
        if save || choose_filter {
            // A grid puts the Name and Filter labels in one column, so their fields line up
            // with each other rather than with whatever width each label happens to have.
            egui::Grid::new("file-chooser-form").num_columns(2).show(ui, |ui| {
                if save {
                    ui.label("Name");
                    let mut name = self.chooser.name().to_owned();
                    let response = ui.add(TextEdit::singleline(&mut name));
                    if !self.focused {
                        response.request_focus();
                        self.focused = true;
                    }
                    if response.changed() {
                        self.chooser.set_name(name);
                    }
                    name_response = Some(response);
                    ui.end_row();
                }
                if choose_filter {
                    let current = self.chooser.filter_index();
                    let mut next = current;
                    let label = self.chooser.filters()[current].name.clone();
                    ui.label("Filter");
                    // An id salt, not a label: the label sits in the grid's own column.
                    egui::ComboBox::from_id_salt("file-chooser-filter")
                        .selected_text(label)
                        .show_ui(ui, |ui| {
                            for (index, filter) in self.chooser.filters().iter().enumerate() {
                                ui.selectable_value(&mut next, index, filter.name.as_str());
                            }
                        });
                    if next != current {
                        next_filter = Some(next);
                    }
                    ui.end_row();
                }
            });
        }
        if let Some(next) = next_filter {
            self.chooser.set_filter(next);
        }

        let typing = name_response.as_ref().is_some_and(egui::Response::has_focus);
        let enter_confirms = enter && (name_response.as_ref().is_some_and(egui::Response::lost_focus) || !typing);
        ui.horizontal(|ui| {
            if ui.button("Cancel").clicked() {
                settled = Some(Settled::Cancelled);
            }
            let confirm_clicked = ui.button(self.confirm_label()).clicked();
            if (confirm_clicked || enter_confirms) && settled.is_none() {
                settled = self.confirm();
            }
        });
        settled
    }

    fn listing_request(&self) -> Option<&str> {
        self.request.as_deref()
    }

    fn list(&mut self, listing: DirectoryListing) -> bool {
        if self.request.as_deref() != Some(listing.folder.as_str()) {
            return false;
        }
        self.request = None;
        self.chooser.set_entries(listing.entries);
        true
    }
}
