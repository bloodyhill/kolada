# Changelog

Everything that changes in a Kolada release is listed here, newest first.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html). Before 0.2.0 the app was called Notsy.

## [Unreleased]

### Fixed
- Emoji draw in colour on Linux. A smiley typed into a note came out black and white, because the note's font has plain smileys of its own.
- Symbols that are emoji by default, such as ⭐ ✅ ❌, draw as colour emoji on every platform, as Unicode asks. Text symbols such as ✓ and ❤ stay as text.

## [0.2.0] - 2026-10-14

### Changed
- Notsy is now **Kolada**, at [kolada.app](https://kolada.app). The new name shows everywhere: the window, the menus, the installer, Settings > Apps, and the Mac and iOS apps.
- A new look: the bird and the accent are teal, and the dark theme is a lighter, neutral graphite. Text, muted text and hints keep the contrast they had, and links read as text in both themes.
- The Mac app is signed with a Developer ID and notarized by Apple, so it opens like any other app.
- Updates now come from kolada.app.

### Fixed
- Writing help starts a moment after the app again; since 0.1.0 it could wait a minute. Reminders no longer miss their time.

### Migration
- Nothing to do. On its first start Kolada takes over Notsy's settings, history and search index. Your notes stay in the folder they were in.
- A website install moves itself to the Kolada folder and keeps your choices: the desktop shortcut, `.md` files and starting with Windows. A Notsy from the website updates to Kolada by itself.

## [0.1.1] - 2026-10-11

### Added
- A new installer with a welcome, a choice of where your notes live (a new folder, or one you already have from Obsidian, Logseq or another app), and choices for a desktop shortcut, `.md` files and starting with Windows.
- The first Mac beta, for Apple silicon.

## [0.1.0] - 2026-10-10

### Added
- Installs from the website, per user and without admin rights, and updates itself: it checks once a day, downloads in the background, and the update goes in when you click it or at the next start.

## [0.0.1] - 2026-10-10

### Added
- The first release, in the Microsoft Store.

[Unreleased]: https://github.com/bloodyhill/kolada/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/bloodyhill/kolada/releases/tag/v0.2.0
[0.1.1]: https://kolada.app
[0.1.0]: https://kolada.app
[0.0.1]: https://apps.microsoft.com/detail/9pjf28mdqqt4
