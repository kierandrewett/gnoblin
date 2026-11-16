//! `--list-shells`: one row for every shell in every tier, in precedence order, with the one that would run marked.
//!
//! ```text
//! NAME     TIER  LANGUAGE  DEFAULT   LOCATION
//! tiler    5     native    yes       /home/k/.local/share/gnoblin/shells/tiler.toml -> /opt/tiler/libtiler.so
//! lines    6     python              /usr/share/gnoblin/shells/lines.py
//! default  7     ts                  /usr/share/gnoblin/default-shell/shell.ts  (packaged)
//! ```
//!
//! The same name in an earlier tier shadows a later one; the shadowed copy is listed and marked `shadowed`. The row
//! model is data ([`Row`]); [`Listing::render`] is the one text layout, so the command and its golden tests agree.

use std::path::{Path, PathBuf};

use crate::candidate::{Candidate, PathOutcome, candidates_at, conventional_candidates};
use crate::discover::{Context, Selection, Warning, discover, nameable_places, scan_place};
use crate::environment::Environment;
use crate::error::DiscoveryError;
use crate::language::Language;
use crate::tier::{Request, Source, Tier};

/// One shell in the listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// The name `--shell NAME` takes.
    pub name: String,
    /// The tier it was found in.
    pub tier: Tier,
    /// How it is loaded, or `None` when it cannot be used.
    pub language: Option<Language>,
    /// What was found: the file, the directory, the launcher or the link.
    pub location: PathBuf,
    /// The file a launcher or a directory leads to, when it is not the location itself.
    pub target: Option<PathBuf>,
    /// The shell that would run now.
    pub is_default: bool,
    /// An earlier tier has a shell of this name, so this one is never picked by name.
    pub shadowed: bool,
    /// The packaged default.
    pub packaged: bool,
    /// Why the shell cannot be used, one line.
    pub problem: Option<String>,
}

/// What `--list-shells` prints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    /// Every shell found, in precedence order.
    pub rows: Vec<Row>,
    /// What the lookup would select now, or why it would not. A listing is not an error when the lookup is.
    pub selected: Result<Selection, DiscoveryError>,
    /// Things worth telling the user: `config.toml` keys that are not known.
    pub warnings: Vec<Warning>,
}

/// Every shell of every tier, and the one the lookup would pick for `request`.
pub fn list_shells(environment: &Environment, request: &Request) -> Listing {
    let context = Context::new(environment);
    let selected = discover(environment, request);
    let mut candidates: Vec<Candidate> = Vec::new();

    // Tiers 1 to 3 name a path; a name resolves to a shell that tiers 5 to 7 list.
    let mut explicit = |given: &Path, source: Source| {
        if let Some(resolved) = context.resolve_path(given, &source)
            && let PathOutcome::Found(found) = candidates_at(&resolved, source.tier(), environment)
        {
            candidates.extend(found);
        }
    };
    if let Request::Path(path) = request {
        explicit(path, Source::CommandLine);
    }
    if let Some(value) = environment
        .gnoblin_shell
        .as_ref()
        .filter(|value| value.as_encoded_bytes().contains(&b'/'))
    {
        explicit(Path::new(value), Source::Environment);
    }
    if let (Some(config), Some(config_path)) = (context.shell_config(), context.config_path())
        && let Some(file) = &config.path
    {
        explicit(Path::new(file), Source::Config(config_path.to_path_buf()));
    }

    if let Some(directory) = environment.config_dir() {
        candidates.extend(conventional_candidates(&directory));
    }
    for place in nameable_places(environment) {
        candidates.extend(scan_place(&place, environment).unwrap_or_default());
    }

    let mut seen: Vec<String> = Vec::new();
    let mut rows = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let nameable = candidate.tier.holds_named_shells();
        let shadowed = nameable && seen.contains(&candidate.name);
        if nameable {
            seen.push(candidate.name.clone());
        }
        let is_default = selected
            .as_ref()
            .is_ok_and(|selection| selection.found_in == candidate.tier && selection.location == candidate.location);
        rows.push(Row {
            language: candidate.language().cloned(),
            target: candidate
                .path()
                .filter(|path| *path != candidate.location)
                .map(Path::to_path_buf),
            problem: candidate.target.as_ref().err().cloned(),
            packaged: candidate.tier == Tier::Packaged,
            is_default,
            shadowed,
            name: candidate.name,
            tier: candidate.tier,
            location: candidate.location,
        });
    }
    let warnings = selected
        .as_ref()
        .map_or_else(|_| Vec::new(), |selection| selection.warnings.clone());
    Listing {
        rows,
        selected,
        warnings,
    }
}

impl Listing {
    /// The table: a header and one line for each row, no trailing spaces, no trailing newline.
    pub fn render(&self) -> String {
        let mut table: Vec<[String; 5]> = vec![["NAME", "TIER", "LANGUAGE", "DEFAULT", "LOCATION"].map(str::to_owned)];
        for row in &self.rows {
            let mut location = row.location.display().to_string();
            if let Some(target) = &row.target {
                location.push_str(&format!(" -> {}", target.display()));
            }
            if row.packaged {
                location.push_str("  (packaged)");
            }
            if let Some(problem) = &row.problem {
                location.push_str(&format!("  [unusable: {problem}]"));
            }
            let marker = if row.is_default {
                "yes"
            } else if row.shadowed {
                "shadowed"
            } else {
                ""
            };
            table.push([
                row.name.clone(),
                row.tier.number().to_string(),
                row.language.as_ref().map_or("?", Language::as_str).to_owned(),
                marker.to_owned(),
                location,
            ]);
        }
        let widths: Vec<usize> = (0..4)
            .map(|column| {
                table
                    .iter()
                    .map(|cells| cells[column].chars().count())
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        let mut text = String::new();
        for (index, cells) in table.iter().enumerate() {
            if index > 0 {
                text.push('\n');
            }
            let mut line = String::new();
            for (column, &width) in widths.iter().enumerate() {
                line.push_str(&format!("{:<width$}  ", cells[column]));
            }
            line.push_str(&cells[4]);
            text.push_str(line.trim_end());
        }
        text
    }
}
