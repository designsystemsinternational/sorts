//! The debug log (`sorts --debug`): one line for each step of a scan or a sync, and a list of the steps that wait.
//!
//! The log is for a person who must find out why a command is slow or does not end. Each line has the time since the
//! start and the thread. [`DebugLog::waits`] lists the steps that started and did not end after [`WAIT_AFTER`].

use crate::activator::FontActivator;
use crate::scan::{ScanEvent, has_dataless_flag};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A step that runs longer than this is listed by [`DebugLog::waits`].
pub const WAIT_AFTER: Duration = Duration::from_secs(5);

pub struct DebugLog<W> {
    start: Instant,
    inner: Mutex<Inner<W>>,
}

struct Inner<W> {
    out: W,
    /// The last event of the walk and its time. The walk lists folders between events, so a walk that waits shows here.
    walk: Option<(String, Instant)>,
    /// Steps that started and did not end, with their start time.
    steps: BTreeMap<String, Instant>,
}

/// The name of the current thread for the log, e.g. `main` or `ThreadId(4)`.
pub fn thread_name() -> String {
    let thread = std::thread::current();
    thread.name().map(String::from).unwrap_or_else(|| format!("{:?}", thread.id()))
}

fn flags_text(flags: u32) -> String {
    let dataless = if has_dataless_flag(flags) { " dataless" } else { "" };
    format!("flags=0x{flags:08x}{dataless}")
}

fn folder_text(id: &str) -> &str {
    if id.is_empty() { "(root)" } else { id }
}

impl<W: Write + Send> DebugLog<W> {
    pub fn new(out: W) -> Self {
        Self { start: Instant::now(), inner: Mutex::new(Inner { out, walk: None, steps: BTreeMap::new() }) }
    }

    /// The time the log started. Each line shows the time since then.
    pub fn start(&self) -> Instant {
        self.start
    }

    pub fn line(&self, text: &str) {
        self.line_at(text, Instant::now(), &thread_name())
    }

    /// Like [`DebugLog::line`], at the time `now` on `thread` (for tests).
    pub fn line_at(&self, text: &str, now: Instant, thread: &str) {
        let mut inner = self.inner.lock().unwrap();
        self.write(&mut inner, text, now, thread);
    }

    fn write(&self, inner: &mut Inner<W>, text: &str, now: Instant, thread: &str) {
        let seconds = now.saturating_duration_since(self.start).as_secs_f64();
        let _ = writeln!(inner.out, "[{seconds:9.3}s] [{thread}] {text}");
        let _ = inner.out.flush();
    }

    /// Logs the start of a step. [`DebugLog::waits`] lists it until [`DebugLog::end`].
    pub fn begin(&self, step: &str) {
        self.begin_at(step, Instant::now(), &thread_name())
    }

    pub fn begin_at(&self, step: &str, now: Instant, thread: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.steps.insert(step.to_string(), now);
        self.write(&mut inner, &format!("start {step}"), now, thread);
    }

    /// Logs the end of a step with its duration and `outcome`.
    pub fn end(&self, step: &str, outcome: &str) {
        self.end_at(step, outcome, Instant::now(), &thread_name())
    }

    pub fn end_at(&self, step: &str, outcome: &str, now: Instant, thread: &str) {
        let mut inner = self.inner.lock().unwrap();
        let duration = match inner.steps.remove(step) {
            Some(start) => format!(" {:.3}s", now.saturating_duration_since(start).as_secs_f64()),
            None => String::new(),
        };
        self.write(&mut inner, &format!("end {step}{duration} {outcome}"), now, thread);
    }

    pub fn event(&self, event: &ScanEvent) {
        self.event_at(event, Instant::now(), &thread_name())
    }

