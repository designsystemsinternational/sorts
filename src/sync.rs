//! Reconciliation: makes the registered fonts match the active set.

use crate::activator::FontActivator;
use crate::settings::State;
use std::collections::BTreeSet;
use std::path::PathBuf;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub registered: Vec<PathBuf>,
    pub unregistered: Vec<PathBuf>,
    pub errors: Vec<(PathBuf, String)>,
}

/// What a sync would do: (to register, to unregister).
pub fn plan(desired: &BTreeSet<PathBuf>, state: &State) -> (Vec<PathBuf>, Vec<PathBuf>) {
    (desired.difference(&state.registered).cloned().collect(), state.registered.difference(desired).cloned().collect())
}

/// Unregisters first (to free PostScript names), then registers. Updates `state`.
///
/// Registers every desired font again, also the fonts in the state: the system can lose or
/// disable a registration, and registering is idempotent. Only new fonts are reported.
///
/// A font that fails to unregister stays in the state so the next sync retries it,
/// unless the file no longer exists.
pub fn apply(desired: &BTreeSet<PathBuf>, state: &mut State, activator: &mut dyn FontActivator) -> SyncReport {
    let (to_register, to_unregister) = plan(desired, state);
    let known: Vec<PathBuf> = desired.intersection(&state.registered).cloned().collect();
    let mut report = run(to_register, to_unregister, state, activator);
    for path in known {
        if let Err(error) = activator.register(&path) {
            report.errors.push((path, error));
        }
    }
    report
}

/// Applies only the difference between two active sets. Used by toggles in the interactive
/// list, so a toggle never applies unrelated changes in the Fonts folder (that is `sync`'s job).
pub fn apply_changes(
    before: &BTreeSet<PathBuf>,
    after: &BTreeSet<PathBuf>,
    state: &mut State,
    activator: &mut dyn FontActivator,
) -> SyncReport {
    let to_register = after.difference(before).cloned().collect();
    let to_unregister = before.difference(after).filter(|p| state.registered.contains(*p)).cloned().collect();
    run(to_register, to_unregister, state, activator)
}

fn run(
    to_register: Vec<PathBuf>,
    to_unregister: Vec<PathBuf>,
    state: &mut State,
    activator: &mut dyn FontActivator,
) -> SyncReport {
    let mut report = SyncReport::default();

    for path in to_unregister {
        match activator.unregister(&path) {
            Ok(()) => {
                state.registered.remove(&path);
                report.unregistered.push(path);
            }
            Err(error) => {
                if !path.exists() {
                    state.registered.remove(&path);
                }
                report.errors.push((path, error));
            }
        }
    }
    for path in to_register {
        match activator.register(&path) {
            Ok(()) => {
                state.registered.insert(path.clone());
                report.registered.push(path);
            }
            Err(error) => report.errors.push((path, error)),
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activator::FakeActivator;

    fn paths(items: &[&str]) -> BTreeSet<PathBuf> {
        items.iter().map(PathBuf::from).collect()
    }

    fn state(items: &[&str]) -> State {
        State { registered: paths(items) }
    }

    #[test]
    fn plan_registers_new_fonts_and_unregisters_old_fonts() {
        let (register, unregister) = plan(&paths(&["/a.otf", "/b.otf"]), &state(&["/b.otf", "/c.otf"]));
        assert_eq!(register, [PathBuf::from("/a.otf")]);
        assert_eq!(unregister, [PathBuf::from("/c.otf")]);
    }

    #[test]
    fn apply_updates_the_activator_and_the_state() {
        let mut activator =
            FakeActivator { registered: paths(&["/b.otf", "/c.otf", "/other-app.otf"]), ..Default::default() };
        let mut state = state(&["/b.otf", "/c.otf"]);

        let report = apply(&paths(&["/a.otf", "/b.otf"]), &mut state, &mut activator);

        assert_eq!(report.registered, [PathBuf::from("/a.otf")]);
        assert_eq!(report.unregistered, [PathBuf::from("/c.otf")]);
        assert!(report.errors.is_empty());
        assert_eq!(state, self::state(&["/a.otf", "/b.otf"]));
        // Fonts that the CLI did not register are never touched.
        assert_eq!(activator.registered, paths(&["/a.otf", "/b.otf", "/other-app.otf"]));
    }

    #[test]
    fn apply_registers_desired_fonts_again_even_if_the_state_has_them() {
        // The system can lose or disable a registration (e.g. another font manager disabled the font).
        let mut activator = FakeActivator::default();
        let mut state = state(&["/a.otf"]);

        let report = apply(&paths(&["/a.otf"]), &mut state, &mut activator);

        assert_eq!(activator.registered, paths(&["/a.otf"]));
        // Only new fonts are reported.
        assert!(report.registered.is_empty());
        assert_eq!(state, self::state(&["/a.otf"]));
    }

    #[test]
    fn a_font_that_fails_to_register_is_reported_and_not_saved() {
        let mut activator = FakeActivator { fail: paths(&["/bad.otf"]), ..Default::default() };
        let mut state = State::default();

        let report = apply(&paths(&["/bad.otf", "/good.otf"]), &mut state, &mut activator);

        assert_eq!(report.registered, [PathBuf::from("/good.otf")]);
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].0, PathBuf::from("/bad.otf"));
        assert_eq!(state, self::state(&["/good.otf"]));
    }

    #[test]
    fn a_font_that_fails_to_unregister_stays_in_the_state_if_the_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("exists.otf");
        std::fs::write(&existing, b"x").unwrap();
        let missing = dir.path().join("deleted.otf");
        let mut activator = FakeActivator { fail: [existing.clone(), missing.clone()].into(), ..Default::default() };
        let mut state = State { registered: [existing.clone(), missing.clone()].into() };

        let report = apply(&BTreeSet::new(), &mut state, &mut activator);

        assert_eq!(report.errors.len(), 2);
        assert_eq!(state.registered, BTreeSet::from([existing]));
    }

    #[test]
    fn apply_changes_only_touches_fonts_that_changed() {
        // "/new-in-folder.otf" is desired in both sets but not registered yet: it waits for `sync`.
        let before = paths(&["/a.otf", "/b.otf", "/new-in-folder.otf"]);
        let after = paths(&["/b.otf", "/c.otf", "/new-in-folder.otf"]);
        let mut state = state(&["/a.otf", "/b.otf"]);
        let mut activator = FakeActivator { registered: paths(&["/a.otf", "/b.otf"]), ..Default::default() };

        let report = apply_changes(&before, &after, &mut state, &mut activator);

        assert_eq!(report.registered, [PathBuf::from("/c.otf")]);
        assert_eq!(report.unregistered, [PathBuf::from("/a.otf")]);
        assert_eq!(state, self::state(&["/b.otf", "/c.otf"]));
        assert_eq!(activator.registered, paths(&["/b.otf", "/c.otf"]));
    }

    #[test]
    fn apply_changes_does_not_unregister_fonts_the_cli_did_not_register() {
        let mut state = State::default();
        let mut activator = FakeActivator { registered: paths(&["/a.otf"]), ..Default::default() };

        let report = apply_changes(&paths(&["/a.otf"]), &BTreeSet::new(), &mut state, &mut activator);

        assert!(report.unregistered.is_empty());
        assert_eq!(activator.registered, paths(&["/a.otf"]));
    }

    #[test]
    fn apply_with_nothing_to_do_reports_nothing() {
        let mut activator = FakeActivator::default();
        let mut state = state(&["/a.otf"]);
        let report = apply(&paths(&["/a.otf"]), &mut state, &mut activator);
        assert_eq!(report, SyncReport::default());
    }
}
