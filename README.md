# PaperView for desktop

**A free PDF reader for your computer. No ads, no accounts.**

The desktop companion to the [PaperView Android app](https://github.com/nagasab221/Paperview), with the same look and the same cloud. Runs on **Linux** today; a Windows version is coming.

---

## What it can do

### 📖 Comfortable reading
- Open several documents at once, each in its own tab
- Smooth scrolling, or one page at a time
- Zoom that stays sharp: fit the page, fit the width, or anything in between
- Opens right where you left off: same page, same zoom
- Search the document, select and copy text, and click links to follow them
- Contents and page thumbnails in a side panel
- Opens password-protected PDFs

### 👁 Peek: check the answer key without losing your place
Made for working through questions while the answers are somewhere else in the document.

1. Go to the answer key and press **Shift+Q** (or ☰ menu → **Set peek spot here**).
2. **Hold Q** to look at it. Let go and you're back exactly where you were.
3. **Tap Q** to go there, and tap again to come back.

There's an eye button next to the page number that does the same with the mouse: hold it to peek, click it to jump. Each document remembers its own peek spot, and pressing **Shift+Q** again moves it.

### 📚 Your library
All your recent PDFs on a shelf, showing their first page and the page you're on. Right-click a cover to show it in its folder or remove it.

### ☁ Your own cloud (optional)
Keep documents in your own **Backblaze B2** bucket. It's the same bucket as the phone app, so a PDF saved on one shows up on the other. A **Cloud** tab appears in the Library once it's set up, and nothing uploads unless you ask.

### 🎨 Looks like the phone app
Three styles (**Paper**, **Slate** and **Ink**), each in light and dark, or following your system.

---

## Install on Linux

There's no download yet, so for now PaperView is built from the source. It takes a few minutes, once.

1. Install Rust from [rustup.rs](https://rustup.rs) (one command, no root needed).
2. Get PaperView and build it:
   ```sh
   git clone https://github.com/nagasab221/PaperView-Desktop.git
   cd PaperView-Desktop
   scripts/fetch-pdfium.sh
   cargo build --release
   scripts/install-linux.sh
   ```
3. Open **PaperView** from your app menu. To open PDFs with it from your file manager, right-click a PDF → **Open with** → **PaperView**, and set it as the default there.

**Updating:** `git pull`, `cargo build --release`, then `scripts/install-linux.sh` again. Your library and settings are kept.

**Printing** uses CUPS, the standard Linux printing system. If PaperView says it's missing, install it (on Arch-based systems: `sudo pacman -S cups`, then `sudo systemctl enable --now cups`) and add your printer in your system settings.

---

## Set up the cloud

1. Create a free account at [backblaze.com](https://www.backblaze.com/cloud-storage) (10 GB free) and make a **private bucket**. Already use the cloud on the phone app? Use the same bucket and key.
2. In PaperView, open **Settings** (the ⚙ in the Library, or **Ctrl+,**).
3. Under **Backblaze B2**, enter the **Key ID**, **Application key** and **bucket name**, then click **Save and connect**.
4. The **Cloud** tab appears in your Library. To upload, open a document and choose ☰ menu → **Save to cloud**, or drop PDFs onto the Cloud tab.

Don't want the Cloud tab? Switch it off in Settings.

---

## Keyboard shortcuts

| Keys | Does |
| --- | --- |
| **Ctrl+O** | Open a PDF |
| **Ctrl+F** | Find in the document (Enter for the next match) |
| **Hold Q** / **Tap Q** | Peek at your saved spot / jump there and back |
| **Shift+Q** | Set or move the peek spot |
| **Ctrl+=** / **Ctrl+-** / **Ctrl+0** | Zoom in / out / actual size |
| **PgUp** / **PgDn**, **Home** / **End** | Previous / next page, first / last page |
| **Alt+Left** / **Alt+Right** | Back / forward after following a link |
| **Ctrl+Tab** / **Ctrl+W** | Next document / close document |
| **Ctrl+Shift+T** | Reopen the last closed document |
| **Ctrl+P** / **Ctrl+S** | Print / save a copy |
| **Ctrl+,** | Settings |

---

## Privacy

- No ads, no analytics, no accounts.
- Everything works offline. The internet is only used if you set up the cloud, and then only to talk to your own bucket.
- Your Backblaze details are stored only on your computer.

---

<sub>For developers: Rust with egui, pages drawn by PDFium. How it all works is in [docs/DEVELOPER_NOTES.md](docs/DEVELOPER_NOTES.md).</sub>
