//! A symbol, as SCIP writes it: the indexer's scheme, the package, and
//! the descriptors from the package's top down to the thing, as
//! `rust-analyzer cargo cox 0.1.0 daemon/chat/Chat#attach().` is the
//! method `attach` of the type `Chat` in the module `daemon::chat`.
//!
//! The grammar is `scip.proto`'s: the scheme and the package's three
//! fields end at a space, two spaces standing for one in them; a name
//! is a run of letters, digits and `_+-$`, or anything between
//! backticks, two standing for one; and the suffix after a name says
//! what it is.

use std::borrow::Cow;

/// A symbol, read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol<'a> {
    pub scheme: Cow<'a, str>,
    pub manager: Cow<'a, str>,
    pub package: Cow<'a, str>,
    pub descriptors: Vec<Descriptor<'a>>,
}

/// A step of a symbol's path, from the package down.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Descriptor<'a> {
    pub name: Cow<'a, str>,
    pub suffix: Suffix,
    /// What tells a method from others of its name, between its
    /// parentheses, as overloads have.
    pub disambiguator: &'a str,
    /// Where the descriptor ends in the symbol's text: the text up to it
    /// is the symbol of what it names, as a method's type's.
    pub end: usize,
}

/// What a descriptor names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Suffix {
    /// `name/`: a module, a package or a namespace.
    Namespace,
    /// `name#`
    Type,
    /// `name.`: a field, a variable or a constant.
    Term,
    /// `name(disambiguator).`
    Method,
    /// `[name]`
    TypeParameter,
    /// `(name)`
    Parameter,
    /// `name:`
    Meta,
    /// `name!`
    Macro,
}

/// Reads `symbol`, or nothing for a local symbol, one of a file's own,
/// or one that does not read.
#[must_use]
pub fn parse(symbol: &str) -> Option<Symbol<'_>> {
    if symbol.is_empty() || symbol.starts_with("local ") {
        return None;
    }
    let (scheme, at) = spaced(symbol, 0);
    let (manager, at) = spaced(symbol, at);
    let (package, at) = spaced(symbol, at);
    // The package's version, which the map has no use for.
    let (_, mut at) = spaced(symbol, at);
    let mut descriptors = Vec::new();
    let bytes = symbol.as_bytes();
    while at < symbol.len() {
        let descriptor = match bytes[at] {
            b'[' => {
                let (name, end) = name(symbol, at + 1)?;
                Descriptor {
                    name,
                    suffix: Suffix::TypeParameter,
                    disambiguator: "",
                    end: end + 1,
                }
            }
            b'(' => {
                let (name, end) = name(symbol, at + 1)?;
                Descriptor {
                    name,
                    suffix: Suffix::Parameter,
                    disambiguator: "",
                    end: end + 1,
                }
            }
            _ => {
                let (name, end) = name(symbol, at)?;
                let suffix = match bytes.get(end)? {
                    b'(' => {
                        let close = end + symbol[end..].find(')')?;
                        descriptors.push(Descriptor {
                            name,
                            suffix: Suffix::Method,
                            disambiguator: &symbol[end + 1..close],
                            end: close + 2,
                        });
                        at = close + 2;
                        continue;
                    }
                    b'/' => Suffix::Namespace,
                    b'#' => Suffix::Type,
                    b'.' => Suffix::Term,
                    b':' => Suffix::Meta,
                    b'!' => Suffix::Macro,
                    _ => return None,
                };
                Descriptor {
                    name,
                    suffix,
                    disambiguator: "",
                    end: end + 1,
                }
            }
        };
        at = descriptor.end;
        descriptors.push(descriptor);
    }
    Some(Symbol {
        scheme,
        manager,
        package,
        descriptors,
    })
}

/// The field of `symbol` from `at` to the next single space, and where
/// the field after it starts.
fn spaced(symbol: &str, at: usize) -> (Cow<'_, str>, usize) {
    let rest = &symbol[at.min(symbol.len())..];
    let mut end = 0;
    let mut escaped = false;
    let bytes = rest.as_bytes();
    while end < bytes.len() {
        if bytes[end] == b' ' {
            if bytes.get(end + 1) == Some(&b' ') {
                escaped = true;
                end += 2;
                continue;
            }
            let field = &rest[..end];
            let field = if escaped {
                Cow::Owned(field.replace("  ", " "))
            } else {
                Cow::Borrowed(field)
            };
            return (field, at + end + 1);
        }
        end += 1;
    }
    let field = if escaped {
        Cow::Owned(rest.replace("  ", " "))
    } else {
        Cow::Borrowed(rest)
    };
    (field, symbol.len())
}

