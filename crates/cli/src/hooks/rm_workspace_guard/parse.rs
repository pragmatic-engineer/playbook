// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

/// The operator between `before` and `after`, such as `&&`, `||`, `;` or
/// `|`, with whitespace and parentheses left out, or `;` for the first
/// command. A line break alone reads as empty, which is not `&&`.
fn separator(source: &[char], before: &shell::Command, after: &shell::Command) -> String {
    let (Some(end), Some(start)) = (before.spans.last(), after.spans.first()) else {
        return ";".to_string();
    };
    source
        .get(end.end..start.start)
        .unwrap_or_default()
        .iter()
        .filter(|c| !c.is_whitespace() && !matches!(c, '(' | ')'))
        .collect()
}

/// `find` tests that a start path given by name rarely passes, so a `find`
/// that uses one is taken to delete only below its start paths.
const FIND_TESTS: &[&str] = &[
    "-name",
    "-iname",
    "-path",
    "-ipath",
    "-wholename",
    "-iwholename",
    "-regex",
    "-iregex",
    "-type",
    "-newer",
    "-mtime",
    "-mmin",
    "-size",
    "-empty",
    "-user",
    "-perm",
    "-mindepth",
];

/// The parts of a `find` command line, by index into its arguments.
struct FindParts {
    starts: Vec<usize>,
    /// Where the expression starts.
    expression: usize,
    follows_links: bool,
    /// The expression tests names, types or depth, so it never takes a
    /// start path itself.
    filtered: bool,
}

impl FindParts {
    /// Reads past the leading options of GNU and BSD `find` (`-H`, `-L`, `-P`,
    /// `-D x`, `-O3`, `-E`, `-X`, `-d`, `-s`, `-x`, `-f path`) to the start
    /// paths.
    fn read(args: &[String]) -> Self {
        let mut starts = Vec::new();
        let mut follows_links = false;
        let mut i = 0;
        while let Some(word) = args.get(i) {
            match word.as_str() {
                "-H" | "-P" | "-E" | "-X" | "-d" | "-s" | "-x" => i += 1,
                "-L" => {
                    follows_links = true;
                    i += 1;
                }
                "-D" => i += 2,
                "-f" => {
                    if i + 1 < args.len() {
                        starts.push(i + 1);
                    }
                    i += 2;
                }
                w if w.starts_with("-O") => i += 1,
                _ => break,
            }
        }
        let mut end = i.min(args.len());
        while args
            .get(end)
            .is_some_and(|w| !w.starts_with('-') && !matches!(w.as_str(), "(" | ")" | "!" | ","))
        {
            starts.push(end);
            end += 1;
        }
        let expression = &args[end..];
        FindParts {
            starts,
            expression: end,
            follows_links: follows_links || expression.iter().any(|w| w == "-follow"),
            filtered: expression.iter().any(|w| FIND_TESTS.contains(&w.as_str())),
        }
    }
}

/// The program of a command, past wrappers (`program_index`), runners such
/// as `nohup` with their options, and `builtin`. `xargs` is a program here,
/// since its input matters.
fn program_at(words: &[String]) -> Option<usize> {
    let mut from = 0;
    loop {
        let at = from + program_index(words.get(from..)?)?;
        let name = program_name(&words[at]);
        let next = match runner_value_options(&name) {
            Some(takes_value) if name != "xargs" => skip_options(words, at + 1, takes_value),
            _ if name == "builtin" => at + 1,
            _ => return Some(at),
        };
        if next >= words.len() {
            return None;
        }
        from = next;
    }
}

/// Whether wrapper words change the directory the program runs in, as
/// `env -C dir` and `sudo -D dir` do.
fn changes_directory(wrappers: &[String]) -> bool {
    wrappers
        .iter()
        .any(|w| matches!(program_name(w).as_str(), "env" | "sudo"))
        && wrappers
            .iter()
            .any(|w| matches!(w.as_str(), "-C" | "-D" | "--chdir") || w.starts_with("--chdir="))
}

