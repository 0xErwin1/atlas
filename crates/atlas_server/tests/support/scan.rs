//! Shared Rust-source tokenizer: separates string literals from code and
//! comments, so a source-walking test can run a plain regex against `code`
//! without a literal or a comment producing a false hit.
//!
//! Originally `api_path_literal_guard.rs`'s own `scan`/`Scanner` (`v2-e3-s7`
//! D3.2); extracted here (`v2-e11-s4` PR2) so `client_call_shape_guard.rs`
//! reuses the same comment/string-masking discipline instead of duplicating
//! a naive regex that would count a method name mentioned in a doc comment
//! or a string literal as a real call site.

/// A string literal's contents (without its delimiters) and the 1-based
/// line its opening delimiter sits on.
pub(crate) struct Literal {
    pub(crate) line: usize,
    pub(crate) text: String,
}

/// The parts of a Rust source file the guards care about: every string
/// literal, and the remaining code text with comments and literals removed.
pub(crate) struct Scanned {
    pub(crate) literals: Vec<Literal>,
    pub(crate) code: String,
}

/// Tokenizes `content` far enough to separate string literals from code
/// and comments: `//` line comments (including `///` and `//!`) and nested
/// `/* … */` block comments are dropped; normal `"…"` literals honour
/// backslash escapes; raw `r"…"`, `r#"…"#` (any number of hashes) literals
/// end only at their matching delimiter; `'"'`, `'\''`, and other char
/// literals never open a string, while lifetimes and labels pass through as
/// code. Everything else is code.
pub(crate) fn scan(content: &str) -> Scanned {
    let mut scanner = Scanner::new(content);
    let mut literals = Vec::new();
    let mut code = String::new();

    while let Some(current) = scanner.peek(0) {
        match current {
            '/' if scanner.peek(1) == Some('/') => scanner.skip_line_comment(),
            '/' if scanner.peek(1) == Some('*') => scanner.skip_block_comment(),
            '"' => {
                let line = scanner.line;
                let text = scanner.read_string();
                literals.push(Literal { line, text });
            }
            'r' => match scanner.raw_string_hashes() {
                Some(hashes) => {
                    let line = scanner.line;
                    let text = scanner.read_raw_string(hashes);
                    literals.push(Literal { line, text });
                }
                None => code.push(scanner.bump()),
            },
            '\'' => {
                let len = scanner.char_literal_len().unwrap_or(1);
                for _ in 0..len {
                    code.push(scanner.bump());
                }
            }
            _ => code.push(scanner.bump()),
        }
    }

    Scanned { literals, code }
}

struct Scanner {
    chars: Vec<char>,
    pos: usize,
    line: usize,
}

impl Scanner {
    fn new(content: &str) -> Self {
        Self {
            chars: content.chars().collect(),
            pos: 0,
            line: 1,
        }
    }

    fn peek(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    /// Consumes one char, tracking line numbers.
    ///
    /// # Panics
    /// Panics at end of input; callers only bump after a successful `peek`.
    fn bump(&mut self) -> char {
        let current = self.peek(0).expect("bump past end of input");
        self.pos += 1;

        if current == '\n' {
            self.line += 1;
        }

        current
    }

    fn skip_line_comment(&mut self) {
        while self.peek(0).is_some_and(|current| current != '\n') {
            self.bump();
        }
    }

    /// Positioned on `/*`; consumes through the matching `*/`, honouring
    /// Rust's nested block comments. An unterminated comment runs to end of
    /// input, as it would for `rustc`.
    fn skip_block_comment(&mut self) {
        self.bump();
        self.bump();
        let mut depth = 1;

        while depth > 0 {
            match (self.peek(0), self.peek(1)) {
                (Some('/'), Some('*')) => {
                    self.bump();
                    self.bump();
                    depth += 1;
                }
                (Some('*'), Some('/')) => {
                    self.bump();
                    self.bump();
                    depth -= 1;
                }
                (Some(_), _) => {
                    self.bump();
                }
                (None, _) => break,
            }
        }
    }

    /// Positioned on the opening `"`; returns the contents up to the
    /// closing unescaped `"`, keeping escape sequences verbatim.
    fn read_string(&mut self) -> String {
        self.bump();
        let mut text = String::new();

        while let Some(current) = self.peek(0) {
            self.bump();

            match current {
                '"' => break,
                '\\' => {
                    text.push('\\');
                    if self.peek(0).is_some() {
                        text.push(self.bump());
                    }
                }
                _ => text.push(current),
            }
        }

        text
    }

    /// Positioned on `r`: `Some(n)` when `r` + `n` hashes + `"` follows,
    /// i.e. a raw string opens here.
    fn raw_string_hashes(&self) -> Option<usize> {
        let mut hashes = 0;

        loop {
            match self.peek(1 + hashes) {
                Some('#') => hashes += 1,
                Some('"') => return Some(hashes),
                _ => return None,
            }
        }
    }

    /// Positioned on `r`; returns the contents up to `"` followed by
    /// `hashes` hashes.
    fn read_raw_string(&mut self, hashes: usize) -> String {
        for _ in 0..hashes + 2 {
            self.bump();
        }
        let mut text = String::new();

        while let Some(current) = self.peek(0) {
            self.bump();

            let closes = current == '"' && (0..hashes).all(|offset| self.peek(offset) == Some('#'));
            if closes {
                for _ in 0..hashes {
                    self.bump();
                }
                break;
            }

            text.push(current);
        }

        text
    }

    /// Positioned on `'`: the length of the char literal starting here
    /// (`'x'`, `'"'`, `'\''`, `'\u{41}'`), or `None` for a lifetime or
    /// label such as `'static` or `'outer:`.
    fn char_literal_len(&self) -> Option<usize> {
        match (self.peek(1), self.peek(2)) {
            (Some('\\'), _) => {
                let mut offset = 3;

                while let Some(current) = self.peek(offset) {
                    offset += 1;

                    match current {
                        '\'' => return Some(offset),
                        '\n' => return None,
                        _ => {}
                    }
                }

                None
            }
            (Some(_), Some('\'')) => Some(3),
            _ => None,
        }
    }
}
