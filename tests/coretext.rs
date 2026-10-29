//! Integration tests against the real CoreText API.
//!
//! They use the process scope, so nothing persists on the machine. The tests for disabled
//! fonts need the session scope, because the process scope does not keep a disabled
//! registration; session registrations end at logout and the tests unregister their fonts.
//! They are ignored by default; run them with `cargo test -- --ignored`.

use sorts::activator::{CoreTextActivator, FontActivator, is_available, set_enabled};
use sorts::testing::write_font;

#[test]
#[ignore = "uses CoreText; run with `cargo test -- --ignored`"]
fn registers_and_unregisters_a_font_for_this_process() {
    let dir = tempfile::tempdir().unwrap();
    let postscript = format!("SortsTest-{}", std::process::id());
    let path = write_font(dir.path(), "Test.otf", &postscript);
    let mut activator = CoreTextActivator::process();
    assert!(!is_available(&postscript));

    activator.register(&path).unwrap();
    assert!(is_available(&postscript));
    activator.register(&path).expect("registering twice counts as success");

    activator.unregister(&path).unwrap();
    assert!(!is_available(&postscript));
    activator.unregister(&path).expect("unregistering twice counts as success");
}

#[test]
#[ignore = "uses CoreText; run with `cargo test -- --ignored`"]
fn a_file_that_is_not_a_font_fails_to_register() {
    let dir = tempfile::tempdir().unwrap();
    let path = sorts::testing::write_file(dir.path(), "Broken.otf", b"not a font");
    assert!(CoreTextActivator::process().register(&path).is_err());
}

#[test]
#[ignore = "uses CoreText; run with `cargo test -- --ignored`"]
fn registering_a_disabled_font_enables_it() {
    // Other font managers (e.g. RightFont) disable fonts instead of unregistering them.
    let dir = tempfile::tempdir().unwrap();
    let postscript = format!("SortsDisabled-{}", std::process::id());
    let path = write_font(dir.path(), "Disabled.otf", &postscript);
    let mut activator = CoreTextActivator::session();
    activator.register(&path).unwrap();
    set_enabled(&path, false);
    assert!(!is_available(&postscript));

    activator.register(&path).unwrap();
    assert!(is_available(&postscript));

    activator.unregister(&path).unwrap();
}

#[test]
#[ignore = "uses CoreText; run with `cargo test -- --ignored`"]
fn unregistering_a_disabled_font_removes_it() {
    let dir = tempfile::tempdir().unwrap();
    let postscript = format!("SortsDisabledGone-{}", std::process::id());
    let path = write_font(dir.path(), "DisabledGone.otf", &postscript);
    let mut activator = CoreTextActivator::session();
    activator.register(&path).unwrap();
    set_enabled(&path, false);

    activator.unregister(&path).unwrap();
    // A fresh registration works, so no registration was left behind.
    activator.register(&path).unwrap();
    assert!(is_available(&postscript));

    activator.unregister(&path).unwrap();
}
