# 💾 diskspace

A beautiful, fast terminal UI for visualizing disk usage on macOS — like DaisyDisk or GrandPerspective, but free and runs in your terminal.

Built specifically to answer *"why is System Data 110 GB?"*

![screenshot placeholder]

## Features

- **Live streaming scan** — results appear as the scanner runs, no waiting
- **Three views** — drill-down tree, largest files, and category breakdown
- **Smart categorization** — detects caches, build artifacts, Docker images, iOS simulators, VM disks, orphaned app leftovers, and more
- **👻 Orphan detection** — identifies data folders left behind by deleted apps by cross-referencing `/Applications`
- **macOS-aware** — skips `/System/Volumes/Data` firmlinks and Time Machine snapshots that would otherwise cause double-counting
- **Colorful** — size-coded colors, emoji icons, inline bar charts

## Categories detected

| Emoji | Category | What it includes |
|-------|----------|-----------------|
| 🧹 | **CACHE** | `~/Library/Caches`, `.cache`, npm/yarn/cargo caches |
| 🔨 | **BUILD** | `node_modules`, `target/`, `DerivedData`, `.gradle`, `.next`, `__pycache__` |
| 🐳 | **DOCKER** | Docker images and volumes |
| 📱 | **iOS SIM** | Xcode CoreSimulator devices |
| 🔧 | **XCODE** | Xcode DerivedData, archives |
| 👻 | **ORPHANED** | Data from apps that are no longer installed |
| 🖥️  | **VM** | VMware, Parallels, UTM disk images |
| 🌐 | **BROWSER** | Chrome, Firefox, Safari, Arc, Edge profiles |
| 📧 | **MAIL** | Mail.app message store |
| 📸 | **PHOTOS** | Photos library |
| 🎵 | **MUSIC** | iTunes/Music media library |
| 💾 | **BACKUP** | iOS device backups |
| 🐍 | **PYTHON** | venvs, pyenv versions, `.tox` |
| ☁️  | **iCLOUD** | Mobile Documents |
| 💬 | **MESSAGES** | iMessage attachments |
| 📝 | **LOGS** | `~/Library/Logs` |
| 🗑️  | **TRASH** | `.Trash` |

## Install

```bash
cargo build --release
# binary at ./target/release/diskspace
```

Or copy the binary somewhere on your `$PATH`:
```bash
cp target/release/diskspace /usr/local/bin/
```

## Usage

```bash
diskspace ~              # scan your home directory
diskspace ~/Library      # drill into Library
sudo diskspace /         # scan the whole disk (needs sudo for some paths)
```

## Navigation

| Key | Action |
|-----|--------|
| `↑` / `↓` or `j` / `k` | Navigate list |
| `Enter` / `→` / `l` | Drill into directory |
| `Backspace` / `←` / `h` | Go back |
| `Tab` / `Shift+Tab` | Switch between Tree / Top Files / Categories views |
| `/` | Search/filter by name |
| `g` / `G` | Jump to top / bottom |
| `PgUp` / `PgDn` | Page up / down |
| `r` | Rescan |
| `q` or `Ctrl+C` | Quit |

## Why not just use `du`?

`du -sh * | sort -h` shows you sizes but not percentages, doesn't drill down interactively, and has no idea what any of the directories *are*. This tool shows you the same data but in a navigable, color-coded UI with category labels so you know what's safe to delete.

## License

MIT
