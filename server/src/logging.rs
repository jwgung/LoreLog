//! Where the library's own notes go (a failed request, a skipped file, the librarian being unreachable). The library
//! writes them with the `log` macros; the program that runs it chooses the place once: the command line tools print
//! them to stderr as they always did (systemd keeps them in the journal), the desktop app writes them to a file,
//! because a desktop program has no stderr anyone sees.

struct Sink(Box<dyn Fn(&str) + Send + Sync>);

impl log::Log for Sink {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Info
    }

    fn log(&self, r: &log::Record) {
        // only this library's notes (other crates in the same program log too); a host adding its own notes uses a
        // target under `life_server` (e.g. `life_server::desktop`)
        if self.enabled(r.metadata()) && r.target().starts_with("life_server") {
            (self.0)(&r.args().to_string());
        }
    }

    fn flush(&self) {}
}

/// Send the library's notes to `write`, one message per call, as written (no level or time added). Only the first call
/// in a process takes effect.
pub fn init(write: impl Fn(&str) + Send + Sync + 'static) {
    if log::set_boxed_logger(Box::new(Sink(Box::new(write)))).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}

/// The command line tools: every note on its own line on stderr.
pub fn to_stderr() {
    init(|m| eprintln!("{m}"));
}
