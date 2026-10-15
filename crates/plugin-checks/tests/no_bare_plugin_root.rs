// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Claude Code fills in only the braced `${CLAUDE_PLUGIN_ROOT}` in plugin text.
//! A bare `$CLAUDE_PLUGIN_ROOT` reaches the model unresolved, so no plugin
//! markdown may use it.

use std::fs;
use std::path::{Path, PathBuf};

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in fs::read_dir(dir).expect("dir").flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x == "md") {
            out.push(p);
        }
    }
}

#[test]
fn no_bare_claude_plugin_root_in_plugin_markdown() {
    let mut files = Vec::new();
    walk(&PathBuf::from(env!("PLAYBOOK_PLUGIN_ROOT")), &mut files);
    assert!(!files.is_empty(), "no plugin markdown found");
    let mut bad = Vec::new();
    for f in files {
        let text = fs::read_to_string(&f).expect("read");
        for (i, line) in text.lines().enumerate() {
            if line
                .replace("${CLAUDE_PLUGIN_ROOT}", "")
                .contains("$CLAUDE_PLUGIN_ROOT")
            {
                bad.push(format!("{}:{}", f.display(), i + 1));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "bare $CLAUDE_PLUGIN_ROOT (use the braced form or no root): {bad:#?}"
    );
}
