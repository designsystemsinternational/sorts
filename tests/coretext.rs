//! Integration tests against the real CoreText API.
//!
//! They use the process scope, so nothing persists on the machine. They are ignored by
//! default; run them with `cargo test -- --ignored`.

use sorts::activator::{CoreTextActivator, FontActivator, is_available};
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