    /// Like [`DebugLog::event`], at the time `now` on `thread` (for tests).
    pub fn event_at(&self, event: &ScanEvent, now: Instant, thread: &str) {
        match event {
            ScanEvent::Folder { id, flags } => {
                let text = format!("folder {} {}", folder_text(id), flags_text(*flags));
                self.walk_event(text, now, thread);
            }
            ScanEvent::Found { count, id, size, flags, online_only } => {
                let online = if *online_only { " online-only" } else { "" };
                let text = format!("found #{count} {id} size={size} {}{online}", flags_text(*flags));
                self.walk_event(text, now, thread);
            }
            ScanEvent::Downloading { files, bytes } => {
                let mut inner = self.inner.lock().unwrap();
                inner.walk = None;
                let text = format!("walk done. {files} online-only files to download ({bytes} bytes)");
                self.write(&mut inner, &text, now, thread);
            }
            ScanEvent::Reading { files } => {
                let mut inner = self.inner.lock().unwrap();
                inner.walk = None;
                self.write(&mut inner, &format!("walk done. {files} font files to read"), now, thread);
            }
            ScanEvent::ReadStart { id } => self.begin_at(&format!("read {id}"), now, thread),
            ScanEvent::Read { id, downloaded, error } => {
                let mut outcome = match error {
                    Some(error) => format!("error: {error}"),
                    None => "ok".to_string(),
                };
                if let Some(bytes) = downloaded {
                    outcome.push_str(&format!(", downloaded {bytes} bytes"));
                }
                self.end_at(&format!("read {id}"), &outcome, now, thread);
            }
        }
    }

    fn walk_event(&self, text: String, now: Instant, thread: &str) {
        let mut inner = self.inner.lock().unwrap();
        self.write(&mut inner, &text, now, thread);
        inner.walk = Some((text, now));
    }

    /// Lists the steps that run longer than [`WAIT_AFTER`], and the walk if it had no event for that time.
    pub fn waits(&self) {
        self.waits_at(Instant::now())
    }

    /// Like [`DebugLog::waits`], at the time `now` (for tests).
    pub fn waits_at(&self, now: Instant) {
        let mut inner = self.inner.lock().unwrap();
        let age = |since: Instant| now.saturating_duration_since(since);
        let mut lines = Vec::new();
        if let Some((text, since)) = &inner.walk
            && age(*since) >= WAIT_AFTER
        {
            lines.push(format!("WAIT {:.1}s walk, no new event since: {text}", age(*since).as_secs_f64()));
        }
        for (step, since) in &inner.steps {
            if age(*since) >= WAIT_AFTER {
                lines.push(format!("WAIT {:.1}s {step}", age(*since).as_secs_f64()));
            }
        }
        for line in lines {
            self.write(&mut inner, &line, now, "watchdog");
        }
    }

    /// Logs the root folder of the scan and its flags.
    pub fn root(&self, root: &Path) {
        use std::os::macos::fs::MetadataExt;
        let step = format!("stat of the Fonts folder {}", root.display());
        self.begin(&step);
        let outcome = match std::fs::metadata(root) {
            Ok(metadata) => flags_text(metadata.st_flags()),
            Err(error) => format!("error: {error}"),
        };
        self.end(&step, &outcome);
    }

    pub fn into_inner(self) -> W {
        self.inner.into_inner().unwrap().out
    }
}

/// An activator that logs each call to a [`DebugLog`] and passes it on.
pub struct DebugActivator<'a, W> {
    pub activator: &'a mut dyn FontActivator,
    pub log: &'a DebugLog<W>,
}

impl<W: Write + Send> DebugActivator<'_, W> {
    fn call(
        &mut self,
        name: &str,
        path: &Path,
        f: impl FnOnce(&mut dyn FontActivator) -> Result<(), String>,
    ) -> Result<(), String> {
        let step = format!("{name} {}", path.display());
        self.log.begin(&step);
        let result = f(self.activator);
        self.log.end(&step, &result.as_ref().map(|_| "ok".to_string()).unwrap_or_else(|e| format!("error: {e}")));
        result
    }
}

