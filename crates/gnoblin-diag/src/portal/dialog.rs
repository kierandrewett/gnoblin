//! The built-in dialog models: pure state and events, with no drawing (D19 "UI step").
//!
//! Each model takes input and returns a decision or a confirmation. The
//! compositor draws whatever the model holds and feeds the input back in.

use std::collections::BTreeSet;

use gnoblin_types::Tick;

use super::interface::Grant;

/// Consent dialogs arm this many milliseconds after they appear (D19, proposed).
pub const ARM_DELAY_MS: u64 = 500;

/// A key the dialogs understand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Escape,
    Enter,
    Tab,
}

/// Where a pointer event landed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsentTarget {
    Allow,
    Deny,
    Elsewhere,
}

/// Which button has focus. Deny is the default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsentFocus {
    Deny,
    Allow,
}

/// Input to a consent dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsentInput {
    Key { key: Key, repeat: bool },
    PointerPress { target: ConsentTarget },
    PointerRelease { target: ConsentTarget },
}

/// The consent dialog's decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsentDecision {
    Allowed,
    Denied,
}

/// A consent dialog for one request. Allow needs one deliberate press and
/// release on Allow (or Enter on a focused Allow), after the arm delay. Deny
/// has focus, and Escape denies at any time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Consent {
    app_id: String,
    prompt: String,
    focus: ConsentFocus,
    armed_at: Tick,
    pressed_allow: bool,
    pressed_deny: bool,
    decision: Option<ConsentDecision>,
}

impl Consent {
    /// Opens a dialog at `now`. `app_id` is the compositor's attribution of the
    /// caller, never the request body.
    #[must_use]
    pub fn open(app_id: impl Into<String>, prompt: impl Into<String>, now: Tick) -> Self {
        Self {
            app_id: app_id.into(),
            prompt: prompt.into(),
            focus: ConsentFocus::Deny,
            armed_at: now.saturating_add(ARM_DELAY_MS),
            pressed_allow: false,
            pressed_deny: false,
            decision: None,
        }
    }

    /// The attributed app id shown as identity.
    #[must_use]
    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    /// The prompt text.
    #[must_use]
    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    /// The focused button.
    #[must_use]
    pub const fn focus(&self) -> ConsentFocus {
        self.focus
    }

    /// `true` once Allow accepts input.
    #[must_use]
    pub const fn armed(&self, now: Tick) -> bool {
        now.get() >= self.armed_at.get()
    }

    /// The decision, once made.
    #[must_use]
    pub const fn decision(&self) -> Option<ConsentDecision> {
        self.decision
    }

    /// Applies one input at `now`. Returns the decision the first time one is
    /// reached; after that the dialog is closed and input is ignored.
    pub fn input(&mut self, input: ConsentInput, now: Tick) -> Option<ConsentDecision> {
        if self.decision.is_some() {
            return None;
        }
        let armed = self.armed(now);
        let decided = match input {
            ConsentInput::Key { key: Key::Escape, .. } => Some(ConsentDecision::Denied),
            ConsentInput::Key {
                key: Key::Tab,
                repeat: _,
            } => {
                self.focus = match self.focus {
                    ConsentFocus::Deny => ConsentFocus::Allow,
                    ConsentFocus::Allow => ConsentFocus::Deny,
                };
                None
            }
            ConsentInput::Key {
                key: Key::Enter,
                repeat,
            } => match self.focus {
                ConsentFocus::Deny => Some(ConsentDecision::Denied),
                // Key repeat never counts as the gesture to allow.
                ConsentFocus::Allow if armed && !repeat => Some(ConsentDecision::Allowed),
                ConsentFocus::Allow => None,
            },
            ConsentInput::PointerPress { target } => {
                self.pressed_allow = target == ConsentTarget::Allow && armed;
                self.pressed_deny = target == ConsentTarget::Deny;
                None
            }
            ConsentInput::PointerRelease { target } => {
                let allowed = self.pressed_allow && target == ConsentTarget::Allow && armed;
                let denied = self.pressed_deny && target == ConsentTarget::Deny;
                self.pressed_allow = false;
                self.pressed_deny = false;
                if allowed {
                    Some(ConsentDecision::Allowed)
                } else if denied {
                    Some(ConsentDecision::Denied)
                } else {
                    None
                }
            }
        };
        self.decision = decided;
        decided
    }

