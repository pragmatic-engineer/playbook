// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Whether a simple command writes playbook's config store: the config
//! database, a config file or the memory store. Reading it is fine, so only a
//! redirect into it, a program that writes where it is pointed, `sed` with
//! `-i` or a `w` command, or `sqlite3` with anything but a read counts. A copy
//! counts only where it lands, so `cp config.db /tmp/backup` is a read.
//! Deleting is the rm guard's call, so `rm` is left to it.

use super::settings_writes::redirect_targets;
use super::{is_short_cluster_with, names_config_store, names_memory_store, WRITER_PROGRAMS};
use crate::common::shell::{program_index, program_name, redirect_len, SHELLS};

/// Interpreters, which can write any file they are handed.
const INTERPRETERS: [&str; 5] = ["python", "python3", "node", "ruby", "perl"];

/// `sqlite3` options that take the next word as their value.
const SQLITE_VALUE_OPTIONS: [&str; 10] = [
    "-separator",
    "-newline",
    "-nullvalue",
    "-vfs",
    "-mmap",
    "-maxsize",
    "-lookaside",
    "-heap",
    "-pagecache",
    "-A",
];

/// The `sqlite3` dot commands that only read.
const SQLITE_READ_COMMANDS: [&str; 16] = [
    ".tables",
    ".table",
    ".schema",
    ".fullschema",
    ".indexes",
    ".indices",
    ".dump",
    ".headers",
    ".header",
    ".mode",
    ".width",
    ".databases",
    ".help",
    ".show",
    ".timer",
    ".stats",
];

/// How a read statement starts.
const SQL_READ_STARTS: [&str; 4] = ["select", "with", "explain", "values"];

/// SQL words that change a database, or reach outside it. `replace` counts
/// only when it is not the `replace()` function.
const SQL_WRITE_WORDS: [&str; 18] = [
    "insert",
    "update",
    "delete",
    "replace",
    "drop",
    "create",
    "alter",
    "vacuum",
    "reindex",
    "attach",
    "detach",
    "upsert",
    "writefile",
    "edit",
    "load_extension",
    "begin",
    "savepoint",
    "analyze",
];

/// Pragmas that only read when they take no value.
const READ_PRAGMAS: [&str; 16] = [
    "table_info",
    "table_xinfo",
    "table_list",
    "index_list",
    "index_info",
    "index_xinfo",
    "foreign_key_list",
    "database_list",
    "compile_options",
    "integrity_check",
    "quick_check",
    "page_count",
    "page_size",
    "user_version",
    "schema_version",
    "journal_mode",
];

/// Of `READ_PRAGMAS`, the ones whose argument in parentheses names what to
/// read, as in `pragma table_info(settings)`.
const ARGUMENT_READ_PRAGMAS: [&str; 9] = [
    "table_info",
    "table_xinfo",
    "table_list",
    "index_list",
    "index_info",
    "index_xinfo",
    "foreign_key_list",
    "integrity_check",
    "quick_check",
];

/// Whether this simple command writes the config store. A command that is
/// only a redirection, as `>f` after `&` or `|` splits it off, counts too.
pub(crate) fn writes_config_store(words: &[String], home: &str) -> bool {
    let store = |w: &str| names_config_store(w, home) || names_memory_store(w, home);
    if redirect_targets(words).iter().any(|t| store(t)) {
        return true;
    }
    let Some(at) = program_index(words) else {
        return false;
    };
    let program = program_name(&words[at]);
    let args = &words[at + 1..];
    if !program_writes(&program, args) {
        return false;
    }
    match program.as_str() {
        // Deleting is judged by the rm guard, which keeps what cannot be made
        // again and lets the rest of playbook's folder go.
        "rm" => false,
        // A copy or link writes only where it lands.
        "cp" | "install" => copies_into(args, true, store),
        // `rsync -t` keeps times; it names no folder.
        "rsync" => copies_into(args, false, store),
        "dd" => args.iter().filter_map(|w| w.strip_prefix("of=")).any(store),
        // `mv` also takes its sources away, `tee` and `sed -i` write every
        // file they name, and a link is a second name to write through.
        "mv" | "tee" | "sed" | "ln" => args.iter().any(|w| store(w)),
        // Making a folder or touching a file in the memory store is how a
        // fact gets a home; only the config files are kept from these, and
        // from interpreters, which may only read the memory store.
        _ => args.iter().any(|w| names_config_store(w, home)),
    }
}

