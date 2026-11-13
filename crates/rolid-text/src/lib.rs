//! Messages for the user, made in the library crates and translated in the UI.
//!
//! The library crates (ssh, config, archive, core…) know nothing about
//! languages. Where they have something to tell the user, they return a
//! [`Text`]: the English message as a template with named placeholders
//! (`"Key {path} is encrypted"`), an optional context and plural, and the
//! values for the placeholders. The UI translates it (`rolid_i18n::t`); its
//! `Display` gives the English text, used for the log.
//!
//! The macros mark the messages for the extraction into `po/rolidscp.pot`
//! (gettext style, like `N_()`): `txt!`, `txtc!` (with context), `txtn!`
//! (plural), and `n!` / `nc!` for a bare `&'static str` that the UI
//! translates later (labels in constant tables).
//!
//! Placeholders are `{name}`; `{{` and `}}` are literal braces. A plural
//! message gets the count as `{n}`.

use std::fmt;

/// A value for a placeholder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arg {
    /// Used as it is (a name, a path, a number formatted already).
    Str(String),
    /// A whole number: the UI writes it the way the language does (grouping).
    Int(i128),
    /// A message inside a message (a reason, an error): translated too.
    Text(Box<Text>),
}

macro_rules! from_int {
    ($($t:ty),*) => {$(
        impl From<$t> for Arg {
            fn from(v: $t) -> Self {
                Arg::Int(v as i128)
            }
        }
    )*};
}
from_int!(u8, u16, u32, u64, usize, i8, i16, i32, i64, isize);

impl From<String> for Arg {
    fn from(v: String) -> Self {
        Arg::Str(v)
    }
}

impl From<&String> for Arg {
    fn from(v: &String) -> Self {
        Arg::Str(v.clone())
    }
}

impl From<&str> for Arg {
    fn from(v: &str) -> Self {
        Arg::Str(v.to_string())
    }
}

impl From<char> for Arg {
    fn from(v: char) -> Self {
        Arg::Str(v.to_string())
    }
}

impl From<Text> for Arg {
    fn from(v: Text) -> Self {
        Arg::Text(Box::new(v))
    }
}

impl From<&Text> for Arg {
    fn from(v: &Text) -> Self {
        Arg::Text(Box::new(v.clone()))
    }
}

impl From<&std::path::Path> for Arg {
    fn from(v: &std::path::Path) -> Self {
        Arg::Str(v.display().to_string())
    }
}

impl From<std::path::PathBuf> for Arg {
    fn from(v: std::path::PathBuf) -> Self {
        Arg::Str(v.display().to_string())
    }
}

impl From<&std::path::PathBuf> for Arg {
    fn from(v: &std::path::PathBuf) -> Self {
        Arg::Str(v.display().to_string())
    }
}

/// A message for the user, translated by the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Text {
    /// gettext context (`msgctxt`): tells apart the same English words with
    /// different meanings.
    pub ctx: Option<&'static str>,
    /// The English message (`msgid`), singular for a plural message.
    pub id: &'static str,
    /// English plural (`msgid_plural`) and the count deciding the form.
    pub plural: Option<(&'static str, u64)>,
    pub args: Vec<(&'static str, Arg)>,
}

impl Text {
    pub const fn new(id: &'static str) -> Self {
        Self { ctx: None, id, plural: None, args: Vec::new() }
    }

    /// Add a placeholder value.
    #[must_use]
    pub fn arg(mut self, name: &'static str, v: impl Into<Arg>) -> Self {
        self.args.push((name, v.into()));
        self
    }

    /// The English template for the count (`n == 1` singular).
    pub fn english_template(&self) -> &'static str {
        match self.plural {
            Some((p, n)) if n != 1 => p,
            _ => self.id,
        }
    }
}

impl fmt::Display for Text {
    /// The English text (the log, and where no translation exists).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = fill(self.english_template(), |name| {
            if name == "n" {
                if let Some((_, n)) = self.plural {
                    if !self.args.iter().any(|(a, _)| *a == "n") {
                        return Some(n.to_string());
                    }
                }
            }
            self.args.iter().find(|(a, _)| *a == name).map(|(_, v)| match v {
                Arg::Str(s) => s.clone(),
                Arg::Int(i) => i.to_string(),
                Arg::Text(t) => t.to_string(),
            })
        });
        f.write_str(&s)
    }
}