    /// The grant a [`ConsentDecision::Allowed`] produces.
    #[must_use]
    pub const fn grant(decision: ConsentDecision) -> Option<Grant> {
        match decision {
            ConsentDecision::Allowed => Some(Grant::Allowed),
            ConsentDecision::Denied => None,
        }
    }
}

/// The FileChooser mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileChooserMode {
    Open,
    Save,
    Folder,
}

/// A named filter. A pattern is `*` (all), `*.ext` (a case-insensitive suffix)
/// or an exact MIME type matched against the entry's MIME.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileFilter {
    pub name: String,
    pub patterns: Vec<String>,
}

/// One entry in the directory listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
    pub mime: Option<String>,
}

/// A refused dialog action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogError {
    /// The save name is empty.
    EmptyName,
    /// The save name is `.`, `..`, or contains `/` or NUL.
    InvalidName,
    /// Nothing is selected.
    NoSelection,
    /// The selection holds something this mode cannot choose.
    NotSelectable,
    /// The save name names an existing directory.
    NotAFile,
    /// The index is outside the listing or outside the visible entries.
    OutOfRange,
    /// Several files were selected in a single-select dialog.
    TooMany,
    /// The app is not one the request offered.
    NotOffered,
}

/// What a FileChooser confirmation produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Confirmation {
    /// Save would replace this path: the user must confirm the overwrite.
    OverwriteAsk { path: String },
    /// The dialog is done with these paths and this filter.
    Chosen(Grant),
}

/// The FileChooser model: a directory listing, a selection, filters and the
/// overwrite confirmation. No thumbnails, search, bookmarks, rename or delete.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChooser {
    mode: FileChooserMode,
    multiple: bool,
    folder: String,
    filters: Vec<FileFilter>,
    filter: usize,
    entries: Vec<FileEntry>,
    selection: BTreeSet<usize>,
    name: String,
    overwrite_confirmed: bool,
}

impl FileChooser {
    /// Opens the dialog in `folder`, with the first filter in effect.
    #[must_use]
    pub fn new(
        mode: FileChooserMode,
        multiple: bool,
        folder: impl Into<String>,
        filters: Vec<FileFilter>,
        current_name: impl Into<String>,
    ) -> Self {
        Self {
            mode,
            multiple,
            folder: folder.into(),
            filters,
            filter: 0,
            entries: Vec::new(),
            selection: BTreeSet::new(),
            name: current_name.into(),
            overwrite_confirmed: false,
        }
    }

    /// The folder being listed.
    #[must_use]
    pub fn folder(&self) -> &str {
        &self.folder
    }

    /// The mode the dialog opened in.
    #[must_use]
    pub const fn mode(&self) -> FileChooserMode {
        self.mode
    }

    /// Whether several entries may be selected.
    #[must_use]
    pub const fn multiple(&self) -> bool {
        self.multiple
    }

    /// The filters offered, in order.
    #[must_use]
    pub fn filters(&self) -> &[FileFilter] {
        &self.filters
    }

    /// The index of the filter in effect.
    #[must_use]
    pub const fn filter_index(&self) -> usize {
        self.filter
    }

    /// The current listing, unfiltered.
    #[must_use]
    pub fn entries(&self) -> &[FileEntry] {
        &self.entries
    }

    /// The save name as typed.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether a save asked for an overwrite confirmation that is still unanswered.
    #[must_use]
    pub const fn overwrite_pending(&self) -> bool {
        self.overwrite_confirmed
    }

