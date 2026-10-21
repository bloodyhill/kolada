<p align="center">
  <a href="https://kolada.app"><img src="https://kolada.app/logo.svg" width="96" height="96" alt="Kolada"></a>
</p>

<h1 align="center">Kolada</h1>

<p align="center"><strong>The world's fastest notes app.</strong><br>
Opens before you blink, types with no lag, and keeps every note as a plain Markdown file.</p>

<p align="center">
  <a href="https://github.com/bloodyhill/kolada/releases/latest"><img src="https://img.shields.io/github/v/release/bloodyhill/kolada?label=release&color=0E9594" alt="Latest release"></a>
  <a href="https://github.com/bloodyhill/kolada/actions/workflows/ci.yml"><img src="https://github.com/bloodyhill/kolada/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <img src="https://img.shields.io/badge/platforms-Windows%20%7C%20macOS%20%7C%20Linux-0E9594" alt="Platforms: Windows, macOS, Linux">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-source%20available-555" alt="License: source available"></a>
  <a href="https://kolada.app"><img src="https://img.shields.io/badge/web-kolada.app-0E9594" alt="kolada.app"></a>
</p>

<p align="center">
  <a href="https://apps.microsoft.com/detail/9pjf28mdqqt4?referrer=appbadge"><img src="https://get.microsoft.com/images/en-us%20dark.svg" width="200" alt="Get it from Microsoft"></a>
</p>

<p align="center">
  <a href="https://kolada.app"><img src="https://kolada.app/shots/hero.webp" alt="Kolada on Windows: a note with a checklist in the side menu's vault" width="860"></a>
</p>

## Get Kolada

| Platform | Download | Notes |
| --- | --- | --- |
| **Windows 10 and 11** | [Microsoft Store](https://apps.microsoft.com/detail/9pjf28mdqqt4) or [Kolada-Setup.exe](https://kolada.app/download/Kolada-Setup.exe) | The website's version updates itself; the Store updates its own. |
| **macOS** (Apple silicon) | [Kolada-0.4.0.dmg](https://kolada.app/download/Kolada-0.4.0.dmg) | Beta. Signed and notarized by Apple. |
| **iPad and iPhone** | In progress | |
| **Linux** (x86-64) | [kolada_0.4.0_amd64.deb](https://kolada.app/download/kolada_0.4.0_amd64.deb) or [kolada-0.4.0-linux-x86_64.tar.gz](https://kolada.app/download/kolada-0.4.0-linux-x86_64.tar.gz) | Beta. Debian 12+, Ubuntu 22.04+, or any distribution with glibc 2.35+ (the .tar.gz installs with `./install.sh`). |

Every version is on the [releases page](https://github.com/bloodyhill/kolada/releases), with its checksums. What changed in each is in the [changelog](CHANGELOG.md).

## Why Kolada

- **Fast, measured.** Kolada is ready to type in 47 ms. Notepad takes 723 ms, OneNote about a second and Obsidian 1.6 s on the same machine. A keystroke costs 0.09 ms even at 4K, and the app uses no CPU when you are not typing.
- **Your notes are files.** A vault is a plain folder of Markdown files. Obsidian and any other editor open it as it is, and nothing is locked in.
- **Private by design.** No account, no analytics, no tracking. The writing help is a small model that runs on your computer and never calls the network.
- **Quiet until you need it.** The page is the app. Menus and panels show when you reach for them and go when you are done.

## Features

- **Clean editing:** notes show their formatting, never Markdown symbols.
- **Version history:** scrub back through every version of a note, and restore one.
- **Search:** forgives typos, and reads the text of PDFs and pictures.
- **Smart paste:** a web page, Word or Google Docs selection lands as a clean note, pictures included.
- **Writing help:** fixes slips such as "Teh" or "becuase" on your own machine. Ctrl+Z undoes it.
- **Command palette:** every command a few letters away (Ctrl+P).
- **Minimal mode:** a small note window that stays on top.
- **Today:** tasks with dates gather in one card, with reminders at their time.
- **Imports and exports:** Evernote (`.enex`), Notion (`.zip`) and Notes (`.nbk`) exports, and Word, PDF and HTML files, become notes. A note exports as Markdown, a web page or a PDF.
- **Every script:** right-to-left text, tables and input methods for Japanese, Chinese and Korean.

## Privacy

Kolada goes online only for what you ask of it: pictures you paste from the web, notes you share, the writing model you choose, and, in the version from the website, a daily check for updates. Details are on the [privacy page](https://kolada.app/privacy/).

## Building from source

You need a recent stable [Rust](https://rustup.rs). Clone with `--recursive` to get the `llama.cpp` submodule; only the writing model uses it.

```bash
git clone --recursive https://github.com/bloodyhill/kolada.git
cd kolada
```

| Task | Command |
| --- | --- |
| Windows app | `cargo build --release -p kolada-win` |
| Mac app | `bash tools/mac-bundle.sh --open` |
| Linux app | `cargo build --release -p kolada-linux` |
| Shared tests | `cargo test -p kolada-core -p kolada-editor -p kolada-markdown -p kolada-ui -p kolada-app` |
| Windows tests | `cargo test -p kolada-win` |

## Repository layout

| Path | What it holds |
| --- | --- |
| `crates/kolada-core` | Vaults, sessions, history, search and imports from other notes apps |
| `crates/kolada-editor` | Text, selection and undo |
| `crates/kolada-markdown` | Markdown parsing and styling |
| `crates/kolada-text` | Text shaping and layout |
| `crates/kolada-ui` | The scene, chrome and theme every platform draws |
| `crates/kolada-convert` | DOCX, PDF and HTML to notes |
| `crates/kolada-assist`, `crates/kolada-ai` | The local writing model, in a process of its own |
| `crates/kolada-app` | The app model the Mac, Linux and iOS shells share |
| `crates/kolada-win`, `crates/kolada-mac`, `crates/kolada-linux`, `crates/kolada-ios` | The platform shells |
| `apps/ios` | The iPad and iPhone app's Swift shell |
| `tools` | Build, release, benchmark and icon tools |

## Contributing

Bug reports and ideas are welcome as [issues](https://github.com/bloodyhill/kolada/issues). The code is not open for pull requests. See [CONTRIBUTING.md](CONTRIBUTING.md).

## Security

Please report vulnerabilities privately, as [SECURITY.md](SECURITY.md) describes, not in a public issue.

## License

The source is published so it can be read, with all rights reserved: see [LICENSE](LICENSE). Third-party components keep their own licenses: see [THIRD-PARTY.md](THIRD-PARTY.md).

<p align="center"><a href="https://kolada.app">kolada.app</a> · <a href="https://kolada.app/docs/">Docs</a> · <a href="https://kolada.app/privacy/">Privacy</a> · <a href="https://apps.microsoft.com/detail/9pjf28mdqqt4">Microsoft Store</a></p>
