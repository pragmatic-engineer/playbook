// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Splits a word into the parts the rm guard can expand itself: literal text,
//! `$NAME` and `${NAME}`, `${NAME:-default}`, `$$`, and the substitutions
//! `$(pwd)` and `$(mktemp)`. Anything else that expands, such as `$1`,
//! `${X%y}` or another substitution, makes the whole word unknown.

/// One part of a word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Piece {
    Text(String),
    /// `$NAME`, `${NAME}`, or `${NAME:-default}` / `${NAME-default}` with a
    /// literal default.
    Var {
        name: String,
        default: Option<String>,
    },
    /// `$$`, the shell's process id: digits only.
    Pid,
    /// `$(pwd)` or a backtick `pwd`, with `-L` or `-P` at most.
    Pwd,
    /// `$(mktemp ...)` with its arguments.
    Mktemp(Vec<String>),
}

/// The parts of `word`, a word with its quotes removed, or `None` when it
/// holds an expansion the guard does not model.
pub(super) fn pieces(word: &str) -> Option<Vec<Piece>> {
    let chars: Vec<char> = word.chars().collect();
    let mut out = Vec::new();
    let mut text = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c != '$' && c != '`' {
            text.push(c);
            i += 1;
            continue;
        }
        if !text.is_empty() {
            out.push(Piece::Text(std::mem::take(&mut text)));
        }
        let (piece, next) = if c == '`' {
            let end = (i + 1..chars.len()).find(|&j| chars[j] == '`')?;
            (substitution(&chars[i + 1..end])?, end + 1)
        } else {
            dollar(&chars, i)?
        };
        out.push(piece);
        i = next;
    }
    if !text.is_empty() {
        out.push(Piece::Text(text));
    }
    Some(out)
}

/// The piece that starts with the `$` at `at`, and where the word goes on.
fn dollar(chars: &[char], at: usize) -> Option<(Piece, usize)> {
    match chars.get(at + 1)? {
        '$' => Some((Piece::Pid, at + 2)),
        '(' => {
            if chars.get(at + 2) == Some(&'(') {
                return None;
            }
            let end = (at + 2..chars.len()).find(|&j| chars[j] == ')')?;
            Some((substitution(&chars[at + 2..end])?, end + 1))
        }
        '{' => {
            let end = (at + 2..chars.len()).find(|&j| chars[j] == '}')?;
            let inner: String = chars[at + 2..end].iter().collect();
            let name_len = identifier_len(&inner);
            if name_len == 0 {
                return None;
            }
            let (name, rest) = inner.split_at(name_len);
            let default = match rest {
                "" => None,
                _ => {
                    let value = rest.strip_prefix(":-").or_else(|| rest.strip_prefix('-'))?;
                    if value.contains(['$', '`', '{', '}']) {
                        return None;
                    }
                    Some(value.to_string())
                }
            };
            let name = name.to_string();
            Some((Piece::Var { name, default }, end + 1))
        }
        _ => {
            let rest: String = chars[at + 1..].iter().collect();
            let len = identifier_len(&rest);
            if len == 0 {
                return None;
            }
            let name = rest[..len].to_string();
            Some((
                Piece::Var {
                    name,
                    default: None,
                },
                at + 1 + len,
            ))
        }
    }
}

/// `pwd` or `mktemp` as the whole text of a substitution.
fn substitution(inner: &[char]) -> Option<Piece> {
    let text: String = inner.iter().collect();
    let words: Vec<String> = text.split_whitespace().map(str::to_string).collect();
    match words.split_first()? {
        (first, rest) if first == "pwd" => rest
            .iter()
            .all(|w| w == "-L" || w == "-P")
            .then_some(Piece::Pwd),
        (first, rest) if first == "mktemp" => Some(Piece::Mktemp(rest.to_vec())),
        _ => None,
    }
}

/// The length of the shell variable name at the start of `text`.
pub(super) fn identifier_len(text: &str) -> usize {
    let mut len = 0;
    for (i, c) in text.char_indices() {
        let ok = c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit());
        if !ok {
            break;
        }
        len = i + c.len_utf8();
    }
    len
}