impl From<Text> for String {
    fn from(t: Text) -> String {
        t.to_string()
    }
}

/// Replace `{name}` in `template` by `value(name)`; `{{` / `}}` are literal
/// braces; a placeholder without a value stays as it is (so a mistake in a
/// translation shows, but never breaks the text).
pub fn fill(template: &str, mut value: impl FnMut(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(i) = rest.find(['{', '}']) {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        if let Some(r) = tail.strip_prefix("{{") {
            out.push('{');
            rest = r;
        } else if let Some(r) = tail.strip_prefix("}}") {
            out.push('}');
            rest = r;
        } else if let Some(r) = tail.strip_prefix('{') {
            match r.find('}') {
                Some(end) if is_name(&r[..end]) => {
                    let name = &r[..end];
                    match value(name) {
                        Some(v) => out.push_str(&v),
                        None => out.push_str(&tail[..end + 2]),
                    }
                    rest = &r[end + 1..];
                }
                _ => {
                    out.push('{');
                    rest = r;
                }
            }
        } else {
            out.push('}');
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out
}

fn is_name(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The placeholder names in a template (to check translations).
pub fn placeholders(template: &str) -> Vec<String> {
    let mut names = Vec::new();
    fill(template, |n| {
        names.push(n.to_string());
        Some(String::new())
    });
    names.sort();
    names.dedup();
    names
}

/// A message for the user: `txt!("Cannot read {path}", path = p)`.
#[macro_export]
macro_rules! txt {
    ($id:literal $(, $name:ident = $val:expr)* $(,)?) => {
        $crate::Text::new($id)$(.arg(stringify!($name), $val))*
    };
}

/// A message with a gettext context: `txtc!("verb", "Open")`.
#[macro_export]
macro_rules! txtc {
    ($ctx:literal, $id:literal $(, $name:ident = $val:expr)* $(,)?) => {
        $crate::Text { ctx: Some($ctx), ..$crate::Text::new($id) }$(.arg(stringify!($name), $val))*
    };
}

/// A plural message: `txtn!("{n} file", "{n} files", count)`; `{n}` is the count.
#[macro_export]
macro_rules! txtn {
    ($id:literal, $plural:literal, $n:expr $(, $name:ident = $val:expr)* $(,)?) => {
        $crate::Text { plural: Some(($plural, ($n) as u64)), ..$crate::Text::new($id) }$(.arg(stringify!($name), $val))*
    };
}

/// A plural message with a context.
#[macro_export]
macro_rules! txtnc {
    ($ctx:literal, $id:literal, $plural:literal, $n:expr $(, $name:ident = $val:expr)* $(,)?) => {
        $crate::Text { ctx: Some($ctx), plural: Some(($plural, ($n) as u64)), ..$crate::Text::new($id) }$(.arg(stringify!($name), $val))*
    };
}

/// Mark a bare English string for translation later (`N_()` in gettext):
/// it stays a `&'static str`, usable in constants; the UI translates it.
#[macro_export]
macro_rules! n {
    ($id:literal) => {
        $id
    };
}

/// Like [`n!`] with a context (the UI must translate it with the same context).
#[macro_export]
macro_rules! nc {
    ($ctx:literal, $id:literal) => {
        $id
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_display_and_placeholders() {
        let t = txt!("Cannot read {path}: {why}", path = "/a b", why = txt!("access denied"));
        assert_eq!(t.to_string(), "Cannot read /a b: access denied");
        let p = |n| txtn!("{n} file", "{n} files", n).to_string();
        assert_eq!((p(1), p(0), p(2)), ("1 file".into(), "0 files".into(), "2 files".into()));
        assert_eq!(txtc!("verb", "Open").ctx, Some("verb"));
        assert_eq!(fill("{{x}} {a} {missing} {", |n| (n == "a").then(|| "A".into())), "{x} A {missing} {");
        assert_eq!(placeholders("{b} and {a} and {b} {{c}}"), vec!["a".to_string(), "b".to_string()]);
        const LABEL: &str = n!("Pink");
        assert_eq!(LABEL, "Pink");
    }
}