/// Whether `cp`, `install` or `rsync` with `args` may write into the store:
/// a `-t` folder in it, or any operand after the first in it. Options can
/// follow the operands and take values, so the last operand is not trusted to
/// be the destination; only the first operand, which is always a source, is
/// left out. `cp <store file> /tmp/x` is a read, while `cp a <store file> -S x`
/// and `cp <store file> <store file>` are writes.
fn copies_into(args: &[String], target_option: bool, store: impl Fn(&str) -> bool) -> bool {
    let mut operands = Vec::new();
    let mut words = args.iter();
    while let Some(word) = words.next() {
        if let Some(dir) = word.strip_prefix("--target-directory=") {
            if store(dir) {
                return true;
            }
            continue;
        }
        let short_t = target_option
            && word.starts_with('-')
            && !word.starts_with("--")
            && word.ends_with('t');
        if word == "--target-directory" || short_t {
            if words.next().is_some_and(|dir| store(dir)) {
                return true;
            }
            continue;
        }
        if let Some(dir) = target_option
            .then_some(word)
            .and_then(|w| w.strip_prefix('-'))
            .filter(|w| !w.starts_with('-'))
            .and_then(|w| w.split_once('t'))
            .map(|(_, dir)| dir)
            .filter(|d| !d.is_empty())
        {
            if store(dir) {
                return true;
            }
            continue;
        }
        if !word.starts_with('-') && redirect_len(word) == 0 {
            operands.push(word.as_str());
        }
    }
    operands.iter().skip(1).any(|w| store(w))
}

/// Whether the previous command ended in a bare redirect operator (the lexer
/// splits `>|`, `>&` and `&>` from their target) and this one starts with
/// the config store.
pub(crate) fn is_open_redirect_into_store(
    previous: Option<&[String]>,
    words: &[String],
    home: &str,
) -> bool {
    previous
        .and_then(|p| p.last())
        .is_some_and(|w| w.ends_with('>'))
        && words
            .first()
            .is_some_and(|w| names_config_store(w, home) || names_memory_store(w, home))
}

/// Whether `program` with `args` can write a file it is handed.
fn program_writes(program: &str, args: &[String]) -> bool {
    match program {
        "sqlite3" => !sqlite_only_reads(args),
        "sed" => args.iter().any(|w| {
            is_short_cluster_with(w, 'i') || w.starts_with("--in-place") || sed_script_writes(w)
        }),
        _ => {
            WRITER_PROGRAMS.contains(&program)
                || INTERPRETERS.contains(&program)
                || SHELLS.contains(&program)
        }
    }
}

/// Whether a `sed` script holds a `w` or `W` command or flag, which writes
/// the file named after it, as in `w f`, `1w f` or `s/a/b/w f`.
fn sed_script_writes(script: &str) -> bool {
    let chars: Vec<char> = script.chars().collect();
    chars.iter().enumerate().any(|(i, c)| {
        matches!(c, 'w' | 'W')
            && (i == 0 || !chars[i - 1].is_ascii_alphabetic())
            && chars.get(i + 1).is_some_and(|n| *n == ' ' || *n == '\t')
    })
}

/// Whether a `sqlite3` call only reads: every statement on its command line
/// is a read, none holds a shell expansion, and no option runs other SQL. SQL
/// fed on standard input is not seen, so it counts as a write.
fn sqlite_only_reads(args: &[String]) -> bool {
    let mut operands = Vec::new();
    let mut i = 0;
    while let Some(word) = args.get(i) {
        let option = word.trim_start_matches('-');
        if matches!(option, "cmd" | "init") && word.starts_with('-') {
            return false;
        }
        let redirect = redirect_len(word);
        if redirect > 0 {
            i += redirect;
        } else if SQLITE_VALUE_OPTIONS.contains(&format!("-{option}").as_str())
            && word.starts_with('-')
        {
            i += 2;
        } else {
            if !word.starts_with('-') {
                operands.push(word);
            }
            i += 1;
        }
    }
    let Some((_, statements)) = operands.split_first() else {
        return false;
    };
    !statements.is_empty()
        && statements
            .iter()
            .all(|sql| !sql.contains(['$', '`']) && sql_only_reads(sql))
}

fn sql_only_reads(sql: &str) -> bool {
    let lower = without_string_literals(&sql.to_lowercase());
    if lower.trim_start().starts_with('.') {
        return lower
            .split_whitespace()
            .next()
            .is_some_and(|c| SQLITE_READ_COMMANDS.contains(&c));
    }
    let words: Vec<&str> = lower
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .collect();
    let writes = words.iter().any(|w| SQL_WRITE_WORDS.contains(w))
        && !(words.contains(&"replace") && only_replace_calls(&lower, &words));
    let reads = lower
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .all(|s| SQL_READ_STARTS.iter().any(|k| s.starts_with(k)) || pragma_only_reads(s));
    reads && !writes
}

/// Whether `replace` is the only write word and every use is the function.
fn only_replace_calls(lower: &str, words: &[&str]) -> bool {
    words
        .iter()
        .filter(|w| SQL_WRITE_WORDS.contains(w))
        .all(|w| *w == "replace")
        && lower
            .match_indices("replace")
            .all(|(at, _)| lower[at + "replace".len()..].trim_start().starts_with('('))
}

fn pragma_only_reads(statement: &str) -> bool {
    let Some(rest) = statement.strip_prefix("pragma") else {
        return false;
    };
    let rest = rest.trim();
    let name_end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'))
        .unwrap_or(rest.len());
    let name = rest[..name_end].rsplit('.').next().unwrap_or_default();
    let tail = rest[name_end..].trim();
    if tail.is_empty() {
        READ_PRAGMAS.contains(&name)
    } else {
        tail.starts_with('(') && ARGUMENT_READ_PRAGMAS.contains(&name)
    }
}

