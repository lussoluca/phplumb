//! Names written in Twig templates: string literals inside `{{ }}` and `{% %}`.

/// Tags whose first argument is a template name.
const TEMPLATE_TAGS: [&str; 6] = ["extends", "include", "embed", "import", "from", "use"];

/// Tags whose first argument may be a list of template names, the first existing
/// one being used.
const LIST_TAGS: [&str; 2] = ["extends", "include"];

/// What a name written in a template stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TwigName {
    Route,
    Template,
}

/// Tags whose content Twig outputs as written.
const VERBATIM_TAGS: [(&str, &str); 2] = [("verbatim", "endverbatim"), ("raw", "endraw")];

/// A string literal inside a Twig expression or tag.
struct Literal {
    /// The range between the delimiters of the `{{ }}` or `{% %}` around it.
    block: (usize, usize),
    /// Whether that block is a `{% %}` tag.
    tag: bool,
    /// The opening quote, and the range inside the quotes.
    quote: usize,
    content: usize,
    close: usize,
}

impl Literal {
    fn value(&self, source: &[u8]) -> (String, u32, u32) {
        (
            String::from_utf8_lossy(&source[self.content..self.close]).into_owned(),
            self.content as u32,
            self.close as u32,
        )
    }
}

/// The route name under `offset`: a string literal that is the whole first
/// argument of `path(` or `url(`, or their argument named `name`, with its range
/// inside the quotes.
pub(crate) fn route_name_at(source: &[u8], offset: u32) -> Option<(String, u32, u32)> {
    let literal = literal_at(source, offset as usize)?;
    is_route_argument(source, &literal).then(|| literal.value(source))
}

/// Whether the literal is the route name of a `path()` or `url()` call.
fn is_route_argument(source: &[u8], literal: &Literal) -> bool {
    let functions = ["path", "url"];
    is_function_argument(source, literal, &functions)
        || is_named_argument(source, literal, &functions, "name")
}

/// The template name under `offset`: the whole first argument of `include(` or
/// `source(` or an element of a list there, the second argument of `block(`, the
/// template a tag names (also an element of a list for `extends` and `include`, or
/// a branch of a conditional), or a `form_theme` theme.
pub(crate) fn template_name_at(source: &[u8], offset: u32) -> Option<(String, u32, u32)> {
    let literal = literal_at(source, offset as usize)?;
    template_use(source, &literal).map(|_| literal.value(source))
}

/// Whether the literal names a template, and whether that template must exist:
/// one of a list, or a branch of a conditional, may be missing.
fn template_use(source: &[u8], literal: &Literal) -> Option<bool> {
    // `{% include('a') %}` is the tag, whose argument goes on past the parenthesis.
    let function = is_function_argument(source, literal, &["include", "source"])
        && !is_tag_parenthesis(source, literal);
    if function || is_second_argument(source, literal, "block") {
        return Some(true);
    }
    if is_function_list_element(source, literal, &["include"]) {
        return Some(false);
    }
    if is_tag_argument(source, literal) {
        return Some(!in_list(source, literal));
    }
    if is_conditional_branch(source, literal) {
        return Some(false);
    }
    form_theme(source, literal)
}

