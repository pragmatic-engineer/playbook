// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! A tolerant scanner for the options of one `git` subcommand, for hooks that
//! need an option's value, and where it sits, without running the program.
//!
//! It understands the spellings git accepts: `--name value`, `--name=value`,
//! `-x value`, `-xvalue`, clusters such as `-anm text`, and any unambiguous
//! abbreviation of a long option, such as `--am` for `--amend`. Options
//! outside the [`Spec`] are skipped, and positional words are ignored.
//!
//! An abbreviation is resolved among the spec's options and the subcommand's
//! other long names, as git does: an exact name wins, and a prefix shared by
//! two options is left unresolved, since git refuses to run on it.

/// The options of one subcommand the caller cares about.
pub struct Spec {
    /// Short options whose value is the rest of the cluster or the next word.
    pub short_values: &'static str,
    /// Short options whose value, if any, is only the rest of the cluster.
    pub short_attached: &'static str,
    pub long_values: &'static [&'static str],
    pub long_flags: &'static [&'static str],
    /// The subcommand's other long names, which no caller reads but which an
    /// exact spelling or a shared prefix must not be mistaken for.
    pub other_long: &'static [&'static str],
    /// Whether an unambiguous prefix of a long option names it, as in git. A
    /// program built on cobra, such as `gh`, accepts the exact name only.
    pub abbreviate: bool,
}

/// One option found on the command line. `name` is the long name without
/// dashes, or the single short letter.
#[derive(Debug, PartialEq, Eq)]
pub struct Opt {
    pub name: String,
    pub value: Option<String>,
    /// Index of the word the option starts in.
    pub word: usize,
    /// Index of the word that holds the value, the option's own word for an
    /// inline value.
    pub value_word: usize,
    /// Characters of the word before an inline value, `None` when the whole
    /// of `value_word` is the value.
    pub inline_at: Option<usize>,
}

impl Spec {
    pub fn scan(&self, args: &[String]) -> Vec<Opt> {
        let mut opts = Vec::new();
        let mut i = 0;
        while i < args.len() {
            let word = args[i].as_str();
            let at = i;
            i += 1;
            if word == "--" {
                break;
            }
            if let Some(long) = word.strip_prefix("--") {
                self.scan_long(long, at, args, &mut i, &mut opts);
            } else if word.len() > 1 && word.starts_with('-') {
                self.scan_cluster(&word[1..], at, args, &mut i, &mut opts);
            }
        }
        opts
    }

    fn scan_long(
        &self,
        long: &str,
        at: usize,
        args: &[String],
        next: &mut usize,
        out: &mut Vec<Opt>,
    ) {
        let (name, inline) = match long.split_once('=') {
            Some((name, value)) => (name, Some(value.to_string())),
            None => (long, None),
        };
        let known = self.resolve(name);
        let takes_value = known.is_some_and(|k| self.long_values.contains(&k));
        let mut opt = Opt {
            name: known.unwrap_or(name).to_string(),
            value: None,
            word: at,
            value_word: at,
            inline_at: None,
        };
        if takes_value {
            match inline {
                Some(value) => {
                    opt.inline_at = Some(2 + name.chars().count() + 1);
                    opt.value = Some(value);
                }
                None => take_next(&mut opt, args, next),
            }
        }
        out.push(opt);
    }

    fn scan_cluster(
        &self,
        cluster: &str,
        at: usize,
        args: &[String],
        next: &mut usize,
        out: &mut Vec<Opt>,
    ) {
        for (index, letter) in cluster.chars().enumerate() {
            let rest: String = cluster.chars().skip(index + 1).collect();
            let attached = (!rest.is_empty()).then_some(rest);
            let inline_at = Some(1 + index + 1);
            let mut opt = Opt {
                name: letter.to_string(),
                value: None,
                word: at,
                value_word: at,
                inline_at: None,
            };
            if self.short_values.contains(letter) || self.short_attached.contains(letter) {
                match attached {
                    Some(value) => {
                        opt.value = Some(value);
                        opt.inline_at = inline_at;
                    }
                    None if self.short_values.contains(letter) => take_next(&mut opt, args, next),
                    None => {}
                }
                out.push(opt);
                return;
            }
            out.push(opt);
        }
    }

    /// The known long option `word` names, exactly or as a unique abbreviation.
    fn resolve(&self, word: &str) -> Option<&'static str> {
        let known = || self.long_values.iter().chain(self.long_flags).copied();
        if let Some(exact) = known().find(|k| *k == word) {
            return Some(exact);
        }
        if word.is_empty() || !self.abbreviate || self.other_long.contains(&word) {
            return None;
        }
        let mut matches = known()
            .chain(self.other_long.iter().copied())
            .filter(|k| k.starts_with(word));
        match (matches.next(), matches.next()) {
            (Some(only), None) => known().find(|k| *k == only),
            _ => None,
        }
    }
}

