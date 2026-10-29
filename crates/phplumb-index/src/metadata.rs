//! Class metadata: PHP attributes and Doctrine docblock annotations, as small value
//! trees with class names resolved.

use serde::{Deserialize, Serialize};

use crate::phpdoc::NameContext;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MetaValue {
    String(String),
    /// `X::class` in attributes; fully qualified, no leading backslash.
    Class(String),
    Bool(bool),
    Number(String),
    /// Arrays, `{ ... }` annotation maps and lists: keyed entries keep their key.
    List(Vec<(Option<String>, MetaValue)>),
    Null,
    /// `X::NAME`, fully qualified, no leading backslash; `self` for `self::NAME` and
    /// `static::NAME`.
    Constant {
        class: String,
        name: String,
    },
    /// A nested attribute (`new X(...)`) or annotation (`@X(...)`).
    Nested(Box<Metadata>),
    Other,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Metadata {
    /// Fully qualified attribute or annotation class name.
    pub name: String,
    /// Named arguments by name; positional ones with no name.
    pub arguments: Vec<(Option<String>, MetaValue)>,
    /// Where the attribute or annotation name is in the file.
    pub start: u32,
    pub end: u32,
    /// The whole attribute or annotation, arguments included.
    pub span: (u32, u32),
    /// A docblock annotation rather than an attribute.
    #[serde(default)]
    pub annotation: bool,
    /// The name is qualified or imported, so it names its class itself; always
    /// true for an attribute.
    #[serde(default)]
    pub explicit: bool,
}

impl Metadata {
    /// The argument named `name`.
    pub fn argument(&self, name: &str) -> Option<&MetaValue> {
        self.arguments
            .iter()
            .find(|(key, _)| key.as_deref() == Some(name))
            .map(|(_, value)| value)
    }
}

impl MetaValue {
    /// The entry keyed `key` of a list.
    pub fn entry(&self, key: &str) -> Option<&MetaValue> {
        match self {
            MetaValue::List(entries) => entries
                .iter()
                .find(|(k, _)| k.as_deref() == Some(key))
                .map(|(_, value)| value),
            _ => None,
        }
    }

    /// A string or a class name, as text.
    pub fn text(&self) -> Option<&str> {
        match self {
            MetaValue::String(text) | MetaValue::Class(text) => Some(text),
            _ => None,
        }
    }
}

/// The Doctrine annotations of a docblock whose text starts at byte `offset` of the
/// file. An annotation that does not parse is left out.
pub(crate) fn annotations(doc: &str, offset: u32, names: &dyn NameContext) -> Vec<Metadata> {
    if !doc.contains("@") || !doc.contains('(') {
        return Vec::new();
    }
    let mut text = blank_margins(doc);
    blank_code_examples(&mut text);
    let mut parser = Annotations {
        text: &text,
        pos: 0,
        offset,
        names,
        depth: 0,
    };
    let mut found = Vec::new();
    while let Some(at) = parser.next_top_level() {
        parser.pos = at;
        match parser.annotation() {
            Some(metadata) => found.push(metadata),
            None => parser.pos = at + 1,
        }
    }
    found
}

/// Turns `@code` ... `@endcode` examples into spaces: annotations written there
/// document something, they declare nothing.
fn blank_code_examples(text: &mut [u8]) {
    let mut from = 0;
    while let Some(start) = find(&text[from..], b"@code").map(|i| from + i) {
        let end =
            find(&text[start..], b"@endcode").map_or(text.len(), |i| start + i + b"@endcode".len());
        text[start..end]
            .iter_mut()
            .filter(|b| **b != b'\n')
            .for_each(|b| *b = b' ');
        from = end;
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// The comment with its delimiters and the `*` margin of each line turned into
/// spaces, so byte positions stay those of the comment.
fn blank_margins(doc: &str) -> Vec<u8> {
    let mut bytes = doc.as_bytes().to_vec();
    let length = bytes.len();
    if bytes.starts_with(b"/**") {
        bytes[..3].fill(b' ');
    }
    if bytes.ends_with(b"*/") {
        bytes[length - 2..].fill(b' ');
    }
    let mut line_start = true;
    for byte in bytes.iter_mut() {
        if *byte == b'\n' {
            line_start = true;
        } else if line_start && *byte == b'*' {
            *byte = b' ';
            line_start = false;
        } else if !byte.is_ascii_whitespace() {
            line_start = false;
        }
    }
    bytes
}

struct Annotations<'a> {
    text: &'a [u8],
    pos: usize,
    /// Where the comment starts in the file.
    offset: u32,
    names: &'a dyn NameContext,
    /// How many maps and annotations enclose the current position.
    depth: usize,
}

/// Deeper nesting is treated as a broken annotation.
const MAX_DEPTH: usize = 32;

impl Annotations<'_> {
    /// The next `@Name(` that starts a line.
    fn next_top_level(&mut self) -> Option<usize> {
        while self.pos < self.text.len() {
            let at = self.pos;
            self.pos += 1;
            if self.text[at] != b'@' {
                continue;
            }
            let line_start = self.text[..at]
                .iter()
                .rev()
                .take_while(|b| **b != b'\n')
                .all(u8::is_ascii_whitespace);
            let starts_name = self
                .text
                .get(at + 1)
                .is_some_and(|b| b.is_ascii_uppercase() || *b == b'\\');
            if line_start && starts_name {
                return Some(at);
            }
        }
        None
    }

    fn annotation(&mut self) -> Option<Metadata> {
        let start = self.pos;
        self.expect(b'@')?;
        let name_start = self.pos;
        let name = self.name()?;
        let name_end = self.pos;
        self.skip_space();
        self.expect(b'(')?;
        let arguments = self.entries(b')')?;
        let resolved = self.names.resolve_class(&name);
        Some(Metadata {
            name: resolved.trim_start_matches('\\').to_string(),
            arguments,
            start: self.offset + name_start as u32,
            end: self.offset + name_end as u32,
            span: (self.offset + start as u32, self.offset + self.pos as u32),
            annotation: true,
            explicit: name.contains('\\') || self.names.is_imported(&name),
        })
    }

    /// Entries up to and including `close`: `key = value`, `key: value` or `value`,
    /// separated by commas, with an optional trailing comma.
    fn entries(&mut self, close: u8) -> Option<Vec<(Option<String>, MetaValue)>> {
        if self.depth >= MAX_DEPTH {
            return None;
        }
        self.depth += 1;
        let entries = self.entries_inside(close);
        self.depth -= 1;
        entries
    }

    fn entries_inside(&mut self, close: u8) -> Option<Vec<(Option<String>, MetaValue)>> {
        let mut entries = Vec::new();
        loop {
            self.skip_space();
            if self.peek()? == close {
                self.pos += 1;
                return Some(entries);
            }
            let before = self.pos;
            let key = self.key();
            self.skip_space();
            let key = match (key, self.peek()?) {
                (Some(key), b'=' | b':') => {
                    self.pos += 1;
                    Some(key)
                }
                _ => {
                    self.pos = before;
                    None
                }
            };
            let value = self.value()?;
            entries.push((key, value));
            self.skip_space();
            match self.peek()? {
                b',' => self.pos += 1,
                byte if byte == close => {}
                _ => return None,
            }
        }
    }

    /// A key: a quoted string, a name or a number.
    fn key(&mut self) -> Option<String> {
        self.skip_space();
        match self.peek()? {
            b'"' => self.string(),
            byte if byte.is_ascii_alphanumeric() || byte == b'_' => {
                let start = self.pos;
                while self
                    .peek()
                    .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_')
                {
                    self.pos += 1;
                }
                Some(String::from_utf8_lossy(&self.text[start..self.pos]).into_owned())
            }
            _ => None,
        }
    }

    fn value(&mut self) -> Option<MetaValue> {
        self.skip_space();
        match self.peek()? {
            b'"' => self.string().map(MetaValue::String),
            b'{' => {
                self.pos += 1;
                self.entries(b'}').map(MetaValue::List)
            }
            b'@' => self.annotation().map(|m| MetaValue::Nested(Box::new(m))),
            byte if byte.is_ascii_digit() || byte == b'-' => {
                let start = self.pos;
                self.pos += 1;
                while self.peek().is_some_and(|b| b.is_ascii_digit() || b == b'.') {
                    self.pos += 1;
                }
                Some(MetaValue::Number(
                    String::from_utf8_lossy(&self.text[start..self.pos]).into_owned(),
                ))
            }
            _ => {
                let name = self.name()?;
                // `Foo::class` and `Foo::BAR` stay unknown.
                if self.text[self.pos..].starts_with(b"::") {
                    self.pos += 2;
                    self.name()?;
                    return Some(MetaValue::Other);
                }
                Some(match name.to_ascii_lowercase().as_str() {
                    "true" => MetaValue::Bool(true),
                    "false" => MetaValue::Bool(false),
                    _ => MetaValue::Other,
                })
            }
        }
    }

    /// A double-quoted string; `""` stands for one quote.
    fn string(&mut self) -> Option<String> {
        self.expect(b'"')?;
        let mut value = Vec::new();
        loop {
            let byte = self.peek()?;
            self.pos += 1;
            if byte == b'"' {
                if self.peek() == Some(b'"') {
                    value.push(b'"');
                    self.pos += 1;
                    continue;
                }
                return Some(String::from_utf8_lossy(&value).into_owned());
            }
            value.push(byte);
        }
    }

    /// A class-like name, possibly qualified.
    fn name(&mut self) -> Option<String> {
        let start = self.pos;
        while self
            .peek()
            .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'\\')
        {
            self.pos += 1;
        }
        (self.pos > start)
            .then(|| String::from_utf8_lossy(&self.text[start..self.pos]).into_owned())
    }

    fn expect(&mut self, byte: u8) -> Option<()> {
        (self.peek()? == byte).then(|| self.pos += 1)
    }

    fn peek(&self) -> Option<u8> {
        self.text.get(self.pos).copied()
    }

    fn skip_space(&mut self) {
        while self.peek().is_some_and(|b| b.is_ascii_whitespace()) {
            self.pos += 1;
        }
    }
}