/// Every route name and template name `source` writes as a whole literal argument,
/// with its range inside the quotes. Names a template may lack, in an element of a
/// list, a branch of a conditional or with `ignore missing`, are left out, as is
/// text inside `verbatim`.
pub(crate) fn names(source: &[u8]) -> Vec<(TwigName, String, u32, u32)> {
    let mut found = Vec::new();
    let mut i = 0;
    while i + 1 < source.len() {
        let close: &[u8] = match &source[i..i + 2] {
            b"{{" => b"}}",
            b"{%" => b"%}",
            b"{#" => b"#}",
            _ => {
                i += 1;
                continue;
            }
        };
        let start = i + 2;
        let end = find(source, start, close).unwrap_or(source.len());
        i = end + 2;
        if close == b"#}" {
            continue;
        }
        let tag = close == b"%}";
        let text = String::from_utf8_lossy(&source[start..end]);
        let words: Vec<&str> = text.split_whitespace().collect();
        if tag
            && let Some((_, closing)) = VERBATIM_TAGS.iter().find(|(opening, _)| {
                words.first().map(|w| w.trim_start_matches(['-', '~'])) == Some(*opening)
            })
        {
            // Up to the closing tag, whatever its spacing.
            while let Some(next) = find(source, i, b"{%") {
                let after = find(source, next + 2, b"%}").unwrap_or(source.len());
                let inner = String::from_utf8_lossy(&source[next + 2..after]);
                i = after + 2;
                if inner.trim().trim_matches(['-', '~']).trim() == *closing {
                    break;
                }
            }
            continue;
        }
        let optional =
            words.join(" ").contains("ignore missing") || text.contains("ignore_missing");
        for literal in block_literals(source, start, end, tag) {
            // `#{...}` interpolates an expression into a double-quoted string.
            let interpolated = source[literal.quote] == b'"'
                && find(source, literal.content, b"#{").is_some_and(|at| at < literal.close);
            if interpolated {
                continue;
            }
            // `ignore_missing` is the 4th argument of `include()`, the 2nd of `source()`.
            let lenient = |function: &str, arguments: usize| {
                is_function_argument(source, &literal, &[function])
                    && argument_count(source, &literal) >= arguments
            };
            let kind = if is_route_argument(source, &literal) {
                TwigName::Route
            } else if optional || lenient("include", 4) || lenient("source", 2) {
                continue;
            } else if template_use(source, &literal) == Some(true) {
                TwigName::Template
            } else {
                continue;
            };
            let (name, start, end) = literal.value(source);
            found.push((kind, name, start, end));
        }
    }
    found
}

