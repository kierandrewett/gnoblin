//! Renders every portal dialog state to a PNG, so the built-in screens can be
//! looked at without a running session.
//!
//! `cargo run -p gnoblin-portal --example gallery -- <dir>` writes one PNG per
//! state into `<dir>`, creating it if needed. Each image is the 640x360 frame the
//! compositor opens a portal dialog in. The model has one palette (dark) and no
//! scale, so every screen is drawn once, at 1x, in the default theme.

use std::error::Error;
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use gnoblin_portal::{PortalScreen, Setup};
use gnoblin_portal_core::model::{
    ARM_DELAY_MS, AppChoice, AppChooser, Consent, Dialog, DirectoryListing, FileChooser, FileChooserMode, FileEntry,
    FileFilter, ScreenKey,
};
use gnoblin_portal_core::paint::Frame;
use gnoblin_types::Tick;

/// The frame size the compositor opens a portal dialog at (`ipc/portal.rs`).
const WIDTH: usize = 640;
const HEIGHT: usize = 360;

/// The folder every FileChooser shows.
const FOLDER: &str = "/home/u/Pictures";

/// A screen and the file name its image is written to, without `.png`.
type Shot = (&'static str, PortalScreen);

fn open_screen(dialog: Dialog, now: u64) -> PortalScreen {
    let setup = Setup {
        width: WIDTH,
        height: HEIGHT,
        reduced_motion: false,
    };
    PortalScreen::new(dialog, setup, Tick::new(now))
}

fn press(screen: &mut PortalScreen, keys: &[ScreenKey]) {
    for key in keys {
        screen.key(*key, false);
    }
}

fn consent(prompt: &str, now: u64) -> PortalScreen {
    open_screen(
        Dialog::Consent(Consent::open("org.example.Recorder", prompt, Tick::new(now))),
        now,
    )
}

/// Focus starts on Deny. Tab moves it to Allow, then `now` decides whether Allow is armed.
fn consent_allow_focused(now: u64) -> PortalScreen {
    let mut screen = consent("Share the contents of your screen?", 0);
    press(&mut screen, &[ScreenKey::Tab]);
    screen.tick(Tick::new(now));
    screen
}

fn file(name: &str, mime: &str) -> FileEntry {
    FileEntry {
        name: name.to_owned(),
        is_dir: false,
        mime: Some(mime.to_owned()),
    }
}

fn dir(name: &str) -> FileEntry {
    FileEntry {
        name: name.to_owned(),
        is_dir: true,
        mime: None,
    }
}

/// A realistic folder. Folders come first, as a file manager sorts them.
fn pictures() -> Vec<FileEntry> {
    vec![
        dir("Exports"),
        dir("Holiday 2025"),
        dir("Screenshots"),
        file("IMG_2041.png", "image/png"),
        file("beach-sunset.jpg", "image/jpeg"),
        file(
            "a-very-long-file-name-that-shows-how-the-list-truncates-long-names-v3-final.png",
            "image/png",
        ),
        file("receipt-march.pdf", "application/pdf"),
        file("notes.txt", "text/plain"),
    ]
}

fn filters() -> Vec<FileFilter> {
    vec![
        FileFilter {
            name: "Images".to_owned(),
            patterns: vec!["*.png".to_owned(), "*.jpg".to_owned()],
        },
        FileFilter {
            name: "All files".to_owned(),
            patterns: vec!["*".to_owned()],
        },
    ]
}

/// A FileChooser in `FOLDER` whose listing is already delivered.
fn chooser(mode: FileChooserMode, multiple: bool, name: &str, entries: Vec<FileEntry>) -> PortalScreen {
    let chooser = FileChooser::new(mode, multiple, FOLDER, filters(), name);
    let mut screen = open_screen(Dialog::Files(chooser), 0);
    assert!(
        screen.list(DirectoryListing {
            folder: FOLDER.to_owned(),
            entries,
        }),
        "the listing must match the folder"
    );
    screen
}

fn app_chooser(last: Option<&str>, filter_text: &str) -> PortalScreen {
    let choices = [
        ("org.gnome.Loupe", "Loupe"),
        ("org.gimp.GIMP", "GIMP"),
        ("org.inkscape.Inkscape", "Inkscape"),
        ("org.kde.gwenview", "Gwenview"),
        ("org.pinta.Pinta", "Pinta"),
        ("org.mozilla.firefox", "Firefox"),
    ]
    .into_iter()
    .map(|(id, name)| AppChoice {
        desktop_file_id: id.to_owned(),
        name: name.to_owned(),
    })
    .collect();
    let chooser = AppChooser::new("image/png", choices, last.map(str::to_owned), filter_text);
    open_screen(Dialog::App(chooser), 0)
}

fn shots() -> Vec<Shot> {
    // Consent: Allow ignores input until ARM_DELAY_MS has passed.
    let mut shots: Vec<Shot> = vec![("consent-default", consent("Share the contents of your screen?", 0))];
    shots.push(("consent-disarmed", consent_allow_focused(ARM_DELAY_MS - 1)));
    shots.push(("consent-armed", consent_allow_focused(ARM_DELAY_MS)));
    shots.push((
        "consent-long-prompt",
        consent(
            "Allow this application to record everything shown on every monitor, including windows you have not opened in this session?",
            0,
        ),
    ));

    // FileChooser, Open. Focus starts on the list; the cursor is on the first row.
    shots.push((
        "file-chooser-open",
        chooser(FileChooserMode::Open, false, "", pictures()),
    ));
    let mut multiple = chooser(FileChooserMode::Open, true, "", pictures());
    press(
        &mut multiple,
        &[
            ScreenKey::Down,
            ScreenKey::Down,
            ScreenKey::Down,
            ScreenKey::Space,
            ScreenKey::Down,
            ScreenKey::Space,
        ],
    );
    shots.push(("file-chooser-open-multiple", multiple));
    // Space on a folder in Open mode is refused: the error line shows why.
    let mut error = chooser(FileChooserMode::Open, false, "", pictures());
    press(&mut error, &[ScreenKey::Space]);
    shots.push(("file-chooser-open-error", error));
    // Tab moves to the filter; Down changes it to the second filter.
    let mut all_files = chooser(FileChooserMode::Open, false, "", pictures());
    press(&mut all_files, &[ScreenKey::Tab, ScreenKey::Down]);
    shots.push(("file-chooser-open-all-files", all_files));
    // Enter on the filter confirms with nothing selected: DialogError::NoSelection.
    let mut no_selection = chooser(FileChooserMode::Open, false, "", pictures());
    press(&mut no_selection, &[ScreenKey::Tab, ScreenKey::Enter]);
    shots.push(("file-chooser-open-no-selection", no_selection));
    shots.push((
        "file-chooser-open-empty",
        chooser(FileChooserMode::Open, false, "", Vec::new()),
    ));

    // FileChooser, Save: the name field, and the overwrite confirmation.
    shots.push((
        "file-chooser-save",
        chooser(FileChooserMode::Save, false, "holiday-edit.png", pictures()),
    ));
    // Tab moves from the list to the name; Enter confirms and the name exists.
    let mut overwrite = chooser(FileChooserMode::Save, false, "IMG_2041.png", pictures());
    press(&mut overwrite, &[ScreenKey::Tab, ScreenKey::Enter]);
    shots.push(("file-chooser-save-overwrite", overwrite));

    // FileChooser, Folder: the current folder is chosen when nothing is selected.
    shots.push((
        "file-chooser-folder",
        chooser(FileChooserMode::Folder, false, "", pictures()),
    ));

    // AppChooser: focus starts on the filter. The last choice is under the cursor.
    shots.push(("app-chooser", app_chooser(Some("org.gimp.GIMP"), "")));
    shots.push(("app-chooser-filtered", app_chooser(None, "in")));
    shots.push(("app-chooser-no-match", app_chooser(None, "zzz")));
    // Focus order is Filter, List, Remember: Tab, Tab, Space ticks "remember".
    let mut remember = app_chooser(None, "");
    press(&mut remember, &[ScreenKey::Tab, ScreenKey::Tab, ScreenKey::Space]);
    shots.push(("app-chooser-remember", remember));

    shots
}

/// Writes `frame` as an 8-bit RGBA PNG.
fn write_png(path: &Path, frame: &Frame) -> Result<(), Box<dyn Error>> {
    let writer = BufWriter::new(File::create(path)?);
    let mut encoder = png::Encoder::new(writer, u32::try_from(frame.width())?, u32::try_from(frame.height())?);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(frame.bytes())?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let dir = PathBuf::from(std::env::args_os().nth(1).ok_or("usage: gallery <output-dir>")?);
    std::fs::create_dir_all(&dir)?;
    for (name, mut screen) in shots() {
        let path = dir.join(format!("{name}.png"));
        write_png(&path, &screen.render())?;
        println!("{}", path.display());
    }
    Ok(())
}