    /// Replaces the listing (after a navigation or a refresh) and clears the selection.
    pub fn set_entries(&mut self, entries: Vec<FileEntry>) {
        self.entries = entries;
        self.selection.clear();
    }

    /// Changes the filter. Returns `false` when the index is out of range.
    pub fn set_filter(&mut self, index: usize) -> bool {
        if index < self.filters.len() {
            self.filter = index;
            self.selection.clear();
            true
        } else {
            false
        }
    }

    /// The indices of the entries shown under the current filter. Folders always show.
    #[must_use]
    pub fn visible(&self) -> Vec<usize> {
        let filter = self.filters.get(self.filter);
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.is_dir || filter.is_none_or(|filter| filter_matches(filter, entry)))
            .map(|(index, _)| index)
            .collect()
    }

    /// The current selection, as listing indices.
    #[must_use]
    pub fn selection(&self) -> Vec<usize> {
        self.selection.iter().copied().collect()
    }

    /// Selects or toggles one visible entry. In Open mode only files, in Folder
    /// mode only folders, can be selected.
    ///
    /// # Errors
    ///
    /// [`DialogError::OutOfRange`] for an entry not shown, [`DialogError::NotSelectable`]
    /// for the wrong kind, and [`DialogError::TooMany`] when a second entry is
    /// selected in a single-select dialog.
    pub fn select(&mut self, index: usize, toggle: bool) -> Result<(), DialogError> {
        if !self.visible().contains(&index) {
            return Err(DialogError::OutOfRange);
        }
        let entry = &self.entries[index];
        let selectable = match self.mode {
            FileChooserMode::Folder => entry.is_dir,
            FileChooserMode::Open | FileChooserMode::Save => !entry.is_dir,
        };
        if !selectable {
            return Err(DialogError::NotSelectable);
        }
        if self.selection.contains(&index) {
            if toggle {
                self.selection.remove(&index);
            }
            return Ok(());
        }
        if !self.multiple && !toggle {
            self.selection.clear();
        }
        if !self.multiple && !self.selection.is_empty() {
            return Err(DialogError::TooMany);
        }
        self.selection.insert(index);
        Ok(())
    }

    /// Sets the save name. Any change clears a pending overwrite confirmation.
    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
        self.overwrite_confirmed = false;
    }

    /// Enters a folder entry. Returns the new folder path for the caller to list.
    ///
    /// # Errors
    ///
    /// [`DialogError::OutOfRange`] for an entry not shown; [`DialogError::NotSelectable`]
    /// when it is a file.
    pub fn enter(&mut self, index: usize) -> Result<String, DialogError> {
        if !self.visible().contains(&index) {
            return Err(DialogError::OutOfRange);
        }
        let entry = &self.entries[index];
        if !entry.is_dir {
            return Err(DialogError::NotSelectable);
        }
        self.folder = join(&self.folder, &entry.name);
        self.entries.clear();
        self.selection.clear();
        Ok(self.folder.clone())
    }

    /// Confirms the dialog. Save first returns [`Confirmation::OverwriteAsk`] when
    /// the name exists; a second confirm after that answers yes.
    ///
    /// # Errors
    ///
    /// Any [`DialogError`] for the mode's rules (no selection, invalid save name,
    /// or a directory named by a save).
    pub fn confirm(&mut self) -> Result<Confirmation, DialogError> {
        let filter = self.filter;
        match self.mode {
            FileChooserMode::Open => {
                if self.selection.is_empty() {
                    return Err(DialogError::NoSelection);
                }
                let paths = self
                    .selection
                    .iter()
                    .map(|index| join(&self.folder, &self.entries[*index].name))
                    .collect();
                Ok(Confirmation::Chosen(Grant::Files { paths, filter }))
            }
            FileChooserMode::Folder => {
                let path = match self.selection.iter().next() {
                    Some(index) if self.selection.len() == 1 => join(&self.folder, &self.entries[*index].name),
                    Some(_) => return Err(DialogError::TooMany),
                    None => self.folder.clone(),
                };
                Ok(Confirmation::Chosen(Grant::Files {
                    paths: vec![path],
                    filter,
                }))
            }
            FileChooserMode::Save => {
                validate_save_name(&self.name)?;
                let existing = self.entries.iter().find(|entry| entry.name == self.name);
                if existing.is_some_and(|entry| entry.is_dir) {
                    return Err(DialogError::NotAFile);
                }
                let path = join(&self.folder, &self.name);
                if existing.is_some() && !self.overwrite_confirmed {
                    self.overwrite_confirmed = true;
                    return Ok(Confirmation::OverwriteAsk { path });
                }
                Ok(Confirmation::Chosen(Grant::Files {
                    paths: vec![path],
                    filter,
                }))
            }
        }
    }
}

