# Contributing to Kolada

Thank you for helping make Kolada better. The most useful thing you can do is tell us what went wrong, or what you wish it did.

## How to help

- **Report a bug:** open an [issue](https://github.com/bloodyhill/kolada/issues/new?template=bug_report.md).
- **Suggest a feature:** open an [issue](https://github.com/bloodyhill/kolada/issues/new?template=feature_request.md). Say what you were trying to do, not only the feature you have in mind.
- **Ask a question:** the [docs](https://kolada.app/docs/) may already answer it; if not, an issue is fine.

## Pull requests

The source is published so it can be read, with all rights reserved (see [LICENSE](LICENSE)), so **pull requests are not accepted**. A pull request will be closed, however good it is. If you found a fix, describe it in an issue: that is just as welcome, and it will be credited in the [changelog](CHANGELOG.md) if you like.

## A good bug report

- **Your system:** Windows or macOS version, and Kolada's version (Settings shows it).
- **Where Kolada came from:** the Microsoft Store, the website's installer or the Mac DMG.
- **What you did, what you expected, and what happened.** Steps to reproduce help the most.
- **A screenshot or screen recording,** if it shows the problem.

**Never paste the text of your own notes.** If the problem depends on what a note contains, describe its shape instead: for example "a table with 3 columns and 40 rows, Arabic text in the second column", or the text with every letter replaced by `x` and every digit by `9`.

## Security issues

Please don't open a public issue for a vulnerability. See [SECURITY.md](SECURITY.md).

## Building and testing

If you want to look into a bug yourself, the [README](README.md#building-from-source) has the build and test commands. Run Kolada with `KOLADA_TEST_INSTANCE=1` set and it keeps a data folder, window and instance of its own, apart from the Kolada you use. Point it at a copy of your notes, never the folder itself.
