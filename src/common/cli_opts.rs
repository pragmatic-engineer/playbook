// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! A tolerant scanner for the options of one `git` or `gh` subcommand, shared
//! by hooks that need an option's value without running the program.
//!
//! It understands the spellings these CLIs accept: `--name value`,
//! `--name=value`, `-x value`, `-xvalue`, clusters such as `-anm text`, and
//! unambiguous abbreviations of long options. Options outside the [`Spec`]
//! are skipped, and positional words are ignored.

/// The options of one subcommand the caller cares about.
pub struct Spec {
    /// Short options whose value is the rest of the cluster or the next word.
    pub short_values: &'static str,
    /// Short options whose value, if any, is only the rest of the cluster.
    pub short_attached: &'static str,
    pub long_values: &'static [&'static str],
    pub long_flags: &'static [&'static str],
}

/// One option found on the command line. `name` is the long name without
/// dashes, or the single short letter.
#[derive(Debug, PartialEq, Eq)]
pub struct Opt {
    pub name: String,
    pub value: Option<String>,
}

/// An abbreviation shorter than this is never expanded.
const MIN_ABBREVIATION: usize = 3;

impl Spec {
    pub fn scan(&self, args: &[String]) -> Vec<Opt> {
        let mut opts = Vec::new();
        let mut i = 0;
        while i < args.len() {
            let word = args[i].as_str();
            i += 1;
            if word == "--" {
                break;
            }
            if let Some(long) = word.strip_prefix("--") {
                let (name, inline) = match long.split_once('=') {
                    Some((name, value)) => (name, Some(value.to_string())),
                    None => (long, None),
                };
                let known = self.resolve(name);
                let takes_value = known.is_some_and(|k| self.long_values.contains(&k));
                let value = if takes_value {
                    inline.or_else(|| next_word(args, &mut i))
                } else {
                    None
                };
                opts.push(Opt {
                    name: known.unwrap_or(name).to_string(),
                    value,
                });
            } else if word.len() > 1 && word.starts_with('-') {
                self.scan_cluster(&word[1..], args, &mut i, &mut opts);
            }
        }
        opts
    }

    fn scan_cluster(&self, cluster: &str, args: &[String], next: &mut usize, opts: &mut Vec<Opt>) {
        for (at, letter) in cluster.char_indices() {
            let rest = &cluster[at + letter.len_utf8()..];
            let attached = (!rest.is_empty()).then(|| rest.to_string());
            if self.short_values.contains(letter) {
                let value = attached.or_else(|| next_word(args, next));
                opts.push(Opt::short(letter, value));
                return;
            }
            if self.short_attached.contains(letter) {
                opts.push(Opt::short(letter, attached));
                return;
            }
            opts.push(Opt::short(letter, None));
        }
    }

    /// The known long option `word` names, exactly or as a unique abbreviation.
    fn resolve(&self, word: &str) -> Option<&'static str> {
        let known = || self.long_values.iter().chain(self.long_flags).copied();
        if let Some(exact) = known().find(|k| *k == word) {
            return Some(exact);
        }
        if word.len() < MIN_ABBREVIATION {
            return None;
        }
        let mut matches = known().filter(|k| k.starts_with(word));
        match (matches.next(), matches.next()) {
            (Some(only), None) => Some(only),
            _ => None,
        }
    }
}

impl Opt {
    fn short(letter: char, value: Option<String>) -> Self {
        Opt {
            name: letter.to_string(),
            value,
        }
    }
}

fn next_word(args: &[String], next: &mut usize) -> Option<String> {
    let word = args.get(*next)?.clone();
    *next += 1;
    Some(word)
}

/// The values of every option called one of `names`, in command-line order.
pub fn values<'a>(opts: &'a [Opt], names: &[&str]) -> Vec<&'a str> {
    opts.iter()
        .filter(|opt| names.contains(&opt.name.as_str()))
        .filter_map(|opt| opt.value.as_deref())
        .collect()
}

/// Whether any option is called one of `names`.
pub fn has(opts: &[Opt], names: &[&str]) -> bool {
    opts.iter().any(|opt| names.contains(&opt.name.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: Spec = Spec {
        short_values: "mFC",
        short_attached: "S",
        long_values: &["message", "file", "fixup"],
        long_flags: &["no-verify", "no-edit", "amend"],
    };

    fn words(line: &str) -> Vec<String> {
        line.split(' ').map(str::to_string).collect()
    }

    fn scan(line: &str) -> Vec<(String, Option<String>)> {
        SPEC.scan(&words(line))
            .into_iter()
            .map(|opt| (opt.name, opt.value))
            .collect()
    }

    fn opt(name: &str, value: Option<&str>) -> (String, Option<String>) {
        (name.to_string(), value.map(str::to_string))
    }

    #[test]
    fn every_spelling_of_a_value_option_yields_its_value() {
        let table = [
            ("-m text", opt("m", Some("text"))),
            ("-mtext", opt("m", Some("text"))),
            ("--message text", opt("message", Some("text"))),
            ("--message=text", opt("message", Some("text"))),
            ("--mess text", opt("message", Some("text"))),
            ("--mess=text", opt("message", Some("text"))),
            ("-F -", opt("F", Some("-"))),
            ("-C HEAD", opt("C", Some("HEAD"))),
        ];
        for (line, expected) in table {
            assert_eq!(scan(line), vec![expected], "{line}");
        }
    }

    #[test]
    fn a_cluster_yields_each_letter_and_the_trailing_value() {
        assert_eq!(
            scan("-anm text"),
            vec![opt("a", None), opt("n", None), opt("m", Some("text"))]
        );
        assert_eq!(
            scan("-aFmsg.txt"),
            vec![opt("a", None), opt("F", Some("msg.txt"))]
        );
    }

    #[test]
    fn an_attached_only_option_never_takes_the_next_word() {
        assert_eq!(scan("-S next"), vec![opt("S", None)]);
        assert_eq!(scan("-SKEY next"), vec![opt("S", Some("KEY"))]);
    }

    #[test]
    fn a_long_flag_resolves_from_a_unique_abbreviation_only() {
        assert_eq!(scan("--no-v"), vec![opt("no-verify", None)]);
        assert_eq!(scan("--ame"), vec![opt("amend", None)]);
        assert_eq!(scan("--no"), vec![opt("no", None)], "ambiguous stays raw");
        assert_eq!(scan("--a"), vec![opt("a", None)], "too short stays raw");
    }

    #[test]
    fn positional_words_and_everything_after_double_dash_are_ignored() {
        assert_eq!(scan("pos -m text other"), vec![opt("m", Some("text"))]);
        assert_eq!(scan("-- -m text"), Vec::new());
        assert_eq!(scan("-"), Vec::new());
    }

    #[test]
    fn a_trailing_value_option_has_no_value() {
        assert_eq!(scan("-m"), vec![opt("m", None)]);
        assert_eq!(scan("--message"), vec![opt("message", None)]);
    }

    #[test]
    fn values_and_has_filter_by_name() {
        let opts = SPEC.scan(&words("-m a --message b -n"));

        assert_eq!(values(&opts, &["m", "message"]), vec!["a", "b"]);
        assert!(has(&opts, &["n"]));
        assert!(!has(&opts, &["no-verify"]));
    }
}