/// Whether `word`, with its quotes removed, names `HOME` other than to read
/// it, as in `HOME=/`, `export HOME`, `unset HOME` or `${HOME:=x}`.
fn names_expanded(word: &str) -> bool {
    word.contains("{HOME:=")
        || word.contains("{HOME=")
        || word
            .strip_prefix("HOME")
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('=') || rest.starts_with("+="))
}

/// The source text of one word, quotes included.
fn raw_text(source: &[char], span: &Span) -> String {
    source
        .get(span.start..span.end.min(source.len()))
        .map(|s| s.iter().collect())
        .unwrap_or_default()
}

/// Whether `producer` is the whole command before the `|` that feeds
/// `consumer`, so its output is all `consumer` reads.
fn same_pipeline(producer: &shell::Command, consumer: &shell::Command) -> bool {
    producer.depth == consumer.depth
        && producer.group == consumer.group
        && !producer.substitution
        && !producer.joined
        && !producer.opens_substitution
}

/// A `find` that only prints paths, so its output is its start paths and
/// what lies under them.
fn lists_paths(words: &[String], at: usize) -> bool {
    program_name(&words[at]) == "find"
        && !words[at + 1..].iter().any(|w| {
            matches!(
                w.as_str(),
                "-exec" | "-execdir" | "-ok" | "-okdir" | "-delete" | "-printf" | "-fprint"
            )
        })
}

