<p align="center"><img src="assets/icon.png" width="128" alt=""></p>

<h1 align="center">BetterNotes</h1>

A handwritten notebook for the desktop. Write with a pen tablet or a mouse, then export to PDF. Built in Rust with [egui](https://github.com/emilk/egui).

## Features

- pressure-sensitive pen, highlighter, eraser, lasso, text
- draw and hold: a wobbly line, circle or box snaps into a clean shape
- blank, lined, grid or dotted paper; A4, A5, Letter
- notebook library with thumbnails; drag a `.bnote` file in to import it
- images from file or clipboard
- export to PDF, PNG or JPG

## Install

**macOS** (Homebrew)

```sh
brew install --cask 1grzyb1/betternotes/betternotes
```

**macOS / Linux**

```sh
curl -fsSL https://raw.githubusercontent.com/1grzyb1/betternotes/main/install.sh | sh
```

**Windows** (PowerShell)

```powershell
irm https://raw.githubusercontent.com/1grzyb1/betternotes/main/install.ps1 | iex
```

Or grab a zip from [Releases](../../releases). The macOS build isn't notarized, so a manually downloaded copy needs right-click → **Open** the first time; the installers above take care of that.

From source:

```sh
cargo run --release
```

On Linux you'll need `libgtk-3-dev` and `libxkbcommon-dev`.

## Shortcuts

| | |
|---|---|
| `P` `H` `E` `L` `S` `T` | pen, highlighter, eraser, lasso, shape, text |
| `Space` + drag | pan |
| `⌘Z` / `⌘⇧Z` | undo / redo |
| `⌘D` | duplicate selection |
| `⌘+` / `⌘-` / `⌘0` | zoom / fit to width |
| `⌘S` / `⌘E` | save / export to PDF |
| `PgUp` / `PgDn` | previous / next page |

Use `Ctrl` instead of `⌘` on Windows and Linux.

## Where notes live

Notebooks are plain `.bnote` files in `~/Documents/BetterNotes`. Set `BETTERNOTES_DIR` to use another folder.

## Releasing

Every push to `main` builds binaries in GitHub Actions. Pushing a `v*` tag also publishes a release:

```sh
git tag v0.2.0 && git push origin v0.2.0
```
