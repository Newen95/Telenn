//! Shared plumbing for the verification CLIs: exit codes and a tiny argument
//! parser. Each bin compiles its own copy (`mod util;`) and uses a different
//! subset, so bins declare it `#[allow(dead_code)] mod util;` — keep this
//! file to genuinely universal helpers regardless.

/// Success, no defects.
pub const EXIT_OK: i32 = 0;
/// Usage or I/O error (bad flag, unreadable file).
pub const EXIT_USAGE: i32 = 1;
/// The input was analyzed and hard defects were found.
pub const EXIT_DEFECTS: i32 = 2;

/// Minimal flag parser: flags are consumed out of the list, whatever remains
/// at the end is positional (or an unknown-flag error).
pub struct Args {
    args: Vec<String>,
}

impl Args {
    pub fn parse() -> Self {
        Self {
            args: std::env::args().skip(1).collect(),
        }
    }

    /// Consume `--flag value`; `None` when the flag is absent.
    pub fn take_value(&mut self, flag: &str) -> Result<Option<String>, String> {
        match self.args.iter().position(|a| a == flag) {
            Some(i) if i + 1 < self.args.len() => {
                self.args.remove(i);
                Ok(Some(self.args.remove(i)))
            }
            Some(_) => Err(format!("{flag} needs a value")),
            None => Ok(None),
        }
    }

    /// Consume every occurrence of a repeatable `--flag value`.
    pub fn take_values(&mut self, flag: &str) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        while let Some(v) = self.take_value(flag)? {
            out.push(v);
        }
        Ok(out)
    }

    /// Consume a boolean `--flag`.
    pub fn take_flag(&mut self, flag: &str) -> bool {
        match self.args.iter().position(|a| a == flag) {
            Some(i) => {
                self.args.remove(i);
                true
            }
            None => false,
        }
    }

    /// Whatever was not consumed. Errors on leftover `--flags` so a typo
    /// never silently becomes a file name.
    pub fn finish_positional(self) -> Result<Vec<String>, String> {
        if let Some(flag) = self.args.iter().find(|a| a.starts_with("--")) {
            return Err(format!("unknown option {flag}"));
        }
        Ok(self.args)
    }
}

/// Parse a float flag value with a readable error.
pub fn parse_f32(flag: &str, value: &str) -> Result<f32, String> {
    value
        .parse::<f32>()
        .map_err(|_| format!("{flag}: '{value}' is not a number"))
}