/// A shell that reads its script from standard input: no `-c`, and no script
/// file unless `-s` says to read standard input anyway.
fn reads_script_from_stdin(words: &[String]) -> bool {
    let Some(at) = program_at(words) else {
        return false;
    };
    if !SHELLS.contains(&program_name(&words[at]).as_str()) {
        return false;
    }
    let args = &words[at + 1..];
    if shell_script_at(args).is_some() || args.iter().any(|w| w == "<<<") {
        return false;
    }
    let mut operands = false;
    let mut i = 0;
    while let Some(word) = args.get(i) {
        let redirect = redirect_len(word);
        if redirect > 0 {
            i += redirect;
            continue;
        }
        operands |= !word.starts_with('-');
        i += 1;
    }
    !operands || args.iter().any(|w| w == "-s")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Scans `case` with `RM` replaced at run time, so this file never holds a
    /// literal deletion that the live guard would judge.
    fn scan(case: &str) -> (String, Vec<Target>) {
        let command = case.replace("RM", &("r".to_string() + "m"));
        let got = targets(
            &command,
            &Env {
                home: "/Users/me",
                cwd: Path::new("/w"),
                cdpath: false,
                tmpdir: None,
                cargo_target_dir: None,
            },
        );
        (command, got)
    }

    /// `p:/a` a path, `t:~/a` a path with an unquoted `~`, `b:/a` a path
    /// the deletion stays under, `f:x` a forbidden option, `w:/a` an overwrite
    /// (`W:` with an unquoted `~`), `u:x` unresolvable with label `x`; `,`
    /// between targets.
    fn spec(want: &str) -> Vec<Target> {
        let one = |item: &str| {
            let (kind, text) = item.split_at(2);
            match kind {
                "p:" | "t:" => Target::Path {
                    text: text.to_string(),
                    tilde: kind == "t:",
                },
                "b:" => Target::Below {
                    text: text.to_string(),
                    tilde: false,
                },
                "f:" => Target::Forbidden(text.to_string()),
                "w:" | "W:" => Target::Overwrite {
                    text: text.to_string(),
                    tilde: kind == "W:",
                },
                _ => Target::Unresolvable(text.to_string()),
            }
        };
        want.split(',').filter(|i| !i.is_empty()).map(one).collect()
    }

    const S: &str = "u:<command substitution>";

    #[rustfmt::skip]
    const FOUND: &[(&str, &str)] = &[
        // Plain forms.
        ("RM -rf /a/b", "p:/a/b"),
        ("RM a b", "p:a,p:b"),
        ("RM -f -- -weird", "p:-weird"),
        ("RM -f /a/x 2>/dev/null", "p:/a/x"),
        ("RM -f /a/x > /dev/null", "p:/a/x"),
        ("RM -rf \"/a/with space\"", "p:/a/with space"),
        ("RM -rf ~/x", "t:~/x"),
        ("RM -rf \"~/x\"", "p:~/x"),
        ("RM -rf /a/b/", "p:/a/b/"),
        // After separators and in groups.
        ("ls && RM /a", "p:/a"),
        ("ls; RM /a", "p:/a"),
        ("ls | RM /a", "p:/a"),
        ("ls || RM /a", "p:/a"),
        ("ls &\nRM /a", "p:/a"),
        ("(ls; RM /a)", "p:/a"),
        ("{ RM /a; }", "p:/a"),
        ("if true; then RM /a; fi", "p:/a"),
        // Wrappers and spellings.
        ("sudo RM -rf /a", "p:/a"),
        ("sudo -u root RM /a", "p:/a"),
        ("command RM /a", "p:/a"),
        ("\\RM /a", "p:/a"),
        ("/bin/RM /a", "p:/a"),
        ("env X=1 RM /a", "p:/a"),
        ("X=1 RM /a", "p:/a"),
        ("nohup RM /a", "p:/a"),
        ("timeout 5 RM /a", "p:/a"),
        ("r'm' /a", "p:/a"),
        // Nested shells.
        ("sh -c 'RM /a'", "p:/a"),
        ("bash -lc \"ls && RM /a\"", "p:/a"),
        ("eval 'RM /a'", "p:/a"),
        ("bash <<EOF\nRM /a\nEOF", "p:/a"),
        ("cat <<EOF | sh\nRM /a\nEOF", "p:/a"),
        // find and xargs.
        ("find /a -name x -delete", "b:/a"),
        ("find -delete", "p:."),
        ("find /a -exec RM -f {} \\;", "p:/a"),
        ("find /a /b -exec RM {} +", "p:/a,p:/b"),
        ("find /a -print0 | xargs -0 RM -f", "p:/a"),
        ("ls | xargs RM", "u:<xargs input>"),
        ("xargs -n 1 RM /a < list", "p:/a,u:<xargs input>"),
        ("find -L /a -delete", "u:find -L"),
        // Substitutions.
        ("RM -rf $(cat list)", S),
        ("RM -rf `cat list`", S),
        ("X=$(pwd) RM /a", "u:<command substitution>,p:/a"),
        ("echo $(date; RM /a)", "u:<command substitution>,p:/a"),
        ("echo \"$(RM /a)\"", "u:<command substitution>,p:/a"),
        ("echo `RM /a`", "u:<command substitution>,p:/a"),
        ("echo $(date) && RM /a", "p:/a"),
        ("RM /a && echo $(date)", "p:/a"),
        // Variables, zsh qualifiers and cd.
        ("RM -rf $HOME/x", "p:/Users/me/x"),
        ("RM -rf \"${HOME}/x\"", "p:/Users/me/x"),
        ("RM -rf $TMPDIR/x", "u:$TMPDIR/x"),
        ("RM -rf ~\"/x\"", "p:~/x"),
        (": ${HOME:=/}; RM -rf $HOME/x", "u:$HOME/x"),
        ("RM -rf '$HOME/x'", "u:'$HOME/x'"),
        ("RM -rf $REPO/x", "u:$REPO/x"),
        ("RM -rf $HOME$X", "u:$HOME$X"),
        ("RM -rf /a/x(:h)", "u:/a/x("),
        ("HOME=/ RM -rf $HOME/x", "u:$HOME/x"),
        ("export HOME=/; RM -rf $HOME/x", "u:$HOME/x"),
        ("cd /x && RM -rf build", "p:/x/build"),
        ("cd /x && RM -rf /x/build ~/y", "p:/x/build,t:~/y"),
        ("builtin cd /x && RM b", "p:/x/b"),
        ("env -C /x RM b", "u:b"),
        ("sudo -D /x RM b", "u:b"),
        ("export HO''ME=/; RM -rf $HOME/x", "u:$HOME/x"),
        // Runners with option values.
        ("ionice -c 3 RM /a", "p:/a"),
        ("doas -u root RM /a", "p:/a"),
        ("busybox RM /a", "p:/a"),
        // Shells reading a script from somewhere else.
        ("bash <<< 'RM /a'", "p:/a"),
        ("bash -s -- x <<EOF\nRM /a\nEOF", "p:/a"),
        ("echo 'RM /a' | sh", "u:<shell input>"),
        ("eval \"$cmd\"", "u:$cmd"),
        ("bash -c \"$cmd\"", "u:$cmd"),
        ("eval eval eval eval eval eval RM /a", "u:<nested shell>"),
        ("cat <<EOF\n$(RM /a)\nEOF", "u:<command substitution>,p:/a"),
        ("echo \"$(printf ')'; RM /a)\"", "u:<command substitution>,p:/a"),
        // More find and xargs forms.
        ("find /a -execdir RM {} +", "p:/a"),
        ("find /a -ok RM {} \\;", "p:/a"),
        ("find /a -exec sh -c 'RM -rf {}' \\;", "p:{},p:/a"),
        ("find /a -exec nohup RM -rf {} +", "p:/a"),
        ("find -D tree /a -delete", "p:/a"),
        ("find -O3 -P /a -delete", "p:/a"),
        ("find -x / -name x -delete", "b:/"),
        ("find -f /a -delete", "p:/a"),
        ("cd /x && find -delete", "p:/x/."),
        ("find $(pwd) -name x -delete", S),
        ("find /a -exec ls {} + | xargs RM", "u:<xargs input>"),
        ("(echo /b; find /a) | xargs RM", "u:<xargs input>"),
        ("ls | xargs nohup RM -rf", "u:<xargs input>"),
        ("ls | xargs -I{} sh -c 'RM -rf {}'", "p:{},u:<xargs input>"),
        // The folder the shell is in.
        ("cd /x; RM b", "p:/x/b,p:/w/b"),
        ("cd /x\nRM b", "p:/x/b,p:/w/b"),
        ("cd /x && RM a; RM b", "p:/x/a,p:/x/b,p:/w/b"),
        ("cd /x && cd y && RM b", "p:/x/y/b"),
        ("cd /x/y/../z && RM b", "p:/x/z/b"),
        ("cd sub && RM ../b", "p:/w/sub/../b"),
        ("cd && RM b", "p:/Users/me/b"),
        ("cd ~ && RM b", "p:/Users/me/b"),
        ("cd ~/src && RM b", "p:/Users/me/src/b"),
        ("cd $HOME/src && RM b", "p:/Users/me/src/b"),
        ("cd -P /x && RM b", "p:/x/b"),
        ("cd \"$X\" && RM b", "u:b"),
        ("cd ~other && RM b", "u:b"),
        ("cd /x && cd - && RM b", "p:b"),
        ("(cd /x && RM a); RM b", "p:/x/a,p:b"),
        ("(cd /x); (RM b)", "p:b"),
        ("X=$(cd /x; pwd) RM b", "u:<command substitution>,p:b"),
        ("pushd /x && RM a && popd && RM b", "p:/x/a,p:b"),
        ("popd && RM b", "u:b"),
        ("eval 'cd /x'; RM b", "p:/w/b,u:b"),
        ("sh -c 'cd /x && RM a' && RM b", "p:/x/a,p:/w/b,u:b"),
        ("cd /x && RM /abs ~/y", "p:/abs,t:~/y"),
        ("cd /x && find . -delete", "p:/x/."),
        ("cd /x &&\n  RM b", "p:/x/b"),
        ("true || cd /x && RM b", "p:/x/b,p:/w/b"),
        ("cd /x || exit 1; RM b", "p:/x/b,p:/w/b"),
        ("cd -P /x/l/.. && RM b", "p:/x/l/../b"),
        ("cd /x | true; RM b", "p:/x/b,p:/w/b"),
        // Variables the command sets.
        ("S=/x; RM -rf $S/y", "p:/x/y"),
        ("S=\"/x\" ; RM -rf \"$S\"", "p:/x"),
        ("S=/x; RM -rf \"${S}/y\"", "p:/x/y"),
        ("export O=/x/o; RM -f $O/r", "p:/x/o/r"),
        ("S=/x; N=$S/n; RM -rf $N", "p:/x/n"),
        ("S=/x; S+=/y; RM $S", "p:/x/y"),
        ("S=~/x; RM $S", "p:/Users/me/x"),
        ("S='~/x'; RM $S", "p:~/x"),
        ("S=rel; RM $S", "p:rel"),
        ("cd /x && S=rel; RM $S", "u:$S"),
        ("cd /x; S=rel; RM $S", "p:/x/rel,p:/w/rel"),
        ("d=/x; for f in a b; do RM \"$d/$f\"; done", "p:/x/a,p:/x/b"),
        ("for f in $(ls); do RM $f; done", "u:$f"),
        ("for f; do RM $f; done", "u:$f"),
        ("S=/x; S=$(cat w); RM $S/y", "u:$S/y"),
        ("S=/x; unset S; RM $S/y", "u:$S/y"),
        ("S=/x; read S; RM $S/y", "u:$S/y"),
        ("S=/x; printf -v S y; RM $S/y", "u:$S/y"),
        ("S=/x; source env.sh; RM $S/y", "u:$S/y"),
        ("S=/x; . ./env.sh; RM $HOME/y", "u:$HOME/y"),
        ("true && S=/x; RM $S/y", "u:$S/y"),
        ("S=/a; true && S=/b; RM $S", "p:/a,p:/b"),
        ("if c; then S=/x; fi; RM $S", "u:$S"),
        ("S=/x; if c; then S=/y; fi; RM $S", "p:/x,p:/y"),
        ("S=/x | cat; RM $S", "u:$S"),
        ("S=/x & RM $S", "u:$S"),
        ("f() { S=/x; }; RM $S", "u:$S"),
        ("S=/x; f() { :; }; RM $S", "u:$S"),
        ("sh -c 'S=/x'; RM $S", "u:$S"),
        ("eval 'S=/x'; RM $S", "u:$S"),
        ("X=1 RM $X", "u:$X"),
        ("S='/a /b'; RM $S", "u:$S"),
        ("declare -a S=(/x); RM $S", "u:$S"),
        ("declare -x S=/x; RM $S", "p:/x"),
        ("S=/x; RM '$S'", "u:'$S'"),
        ("S=/x; RM \\$S", "u:\\$S"),
        ("RM /tmp/st.$$", "p:/tmp/st.12345"),
        ("RM \"$PWD/x\"", "p:/w/x"),
        ("cd /x; RM \"$PWD/y\"", "p:/x/y,p:/w/y"),
        ("cd \"$Z\"; RM \"$PWD/y\"", "u:\"$PWD/y\""),
        ("cd /x && cd /y && RM \"$OLDPWD/z\"", "p:/x/z"),
        ("RM \"$OLDPWD/z\"", "u:\"$OLDPWD/z\""),
        ("RM -rf \"$(pwd)/t\"", "p:/w/t"),
        ("RM -rf \"$(cat x)/t\"", S),
        ("d=$(mktemp -d); RM -r $d", "p:/tmp/tmp.mktemp"),
        ("d=\"$(mktemp)\"; RM -f \"$d\"", "p:/tmp/tmp.mktemp"),
        ("d=$(mktemp -d ./x.XXXX); RM -rf $d", "p:./x.mktemp"),
        ("d=$(mktemp -p /x); RM -rf $d", "u:$d"),
        ("d=$(mktemp -d)x; RM -rf $d", "u:$d"),
        ("RM \"${TMPDIR:-/tmp}/p.json\"", "p:/tmp/p.json"),
        ("RM ${X:-/tmp}/p", "u:${X:-/tmp}/p"),
        ("RM $1/x", "u:$1/x"),
        // Forbidden options and find start paths.
        ("RM -rf --no-preserve-root /tmp/x", "f:--no-preserve-root,p:/tmp/x"),
        ("find /a -delete", "p:/a"),
        ("find . -type f -delete", "b:."),
        ("find /a -mindepth 1 -exec RM -rf {} +", "b:/a"),
        // Other commands that delete or overwrite.
        ("unlink /a", "p:/a"),
        ("rmdir -p a/b", "p:a/b,p:a"),
        ("shred -u -n 3 ~/k", "t:~/k"),
        ("truncate -s 0 /a", "p:/a"),
        ("dd if=/dev/zero of=/a bs=1", "p:/a"),
        ("trash dist", "p:dist"),
        ("chmod -R 777 ~/.ssh", "W:~/.ssh"),
        ("chown -R me:staff /a /b", "w:/a,w:/b"),
        ("chmod 600 ~/.ssh/id", ""),
        ("echo x > /a", "w:/a"),
        ("echo x >/a", "w:/a"),
        ("echo x 2>/a", "w:/a"),
        ("echo x &> /a", "w:/a"),
        ("echo x > ~/.zshrc", "W:~/.zshrc"),
        ("echo x >> /a", ""),
        ("echo x > /dev/null 2>&1", ""),
        ("echo x > \"$UNKNOWN\"", ""),
        ("S=/a; echo x > $S", "w:/a"),
        ("git clean -fd sub", "b:sub"),
        ("git clean -fdx", "b:."),
        ("git clean -f -e keep -- a", "b:a"),
        ("git clean -n", ""),
        ("git clean -fdn", ""),
        ("git clean -d", ""),
        ("git -C /x clean -f", "u:."),
        ("git -C /x clean -f /x/y", "b:/x/y"),
        ("git worktree remove --force /wt", "b:/wt"),
        ("git worktree list", ""),
        ("git rm -r sub", ""),
        ("cargo clean", "p:target"),
        ("cargo +nightly clean --target-dir /t", "p:/t"),
        ("cargo build", ""),
        ("python3 -c 'import shutil; shutil.rmtree(\"/a\")'", "p:/a"),
        ("python3 - <<'EOF'\nimport os\nos.remove('x')\nEOF", "p:x"),
        ("python3 <<'EOF'\nimport os\nos.remove(p)\nEOF", "u:<python expression>"),
        ("python3 script.py <<'EOF'\nos.remove('x')\nEOF", ""),
        ("node -e \"require('fs').rmSync('/a')\"", "p:/a"),
        ("perl -e 'unlink \"x\"'", "p:x"),
        ("ruby -e 'FileUtils.rm_rf(\"/a\")'", "p:/a"),
        ("printf '%s\\n' a b | xargs RM -f", "p:a,p:b"),
        ("echo a b | xargs RM", "p:a,p:b"),
        ("printf '%s\\n' $X | xargs RM", "u:<xargs input>"),
    ];

    /// Commands that only read, search or mention a deletion as data.
    #[rustfmt::skip]
    const DATA: &[&str] = &[
        "grep -E \"a | RM b\" file.txt",
        "grep -rn \"RM -rf\" ~/.config/playbook/memory",
        "rg \"hooks\\.rmGuard|RM -rf\" crates/",
        "echo \"x; RM -rf /a\"",
        "echo RM -rf /a",
        "git commit -m \"fix: guard; RM /a note\"",
        "git commit -m 'docs: RM on /a is fatal'",
        "git commit -F - <<'EOF'\nfix: stop RM -rf /a\nRM -rf /b\nEOF",
        "cat <<EOF\nRM -rf /a\nEOF",
        "cat <<EOF\nfoo RM -rf /a",
        "git commit -m \"$(cat <<'EOF'\nfix: RM -rf /a (twice)\nit's done\nEOF\n)\"",
        "echo hi # RM -rf /a",
        "printf '%s\\n' \"RM /a\" > notes.txt",
        "git RM --cached /a",
        "docker RM box",
        "command -v RM",
        "man RM",
        "sqlite3 ~/.config/playbook/data/config.db .tables",
        "ls ~/.config/playbook/memory 2>/dev/null",
        "cat ~/.config/playbook/config.json",
        "grep -n rmGuard ~/.claude/settings.json",
        "playbook migrate --status",
        "find /a -name '*.md'",
        "echo $((1 << 2))",
        "wc -l $(find . -name '*.rs')",
        "git commit -m \"$(cat <<'EOF'\n1) step\nuse `RM -rf build`\nEOF\n)\"",
        "cat <<'EOF'\n$(RM /a)\nEOF",
        "bash script.sh",
    ];

    #[test]
    fn the_parser_finds_every_real_deletion() {
        let mut wrong = Vec::new();
        for (case, want) in FOUND {
            let (command, got) = scan(case);

            if got != spec(want) {
                wrong.push(format!("{command:?}: got {got:?}"));
            }
        }

        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[test]
    fn data_that_mentions_a_deletion_is_never_a_command() {
        for case in DATA {
            let (command, got) = scan(case);
            // A redirect target is an overwrite, judged on its own.
            let deletions: Vec<Target> = got
                .into_iter()
                .filter(|t| !matches!(t, Target::Overwrite { .. }))
                .collect();

            assert_eq!(deletions, Vec::<Target>::new(), "{command:?}");
        }
    }

    #[test]
    fn tmpdir_expands_to_the_value_this_hook_sees() {
        let env = Env {
            home: "/Users/me",
            cwd: Path::new("/w"),
            cdpath: false,
            tmpdir: Some(Path::new("/var/T")),
            cargo_target_dir: None,
        };
        let rm = "r".to_string() + "m";
        let cases = [
            (format!("{rm} $TMPDIR/x"), "p:/var/T/x"),
            (format!("{rm} \"${{TMPDIR:-/tmp}}/x\""), "p:/var/T/x"),
            (format!("d=$(mktemp -d); {rm} -r $d"), "p:/var/T/tmp.mktemp"),
        ];
        for (command, want) in cases {
            assert_eq!(targets(&command, &env), spec(want), "{command}");
        }
    }

    #[test]
    fn a_relative_cd_with_cdpath_set_lands_in_an_unknown_folder() {
        let env = Env {
            home: "/Users/me",
            cwd: Path::new("/w"),
            cdpath: true,
            tmpdir: None,
            cargo_target_dir: None,
        };
        let rm = "r".to_string() + "m";

        let named = targets(&format!("cd sub && {rm} b"), &env);
        let explicit = targets(&format!("cd ./sub && {rm} b"), &env);

        assert_eq!(named, spec("u:b"));
        assert_eq!(explicit, spec("p:/w/sub/b"));
    }

    #[test]
    fn the_folders_a_command_changes_into_are_reported() {
        let env = Env {
            home: "/Users/me",
            cwd: Path::new("/w"),
            cdpath: false,
            tmpdir: None,
            cargo_target_dir: None,
        };
        let rm = "r".to_string() + "m";

        let found = super::scan(&format!("cd /x && cd y; cd \"$Z\"; {rm} a"), &env);

        assert_eq!(found.dirs, vec![PathBuf::from("/x"), PathBuf::from("/x/y")]);
    }

    #[test]
    fn only_a_write_or_unset_of_home_stops_expansion() {
        assert!(names_expanded("HOME=/"));
        assert!(names_expanded("HOME"));
        assert!(names_expanded("HOME+=x"));
        assert!(names_expanded("${HOME:=/}"));
        assert!(!names_expanded("$HOME"));
        assert!(!names_expanded("${HOME:-/x}"));
        assert!(!names_expanded("HOMEBREW_PREFIX=x"));
    }
}
