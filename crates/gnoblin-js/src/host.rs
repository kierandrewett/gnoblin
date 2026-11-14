//! The host side of hot reload: stage a new generation, then commit it.
//!
//! Staging builds a fresh [`JsGeneration`] and evaluates the entry module in it. The current
//! generation is not touched, so a syntax error, a throw or a runaway loop in the new config leaves the
//! running one callable and the failure comes back as a [`JsError`] with the author's file, line and
//! column. Only [`JsHost::commit`] swaps, and it drops the old generation, which frees its runtime.
//! Committing is the caller's decision, so a later layer can validate the staged generation first.

use std::path::Path;

use crate::error::JsError;
use crate::generation::{JsConfig, JsGeneration};

/// What runs on a fresh generation before the entry module does: a host installs its native objects here.
pub type JsPrepare = Box<dyn FnMut(&JsGeneration) -> Result<(), JsError>>;

/// Owns the current generation and builds its replacements.
pub struct JsHost {
    config: JsConfig,
    current: Option<JsGeneration>,
    next_id: u64,
    prepare: Option<JsPrepare>,
}

impl std::fmt::Debug for JsHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JsHost")
            .field("config", &self.config)
            .field("current", &self.current)
            .field("next_id", &self.next_id)
            .finish_non_exhaustive()
    }
}

impl JsHost {
    /// A host with no generation yet; every generation it stages is built from `config`.
    pub fn new(config: JsConfig) -> Self {
        Self {
            config,
            current: None,
            next_id: 1,
            prepare: None,
        }
    }

    /// Runs `prepare` on every generation this host stages, after it is built and before its entry module is
    /// evaluated. A failure fails the stage like a throw in the entry does.
    pub fn set_prepare(&mut self, prepare: JsPrepare) {
        self.prepare = Some(prepare);
    }

    /// The generation that is running, if one was committed.
    pub fn current(&self) -> Option<&JsGeneration> {
        self.current.as_ref()
    }

    /// Builds a fresh generation and evaluates `entry` in it.
    ///
    /// On success the generation is returned for the caller to inspect and [`commit`](Self::commit). On
    /// failure it is dropped and the current generation is exactly as it was.
    pub fn stage(&mut self, entry: &Path) -> Result<JsGeneration, JsError> {
        let id = self.next_id;
        self.next_id += 1;
        let generation = JsGeneration::new(&self.config, id)?;
        if let Some(prepare) = self.prepare.as_mut() {
            prepare(&generation)?;
        }
        generation.eval_module(entry)?;
        Ok(generation)
    }

    /// Makes `generation` the current one and drops the previous generation.
    pub fn commit(&mut self, generation: JsGeneration) {
        drop(self.current.replace(generation));
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use tempfile::TempDir;

    use super::JsHost;
    use crate::error::JsErrorKind;
    use crate::generation::{JsConfig, JsGeneration};

    fn root_with(files: &[(&str, &str)]) -> (TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = fs::canonicalize(dir.path()).expect("the temp dir exists");
        for (name, text) in files {
            fs::write(path.join(name), text).expect("the module is written");
        }
        (dir, path)
    }

    fn version(generation: &JsGeneration) -> i32 {
        generation
            .call_budgeted(Duration::from_secs(5), |ctx| ctx.eval::<i32, _>("version()"))
            .expect("version() is callable")
    }

    #[test]
    fn commit_swaps_the_current_generation() {
        let (_dir, root) = root_with(&[
            ("one.ts", "globalThis.version = (): number => 1;\n"),
            ("two.ts", "globalThis.version = (): number => 2;\n"),
        ]);
        let mut host = JsHost::new(JsConfig::new(&root));
        assert!(host.current().is_none());
        let first = host.stage(Path::new("one.ts")).expect("one.ts stages");
        host.commit(first);
        let current = host.current().expect("a current generation");
        assert_eq!(version(current), 1);
        let first_id = current.id();

        let second = host.stage(Path::new("two.ts")).expect("two.ts stages");
        // Staging did not touch the current generation.
        assert_eq!(version(host.current().expect("still current")), 1);
        host.commit(second);
        let current = host.current().expect("the new generation");
        assert_eq!(version(current), 2);
        assert!(current.id() > first_id);
    }

    #[test]
    fn failing_stage_leaves_the_current_generation_callable() {
        let (_dir, root) = root_with(&[
            ("good.ts", "globalThis.version = (): number => 1;\n"),
            (
                "throws.ts",
                "const n: number = 1;\n\nthrow new Error('broken config ' + n);\n",
            ),
            ("syntax.ts", "const ok = 1;\nconst broken = ;\n"),
            ("loops.ts", "while (true) {}\n"),
            ("missing.ts", "import './nowhere.ts';\n"),
        ]);
        let mut config = JsConfig::new(&root);
        config.stage_budget = Duration::from_millis(100);
        let mut host = JsHost::new(config);
        let good = host.stage(Path::new("good.ts")).expect("good.ts stages");
        host.commit(good);

        let cases = [
            ("throws.ts", JsErrorKind::Exception),
            ("syntax.ts", JsErrorKind::Ts),
            ("loops.ts", JsErrorKind::Budget),
            ("missing.ts", JsErrorKind::Load),
            ("absent.ts", JsErrorKind::Load),
        ];
        for (entry, kind) in cases {
            let error = host.stage(Path::new(entry)).expect_err("the stage fails");
            assert_eq!(error.kind, kind, "{entry}: {error}");
            assert_eq!(
                version(host.current().expect("the current generation survives")),
                1,
                "after {entry}"
            );
        }
    }

    #[test]
    fn failing_stage_reports_the_ts_location() {
        let (_dir, root) = root_with(&[(
            "throws.ts",
            "const n: number = 1;\n\nthrow new Error('broken config ' + n);\n",
        )]);
        let mut host = JsHost::new(JsConfig::new(&root));
        let error = host.stage(Path::new("throws.ts")).expect_err("the stage fails");
        let at = error.location.expect("a location");
        assert_eq!(at.file, root.join("throws.ts"));
        assert_eq!(at.line, 3);
    }
}