/// `sql` with each single-quoted string literal emptied, so a word inside a
/// search text is not read as SQL.
fn without_string_literals(sql: &str) -> String {
    let mut out = String::new();
    let mut in_literal = false;
    for c in sql.chars() {
        if c == '\'' {
            in_literal = !in_literal;
            out.push(c);
        } else if !in_literal {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn sqlite_reads_are_told_apart_from_writes() {
        let reads: &[&[&str]] = &[
            &["db", "select * from settings"],
            &["db", ".tables"],
            &["-header", "db", "pragma table_info(settings)"],
            &["db", "pragma user_version"],
            &["-separator", "|", "db", "select 1"],
            &["db", "select replace(v, 'a', 'b') from settings"],
            &["db", "select * from facts where body like '%delete%'"],
            &["db", "with x as (select 1) select * from x"],
            &["db", ".schema", "2>/dev/null"],
        ];
        for words in reads {
            assert!(sqlite_only_reads(&args(words)), "{words:?}");
        }
        let writes: &[&[&str]] = &[
            &["db"],
            &["db", "update settings set v=1"],
            &["db", "select 1; delete from settings"],
            &["db", "with x as (select 1) delete from settings"],
            &["db", "select writefile('x', 1)"],
            &["db", "pragma writable_schema=1"],
            &["db", "pragma user_version(9)"],
            &["db", "pragma optimize"],
            &["db", "pragma incremental_vacuum"],
            &["db", ".read x.sql"],
            &["db", ".import x.csv settings"],
            &["db", "select $Q"],
            &["-cmd", "delete from settings", "db", "select 1"],
            &["--init", "x.sql", "db", "select 1"],
            &["-readonly", "db", "update settings set v=1"],
            &["-nullvalue", "-readonly", "db", "update settings set v=1"],
            &["db", "insert or replace into settings values (1)"],
        ];
        for words in writes {
            assert!(!sqlite_only_reads(&args(words)), "{words:?}");
        }
    }

    #[test]
    fn a_copy_writes_only_where_it_may_land() {
        let store = |w: &str| w.starts_with("S");
        let writes: &[&[&str]] = &[
            &["a", "S"],
            &["-r", "a", "b", "S/dir"],
            &["-t", "S", "a", "b"],
            &["-at", "S", "a"],
            &["--target-directory=S", "a"],
            &["--target-directory", "S", "a"],
            &["-tS", "a"],
            &["-atS", "a"],
            &["a", "S", "-S", ".bak"],
            &["S1", "S2"],
        ];
        for words in writes {
            assert!(copies_into(&args(words), true, store), "{words:?}");
        }
        // For rsync `-t` keeps times, so the next word is still an operand.
        assert!(copies_into(&args(&["-avt", "a", "S"]), false, store));
        assert!(!copies_into(&args(&["-avt", "S", "b"]), false, store));
        let reads: &[&[&str]] = &[
            &["S", "b"],
            &["-r", "S", "/tmp/x"],
            &["S", "-S", ".bak", "b"],
            &["-t", "/tmp", "S"],
            &["S"],
            &["S", "b", "2>/dev/null"],
        ];
        for words in reads {
            assert!(!copies_into(&args(words), true, store), "{words:?}");
        }
    }

    #[test]
    fn deleting_and_reading_the_store_are_not_writes() {
        let home = "/h";
        let db = "/h/.config/playbook/data/config.db";
        let fact = "/h/.config/playbook/memory/acme/x.md";
        for words in [
            vec!["rm", "-f", db],
            vec!["cp", db, "/tmp/backup.db"],
            vec!["cp", fact, "/tmp/x.md"],
            vec!["mkdir", "-p", "/h/.config/playbook/memory/acme/app/staging"],
            vec!["python3", "tool.py", fact],
            vec!["dd", &format!("if={db}"), "of=/tmp/x"],
        ] {
            assert!(!writes_config_store(&args(&words), home), "{words:?}");
        }
        for words in [
            vec!["cp", "/tmp/x", db],
            vec!["cp", "/tmp/x.md", fact],
            vec!["cp", "/tmp/x", db, "-S", ".bak"],
            vec!["ln", "-s", db, "/tmp/c.db"],
            vec!["mv", fact, "/tmp/x.md"],
            vec!["tee", fact],
            vec!["sed", "-i", "s/a/b/", fact],
            vec!["dd", "if=/dev/zero", &format!("of={db}")],
            vec!["echo", "x", &format!(">{fact}")],
            vec!["python3", "tool.py", db],
        ] {
            assert!(writes_config_store(&args(&words), home), "{words:?}");
        }
    }

    #[test]
    fn a_sed_w_command_writes() {
        assert!(sed_script_writes("w /x/config.db"));
        assert!(sed_script_writes("1w f"));
        assert!(sed_script_writes("s/a/b/w f"));
        assert!(!sed_script_writes("s/new/old/"));
        assert!(!sed_script_writes("1,5p"));
    }
}
