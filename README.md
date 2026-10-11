# Notsy - Blazing Fast Notes App

The world's fastest notes app. It opens to a visible window in about 25 ms and types in under a tenth of a millisecond, even at 4K. Your notes stay a plain folder of Markdown files, so Obsidian and any other editor can open them too.

**Download:** [notsy.co](https://notsy.co) (Windows, and a Mac beta for Apple silicon; Linux is coming).

## What's here

| Crate | What it does |
| --- | --- |
| `notsy-core` | Vaults, sessions, and imports from other notes apps |
| `notsy-editor` | Text, selection and undo |
| `notsy-markdown` | Markdown parsing and styling |
| `notsy-text` | Text shaping and layout |
| `notsy-ui` | The scene, chrome and theme every platform draws |
| `notsy-convert` | DOCX, PDF and HTML to notes |
| `notsy-assist`, `notsy-ai` | The local writing model: it runs on your machine and never calls the network |
| `notsy-app` | The app model the platform shells share |
| `notsy-win`, `notsy-mac` | The platform shells |

## Building

You need a recent stable Rust. Clone with `--recursive` to get the `llama.cpp` submodule, which only the writing model uses.

- **Windows:** `cargo build --release -p notsy-win`
- **Mac:** `bash tools/mac-bundle.sh --open`
- **Tests:** `cargo test -p notsy-core -p notsy-editor -p notsy-markdown -p notsy-ui`

## Licence

The source is published for reading, with all rights reserved. See [LICENSE](LICENSE). Third-party components keep their own licences: see [THIRD-PARTY.md](THIRD-PARTY.md).
