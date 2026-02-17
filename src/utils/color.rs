//! Minimal ANSI color support for trace output.
//!
//! Respects `--no-color` flag and auto-detects terminal attachment.
//! When stdout is not a terminal (piped), colors are disabled automatically.

const RESET: &str = "\x1b[0m";
const DIM: &str = "\x1b[2m";
const BOLD: &str = "\x1b[1m";
const RED: &str = "\x1b[31m";
const CYAN: &str = "\x1b[36m";
const YELLOW: &str = "\x1b[33m";

/// Color palette for trace output.
#[derive(Clone, Copy)]
pub struct Palette {
    enabled: bool,
}

impl Palette {
    #[must_use]
    pub fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    /// Detect whether to enable colors based on terminal attachment and `NO_COLOR` env.
    ///
    /// Per <https://no-color.org/>, the mere presence of `NO_COLOR` (regardless of
    /// value, including empty string) disables color output.
    #[must_use]
    pub fn auto() -> Self {
        use std::io::IsTerminal;
        let no_color = std::env::var_os("NO_COLOR").is_some();
        Self {
            enabled: !no_color && std::io::stdout().is_terminal(),
        }
    }

    fn wrap(self, code: &str, text: &str) -> String {
        if self.enabled {
            format!("{code}{text}{RESET}")
        } else {
            text.to_string()
        }
    }

    /// Dim/muted text (gas, tree connectors, annotations).
    pub fn dim(self, text: &str) -> String {
        self.wrap(DIM, text)
    }

    /// Bold text.
    pub fn bold(self, text: &str) -> String {
        self.wrap(BOLD, text)
    }

    /// Red text (errors).
    pub fn red(self, text: &str) -> String {
        self.wrap(RED, text)
    }

    /// Cyan text (addresses).
    pub fn cyan(self, text: &str) -> String {
        self.wrap(CYAN, text)
    }

    /// Yellow text (ETH values, event names).
    pub fn yellow(self, text: &str) -> String {
        self.wrap(YELLOW, text)
    }
}