fn validate_save_name(name: &str) -> Result<(), DialogError> {
    if name.is_empty() {
        return Err(DialogError::EmptyName);
    }
    if name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        return Err(DialogError::InvalidName);
    }
    Ok(())
}

fn join(folder: &str, name: &str) -> String {
    if folder.ends_with('/') {
        format!("{folder}{name}")
    } else {
        format!("{folder}/{name}")
    }
}

fn filter_matches(filter: &FileFilter, entry: &FileEntry) -> bool {
    filter.patterns.iter().any(|pattern| {
        if pattern == "*" {
            return true;
        }
        if let Some(suffix) = pattern.strip_prefix("*.") {
            return entry
                .name
                .to_ascii_lowercase()
                .ends_with(&format!(".{}", suffix.to_ascii_lowercase()));
        }
        entry.mime.as_deref() == Some(pattern.as_str())
    })
}

/// One application the AppChooser offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppChoice {
    pub desktop_file_id: String,
    pub name: String,
}

/// The AppChooser model: a filtered list, and one choice with optional "remember".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppChooser {
    content_type: String,
    choices: Vec<AppChoice>,
    last_choice: Option<String>,
    filter_text: String,
}

impl AppChooser {
    /// Opens the chooser for `content_type` with the offered choices.
    #[must_use]
    pub fn new(
        content_type: impl Into<String>,
        choices: Vec<AppChoice>,
        last_choice: Option<String>,
        filter_text: impl Into<String>,
    ) -> Self {
        Self {
            content_type: content_type.into(),
            choices,
            last_choice,
            filter_text: filter_text.into(),
        }
    }

    /// The content type being opened.
    #[must_use]
    pub fn content_type(&self) -> &str {
        &self.content_type
    }

    /// The previous choice, pre-selected when still offered.
    #[must_use]
    pub fn last_choice(&self) -> Option<&str> {
        self.last_choice.as_deref()
    }

    /// Replaces the filter text. The caller types; the model only narrows the list.
    pub fn set_filter_text(&mut self, text: impl Into<String>) {
        self.filter_text = text.into();
    }

    /// The filter text as last set.
    #[must_use]
    pub fn filter_text(&self) -> &str {
        &self.filter_text
    }

    /// Every offered choice, unfiltered.
    #[must_use]
    pub fn choices(&self) -> &[AppChoice] {
        &self.choices
    }

    /// The choices whose name contains the filter text, case-insensitively.
    #[must_use]
    pub fn visible(&self) -> Vec<&AppChoice> {
        let needle = self.filter_text.to_lowercase();
        self.choices
            .iter()
            .filter(|choice| choice.name.to_lowercase().contains(&needle))
            .collect()
    }

    /// Picks one offered application.
    ///
    /// # Errors
    ///
    /// [`DialogError::NotOffered`] when `desktop_file_id` is not among the choices.
    pub fn choose(&self, desktop_file_id: &str, remember: bool) -> Result<Grant, DialogError> {
        if !self
            .choices
            .iter()
            .any(|choice| choice.desktop_file_id == desktop_file_id)
        {
            return Err(DialogError::NotOffered);
        }
        Ok(Grant::App {
            desktop_file_id: desktop_file_id.to_owned(),
            remember,
        })
    }
}
