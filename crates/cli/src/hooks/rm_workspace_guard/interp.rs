// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Finds the deletions in a one-line or heredoc script for Python, Node,
//! Perl or Ruby: `shutil.rmtree`, `os.remove`, `fs.rmSync`, `unlink`,
//! `FileUtils.rm_rf` and the like. A call whose path is a plain string
//! literal names that path; any other argument (a variable, a joined or
//! expanded path, an f-string) is unknown, so the guard blocks it. A script
//! that deletes nothing gives nothing.

use crate::common::re::static_regex;
use regex::Regex;
use std::sync::LazyLock;

/// One path a script deletes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Arg {
    /// A path written as a plain string literal.
    Literal(String),
    /// A path the guard cannot read, named for the denial.
    Unknown(String),
}

/// The interpreters the guard reads scripts for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Lang {
    Python,
    Node,
    Perl,
    Ruby,
}

impl Lang {
    /// The language of a program name such as `python3.12` or `node`.
    pub(super) fn of(program: &str) -> Option<Lang> {
        let name = program.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
        match name {
            "python" | "pypy" => Some(Lang::Python),
            "node" | "nodejs" | "bun" | "deno" => Some(Lang::Node),
            "perl" => Some(Lang::Perl),
            "ruby" => Some(Lang::Ruby),
            _ => None,
        }
    }

    /// The option that takes the script as its value.
    pub(super) fn inline_option(self) -> &'static [&'static str] {
        match self {
            Lang::Python => &["-c"],
            Lang::Node => &["-e", "--eval", "-p", "--print"],
            Lang::Perl => &["-e", "-E"],
            Lang::Ruby => &["-e"],
        }
    }

    fn label(self) -> &'static str {
        match self {
            Lang::Python => "<python expression>",
            Lang::Node => "<node expression>",
            Lang::Perl => "<perl expression>",
            Lang::Ruby => "<ruby expression>",
        }
    }
}

static PYTHON: LazyLock<Regex> = LazyLock::new(|| {
    static_regex(
        r"\b(?:shutil\s*\.\s*rmtree|os\s*\.\s*(?:remove|unlink|rmdir|removedirs)|send2trash(?:\s*\.\s*send2trash)?|rmtree)\s*\(",
    )
});
static PYTHON_METHOD: LazyLock<Regex> =
    LazyLock::new(|| static_regex(r"\.\s*(?:unlink|rmdir)\s*\("));
static NODE: LazyLock<Regex> = LazyLock::new(|| {
    static_regex(
        r"(?:\b(?:rmSync|unlinkSync|rmdirSync|rimraf|rimrafSync)|\.\s*(?:rm|unlink|rmdir))\s*\(",
    )
});
static PERL: LazyLock<Regex> =
    LazyLock::new(|| static_regex(r"\b(?:unlink|rmdir|rmtree|remove_tree)\b\s*\(?"));
