//! The dictionary's guesses the helper hands a fix, for training data made
//! to match them (tools/train/vast/hints.py). Records on stdin are
//! `text\x1fcontext`, each ended by \x1e; the hints for each come back ended
//! by \x1e, in order (empty where the helper would give none).

#[path = "../src/spell.rs"]
#[allow(dead_code)]
mod spell;

use std::io::{Read, Write};

use kolada_assist::prompt;
use kolada_assist::wire::{Kind, Request};

fn main() {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    let mut dictionaries = spell::Dictionaries::new();
    let mut out = std::io::stdout().lock();
    for record in input.split('\x1e').filter(|r| !r.trim().is_empty()) {
        let (text, context) = record.trim_start_matches(['\n', '\r']).split_once('\x1f').unwrap_or((record, ""));
        let job = Request { kind: Kind::Fix, id: 0, note: 0, slot: 0, context: context.to_string(), text: text.to_string() };
        write!(out, "{}\x1e", dictionaries.hints(prompt::fix_language(&job), &job.text)).unwrap();
    }
}