/// The name of `symbol` at `at`, and where it ends; nothing when the
/// symbol ends in a name between backticks.
fn name(symbol: &str, at: usize) -> Option<(Cow<'_, str>, usize)> {
    let rest = symbol.get(at..)?;
    if let Some(quoted) = rest.strip_prefix('`') {
        let mut name = String::new();
        let mut chars = quoted.char_indices().peekable();
        while let Some((offset, c)) = chars.next() {
            if c == '`' {
                if chars.peek().map(|&(_, c)| c) == Some('`') {
                    chars.next();
                    name.push('`');
                    continue;
                }
                let end = at + 1 + offset + 1;
                return Some((Cow::Owned(name), end));
            }
            name.push(c);
        }
        return None;
    }
    let length = rest
        .char_indices()
        .find(|&(_, c)| !(c.is_alphanumeric() || "_+-$".contains(c)))
        .map_or(rest.len(), |(offset, _)| offset);
    Some((Cow::Borrowed(&rest[..length]), at + length))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steps(symbol: &str) -> Vec<(String, Suffix)> {
        parse(symbol)
            .unwrap()
            .descriptors
            .into_iter()
            .map(|descriptor| (descriptor.name.into_owned(), descriptor.suffix))
            .collect()
    }

    #[test]
    fn a_symbol_reads_into_its_package_and_descriptors() {
        let symbol = "rust-analyzer cargo cox 0.1.0 daemon/chat/impl#[Chat][Drop]drop().";
        let read = parse(symbol).unwrap();
        assert_eq!(read.scheme, "rust-analyzer");
        assert_eq!(read.manager, "cargo");
        assert_eq!(read.package, "cox");
        assert_eq!(
            steps(symbol),
            [
                ("daemon".to_owned(), Suffix::Namespace),
                ("chat".to_owned(), Suffix::Namespace),
                ("impl".to_owned(), Suffix::Type),
                ("Chat".to_owned(), Suffix::TypeParameter),
                ("Drop".to_owned(), Suffix::TypeParameter),
                ("drop".to_owned(), Suffix::Method),
            ]
        );
        let impl_type = &read.descriptors[2];
        assert_eq!(
            &symbol[..impl_type.end],
            "rust-analyzer cargo cox 0.1.0 daemon/chat/impl#"
        );
        assert_eq!(read.descriptors[5].end, symbol.len());
    }

    #[test]
    fn escaped_names_and_spaces_read_as_what_they_stand_for() {
        let symbol = "scip-typescript npm my  pkg 1.0.0 `src/a.ts`/`x``y`#(p)[T]m(+1).";
        let read = parse(symbol).unwrap();
        assert_eq!(read.package, "my pkg");
        assert_eq!(
            steps(symbol),
            [
                ("src/a.ts".to_owned(), Suffix::Namespace),
                ("x`y".to_owned(), Suffix::Type),
                ("p".to_owned(), Suffix::Parameter),
                ("T".to_owned(), Suffix::TypeParameter),
                ("m".to_owned(), Suffix::Method),
            ]
        );
        assert_eq!(read.descriptors[4].disambiguator, "+1");
    }

    #[test]
    fn local_and_broken_symbols_do_not_read() {
        assert_eq!(parse("local 12"), None);
        assert_eq!(parse(""), None);
        assert_eq!(parse("s m p v a/b"), None);
        assert_eq!(parse("s m p v m(."), None);
        assert_eq!(parse("s m p v `open/"), None);
        assert_eq!(
            steps("scip-go gomod example.com/m v1 `example.com/m/x`/T#f."),
            [
                ("example.com/m/x".to_owned(), Suffix::Namespace),
                ("T".to_owned(), Suffix::Type),
                ("f".to_owned(), Suffix::Term),
            ]
        );
    }
}
