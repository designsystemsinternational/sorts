//! Font activation with the system, behind a trait so tests can use a fake.

use objc2_core_foundation::{CFArray, CFError, CFRetained, CFString, CFURL};
use objc2_core_text::{
    CTFontManagerCopyAvailablePostScriptNames, CTFontManagerCreateFontDescriptorsFromURL,
    CTFontManagerEnableFontDescriptors, CTFontManagerRegisterFontsForURL, CTFontManagerScope,
    CTFontManagerUnregisterFontsForURL,
};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::ptr::{self, NonNull};

/// If set, the CLI uses a fake activator that stores registrations in this JSON file.
/// Only for end-to-end tests.
pub const FAKE_ACTIVATOR_ENV: &str = "SORTS_FAKE_ACTIVATOR";

pub trait FontActivator {
    /// Registers a font file. A file that is already registered counts as success.
    fn register(&mut self, path: &Path) -> Result<(), String>;
    /// Unregisters a font file. A file that is not registered counts as success.
    fn unregister(&mut self, path: &Path) -> Result<(), String>;
}

/// In-memory activator for tests.
#[derive(Debug, Default)]
pub struct FakeActivator {
    pub registered: BTreeSet<PathBuf>,
    /// Paths that fail to register and unregister.
    pub fail: BTreeSet<PathBuf>,
}

impl FakeActivator {
    fn check(&self, path: &Path) -> Result<(), String> {
        match self.fail.contains(path) {
            true => Err(format!("simulated failure for {}", path.display())),
            false => Ok(()),
        }
    }
}

impl FontActivator for FakeActivator {
    fn register(&mut self, path: &Path) -> Result<(), String> {
        self.check(path)?;
        self.registered.insert(path.to_path_buf());
        Ok(())
    }

    fn unregister(&mut self, path: &Path) -> Result<(), String> {
        self.check(path)?;
        self.registered.remove(path);
        Ok(())
    }
}

/// Activator that stores registrations in a JSON file, so they survive between
/// processes. Only for end-to-end tests of the binary.
pub struct FileFakeActivator {
    pub path: PathBuf,
}

impl FileFakeActivator {
    pub fn registered(&self) -> BTreeSet<PathBuf> {
        std::fs::read(&self.path).ok().and_then(|data| serde_json::from_slice(&data).ok()).unwrap_or_default()
    }

    fn update(&self, change: impl FnOnce(&mut BTreeSet<PathBuf>)) -> Result<(), String> {
        let mut registered = self.registered();
        change(&mut registered);
        crate::settings::write_json(&self.path, &registered).map_err(|e| e.to_string())
    }
}

impl FontActivator for FileFakeActivator {
    fn register(&mut self, path: &Path) -> Result<(), String> {
        self.update(|r| {
            r.insert(path.to_path_buf());
        })
    }

    fn unregister(&mut self, path: &Path) -> Result<(), String> {
        self.update(|r| {
            r.remove(path);
        })
    }
}

/// Registers fonts with CoreText. The fonts stay where they are; nothing is copied.
pub struct CoreTextActivator {
    pub scope: CTFontManagerScope,
}

impl CoreTextActivator {
    /// Registrations that survive logout and restart.
    pub fn persistent() -> Self {
        Self { scope: CTFontManagerScope::Persistent }
    }

    /// Registrations that end with the process. Used by tests.
    pub fn process() -> Self {
        Self { scope: CTFontManagerScope::Process }
    }

    /// Registrations that end at logout. Used by tests that need a disabled registration.
    pub fn session() -> Self {
        Self { scope: CTFontManagerScope::Session }
    }

