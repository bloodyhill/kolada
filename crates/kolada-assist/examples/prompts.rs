//! The app's own prompts and checks, for training data made to match them
//! (tools/train/vast/mix.py). Records on stdin end with \x1e; their fields
//! are split by \x1f, the first naming what to do:
//!
//! - `gist`, note: the prompt a gist of the note is asked with;
//! - `shape`, line: the prompt, \x1f, `1` if the app would ask about the line;
//! - `gistok`, note, answer: the gist the app would show, or nothing;
//! - `listok`, line, answer: the list the app would make, or nothing;
//! - `lang`, text: the language the app takes the text to be in (en, ar…), or nothing.
//!
//! Each answer comes back ended by \x1e, in order.

use std::io::{Read, Write};

use kolada_assist::wire::{Kind, Request};
use kolada_assist::{gist, prompt, shape};

fn ask(kind: Kind, text: String) -> String {
    prompt::build(&Request { kind, id: 0, note: 0, slot: 0, context: String::new(), text }).text
}

fn main() {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    let mut out = std::io::stdout().lock();
    for record in input.split('\x1e').filter(|r| !r.trim().is_empty()) {
        let f: Vec<&str> = record.trim_start_matches(['\n', '\r']).split('\x1f').collect();
        let answer = match f[0] {
            "gist" => ask(Kind::Gist, gist::source(f[1])),
            "shape" => format!("{}\x1f{}", ask(Kind::Shape, f[1].to_string()), u8::from(shape::worth_asking(f[1]))),
            "gistok" => gist::accept(f[2], "", f[1]).unwrap_or_default(),
            "listok" => shape::list(f[1], f[2]).unwrap_or_default(),
            "lang" => prompt::language(f[1]).map(|l| format!("{l:?}").to_lowercase()).unwrap_or_default(),
            other => panic!("unknown record kind {other}"),
        };
        write!(out, "{answer}\x1e").unwrap();
    }
}
