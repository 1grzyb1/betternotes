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

Prebuilt binaries for macOS, Windows and Linux are on the [Releases](../../releases) page.

The macOS build isn't notarized. On first launch, right-click the app and choose **Open**.

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
