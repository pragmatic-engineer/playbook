// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Pins the exact `possible_relates_to` edges the memory graph rebuild
//! produces for a generated corpus, so an optimisation of the pairwise
//! similarity pass cannot change the output.

use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

const FACTS: usize = 240;
const EXPECTED_EDGES: usize = 623;
const EXPECTED_HASH: u64 = 15118094020544539416;

struct Lcg(u64);

impl Lcg {
    fn next(&mut self, bound: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % bound
    }
}

fn fnv(data: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in data.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn write_corpus(mem: &Path) {
    let mut rng = Lcg(42);
    let kinds = ["gotcha", "reference", "decision"];
    let dirs = ["src/a", "src/b", "src/c", "lib/x", "lib/y", "docs"];
    for i in 0..FACTS {
        // Each fact draws from a topic window of a 90-word vocabulary so
        // Jaccard scores spread around the 0.35 threshold.
        let topic = rng.next(6) * 12;
        let len = 8 + rng.next(30);
        let mut words = Vec::new();
        for _ in 0..len {
            let w = if rng.next(10) < 7 {
                topic + rng.next(24)
            } else {
                rng.next(90)
            };
            words.push(format!("Word{w}"));
        }
        let kind = kinds[rng.next(3) as usize];
        let anchors = match rng.next(3) {
            0 => String::new(),
            n => {
                let mut s = String::from("anchors:\n");
                for _ in 0..n {
                    let d = dirs[rng.next(6) as usize];
                    s.push_str(&format!("  - {d}/f{}.rs\n", rng.next(5)));
                }
                s
            }
        };
        let rel = match rng.next(3) {
            0 => format!("fact{i}.md"),
            1 => format!("org{}/fact{i}.md", rng.next(2)),
            _ => format!("org{}/proj{}/fact{i}.md", rng.next(2), rng.next(2)),
        };
        let full = mem.join(&rel);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        let body = format!(
            "---\nname: fact{i}\ntype: {kind}\n{anchors}---\n\n{}\n",
            words.join(" ")
        );
        fs::write(full, body).unwrap();
    }
}

#[test]
fn rebuild_of_a_generated_corpus_pins_the_similarity_edges() {
    // Arrange
    let home = std::env::temp_dir().join(format!("playbook-graphchar-{}", std::process::id()));
    let _ = fs::remove_dir_all(&home);
    let mem = home.join(".config").join("playbook").join("memory");
    fs::create_dir_all(&mem).unwrap();
    write_corpus(&mem);

    // Act
    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .env_remove("CI")
        .env_remove("PLAYBOOK_HEADLESS")
        .args(["memory", "rebuild"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("HOME", &home)
        .stdin(Stdio::null())
        .output()
        .expect("playbook binary should run");

    // Assert
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let graph: Value =
        serde_json::from_str(&fs::read_to_string(mem.join("memory.graph.json")).unwrap()).unwrap();
    let lines: Vec<String> = graph["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["relation"] == "possible_relates_to")
        .map(|e| format!("{}>{}:{}", e["from"], e["to"], e["signals"]))
        .collect();
    let mut sorted = lines.clone();
    sorted.sort();
    let body = lines.iter().filter(|l| l.contains("body_overlap")).count();
    assert_eq!(body, 126);
    assert_eq!(lines.len(), EXPECTED_EDGES);
    assert_eq!(fnv(&sorted.join("\n")), EXPECTED_HASH);
    let _ = fs::remove_dir_all(&home);
}