/// Where `mktemp` with these arguments makes its file or folder: under
/// `temp` for the plain forms, with `-d`, `-q`, `-u` or `-t` and no
/// template, or where a template with three or more trailing `X`s and no
/// `-t` says, relative to the current folder when it has no leading `/`.
/// Any other option, such as `-p`, is unknown.
pub(super) fn mktemp_path(args: &[String], temp: &str) -> Option<String> {
    let (flags, operands): (Vec<&String>, Vec<&String>) =
        args.iter().partition(|a| a.len() > 1 && a.starts_with('-'));
    let plain = flags
        .iter()
        .all(|a| a[1..].chars().all(|c| matches!(c, 'd' | 'q' | 'u' | 't')));
    if !plain {
        return None;
    }
    match operands.as_slice() {
        [] => Some(format!("{}/tmp.mktemp", temp.trim_end_matches('/'))),
        [template] if !flags.iter().any(|f| f.contains('t')) => {
            let stem = template.trim_end_matches('X');
            let random = template.len() - stem.len();
            let literal = !stem.contains(['$', '`', '*', '?', '[']);
            (random >= 3 && literal).then(|| format!("{stem}mktemp"))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var(name: &str) -> Piece {
        Piece::Var {
            name: name.to_string(),
            default: None,
        }
    }

    fn text(t: &str) -> Piece {
        Piece::Text(t.to_string())
    }

    #[test]
    fn known_forms_split_into_pieces() {
        let cases: Vec<(&str, Vec<Piece>)> = vec![
            ("$S/x", vec![var("S"), text("/x")]),
            ("${S}/x", vec![var("S"), text("/x")]),
            ("$d/$f", vec![var("d"), text("/"), var("f")]),
            ("/tmp/st.$$", vec![text("/tmp/st."), Piece::Pid]),
            (
                "${TMPDIR:-/tmp}/a",
                vec![
                    Piece::Var {
                        name: "TMPDIR".to_string(),
                        default: Some("/tmp".to_string()),
                    },
                    text("/a"),
                ],
            ),
            ("$(pwd)/t", vec![Piece::Pwd, text("/t")]),
            ("`pwd -P`/t", vec![Piece::Pwd, text("/t")]),
            ("$(mktemp -d)", vec![Piece::Mktemp(vec!["-d".to_string()])]),
            ("$PWD/.b", vec![var("PWD"), text("/.b")]),
        ];
        for (word, want) in cases {
            assert_eq!(pieces(word), Some(want), "{word}");
        }
    }

    #[test]
    fn other_expansions_are_unknown() {
        for word in [
            "$1/x",
            "$@",
            "$?",
            "${#S}",
            "${S%/*}",
            "${S:=/x}",
            "${S:-$T}",
            "$(cat f)",
            "$((1+2))",
            "`ls`",
            "$",
            "${S",
            "$(pwd -x)",
        ] {
            assert_eq!(pieces(word), None, "{word}");
        }
    }

    #[test]
    fn only_plain_mktemp_forms_are_known() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            mktemp_path(&args(&["-d"]), "/t/"),
            Some("/t/tmp.mktemp".to_string())
        );
        assert_eq!(
            mktemp_path(&args(&[]), "/t"),
            Some("/t/tmp.mktemp".to_string())
        );
        assert!(mktemp_path(&args(&["-dq"]), "/t").is_some());
        assert_eq!(
            mktemp_path(&args(&["-d", "./.tmp-XXXX"]), "/t"),
            Some("./.tmp-mktemp".to_string())
        );
        assert_eq!(
            mktemp_path(&args(&["/x/y.XXXXXX"]), "/t"),
            Some("/x/y.mktemp".to_string())
        );
        assert_eq!(mktemp_path(&args(&["-d", "./x.XX"]), "/t"), None);
        assert_eq!(mktemp_path(&args(&["-t", "x.XXXX"]), "/t"), None);
        assert_eq!(mktemp_path(&args(&["--tmpdir=/x"]), "/t"), None);
        assert_eq!(mktemp_path(&args(&["-p", "/x"]), "/t"), None);
    }
}