static RUBY: LazyLock<Regex> = LazyLock::new(|| {
    static_regex(
        r"\b(?:FileUtils\s*\.\s*(?:rm_rf|rm_r|rm_f|rm|remove_dir|remove_entry_secure|remove_entry|remove_file|rmtree|rmdir)|File\s*\.\s*(?:delete|unlink)|Dir\s*\.\s*(?:rmdir|delete|unlink))\b\s*\(?",
    )
});
/// `Path("x")` right before a method call, as in `Path("x").unlink()`.
static PATH_CALL: LazyLock<Regex> = LazyLock::new(|| {
    static_regex(r#"(?:Path|PurePath|PosixPath)\s*\(\s*(?:'([^'\\]*)'|"([^"\\]*)")\s*\)\s*$"#)
});

/// Every path `code`, a script in `lang`, deletes.
pub(super) fn deletions(lang: Lang, code: &str) -> Vec<Arg> {
    let mut out = Vec::new();
    let calls: &Regex = match lang {
        Lang::Python => &PYTHON,
        Lang::Node => &NODE,
        Lang::Perl => &PERL,
        Lang::Ruby => &RUBY,
    };
    let mut spans = Vec::new();
    for found in calls.find_iter(code) {
        spans.push(found.range());
        out.extend(arguments(lang, &code[found.end()..]));
    }
    if lang == Lang::Python {
        let seen = |at: usize| spans.iter().any(|r| r.contains(&at));
        for found in PYTHON_METHOD.find_iter(code).filter(|f| !seen(f.start())) {
            let before = &code[..found.start()];
            let arg = PATH_CALL.captures(before).and_then(|c| {
                c.get(1)
                    .or_else(|| c.get(2))
                    .map(|m| m.as_str().to_string())
            });
            out.push(match arg {
                Some(path) => Arg::Literal(path),
                None => Arg::Unknown(lang.label().to_string()),
            });
        }
    }
    out
}

/// The paths a deletion call takes, read from the text just after its name
/// or opening parenthesis: plain string literals separated by commas. Perl
/// and Ruby take a list, the others one path.
fn arguments(lang: Lang, rest: &str) -> Vec<Arg> {
    let list = matches!(lang, Lang::Perl | Lang::Ruby);
    let mut out = Vec::new();
    let mut text = rest.trim_start();
    loop {
        let Some((literal, after)) = string_literal(text) else {
            out.push(Arg::Unknown(lang.label().to_string()));
            return out;
        };
        out.push(Arg::Literal(literal));
        let after = after.trim_start();
        match after.chars().next() {
            Some(',') if list => text = after[1..].trim_start(),
            // A keyword argument such as `ignore_errors=True` or an options
            // object follows the path.
            Some(',') | Some(')') | Some(';') | None => return out,
            Some(_) if list => return out,
            Some(_) => {
                out.push(Arg::Unknown(lang.label().to_string()));
                return out;
            }
        }
    }
}

/// A plain single- or double-quoted string at the start of `text`, and the
/// text after it. A string with a backslash, an interpolation or a prefix
/// such as `f"` is not plain.
fn string_literal(text: &str) -> Option<(String, &str)> {
    let quote = text.chars().next().filter(|c| *c == '\'' || *c == '"')?;
    let body = &text[1..];
    let end = body.find(quote)?;
    let literal = &body[..end];
    let interpolates = quote == '"' && (literal.contains('$') || literal.contains("#{"));
    if literal.contains('\\') || interpolates || literal.contains('`') {
        return None;
    }
    Some((literal.to_string(), &body[end + 1..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(path: &str) -> Arg {
        Arg::Literal(path.to_string())
    }

    #[test]
    fn literal_paths_are_named_and_other_arguments_are_unknown() {
        let unknown = |lang: Lang| Arg::Unknown(lang.label().to_string());
        let cases: Vec<(Lang, &str, Vec<Arg>)> = vec![
            (
                Lang::Python,
                "import shutil; shutil.rmtree(\"/tmp/v\")",
                vec![lit("/tmp/v")],
            ),
            (
                Lang::Python,
                "shutil.rmtree('/tmp/v', ignore_errors=True)",
                vec![lit("/tmp/v")],
            ),
            (
                Lang::Python,
                "import os; os.remove('crates/x.new')",
                vec![lit("crates/x.new")],
            ),
            (
                Lang::Python,
                "shutil.rmtree(os.path.expanduser(\"~\"))",
                vec![unknown(Lang::Python)],
            ),
            (
                Lang::Python,
                "os.unlink(f'/tmp/{x}')",
                vec![unknown(Lang::Python)],
            ),
            (Lang::Python, "Path('/tmp/a').unlink()", vec![lit("/tmp/a")]),
            (
                Lang::Python,
                "p.unlink(missing_ok=True)",
                vec![unknown(Lang::Python)],
            ),
            (
                Lang::Python,
                "s=open(p).read()\nopen(p,'w').write(s)",
                vec![],
            ),
            (
                Lang::Node,
                "require('fs').rmSync('/etc/x', {recursive: true})",
                vec![lit("/etc/x")],
            ),
            (Lang::Node, "fs.unlinkSync(p)", vec![unknown(Lang::Node)]),
            (
                Lang::Perl,
                "unlink \"crates/x.tmp\"",
                vec![lit("crates/x.tmp")],
            ),
            (Lang::Perl, "unlink 'a', 'b';", vec![lit("a"), lit("b")]),
            (Lang::Perl, "unlink $f", vec![unknown(Lang::Perl)]),
            (
                Lang::Ruby,
                "FileUtils.rm_rf(\"/tmp/x\")",
                vec![lit("/tmp/x")],
            ),
            (
                Lang::Ruby,
                "File.delete(\"#{d}/x\")",
                vec![unknown(Lang::Ruby)],
            ),
            (Lang::Python, "print('rm -rf /')", vec![]),
        ];
        for (lang, code, want) in cases {
            assert_eq!(deletions(lang, code), want, "{lang:?}: {code}");
        }
    }

    #[test]
    fn interpreter_names_are_recognised_with_versions() {
        assert_eq!(Lang::of("python3"), Some(Lang::Python));
        assert_eq!(Lang::of("python3.12"), Some(Lang::Python));
        assert_eq!(Lang::of("node"), Some(Lang::Node));
        assert_eq!(Lang::of("perl5.36"), Some(Lang::Perl));
        assert_eq!(Lang::of("pythonista"), None);
        assert_eq!(Lang::of("bash"), None);
    }
}
