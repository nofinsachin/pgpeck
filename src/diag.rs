//! Problems found while loading user files. Loading never aborts on a bad
//! option. It reports a `Diagnostic` and falls back to the default (CFG R21.2).

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// File label, for example `config.toml`.
    pub file: String,
    /// 1-based line number, when it can be determined.
    pub line: Option<usize>,
    pub message: String,
}

impl Diagnostic {
    pub fn new(file: &str, line: Option<usize>, message: impl Into<String>) -> Self {
        Self {
            file: file.to_string(),
            line,
            message: message.into(),
        }
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "{}:{}: {}", self.file, line, self.message),
            None => write!(f, "{}: {}", self.file, self.message),
        }
    }
}

/// 1-based line number of a byte offset.
pub fn line_of_offset(src: &str, offset: usize) -> usize {
    let end = offset.min(src.len());
    src.as_bytes()[..end]
        .iter()
        .filter(|b| **b == b'\n')
        .count()
        + 1
}

/// 1-based number of the first line that contains `needle` (best effort).
pub fn line_containing(src: &str, needle: &str) -> Option<usize> {
    src.lines().position(|l| l.contains(needle)).map(|i| i + 1)
}