/// Reads the word after an option as its value, when there is one.
fn take_next(opt: &mut Opt, args: &[String], next: &mut usize) {
    if let Some(value) = args.get(*next) {
        opt.value = Some(value.clone());
        opt.value_word = *next;
        *next += 1;
    }
}

/// The options called one of `names`, in command-line order.
pub fn named<'a>(opts: &'a [Opt], names: &[&str]) -> Vec<&'a Opt> {
    opts.iter()
        .filter(|opt| names.contains(&opt.name.as_str()))
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
        other_long: &[],
        abbreviate: true,
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
            ("--m text", opt("message", Some("text"))),
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
    fn any_unambiguous_prefix_of_a_long_option_resolves() {
        assert_eq!(scan("--no-v"), vec![opt("no-verify", None)]);
        assert_eq!(scan("--am"), vec![opt("amend", None)]);
        assert_eq!(scan("--a"), vec![opt("amend", None)]);
        assert_eq!(scan("--no"), vec![opt("no", None)], "ambiguous stays raw");
        assert_eq!(scan("--fi x"), vec![opt("fi", None)], "ambiguous stays raw");
        assert_eq!(scan("--fil x"), vec![opt("file", Some("x"))]);
    }

    #[test]
    fn a_spec_without_abbreviations_takes_exact_long_names_only() {
        let exact = Spec {
            abbreviate: false,
            ..SPEC
        };

        let opts = exact.scan(&words("--mess text --message text"));

        let names: Vec<_> = opts.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, ["mess", "message"]);
    }

    #[test]
    fn positional_words_and_everything_after_double_dash_are_ignored() {
        assert_eq!(scan("pos -m text other"), vec![opt("m", Some("text"))]);
        assert_eq!(scan("-- -m text"), Vec::new());
        assert_eq!(scan("-"), Vec::new());
    }

    #[test]
    fn a_trailing_value_option_has_no_value_and_keeps_its_own_word() {
        assert_eq!(scan("-m"), vec![opt("m", None)]);
        assert_eq!(scan("--message"), vec![opt("message", None)]);

        let args = words("-a -m");
        let opts = SPEC.scan(&args);
        assert_eq!(opts[1].word, 1);
        assert_eq!(opts[1].value_word, 1);
        assert_eq!(SPEC.scan(&words("--message"))[0].value_word, 0);
    }

    #[test]
    fn an_exact_option_outside_the_spec_is_not_read_as_an_abbreviation() {
        let spec = Spec {
            other_long: &["no", "am", "fixup-all"],
            ..SPEC
        };

        let names: Vec<_> = spec
            .scan(&words("--no --am --fixup-all --no-v --amen --fixup"))
            .into_iter()
            .map(|o| o.name)
            .collect();

        assert_eq!(
            names,
            ["no", "am", "fixup-all", "no-verify", "amend", "fixup"]
        );
    }

    #[test]
    fn an_abbreviation_that_other_options_share_stays_unresolved() {
        let spec = Spec {
            other_long: &["amend-all"],
            ..SPEC
        };

        let names: Vec<_> = spec
            .scan(&words("--a --ame --amend"))
            .into_iter()
            .map(|o| o.name)
            .collect();

        assert_eq!(names, ["a", "ame", "amend"]);
    }

    #[test]
    fn each_option_reports_where_its_value_sits() {
        let args = words("--message=a -m b -mc -am d");

        let opts = SPEC.scan(&args);

        let at: Vec<(usize, usize, Option<usize>)> = opts
            .iter()
            .map(|o| (o.word, o.value_word, o.inline_at))
            .collect();
        assert_eq!(
            at,
            vec![
                (0, 0, Some(10)),
                (1, 2, None),
                (3, 3, Some(2)),
                (4, 4, None),
                (4, 5, None),
            ]
        );
    }

    #[test]
    fn named_and_has_filter_by_name() {
        let opts = SPEC.scan(&words("-m a --message b -n"));

        let values: Vec<_> = named(&opts, &["m", "message"])
            .into_iter()
            .filter_map(|o| o.value.as_deref())
            .collect();
        assert_eq!(values, vec!["a", "b"]);
        assert!(has(&opts, &["n"]));
        assert!(!has(&opts, &["no-verify"]));
    }
}