/// How many arguments the call whose first argument is the literal has, counting
/// the commas at its own depth up to its closing parenthesis.
fn argument_count(source: &[u8], literal: &Literal) -> usize {
    let mut count = 1;
    let mut depth = 0;
    let mut i = literal.close + 1;
    while i < literal.block.1 {
        match source[i] {
            quote @ (b'\'' | b'"') => {
                i += 1;
                while i < literal.block.1 && source[i] != quote {
                    i += if source[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' if depth == 0 => break,
            b')' | b']' | b'}' => depth -= 1,
            b',' if depth == 0 => count += 1,
            _ => {}
        }
        i += 1;
    }
    count
}

/// Whether the literal is an element of a list rather than the tag's argument.
fn in_list(source: &[u8], literal: &Literal) -> bool {
    let mut i = literal.quote;
    while i > literal.block.0 && source[i - 1].is_ascii_whitespace() {
        i -= 1;
    }
    i > literal.block.0 && matches!(source[i - 1], b'[' | b',')
}

/// The innermost string literal under `offset` inside a `{{ }}` or `{% %}`.
fn literal_at(source: &[u8], offset: usize) -> Option<Literal> {
    let (start, end, tag) = block_at(source, offset)?;
    block_literals(source, start, end, tag)
        .into_iter()
        .rev()
        .find(|literal| literal.content <= offset && offset <= literal.close)
}

/// The string literals of the block between `start` and `end`, each followed by
/// those inside the `#{...}` interpolations of a double-quoted one.
fn block_literals(source: &[u8], start: usize, end: usize, tag: bool) -> Vec<Literal> {
    let mut found = Vec::new();
    literals_in(source, (start, end), start, end, tag, &mut found);
    found
}

/// The string literals between `from` and `to`, inside `block`.
fn literals_in(
    source: &[u8],
    block: (usize, usize),
    from: usize,
    to: usize,
    tag: bool,
    found: &mut Vec<Literal>,
) {
    let mut i = from;
    while i < to {
        let quote = source[i];
        if quote != b'\'' && quote != b'"' {
            i += 1;
            continue;
        }
        let content = i + 1;
        let mut j = content;
        let mut interpolations = Vec::new();
        while j < to && source[j] != quote {
            if quote == b'"' && source[j] == b'#' && source.get(j + 1) == Some(&b'{') {
                let open = j + 2;
                let close = interpolation_end(source, open, to);
                interpolations.push((open, close));
                j = close + 1;
                continue;
            }
            j += if source[j] == b'\\' { 2 } else { 1 };
        }
        let close = j.min(to);
        found.push(Literal {
            block,
            tag,
            quote: i,
            content,
            close,
        });
        for (open, close) in interpolations {
            literals_in(source, block, open, close, tag, found);
        }
        i = close + 1;
    }
}

/// Where the interpolation whose expression starts at `from` ends: its closing
/// `}`, past nested braces and strings, or `to`.
fn interpolation_end(source: &[u8], from: usize, to: usize) -> usize {
    let mut depth = 0;
    let mut i = from;
    while i < to {
        match source[i] {
            quote @ (b'\'' | b'"') => {
                i += 1;
                while i < to && source[i] != quote {
                    i += if source[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'{' => depth += 1,
            b'}' if depth == 0 => return i,
            b'}' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    to
}

/// The `{{ }}` or `{% %}` around `offset`: the range between its delimiters, and
/// whether it is a tag. Comments hold no names.
fn block_at(source: &[u8], offset: usize) -> Option<(usize, usize, bool)> {
    let mut i = 0;
    while i + 1 < source.len() {
        let close: &[u8] = match &source[i..i + 2] {
            b"{{" => b"}}",
            b"{%" => b"%}",
            b"{#" => b"#}",
            _ => {
                i += 1;
                continue;
            }
        };
        let start = i + 2;
        let end = find(source, start, close).unwrap_or(source.len());
        if close != b"#}" && start <= offset && offset <= end {
            return Some((start, end, close == b"%}"));
        }
        if start > offset {
            return None;
        }
        i = end + 2;
    }
    None
}

fn find(source: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    source[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| from + p)
}

fn skip_spaces(source: &[u8], mut i: usize, end: usize) -> usize {
    while i < end && source[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// Whether the next character after the literal, past spaces, is one of `allowed`.
fn followed_by(source: &[u8], literal: &Literal, allowed: &[u8]) -> bool {
    let after = skip_spaces(source, literal.close + 1, literal.block.1);
    after < literal.block.1 && allowed.contains(&source[after])
}

/// Whether the literal is the whole first argument of one of `functions`: `name(`
/// right before it, the name not part of a longer name or a method call, and `,` or
/// `)` right after it.
fn is_function_argument(source: &[u8], literal: &Literal, functions: &[&str]) -> bool {
    let block = literal.block.0;
    let i = skip_spaces_back(source, literal.quote, block);
    i > block
        && source[i - 1] == b'('
        && calls_one_of(source, block, i - 1, functions)
        && followed_by(source, literal, b",)")
}

/// Whether the literal is the whole argument named `parameter` of one of
/// `functions`, written `parameter: value` or `parameter = value`, with `,` or `)`
/// right after it.
fn is_named_argument(
    source: &[u8],
    literal: &Literal,
    functions: &[&str],
    parameter: &str,
) -> bool {
    // A macro's arguments are its defaults, not a call.
    if literal.tag && tag_name(source, literal.block) == "macro" {
        return false;
    }
    let block = literal.block.0;
    let mut i = skip_spaces_back(source, literal.quote, block);
    if i == block || !matches!(source[i - 1], b':' | b'=') {
        return false;
    }
    i -= 1;
    // `==`, `<=`, `>=` and `!=` compare.
    if i > block && matches!(source[i - 1], b'=' | b'<' | b'>' | b'!') {
        return false;
    }
    i = skip_spaces_back(source, i, block);
    let name_end = i;
    while i > block && (source[i - 1].is_ascii_alphanumeric() || source[i - 1] == b'_') {
        i -= 1;
    }
    if &source[i..name_end] != parameter.as_bytes() {
        return false;
    }
    i = skip_spaces_back(source, i, block);
    let open = match i.checked_sub(1).filter(|&before| before >= block) {
        Some(before) if source[before] == b'(' => before,
        // After other arguments: the parenthesis that opens them.
        Some(before) if source[before] == b',' => match enclosing_open(source, block, before) {
            Some(open) if source[open] == b'(' => open,
            _ => return false,
        },
        _ => return false,
    };
    calls_one_of(source, block, open, functions) && followed_by(source, literal, b",)")
}

/// Whether the parenthesis at `open` follows the name of one of `functions`, the
/// name not part of a longer name or a method call.
fn calls_one_of(source: &[u8], block: usize, open: usize, functions: &[&str]) -> bool {
    let mut i = skip_spaces_back(source, open, block);
    let name_end = i;
    while i > block && (source[i - 1].is_ascii_alphanumeric() || source[i - 1] == b'_') {
        i -= 1;
    }
    let name = &source[i..name_end];
    let attached = i > block && source[i - 1] == b'.';
    !attached && functions.iter().any(|f| f.as_bytes() == name)
}

/// The bracket that opens the group holding `from`, past nested groups and
/// strings, scanning back to `block`.
fn enclosing_open(source: &[u8], block: usize, from: usize) -> Option<usize> {
    let mut depth = 0;
    let mut i = from;
    while i > block {
        i -= 1;
        match source[i] {
            quote @ (b'\'' | b'"') => {
                while i > block {
                    i -= 1;
                    if source[i] == quote && source[i - 1] != b'\\' {
                        break;
                    }
                }
            }
            b')' | b']' | b'}' => depth += 1,
            b'(' | b'[' | b'{' if depth == 0 => return Some(i),
            b'(' | b'[' | b'{' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// The position before the spaces that end at `i`, not before `block`.
fn skip_spaces_back(source: &[u8], mut i: usize, block: usize) -> usize {
    while i > block && source[i - 1].is_ascii_whitespace() {
        i -= 1;
    }
    i
}

/// The name of the tag between `start` and `end`, past whitespace control.
fn tag_name(source: &[u8], (start, end): (usize, usize)) -> &str {
    let mut i = skip_spaces(source, start, end);
    if i < end && matches!(source[i], b'-' | b'~') {
        i = skip_spaces(source, i + 1, end);
    }
    let word_start = i;
    while i < end && (source[i].is_ascii_alphabetic() || source[i] == b'_') {
        i += 1;
    }
    std::str::from_utf8(&source[word_start..i]).unwrap_or_default()
}

/// Whether the literal is the template a tag names: the first argument of one of
/// the template tags, or an element of the list right after `extends` or
/// `include`. A literal that is part of a larger expression is not.
fn is_tag_argument(source: &[u8], literal: &Literal) -> bool {
    if !literal.tag {
        return false;
    }
    let end = literal.block.1;
    let tag = tag_name(source, literal.block);
    if !TEMPLATE_TAGS.contains(&tag) {
        return false;
    }
    let argument = tag_argument(source, literal.block);
    if argument == literal.quote {
        return !continues(source, literal.close + 1, end);
    }
    // `{% extends('a') %}`
    if argument < end
        && source[argument] == b'('
        && skip_spaces(source, argument + 1, end) == literal.quote
    {
        let after = skip_spaces(source, literal.close + 1, end);
        return after < end && source[after] == b')' && !continues(source, after + 1, end);
    }
    if LIST_TAGS.contains(&tag) && argument < end && source[argument] == b'[' {
        // Only a direct element of that first list, not of a later one.
        let mut depth = 0;
        let mut k = argument + 1;
        while k < literal.quote {
            match source[k] {
                quote @ (b'\'' | b'"') => {
                    k += 1;
                    while k < literal.quote && source[k] != quote {
                        k += if source[k] == b'\\' { 2 } else { 1 };
                    }
                }
                b'[' | b'{' | b'(' => depth += 1,
                b']' | b'}' | b')' => {
                    if depth == 0 {
                        return false;
                    }
                    depth -= 1;
                }
                _ => {}
            }
            k += 1;
        }
        if depth != 0 {
            return false;
        }
        let mut before = literal.quote;
        while before > argument && source[before - 1].is_ascii_whitespace() {
            before -= 1;
        }
        let in_list = before > argument && matches!(source[before - 1], b'[' | b',');
        return in_list && followed_by(source, literal, b",]");
    }
    false
}

/// Whether the expression goes on at `from`, past spaces, before `end`: a `~`,
/// `|`, `.`, `+`, `[` or `(`, except a `~` that trims the whitespace after the tag.
fn continues(source: &[u8], from: usize, end: usize) -> bool {
    let next = skip_spaces(source, from, end);
    !at_end(source, next, end) && b"~|.+[(".contains(&source[next])
}

/// Whether `at` is the end of the block: its closing delimiter, or the `-` or `~`
/// right before it that trims the whitespace after it.
fn at_end(source: &[u8], at: usize, end: usize) -> bool {
    at >= end || (matches!(source[at], b'-' | b'~') && at + 1 == end)
}

/// Where the argument of the tag between `start` and `end` starts, past its name.
fn tag_argument(source: &[u8], (start, end): (usize, usize)) -> usize {
    let mut i = skip_spaces(source, start, end);
    if i < end && matches!(source[i], b'-' | b'~') {
        i = skip_spaces(source, i + 1, end);
    }
    let tag = tag_name(source, (start, end));
    skip_spaces(source, i + tag.len(), end)
}

/// Whether the literal is right inside a parenthesis that opens a template tag's
/// argument, as in `{% include('a') %}`.
fn is_tag_parenthesis(source: &[u8], literal: &Literal) -> bool {
    if !literal.tag || !TEMPLATE_TAGS.contains(&tag_name(source, literal.block)) {
        return false;
    }
    let argument = tag_argument(source, literal.block);
    source.get(argument) == Some(&b'(')
        && skip_spaces(source, argument + 1, literal.block.1) == literal.quote
}

/// Where the group opened at `open` closes, past nested groups and strings.
fn closing(source: &[u8], open: usize, end: usize) -> Option<usize> {
    let mut depth = 0;
    let mut i = open + 1;
    while i < end {
        match source[i] {
            quote @ (b'\'' | b'"') => {
                i += 1;
                while i < end && source[i] != quote {
                    i += if source[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' if depth == 0 => return Some(i),
            b')' | b']' | b'}' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    None
}

/// Whether the literal is a direct element of a list that is the whole first
/// argument of one of `functions`.
fn is_function_list_element(source: &[u8], literal: &Literal, functions: &[&str]) -> bool {
    let (block, end) = literal.block;
    let before = skip_spaces_back(source, literal.quote, block);
    if before == block || !matches!(source[before - 1], b'[' | b',') {
        return false;
    }
    let Some(open) = enclosing_open(source, block, literal.quote) else {
        return false;
    };
    let paren = skip_spaces_back(source, open, block);
    let Some(close) = closing(source, open, end) else {
        return false;
    };
    let after = skip_spaces(source, close + 1, end);
    source[open] == b'['
        && paren > block
        && source[paren - 1] == b'('
        && calls_one_of(source, block, paren - 1, functions)
        && followed_by(source, literal, b",]")
        && after < end
        && matches!(source[after], b',' | b')')
}

/// Whether the literal is the whole second argument of `function`.
fn is_second_argument(source: &[u8], literal: &Literal, function: &str) -> bool {
    let block = literal.block.0;
    let before = skip_spaces_back(source, literal.quote, block);
    if before == block || source[before - 1] != b',' {
        return false;
    }
    let Some(open) = enclosing_open(source, block, literal.quote) else {
        return false;
    };
    if source[open] != b'(' || !calls_one_of(source, block, open, &[function]) {
        return false;
    }
    // One comma at the call's own depth, the one before the literal.
    let mut commas = 0;
    let mut depth = 0;
    let mut i = open + 1;
    while i < literal.quote {
        match source[i] {
            quote @ (b'\'' | b'"') => {
                i += 1;
                while i < literal.quote && source[i] != quote {
                    i += if source[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b',' if depth == 0 => commas += 1,
            _ => {}
        }
        i += 1;
    }
    commas == 1 && followed_by(source, literal, b",)")
}

/// Whether the literal is a branch of a conditional that is the argument of a
/// template tag, as in `{% extends x ? 'a' : 'b' %}`.
fn is_conditional_branch(source: &[u8], literal: &Literal) -> bool {
    let (block, end) = literal.block;
    if !literal.tag || !["extends", "include", "embed"].contains(&tag_name(source, literal.block)) {
        return false;
    }
    let before = skip_spaces_back(source, literal.quote, block);
    let after = skip_spaces(source, literal.close + 1, end);
    enclosing_open(source, block, literal.quote).is_none()
        && before > block
        && matches!(source[before - 1], b'?' | b':')
        && (at_end(source, after, end)
            || source[after] == b':'
            || source[after].is_ascii_alphabetic())
}

/// Whether the literal is a theme of a `form_theme` tag, and whether it must exist:
/// a theme of a list after `with` may be missing.
fn form_theme(source: &[u8], literal: &Literal) -> Option<bool> {
    let (block, end) = literal.block;
    if !literal.tag || tag_name(source, literal.block) != "form_theme" {
        return None;
    }
    let before = skip_spaces_back(source, literal.quote, block);
    if let Some(open) = enclosing_open(source, block, literal.quote) {
        let word_end = skip_spaces_back(source, open, block);
        let in_themes = source[open] == b'['
            && enclosing_open(source, block, open).is_none()
            && source[..word_end].ends_with(b"with")
            && matches!(source[before - 1], b'[' | b',')
            && followed_by(source, literal, b",]");
        return in_themes.then_some(false);
    }
    // Past the form: a theme ends its own expression.
    let first = tag_argument(source, literal.block);
    let operator = b"~|.+-*/%=<>?:(,[{".contains(&source[before - 1]);
    let after = skip_spaces(source, literal.close + 1, end);
    let ends = at_end(source, after, end)
        || matches!(source[after], b'\'' | b'"')
        || source[after].is_ascii_alphabetic();
    (literal.quote != first && !operator && ends).then_some(true)
}

#[cfg(test)]
mod tests {
    use super::{TwigName, names, route_name_at, template_name_at};

    fn route(source: &str, needle: &str) -> Option<String> {
        let offset = source.find(needle).unwrap() as u32 + 1;
        route_name_at(source.as_bytes(), offset).map(|(name, _, _)| name)
    }

    fn template(source: &str, needle: &str) -> Option<String> {
        let offset = source.find(needle).unwrap() as u32 + 1;
        template_name_at(source.as_bytes(), offset).map(|(name, _, _)| name)
    }

    #[test]
    fn reads_route_names() {
        assert_eq!(route("{{ path('home') }}", "'home'"), Some("home".into()));
        assert_eq!(route("{% if url( \"a\" ) %}", "\"a\""), Some("a".into()));
        assert_eq!(route("{{ x.path('home') }}", "'home'"), None);
        assert_eq!(route("{{ mypath('home') }}", "'home'"), None);
        assert_eq!(route("{# path('home') #}", "'home'"), None);
        assert_eq!(route("path('home')", "'home'"), None);
        assert_eq!(route("{{ path('a', {b: 'c'}) }}", "'c'"), None);
        assert_eq!(route("{{ path('a' ~ b) }}", "'a'"), None);
    }

    #[test]
    fn reads_more_template_forms() {
        for (source, needle) in [
            ("{{ include(['a', 'b']) }}", "'a'"),
            ("{{ include([ 'x', 'a' ]) }}", "'a'"),
            ("{% extends x ? 'a' : 'b' %}", "'a'"),
            ("{% extends x ? 'b' : 'a' %}", "'a'"),
            ("{% extends x ? 'a' : y %}", "'a'"),
            ("{% extends('a') %}", "'a'"),
            ("{% include 'a' ~%}", "'a'"),
            ("{{ block('n', 'a') }}", "'a'"),
            ("{% form_theme form 'a' %}", "'a'"),
            ("{% form_theme form 'x' 'a' %}", "'a'"),
            ("{% form_theme form with 'a' %}", "'a'"),
            ("{% form_theme form with ['x', 'a'] %}", "'a'"),
            ("{% form_theme form 'a' -%}", "'a'"),
            ("{% form_theme form with 'a' ~%}", "'a'"),
            ("{% extends x ? 'b' : 'a' -%}", "'a'"),
            ("{% include ('a') %}", "'a'"),
        ] {
            assert_eq!(template(source, needle), Some("a".into()), "{source}");
        }
        for (source, needle) in [
            ("{% include 'a' ~ b %}", "'a'"),
            ("{{ block('a') }}", "'a'"),
            ("{{ block('n', 'a') }}", "'n'"),
            ("{% set x = c ? 'a' : 'b' %}", "'a'"),
            ("{% include 'x' with {a: 'b'} %}", "'b'"),
            ("{% form_theme form with {a: 'b'} %}", "'b'"),
            ("{{ include(['a'] ~ b) }}", "'a'"),
            ("{{ x.include(['a']) }}", "'a'"),
            ("{% include('a') ~ b %}", "'a'"),
            ("{% include ('a') ~ b %}", "'a'"),
            ("{% include 'a' ~ %}", "'a'"),
        ] {
            assert_eq!(template(source, needle), None, "{source} {needle}");
        }
    }

    #[test]
    fn lists_only_templates_that_must_exist() {
        let found: Vec<String> = names(
            b"{% extends('a') %}{% include 'b' ~%}{{ block('n', 'c') }}{% form_theme form 'd' %}\
              {{ include(['e', 'f']) }}{% extends x ? 'g' : 'h' %}{% form_theme form with ['i'] %}",
        )
        .into_iter()
        .filter(|(kind, ..)| *kind == TwigName::Template)
        .map(|(_, name, ..)| name)
        .collect();
        assert_eq!(found, ["a", "b", "c", "d"]);
    }

    #[test]
    fn reads_route_names_passed_by_name() {
        assert_eq!(
            route("{{ path(name: 'home') }}", "'home'"),
            Some("home".into())
        );
        assert_eq!(
            route("{{ url(name = 'home') }}", "'home'"),
            Some("home".into())
        );
        assert_eq!(
            route("{{ path(parameters: {id: 1}, name: 'home') }}", "'home'"),
            Some("home".into())
        );
        assert_eq!(route("{{ path('a', {name: 'b'}) }}", "'b'"), None);
        assert_eq!(route("{{ path(other: 'home') }}", "'home'"), None);
        assert_eq!(route("{{ path(x == 'home') }}", "'home'"), None);
        assert_eq!(route("{% macro url(name = 'home') %}", "'home'"), None);
        assert!(names(b"{% macro path(name = 'x') %}{% endmacro %}").is_empty());
    }

    #[test]
    fn reads_route_names_inside_interpolations() {
        assert_eq!(
            route("{{ \"#{path('home')}\" }}", "'home'"),
            Some("home".into())
        );
        assert_eq!(
            route(
                "{{ \"#{ {a: 'b'}|json_encode }\" ~ path('home') }}",
                "'home'"
            ),
            Some("home".into())
        );
        assert_eq!(route("{{ 'x#{path(\"home\")}' }}", "\"home\""), None);
        assert_eq!(
            route("{{ \"#{ '\"' }\" ~ path('home') }}", "'home'"),
            Some("home".into())
        );
    }

    #[test]
    fn lists_interpolated_and_named_route_names() {
        let found: Vec<String> = names(b"{{ \"x#{path('a')}\" }}{{ path(name: 'b') }}")
            .into_iter()
            .map(|(_, name, _, _)| name)
            .collect();
        assert_eq!(found, ["a", "b"]);
    }

    #[test]
    fn reads_template_names() {
        assert_eq!(template("{% extends 'a' %}", "'a'"), Some("a".into()));
        assert_eq!(
            template("{%- include \"a\" with {} -%}", "\"a\""),
            Some("a".into())
        );
        assert_eq!(template("{% from 'a' import b %}", "'a'"), Some("a".into()));
        assert_eq!(
            template("{% extends ['x', 'a'] %}", "'a'"),
            Some("a".into())
        );
        assert_eq!(template("{{ include('a') }}", "'a'"), Some("a".into()));
        assert_eq!(template("{{ source( 'a' ) }}", "'a'"), Some("a".into()));
        assert_eq!(template("{% include 'a' ~ b %}", "'a'"), None);
        assert_eq!(template("{% set x = 'a' %}", "'a'"), None);
        assert_eq!(template("{% embed 'a' with {b: 'c'} %}", "'c'"), None);
        assert_eq!(template("{% import ['a'] as x %}", "'a'"), None);
        assert_eq!(template("{{ 'a' }}", "'a'"), None);
        assert_eq!(
            template("{% include ['a'] with {items: ['b']} %}", "'b'"),
            None
        );
        assert_eq!(template("{% include ['a'] with ['x'] %}", "'x'"), None);
    }
}