    fn call(
        &self,
        path: &Path,
        f: unsafe extern "C-unwind" fn(&CFURL, CTFontManagerScope, *mut *mut CFError) -> bool,
        ignore_code: isize,
    ) -> Result<(), String> {
        let url = CFURL::from_file_path(path).ok_or_else(|| format!("invalid path: {}", path.display()))?;
        let mut error: *mut CFError = ptr::null_mut();
        if unsafe { f(&url, self.scope, &mut error) } {
            return Ok(());
        }
        // SAFETY: on failure CoreText returns a +1 retained CFError, or null.
        let error = NonNull::new(error).map(|e| unsafe { CFRetained::from_raw(e) });
        match error {
            Some(e) if e.code() == ignore_code => Ok(()),
            Some(e) => Err(e.description().map(|d| d.to_string()).unwrap_or_else(|| format!("error {}", e.code()))),
            None => Err("unknown CoreText error".into()),
        }
    }
}

/// `kCTFontManagerErrorAlreadyRegistered`
const ALREADY_REGISTERED: isize = 105;
/// `kCTFontManagerErrorNotRegistered`
const NOT_REGISTERED: isize = 201;

impl FontActivator for CoreTextActivator {
    /// Also enables the font, because a font that another app disabled stays unavailable.
    fn register(&mut self, path: &Path) -> Result<(), String> {
        self.call(path, CTFontManagerRegisterFontsForURL, ALREADY_REGISTERED)?;
        set_enabled(path, true);
        Ok(())
    }

    /// Enables the font first, because CoreText cannot unregister a disabled font.
    fn unregister(&mut self, path: &Path) -> Result<(), String> {
        set_enabled(path, true);
        self.call(path, CTFontManagerUnregisterFontsForURL, NOT_REGISTERED)
    }
}

/// Enables or disables the registered fonts in a file. Other font managers (e.g. RightFont)
/// disable fonts instead of unregistering them, and macOS keeps a disabled font unavailable
/// even after a new registration.
pub fn set_enabled(path: &Path, enable: bool) {
    let Some(url) = CFURL::from_file_path(path) else { return };
    if let Some(descriptors) = unsafe { CTFontManagerCreateFontDescriptorsFromURL(&url) } {
        // SAFETY: CoreText documents the array as containing CTFontDescriptors.
        unsafe { CTFontManagerEnableFontDescriptors(&descriptors, enable) };
    }
}

/// True if a font with this PostScript name is available to this process.
pub fn is_available(postscript: &str) -> bool {
    let names = unsafe { CTFontManagerCopyAvailablePostScriptNames() };
    // SAFETY: CoreText documents the array as containing CFStrings.
    let names: &CFArray<CFString> = unsafe { names.cast_unchecked() };
    names.iter().any(|n| n.to_string() == postscript)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_activator_registers_and_unregisters() {
        let mut fake = FakeActivator::default();
        fake.register(Path::new("/a.otf")).unwrap();
        fake.register(Path::new("/a.otf")).unwrap();
        assert_eq!(fake.registered, BTreeSet::from(["/a.otf".into()]));

        fake.unregister(Path::new("/a.otf")).unwrap();
        fake.unregister(Path::new("/a.otf")).unwrap();
        assert!(fake.registered.is_empty());
    }

    #[test]
    fn fake_activator_can_fail() {
        let mut fake = FakeActivator { fail: BTreeSet::from(["/bad.otf".into()]), ..Default::default() };
        assert!(fake.register(Path::new("/bad.otf")).is_err());
        assert!(fake.unregister(Path::new("/bad.otf")).is_err());
        assert!(fake.registered.is_empty());
    }

    #[test]
    fn file_fake_activator_persists_between_instances() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("fake.json");

        FileFakeActivator { path: file.clone() }.register(Path::new("/a.otf")).unwrap();
        FileFakeActivator { path: file.clone() }.register(Path::new("/b.otf")).unwrap();
        FileFakeActivator { path: file.clone() }.unregister(Path::new("/a.otf")).unwrap();

        assert_eq!(FileFakeActivator { path: file }.registered(), BTreeSet::from(["/b.otf".into()]));
    }
}