impl<W: Write + Send> FontActivator for DebugActivator<'_, W> {
    fn register(&mut self, path: &Path) -> Result<(), String> {
        self.call("register", path, |a| a.register(path))
    }

    fn unregister(&mut self, path: &Path) -> Result<(), String> {
        self.call("unregister", path, |a| a.unregister(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activator::FakeActivator;

    fn output(log: DebugLog<Vec<u8>>) -> String {
        String::from_utf8(log.into_inner()).unwrap()
    }

    #[test]
    fn logs_each_event_with_the_time_and_the_thread() {
        let log = DebugLog::new(Vec::new());
        let t = |ms| log.start() + Duration::from_millis(ms);
        log.event_at(&ScanEvent::Folder { id: String::new(), flags: 0 }, t(1), "main");
        log.event_at(&ScanEvent::Folder { id: "A/".into(), flags: 0x4000_0000 }, t(12), "main");
        let found =
            ScanEvent::Found { count: 1, id: "A/B.otf".into(), size: 900, flags: 0x4000_0000, online_only: true };
        log.event_at(&found, t(20), "main");
        log.event_at(&ScanEvent::Downloading { files: 1, bytes: 900 }, t(21), "main");
        log.event_at(&ScanEvent::Reading { files: 1 }, t(21), "main");
        log.event_at(&ScanEvent::ReadStart { id: "A/B.otf".into() }, t(30), "ThreadId(3)");
        log.event_at(
            &ScanEvent::Read { id: "A/B.otf".into(), downloaded: Some(900), error: None },
            t(1530),
            "ThreadId(3)",
        );

        assert_eq!(
            output(log),
            "\
[    0.001s] [main] folder (root) flags=0x00000000
[    0.012s] [main] folder A/ flags=0x40000000 dataless
[    0.020s] [main] found #1 A/B.otf size=900 flags=0x40000000 dataless online-only
[    0.021s] [main] walk done. 1 online-only files to download (900 bytes)
[    0.021s] [main] walk done. 1 font files to read
[    0.030s] [ThreadId(3)] start read A/B.otf
[    1.530s] [ThreadId(3)] end read A/B.otf 1.500s ok, downloaded 900 bytes
"
        );
    }

    #[test]
    fn logs_read_errors() {
        let log = DebugLog::new(Vec::new());
        let start = log.start();
        log.event_at(&ScanEvent::ReadStart { id: "A.ttf".into() }, start, "main");
        log.event_at(
            &ScanEvent::Read { id: "A.ttf".into(), downloaded: None, error: Some("bad".into()) },
            start,
            "main",
        );
        assert!(output(log).ends_with("end read A.ttf 0.000s error: bad\n"));
    }

    #[test]
    fn lists_the_steps_that_wait() {
        let log = DebugLog::new(Vec::new());
        let t = |s| log.start() + Duration::from_secs(s);
        log.begin_at("read A/Slow.otf", t(0), "ThreadId(2)");
        log.begin_at("read A/Fast.otf", t(0), "ThreadId(3)");
        log.end_at("read A/Fast.otf", "ok", t(1), "ThreadId(3)");
        log.begin_at("read A/New.otf", t(4), "ThreadId(3)");
        log.waits_at(t(6));

        let output = output(log);
        let waits: Vec<_> = output.lines().filter(|l| l.contains("WAIT")).collect();
        assert_eq!(waits, ["[    6.000s] [watchdog] WAIT 6.0s read A/Slow.otf"]);
    }

    #[test]
    fn lists_a_walk_without_new_events() {
        let log = DebugLog::new(Vec::new());
        let t = |s| log.start() + Duration::from_secs(s);
        log.event_at(&ScanEvent::Folder { id: "A/".into(), flags: 0 }, t(0), "main");
        log.waits_at(t(4));
        log.waits_at(t(7));
        log.event_at(&ScanEvent::Reading { files: 0 }, t(8), "main");
        log.waits_at(t(20));

        let output = output(log);
        let waits: Vec<_> = output.lines().filter(|l| l.contains("WAIT")).collect();
        assert_eq!(waits, ["[    7.000s] [watchdog] WAIT 7.0s walk, no new event since: folder A/ flags=0x00000000"]);
    }

    #[test]
    fn the_activator_logs_each_call_and_passes_it_on() {
        let log = DebugLog::new(Vec::new());
        let mut fake = FakeActivator::default();
        fake.fail.insert("/f/Bad.otf".into());
        let mut activator = DebugActivator { activator: &mut fake, log: &log };

        activator.register(Path::new("/f/A.otf")).unwrap();
        assert!(activator.register(Path::new("/f/Bad.otf")).is_err());
        activator.unregister(Path::new("/f/A.otf")).unwrap();

        assert!(fake.registered.is_empty());
        let output = output(log);
        let lines: Vec<_> = output.lines().map(|l| l.split("] ").last().unwrap()).collect();
        assert_eq!(lines.len(), 6);
        assert_eq!(lines[0], "start register /f/A.otf");
        assert!(lines[1].starts_with("end register /f/A.otf ") && lines[1].ends_with("s ok"), "{}", lines[1]);
        assert!(lines[3].ends_with("error: simulated failure for /f/Bad.otf"), "{}", lines[3]);
        assert!(lines[5].starts_with("end unregister /f/A.otf "), "{}", lines[5]);
    }
}
