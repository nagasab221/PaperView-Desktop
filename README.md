<div align="center">

# PaperView

**A calm, fast reader for your PDFs and ebooks. Free, no ads, no accounts.**

[![Latest release](https://img.shields.io/github/v/release/nagasab221/PaperView-Desktop?label=download&color=8a5a2b)](https://github.com/nagasab221/PaperView-Desktop/releases/latest)
[![Linux](https://img.shields.io/badge/Linux-x86__64-555)](#install)
[![MIT licence](https://img.shields.io/badge/licence-MIT-555)](LICENSE)

![PaperView's library: textbooks on a shelf, each showing the page you're on](docs/screenshots/library.png)

</div>

PaperView keeps everything you're reading on one shelf, opens each file right where you left off, and gets out of the way while you read. It's the desktop companion to the [PaperView Android app](https://github.com/nagasab221/Paperview): same look, same cloud. It runs on **Linux** today; a Windows version is on the way.

## Install

Paste this into a terminal (no root or password needed):

```sh
curl -fsSL https://raw.githubusercontent.com/nagasab221/PaperView-Desktop/main/install.sh | sh
```

Then open **PaperView** from your app menu. Run the same line again any time to update; your library and settings are kept.

<details>
<summary>Other ways to install, and how to remove it</summary>

**Download by hand:** get `PaperView-linux-x86_64.tar.gz` from the [latest release](https://github.com/nagasab221/PaperView-Desktop/releases/latest), unpack it and run `./install.sh` inside. You can also run `./paperview` straight from the folder without installing.

**Remove:** `~/.local/opt/paperview/uninstall.sh`. Your library and settings stay in `~/.local/share/paperview`; delete that folder too to forget everything.

**Works on** 64-bit Linux distributions from about 2022 on (Ubuntu 22.04, Debian 12, Fedora 36, Linux Mint 21, Arch and their successors), on Wayland or X11.

**Build from source** (needs [Rust](https://rustup.rs)):

```sh
git clone https://github.com/nagasab221/PaperView-Desktop.git
cd PaperView-Desktop
scripts/fetch-pdfium.sh
cargo build --release
scripts/install-linux.sh
```

**Printing** uses CUPS, the standard Linux printing system. If PaperView says it's missing, install it (Arch: `sudo pacman -S cups && sudo systemctl enable --now cups`; Ubuntu/Debian: `sudo apt install cups`) and add your printer in your system settings.

</details>

## What it does

### 📚 One shelf for everything you read

Every PDF you open lands on the shelf with its cover and the page you're on. Ebooks get their own **Books** tab, showing how far you've read. **Add a folder** (or drop one onto the window) to see every PDF in it, subfolders included. Files you delete or rename leave the shelf by themselves.

![The Books shelf with classic novels and reading progress](docs/screenshots/books.png)

### 📄 PDFs, sharp at any size

Tabs for several documents, smooth scrolling or one page at a time, and zoom that stays crisp: fit the page, fit the width, or anything in between. Search the whole document, select and copy text (**Ctrl+C**), follow links, browse page thumbnails or the table of contents, and jump straight to any page with **Ctrl+G**. Password-protected PDFs and printing work too.

![A calculus textbook open at page 153 with page thumbnails](docs/screenshots/pdf.png)

![Searching a textbook: every match is highlighted](docs/screenshots/search.png)

### 👁 Peek: check the answers without losing your place

Made for working through questions while the answers are at the back of the book.

1. Go to the answer key and press **Shift+Q** to set a peek spot.
2. **Hold Q** to look at it. Let go and you're back exactly where you were.
3. **Tap Q** to go there, and tap again to come back.

The eye button in the bottom bar does the same with the mouse. Each document remembers its own peek spot.

### 📖 Ebooks, set like a printed book

EPUB, Kindle (MOBI, AZW3, without DRM), FB2, comics (CBZ), plain text and HTML open as two facing pages that meet at the spine, or as one page at a time. Turn pages with the arrow keys, Space, the mouse wheel or a click on either side. The bar shows your chapter and how many pages are left in it, and the thin progress line above it takes you anywhere in the book with one click.

![Pride and Prejudice open as a two-page spread with an illustration](docs/screenshots/reader.png)

**Aa** picks the typeface (Literata, Merriweather, EB Garamond, Lora, Source Serif, Noto Serif and Sans, Atkinson Hyperlegible and OpenDyslexic), the text size, line spacing, margins, and indented or spaced paragraphs.

![The reading options: typefaces, size, spacing, margins, paragraphs](docs/screenshots/reading-options.png)

Press **F11** for full screen with nothing but the pages. A small bar appears when you move the mouse, and **Esc** brings everything back.

![Full screen: just the two pages](docs/screenshots/fullscreen.png)

### ☁ Your own cloud, if you want one

Keep documents in your own private **Backblaze B2** bucket (10 GB free): the same bucket the phone app uses, so a PDF saved on one shows up on the other. Save a PDF with the **☁** button or by right-clicking it in Recents, and rename it on the way if you like. Nothing uploads unless you ask, and none of it appears until you connect a bucket.

![Saving a PDF to the cloud under a new name](docs/screenshots/cloud.png)

<details>
<summary>Setting up the cloud</summary>

1. Create a free account at [backblaze.com](https://www.backblaze.com/cloud-storage) and make a **private bucket**. Already use the cloud on the phone app? Use the same bucket and key.
2. In PaperView, open **Settings** (⚙ in the Library, or **Ctrl+,**).
3. Under **Backblaze B2**, enter the **Key ID**, **Application key** and **bucket name**, then click **Save and connect**.
4. A **Cloud** tab appears in your Library. Upload with ☁, from the right-click menu, or by dropping PDFs onto the Cloud tab.

</details>

### 🎨 Eleven styles, light and dark

Paper, Slate and Ink (as on the phone) plus a style for every main colour: Cherry, Apricot, Saffron, Forest, Lagoon, Cobalt, Lavender and Blossom. Each comes in light and dark, or follows your system, and every one keeps text easy to read. A **Size** slider makes the whole app more compact or larger.

![Settings: eleven styles to choose from](docs/screenshots/settings.png)

<table>
<tr>
<td><img src="docs/screenshots/dark-library.png" alt="The library in dark Cobalt"></td>
<td><img src="docs/screenshots/dark-reader.png" alt="A book in dark Cobalt"></td>
</tr>
</table>

## Keyboard shortcuts

| Keys | Does |
| --- | --- |
| **Ctrl+O** | Open a PDF or ebook |
| **Ctrl+F** | Find in the document (Enter for the next match) |
| **Ctrl+G** | Go to a page |
| **Ctrl+C** / **Ctrl+A** | Copy the selected text / select all |
| **F11** / **Esc** | Full screen / leave full screen |
| **Hold Q** / **Tap Q** | Peek at your saved spot / jump there and back |
| **Shift+Q** | Set or move the peek spot |
| **Ctrl+=** / **Ctrl+-** / **Ctrl+0** | Zoom in / out / actual size (text size in books) |
| **PgUp** / **PgDn**, **Home** / **End** | Previous / next page, first / last page |
| **← / →**, **Space** | Turn the page in a book |
| **Alt+Left** / **Alt+Right** | Back / forward after following a link |
| **Ctrl+Tab** / **Ctrl+W** | Next document / close document |
| **Ctrl+Shift+T** | Reopen the last closed document |
| **Ctrl+P** / **Ctrl+S** | Print / save a copy |
| **Ctrl+,** | Settings |

## Privacy

- No ads, no analytics, no accounts.
- Everything works offline. The internet is only used if you connect a cloud bucket, and then only to talk to your own bucket.
- Your Backblaze details are stored only on your computer.

## Credits

PaperView is written in Rust with [egui](https://github.com/emilk/egui); pages are drawn by [PDFium](https://pdfium.googlesource.com/pdfium/). The reading typefaces are open-source fonts under the SIL Open Font License. How it all fits together is in [docs/DEVELOPER_NOTES.md](docs/DEVELOPER_NOTES.md).

The screenshots show free textbooks from [OpenStax](https://openstax.org) (CC BY 4.0) and public-domain classics from [Project Gutenberg](https://www.gutenberg.org).

PaperView is released under the [MIT licence](LICENSE).
