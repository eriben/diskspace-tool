use bytesize::ByteSize;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use jwalk::WalkDir;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs},
    Terminal,
};
use std::{
    collections::{HashMap, HashSet},
    env,
    fs,
    io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

// ── Category system ──────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Category {
    Cache,
    BuildArtifact,
    Docker,
    Simulator,
    Messages,
    ICloud,
    AppData,
    OrphanedAppData, // leftover data from deleted apps
    Downloads,
    Trash,
    NodeModules,
    PackageCache,
    Xcode,
    Logs,
    VirtualMachine,
    BrowserData,
    Mail,
    Photos,
    MusicMedia,
    Backup,
    Python,
    AndroidSdk,
    None,
}

impl Category {
    fn emoji(&self) -> &'static str {
        match self {
            Category::Cache => "🧹",
            Category::BuildArtifact => "🔨",
            Category::Docker => "🐳",
            Category::Simulator => "📱",
            Category::Messages => "💬",
            Category::ICloud => "☁️ ",
            Category::AppData => "🗄️ ",
            Category::OrphanedAppData => "👻",
            Category::Downloads => "📥",
            Category::Trash => "🗑️ ",
            Category::NodeModules => "📦",
            Category::PackageCache => "📦",
            Category::Xcode => "🔧",
            Category::Logs => "📝",
            Category::VirtualMachine => "🖥️ ",
            Category::BrowserData => "🌐",
            Category::Mail => "📧",
            Category::Photos => "📸",
            Category::MusicMedia => "🎵",
            Category::Backup => "💾",
            Category::Python => "🐍",
            Category::AndroidSdk => "🤖",
            Category::None => "📁",
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Category::Cache => "CACHE",
            Category::BuildArtifact => "BUILD",
            Category::Docker => "DOCKER",
            Category::Simulator => "iOS SIM",
            Category::Messages => "MESSAGES",
            Category::ICloud => "iCLOUD",
            Category::AppData => "APP DATA",
            Category::OrphanedAppData => "ORPHANED",
            Category::Downloads => "DOWNLOADS",
            Category::Trash => "TRASH",
            Category::NodeModules => "NODE_MODULES",
            Category::PackageCache => "PKG CACHE",
            Category::Xcode => "XCODE",
            Category::Logs => "LOGS",
            Category::VirtualMachine => "VM",
            Category::BrowserData => "BROWSER",
            Category::Mail => "MAIL",
            Category::Photos => "PHOTOS",
            Category::MusicMedia => "MUSIC",
            Category::Backup => "BACKUP",
            Category::Python => "PYTHON",
            Category::AndroidSdk => "ANDROID SDK",
            Category::None => "",
        }
    }

    fn color(&self) -> Color {
        match self {
            Category::Cache => Color::Green,
            Category::BuildArtifact => Color::Magenta,
            Category::Docker => Color::Blue,
            Category::Simulator => Color::Cyan,
            Category::Messages => Color::Yellow,
            Category::ICloud => Color::White,
            Category::AppData => Color::Gray,
            Category::OrphanedAppData => Color::Rgb(255, 100, 200), // hot pink — stands out
            Category::Downloads => Color::LightCyan,
            Category::Trash => Color::Red,
            Category::NodeModules => Color::LightMagenta,
            Category::PackageCache => Color::LightYellow,
            Category::Xcode => Color::LightBlue,
            Category::Logs => Color::DarkGray,
            Category::VirtualMachine => Color::Rgb(255, 160, 60),
            Category::BrowserData => Color::Rgb(100, 180, 255),
            Category::Mail => Color::Rgb(255, 220, 100),
            Category::Photos => Color::Rgb(255, 140, 200),
            Category::MusicMedia => Color::Rgb(160, 120, 255),
            Category::Backup => Color::Rgb(100, 220, 180),
            Category::Python => Color::Rgb(80, 180, 80),
            Category::AndroidSdk => Color::Rgb(100, 200, 80),
            Category::None => Color::Reset,
        }
    }

    fn hint(&self) -> &'static str {
        match self {
            Category::Cache => "⚡ Safe to clear",
            Category::BuildArtifact => "♻️  Rebuild anytime",
            Category::Docker => "🐳 docker system prune",
            Category::Simulator => "📱 xcrun simctl delete unavailable",
            Category::NodeModules => "📦 rm -rf & npm install",
            Category::PackageCache => "⚡ Safe to clear",
            Category::Trash => "🗑️  Empty trash to reclaim",
            Category::Logs => "⚡ Usually safe to clear",
            Category::Xcode => "🔧 Xcode → Preferences → Locations",
            Category::OrphanedAppData => "👻 App deleted — leftovers safe to remove",
            Category::VirtualMachine => "🖥️  VM disk images — large but intentional",
            Category::BrowserData => "🌐 Browser profiles/cache",
            Category::Backup => "💾 iOS backups — check iTunes/Finder",
            Category::Python => "🐍 venvs/pyenv — rm -rf to reclaim",
            Category::AndroidSdk => "🤖 Android Studio → SDK Manager / AVD Manager",
            _ => "",
        }
    }

    fn all_clearable() -> &'static [Category] {
        &[
            Category::Cache,
            Category::BuildArtifact,
            Category::Docker,
            Category::Simulator,
            Category::NodeModules,
            Category::PackageCache,
            Category::Trash,
            Category::Logs,
            Category::Xcode,
            Category::OrphanedAppData,
            Category::Python,
            Category::AndroidSdk,
        ]
    }
}

// ── Installed-app detection ───────────────────────────────────

struct InstalledApps {
    /// Lower-cased display names e.g. {"slack", "zoom", "1password"}
    names: HashSet<String>,
    /// Lower-cased bundle IDs e.g. {"com.tinyspeck.slackmacgap"}
    bundle_ids: HashSet<String>,
}

impl InstalledApps {
    fn build(home: &Path) -> Self {
        let mut names: HashSet<String> = HashSet::new();
        let mut bundle_ids: HashSet<String> = HashSet::new();

        let app_dirs = [
            PathBuf::from("/Applications"),
            PathBuf::from("/System/Applications"),
            PathBuf::from("/System/Applications/Utilities"),
            home.join("Applications"),
        ];

        for dir in &app_dirs {
            let entries = match fs::read_dir(dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map_or(false, |e| e == "app") {
                    // App display name from folder
                    if let Some(stem) = path.file_stem() {
                        names.insert(stem.to_string_lossy().to_lowercase());
                    }
                    // Bundle ID from Info.plist (handles both XML and binary plists)
                    let plist_path = path.join("Contents/Info.plist");
                    if let Ok(val) = plist::Value::from_file(&plist_path) {
                        if let Some(dict) = val.as_dictionary() {
                            if let Some(bid) = dict
                                .get("CFBundleIdentifier")
                                .and_then(|v| v.as_string())
                            {
                                bundle_ids.insert(bid.to_lowercase());
                            }
                            // CFBundleName can differ from folder name (e.g. "1Password 7")
                            if let Some(bname) =
                                dict.get("CFBundleName").and_then(|v| v.as_string())
                            {
                                names.insert(bname.to_lowercase());
                            }
                            if let Some(bname) =
                                dict.get("CFBundleDisplayName").and_then(|v| v.as_string())
                            {
                                names.insert(bname.to_lowercase());
                            }
                        }
                    }
                }
            }
        }

        Self { names, bundle_ids }
    }

    /// True if an app with this display-name folder is currently installed.
    fn has_name(&self, name: &str) -> bool {
        self.names.contains(&name.to_lowercase())
    }

    /// True if a bundle-id is currently installed (also strips team-ID prefix).
    fn has_bundle_id(&self, id: &str) -> bool {
        let lower = id.to_lowercase();
        if self.bundle_ids.contains(&lower) {
            return true;
        }
        // Group Containers use "TEAMID.com.bundle.id" — strip the team prefix
        if let Some(rest) = lower.splitn(2, '.').nth(1) {
            // rest is everything after first component, could still be "bundle.id"
            if self.bundle_ids.contains(rest) {
                return true;
            }
            // Also try matching suffix: some bundles end with ".bundlesuffix"
            for bid in &self.bundle_ids {
                if bid.ends_with(&format!(".{}", rest)) || rest.starts_with(bid.as_str()) {
                    return true;
                }
            }
        }
        false
    }
}

/// True for Apple-owned bundle IDs that are OS services, not user-installed apps.
fn is_apple_system_bundle(id: &str) -> bool {
    let l = id.to_lowercase();
    l.starts_with("com.apple.")
        || l.starts_with("com.apple.dt.")
        || l == "systempreferencesextension"
}

/// True if the string looks like a reverse-DNS bundle identifier.
fn looks_like_bundle_id(s: &str) -> bool {
    let dots = s.chars().filter(|&c| c == '.').count();
    dots >= 2 && s.len() > 8 && !s.contains(' ')
}

fn classify_path(path: &Path, name: &str, installed: &InstalledApps) -> Category {
    let path_str = path.to_string_lossy();

    // ── Trash ────────────────────────────────────────────────
    if name == ".Trash" || path_str.contains("/.Trash/") {
        return Category::Trash;
    }

    // ── iOS device backups ───────────────────────────────────
    if path_str.contains("/MobileSync/Backup")
        || path_str.contains("Application Support/MobileSync")
    {
        return Category::Backup;
    }

    // ── Android SDK / Emulators ─────────────────────────────
    if path_str.contains("/Library/Android/")
        || path_str.contains("/.android/avd")
        || path_str.contains("/.android/cache")
        || (name == ".android" && path_str.contains("/Users/"))
    {
        return Category::AndroidSdk;
    }

    // ── Xcode / iOS Simulators ───────────────────────────────
    if path_str.contains("/CoreSimulator/Devices")
        || path_str.contains("/CoreSimulator/Caches")
    {
        return Category::Simulator;
    }
    if path_str.contains("/Developer/Xcode")
        || path_str.contains("/Developer/CoreSimulator")
        || name == "DerivedData"
        || (name == "Xcode" && path_str.contains("/Developer/"))
    {
        return Category::Xcode;
    }

    // ── Caches ───────────────────────────────────────────────
    if path_str.contains("/Library/Caches")
        || path_str.contains("/.cache")
        || name == "Cache"
        || name == "Caches"
        || path_str.contains("/.npm/_cacache")
        || path_str.contains("/.yarn/cache")
        || path_str.contains("/pip/cache")
        || path_str.contains("/.cargo/registry")
        || path_str.contains("/.cargo/git")
    {
        return Category::Cache;
    }

    // ── Node modules ─────────────────────────────────────────
    if name == "node_modules" {
        return Category::NodeModules;
    }

    // ── Build artifacts ──────────────────────────────────────
    if name == ".gradle"
        || name == ".grails"
        || name == ".m2"
        || (name == "target"
            && (path.join("debug").exists() || path.join("release").exists()))
        || (name == "build"
            && path.parent().map_or(false, |p| {
                p.join("build.gradle").exists()
                    || p.join("build.gradle.kts").exists()
                    || p.join("CMakeLists.txt").exists()
            }))
        || (name == ".next" && path.parent().map_or(false, |p| p.join("package.json").exists()))
        || (name == "dist"
            && path.parent().map_or(false, |p| p.join("package.json").exists()))
        || (name == "__pycache__")
        || (name == ".turbo")
        || (name == ".parcel-cache")
    {
        return Category::BuildArtifact;
    }

    // ── Python environments ──────────────────────────────────
    if (name == ".venv" || name == "venv" || name == "env" || name == ".env")
        && path.join("pyvenv.cfg").exists()
    {
        return Category::Python;
    }
    if path_str.contains("/.pyenv/versions/") || path_str.contains("/pyenv/versions/") {
        return Category::Python;
    }
    if name == ".tox" || path_str.contains("/.tox/") {
        return Category::Python;
    }

    // ── Docker ───────────────────────────────────────────────
    if path_str.contains("/.docker")
        || path_str.contains("/Docker.app")
        || (path_str.contains("/Containers/") && path_str.contains("com.docker"))
    {
        return Category::Docker;
    }

    // ── Virtual machines ─────────────────────────────────────
    if name.ends_with(".vmwarevm")
        || name.ends_with(".parallels")
        || name.ends_with(".utm")
        || name.ends_with(".vmdk")
        || name.ends_with(".vdi")
        || name.ends_with(".qcow2")
        || name.ends_with(".vbox")
        || path_str.contains("/Parallels/")
        || path_str.contains("/UTM/")
        || (path_str.contains("/Containers/") && path_str.contains("com.utmapp"))
        || (path_str.contains("/Containers/") && path_str.contains("com.parallels"))
    {
        return Category::VirtualMachine;
    }

    // ── Browser data ─────────────────────────────────────────
    let browser_markers = [
        "/Application Support/Google/Chrome",
        "/Application Support/BraveSoftware",
        "/Application Support/Firefox",
        "/Application Support/Microsoft Edge",
        "/Application Support/Chromium",
        "/Application Support/Vivaldi",
        "/Application Support/Arc",
        "/Application Support/Opera",
        "/Library/Safari",
    ];
    if browser_markers.iter().any(|m| path_str.contains(m)) {
        return Category::BrowserData;
    }

    // ── Mail ────────────────────────────────────────────────
    if path_str.contains("/Library/Mail")
        || (path_str.contains("/Containers/") && path_str.contains("com.apple.mail"))
    {
        return Category::Mail;
    }

    // ── Photos ──────────────────────────────────────────────
    if name.ends_with(".photoslibrary")
        || path_str.contains("/Pictures/Photos Library")
        || (path_str.contains("/Containers/") && path_str.contains("com.apple.Photos"))
        || (path_str.contains("/Containers/") && path_str.contains("com.apple.photos"))
    {
        return Category::Photos;
    }

    // ── Music / iTunes ──────────────────────────────────────
    if path_str.contains("/Music/Music/Media")
        || path_str.contains("/Music/iTunes")
        || (path_str.contains("/Containers/") && path_str.contains("com.apple.Music"))
    {
        return Category::MusicMedia;
    }

    // ── iMessages ───────────────────────────────────────────
    if path_str.contains("/Messages/Attachments")
        || (path_str.contains("/Messages/") && name == "Attachments")
    {
        return Category::Messages;
    }

    // ── iCloud ──────────────────────────────────────────────
    if path_str.contains("/Mobile Documents") || path_str.contains("iCloud~") {
        return Category::ICloud;
    }

    // ── Package manager caches ───────────────────────────────
    if path_str.contains("/.npm")
        || path_str.contains("/.yarn")
        || path_str.contains("/.pnpm")
        || path_str.contains("/.nvm")
        || path_str.contains("/.bun")
    {
        return Category::PackageCache;
    }

    // ── Logs ────────────────────────────────────────────────
    if name == "Logs" || name == "logs" || path_str.contains("/Library/Logs") {
        return Category::Logs;
    }

    // ── Downloads ────────────────────────────────────────────
    if name == "Downloads" && path_str.contains("/Users/") {
        return Category::Downloads;
    }

    // ── Orphaned app data ────────────────────────────────────
    // Check if this is a direct child of Application Support / Containers / Group Containers
    // and its corresponding app is no longer installed.
    if let Some(parent) = path.parent() {
        let parent_name = parent
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        if parent_name == "Application Support" {
            // Skip generic Apple-ish folder names
            if !is_apple_system_folder(name) {
                if !installed.has_name(name) && !looks_like_bundle_id(name) {
                    return Category::OrphanedAppData;
                }
                if looks_like_bundle_id(name) && !is_apple_system_bundle(name) && !installed.has_bundle_id(name) {
                    return Category::OrphanedAppData;
                }
            }
        } else if parent_name == "Containers" {
            if looks_like_bundle_id(name) && !is_apple_system_bundle(name) {
                if !installed.has_bundle_id(name) {
                    return Category::OrphanedAppData;
                }
            }
        } else if parent_name == "Group Containers" {
            if looks_like_bundle_id(name) && !is_apple_system_bundle(name) {
                if !installed.has_bundle_id(name) {
                    return Category::OrphanedAppData;
                }
            }
        } else if parent_name == "Saved Application State" {
            let base = name.trim_end_matches(".savedState");
            if looks_like_bundle_id(base) && !is_apple_system_bundle(base) {
                if !installed.has_bundle_id(base) {
                    return Category::OrphanedAppData;
                }
            }
        } else if parent_name == "WebKit" || parent_name == "HTTPStorages" {
            if looks_like_bundle_id(name) && !is_apple_system_bundle(name) {
                if !installed.has_bundle_id(name) {
                    return Category::OrphanedAppData;
                }
            }
        } else if parent_name == "PreferencePanes" {
            // e.g. ~/Library/PreferencePanes/OldApp.prefPane
            let base = name.trim_end_matches(".prefPane");
            if !installed.has_name(base) && !is_apple_system_folder(base) {
                return Category::OrphanedAppData;
            }
        } else if parent_name == "Internet Plug-Ins" || parent_name == "QuickLook" {
            // Orphaned browser plugins and QuickLook generators
            let base = name
                .trim_end_matches(".plugin")
                .trim_end_matches(".qlgenerator");
            if !installed.has_name(base) && !is_apple_system_folder(base) {
                return Category::OrphanedAppData;
            }
        } else if parent_name == "Screen Savers" {
            let base = name.trim_end_matches(".saver");
            if !installed.has_name(base) && !is_apple_system_folder(base) {
                return Category::OrphanedAppData;
            }
        }
    }

    // ── Generic app data (still installed or system) ─────────
    if path_str.contains("/Application Support/")
        || path_str.contains("/Containers/")
        || path_str.contains("/Group Containers/")
    {
        return Category::AppData;
    }

    Category::None
}

/// Folders inside Application Support that are Apple system services, not user apps.
fn is_apple_system_folder(name: &str) -> bool {
    matches!(
        name,
        "Apple"
            | "AddressBook"
            | "CallHistoryDB"
            | "CallHistoryTransactions"
            | "CloudDocs"
            | "CrashReporter"
            | "Dock"
            | "FaceTime"
            | "iCloud"
            | "Knowledge"
            | "networkserviceproxy"
            | "NGL"
            | "NotificationCenter"
            | "osanalyticshelper"
            | "Preferences"
            | "Safari"
            | "Spaces"
            | "SyncServices"
            | "Ticket Viewer"
            | "TV"
    ) || name.starts_with("com.apple.")
}

// ── Data structures ──────────────────────────────────────────

#[derive(Clone)]
struct DirEntry {
    name: String,
    path: PathBuf,
    size: u64,
    is_dir: bool,
    category: Category,
    #[allow(dead_code)]
    children_count: usize,
    error: bool,
}

#[derive(Clone)]
struct ScanResult {
    #[allow(dead_code)]
    root: PathBuf,
    entries: Vec<DirEntry>,
    total_size: u64,
    largest_files: Vec<(PathBuf, u64)>,
    category_sizes: HashMap<String, u64>,
}

// ── Scanner ──────────────────────────────────────────────────

// Paths to skip when scanning from / on macOS to avoid double-counting
// and infinite traversal of virtual/snapshot filesystems
fn should_skip_path(path: &Path, root: &Path) -> bool {
    let path_str = path.to_string_lossy();

    // Only apply these exclusions when scanning from / or /System etc.
    // If user explicitly scans ~/Library, don't skip anything inside it
    let root_str = root.to_string_lossy();
    if root_str.starts_with("/Users/") || root_str.starts_with("/home/") {
        return false;
    }

    // macOS firmlinks / volume mounts that duplicate data
    if path_str.starts_with("/System/Volumes/Data")
        || path_str.starts_with("/System/Volumes/Preboot")
        || path_str.starts_with("/System/Volumes/Recovery")
        || path_str.starts_with("/System/Volumes/Update")
        || path_str.starts_with("/System/Volumes/VM")
        || path_str.starts_with("/System/Volumes/xarts")
        || path_str.starts_with("/System/Volumes/iSCPreboot")
        || path_str.starts_with("/System/Volumes/Hardware")
    {
        return true;
    }

    // Time Machine snapshots
    if path_str.contains("/.MobileBackups")
        || path_str.contains("/.Spotlight-V100")
        || path_str.contains("/.fseventsd")
        || path_str.starts_with("/Volumes/com.apple.TimeMachine")
    {
        return true;
    }

    // /dev, /proc equivalents
    if path_str.starts_with("/dev") || path_str.starts_with("/proc") {
        return true;
    }

    false
}

fn build_entries(
    dir: &Path,
    dir_sizes: &HashMap<PathBuf, u64>,
    dir_children: &HashMap<PathBuf, usize>,
    dir_errors: &HashMap<PathBuf, bool>,
    installed: &InstalledApps,
) -> Vec<DirEntry> {
    let mut entries = Vec::new();

    let read_dir = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return entries,
    };

    for entry in read_dir.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let metadata = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        if metadata.is_symlink() {
            continue;
        }

        let is_dir = metadata.is_dir();
        let size = if is_dir {
            *dir_sizes.get(&path).unwrap_or(&0)
        } else {
            metadata.len()
        };

        let category = classify_path(&path, &name, installed);
        let children_count = if is_dir {
            *dir_children.get(&path).unwrap_or(&0)
        } else {
            0
        };
        let error = dir_errors.get(&path).copied().unwrap_or(false);

        entries.push(DirEntry {
            name,
            path,
            size,
            is_dir,
            category,
            children_count,
            error,
        });
    }

    // Sort by size descending
    entries.sort_by(|a, b| b.size.cmp(&a.size));
    entries
}

// ── Full scan with stored dir_sizes ──────────────────────────

struct FullScan {
    result: ScanResult,
    dir_sizes: HashMap<PathBuf, u64>,
    dir_children: HashMap<PathBuf, usize>,
    dir_errors: HashMap<PathBuf, bool>,
    installed: Arc<InstalledApps>,
}

fn scan_directory_full(
    root: &Path,
    scan_count: Arc<AtomicU64>,
    scan_bytes: Arc<AtomicU64>,
    scanning: Arc<AtomicBool>,
    live_data: Arc<Mutex<Option<FullScan>>>,
) {
    let installed = Arc::new(InstalledApps::build(&dirs_home()));
    let mut dir_sizes: HashMap<PathBuf, u64> = HashMap::new();
    let mut dir_children: HashMap<PathBuf, usize> = HashMap::new();
    let mut dir_errors: HashMap<PathBuf, bool> = HashMap::new();
    let mut largest_files: Vec<(PathBuf, u64)> = Vec::new();
    let mut category_sizes: HashMap<String, u64> = HashMap::new();
    let mut total_size: u64 = 0;
    let mut last_snapshot = Instant::now();

    for entry in WalkDir::new(root)
        .skip_hidden(false)
        .sort(true)
        .process_read_dir(|_depth, _path, _read_dir_state, children| {
            // Remove entries for paths we want to skip entirely (prevents descent)
            children.retain(|child_result| {
                match child_result {
                    Ok(child) => !should_skip_path(&child.path(), &PathBuf::from("/")),
                    Err(_) => true,
                }
            });
        })
        .into_iter()
        .flatten()
    {
        if !scanning.load(Ordering::Relaxed) {
            break;
        }

        let path = entry.path();

        // Double-check skip (process_read_dir root path check uses "/" hardcoded)
        if should_skip_path(&path, root) {
            continue;
        }

        let metadata = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => {
                if let Some(parent) = path.parent() {
                    dir_errors.insert(parent.to_path_buf(), true);
                }
                continue;
            }
        };

        if metadata.is_symlink() {
            continue;
        }

        if metadata.is_file() {
            let size = metadata.len();
            total_size += size;

            scan_count.fetch_add(1, Ordering::Relaxed);
            scan_bytes.fetch_add(size, Ordering::Relaxed);

            if size > 10_000_000 {
                largest_files.push((path.to_path_buf(), size));
            }

            let mut current = path.parent();
            while let Some(parent) = current {
                *dir_sizes.entry(parent.to_path_buf()).or_insert(0) += size;
                if parent == root {
                    break;
                }
                current = parent.parent();
            }

            if let Some(parent) = path.parent() {
                *dir_children.entry(parent.to_path_buf()).or_insert(0) += 1;
            }

            let cat = classify_path(&path, &path.file_name().unwrap_or_default().to_string_lossy(), &installed);
            if cat != Category::None {
                *category_sizes
                    .entry(cat.label().to_string())
                    .or_insert(0) += size;
            } else {
                let mut check = path.parent();
                while let Some(p) = check {
                    let pname = p.file_name().unwrap_or_default().to_string_lossy().to_string();
                    let pcat = classify_path(p, &pname, &installed);
                    if pcat != Category::None {
                        *category_sizes
                            .entry(pcat.label().to_string())
                            .or_insert(0) += size;
                        break;
                    }
                    if p == root {
                        break;
                    }
                    check = p.parent();
                }
            }
        } else if metadata.is_dir() {
            if let Some(parent) = path.parent() {
                if path != root {
                    *dir_children.entry(parent.to_path_buf()).or_insert(0) += 1;
                }
            }
        }

        // Publish incremental snapshots every 500ms so the UI can show live results
        if last_snapshot.elapsed() > Duration::from_millis(500) {
            let mut sorted_files = largest_files.clone();
            sorted_files.sort_by(|a, b| b.1.cmp(&a.1));
            sorted_files.truncate(100);

            let entries = build_entries(root, &dir_sizes, &dir_children, &dir_errors, &installed);
            if let Ok(mut slot) = live_data.lock() {
                *slot = Some(FullScan {
                    result: ScanResult {
                        root: root.to_path_buf(),
                        entries,
                        total_size,
                        largest_files: sorted_files,
                        category_sizes: category_sizes.clone(),
                    },
                    dir_sizes: dir_sizes.clone(),
                    dir_children: dir_children.clone(),
                    dir_errors: dir_errors.clone(),
                    installed: Arc::clone(&installed),
                });
            }
            last_snapshot = Instant::now();
        }
    }

    // Final snapshot
    largest_files.sort_by(|a, b| b.1.cmp(&a.1));
    largest_files.truncate(100);

    let entries = build_entries(root, &dir_sizes, &dir_children, &dir_errors, &installed);
    if let Ok(mut slot) = live_data.lock() {
        *slot = Some(FullScan {
            result: ScanResult {
                root: root.to_path_buf(),
                entries,
                total_size,
                largest_files,
                category_sizes,
            },
            dir_sizes,
            dir_children,
            dir_errors,
            installed,
        });
    }
}

// ── App state ────────────────────────────────────────────────

enum ActiveTab {
    Tree,
    TopFiles,
    Categories,
}

struct App {
    current_path: PathBuf,
    path_stack: Vec<PathBuf>,
    entries: Vec<DirEntry>,
    list_state: ListState,
    active_tab: ActiveTab,
    scan_result: Option<ScanResult>,
    full_scan: Option<Arc<Mutex<Option<FullScan>>>>,
    scanning: Arc<AtomicBool>,
    scan_count: Arc<AtomicU64>,
    scan_bytes: Arc<AtomicU64>,
    scan_start: Option<Instant>,
    disk_total: u64,
    disk_free: u64,
    file_list_state: ListState,
    cat_list_state: ListState,
    search_mode: bool,
    search_query: String,
    filtered_indices: Vec<usize>,
}

impl App {
    fn new(root: PathBuf) -> Self {
        // Get disk space info
        let (disk_total, disk_free) = get_disk_space(&root);

        let mut app = App {
            current_path: root.clone(),
            path_stack: Vec::new(),
            entries: Vec::new(),
            list_state: ListState::default(),
            active_tab: ActiveTab::Tree,
            scan_result: None,
            full_scan: None,
            scanning: Arc::new(AtomicBool::new(false)),
            scan_count: Arc::new(AtomicU64::new(0)),
            scan_bytes: Arc::new(AtomicU64::new(0)),
            scan_start: None,
            disk_total,
            disk_free,
            file_list_state: ListState::default(),
            cat_list_state: ListState::default(),
            search_mode: false,
            search_query: String::new(),
            filtered_indices: Vec::new(),
        };
        app.start_scan(root);
        app
    }

    fn start_scan(&mut self, path: PathBuf) {
        self.scanning.store(true, Ordering::Relaxed);
        self.scan_count.store(0, Ordering::Relaxed);
        self.scan_bytes.store(0, Ordering::Relaxed);
        self.scan_start = Some(Instant::now());

        let scanning = self.scanning.clone();
        let scan_count = self.scan_count.clone();
        let scan_bytes = self.scan_bytes.clone();
        let live_data = Arc::new(Mutex::new(None));
        self.full_scan = Some(live_data.clone());

        thread::spawn(move || {
            scan_directory_full(&path, scan_count, scan_bytes, scanning.clone(), live_data);
            scanning.store(false, Ordering::Relaxed);
        });
    }

    fn check_scan_update(&mut self) {
        // Pull latest snapshot from the scanner (works both during and after scan)
        if let Some(ref slot) = self.full_scan {
            if let Ok(guard) = slot.lock() {
                if let Some(ref full) = *guard {
                    // Only update if we're viewing the scan root (not drilled in)
                    if self.path_stack.is_empty() {
                        let had_selection = self.list_state.selected().is_some();
                        self.entries = full.result.entries.clone();
                        self.scan_result = Some(full.result.clone());
                        if !had_selection && !self.entries.is_empty() {
                            self.list_state.select(Some(0));
                        }
                    } else {
                        // When drilled in, just update scan_result for categories/top files
                        self.scan_result = Some(full.result.clone());
                    }
                }
            }
        }
    }

    fn enter_directory(&mut self) {
        let selected = match self.list_state.selected() {
            Some(i) => i,
            None => return,
        };

        let entry = if self.search_mode && !self.filtered_indices.is_empty() {
            if selected >= self.filtered_indices.len() { return; }
            &self.entries[self.filtered_indices[selected]]
        } else {
            if selected >= self.entries.len() { return; }
            &self.entries[selected]
        };

        if !entry.is_dir {
            return;
        }

        let new_path = entry.path.clone();

        // Try to rebuild entries from cached scan data
        if let Some(ref slot) = self.full_scan {
            if let Ok(guard) = slot.lock() {
                if let Some(ref full) = *guard {
                    let new_entries = build_entries(
                        &new_path,
                        &full.dir_sizes,
                        &full.dir_children,
                        &full.dir_errors,
                        &full.installed,
                    );

                    self.path_stack.push(self.current_path.clone());
                    self.current_path = new_path;
                    self.entries = new_entries;
                    self.list_state.select(if self.entries.is_empty() {
                        None
                    } else {
                        Some(0)
                    });
                    self.search_mode = false;
                    self.search_query.clear();
                    self.filtered_indices.clear();
                    return;
                }
            }
        }
    }

    fn go_back(&mut self) {
        if let Some(prev_path) = self.path_stack.pop() {
            if let Some(ref slot) = self.full_scan {
                if let Ok(guard) = slot.lock() {
                    if let Some(ref full) = *guard {
                        let new_entries = build_entries(
                            &prev_path,
                            &full.dir_sizes,
                            &full.dir_children,
                            &full.dir_errors,
                            &full.installed,
                        );
                        self.current_path = prev_path;
                        self.entries = new_entries;
                        self.list_state.select(if self.entries.is_empty() {
                            None
                        } else {
                            Some(0)
                        });
                        self.search_mode = false;
                        self.search_query.clear();
                        self.filtered_indices.clear();
                    }
                }
            }
        }
    }

    fn visible_entries_len(&self) -> usize {
        if self.search_mode && !self.search_query.is_empty() {
            self.filtered_indices.len()
        } else {
            self.entries.len()
        }
    }

    fn update_search(&mut self) {
        let query = self.search_query.to_lowercase();
        self.filtered_indices = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.name.to_lowercase().contains(&query))
            .map(|(i, _)| i)
            .collect();
        if !self.filtered_indices.is_empty() {
            self.list_state.select(Some(0));
        } else {
            self.list_state.select(None);
        }
    }
}

fn get_disk_space(path: &Path) -> (u64, u64) {
    // Use statvfs on macOS/Linux
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::mem::MaybeUninit;

        let c_path = match CString::new(path.to_string_lossy().as_bytes()) {
            Ok(p) => p,
            Err(_) => return (0, 0),
        };

        unsafe {
            let mut stat = MaybeUninit::<libc::statfs>::uninit();
            if libc::statfs(c_path.as_ptr(), stat.as_mut_ptr()) == 0 {
                let stat = stat.assume_init();
                let total = stat.f_blocks as u64 * stat.f_bsize as u64;
                let free = stat.f_bavail as u64 * stat.f_bsize as u64;
                return (total, free);
            }
        }
        (0, 0)
    }

    #[cfg(not(unix))]
    {
        (0, 0)
    }
}

// ── UI rendering ─────────────────────────────────────────────

const TREEMAP_COLORS: &[Color] = &[
    Color::Rgb(99, 155, 255),   // Blue
    Color::Rgb(120, 220, 120),  // Green
    Color::Rgb(220, 120, 220),  // Magenta
    Color::Rgb(255, 200, 80),   // Gold
    Color::Rgb(80, 220, 220),   // Cyan
    Color::Rgb(255, 120, 100),  // Red/coral
    Color::Rgb(180, 160, 255),  // Lavender
    Color::Rgb(255, 160, 120),  // Peach
    Color::Rgb(120, 200, 180),  // Teal
    Color::Rgb(220, 180, 100),  // Amber
];

fn size_color(size: u64) -> Color {
    if size > 10_000_000_000 {
        Color::Rgb(255, 80, 80) // Bright red
    } else if size > 1_000_000_000 {
        Color::Rgb(255, 180, 60) // Orange
    } else if size > 100_000_000 {
        Color::Yellow
    } else {
        Color::Rgb(180, 180, 180) // Dim gray
    }
}

fn bar_color(ratio: f64) -> Color {
    if ratio > 0.5 {
        Color::Rgb(255, 80, 80)
    } else if ratio > 0.2 {
        Color::Rgb(255, 200, 60)
    } else if ratio > 0.05 {
        Color::Rgb(80, 220, 120)
    } else {
        Color::Rgb(60, 60, 100)
    }
}

fn render_header(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let is_scanning = app.scanning.load(Ordering::Relaxed);
    let count = app.scan_count.load(Ordering::Relaxed);
    let bytes = app.scan_bytes.load(Ordering::Relaxed);

    let path_display = app.current_path.to_string_lossy();
    let elapsed = app
        .scan_start
        .map(|s| s.elapsed().as_secs())
        .unwrap_or(0);

    let disk_total = ByteSize(app.disk_total);
    let disk_used = ByteSize(app.disk_total.saturating_sub(app.disk_free));
    let disk_free = ByteSize(app.disk_free);
    let usage_pct = if app.disk_total > 0 {
        ((app.disk_total - app.disk_free) as f64 / app.disk_total as f64 * 100.0) as u8
    } else {
        0
    };

    let mut spans = vec![
        Span::styled(" 💾 ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::styled(
            format!("{} ", disk_total),
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        ),
        Span::styled("total", Style::default().fg(Color::DarkGray)),
        Span::styled("  ┃  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("🔴 {} ", disk_used),
            Style::default().fg(Color::Rgb(255, 100, 100)),
        ),
        Span::styled(
            format!("used ({}%)", usage_pct),
            Style::default().fg(Color::Rgb(255, 100, 100)),
        ),
        Span::styled("  ┃  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("🟢 {} ", disk_free),
            Style::default().fg(Color::Rgb(100, 255, 100)),
        ),
        Span::styled("free", Style::default().fg(Color::Rgb(100, 255, 100))),
    ];

    if is_scanning {
        spans.push(Span::styled("  ┃  ", Style::default().fg(Color::DarkGray)));
        let spinner = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let idx = (elapsed as usize * 3) % spinner.len();
        spans.push(Span::styled(
            format!(
                "{} Scanning... {} files  {} ",
                spinner[idx],
                count,
                ByteSize(bytes)
            ),
            Style::default()
                .fg(Color::Rgb(255, 200, 60))
                .add_modifier(Modifier::BOLD),
        ));
    }

    let header_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(80, 140, 255)))
        .title(Span::styled(
            format!(" 📂 {} ", path_display),
            Style::default()
                .fg(Color::Rgb(120, 200, 255))
                .add_modifier(Modifier::BOLD),
        ));

    let header = Paragraph::new(Line::from(spans)).block(header_block);
    f.render_widget(header, area);
}

fn render_treemap_bar(f: &mut ratatui::Frame, area: Rect, app: &App) {
    if app.entries.is_empty() {
        return;
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(80, 80, 120)))
        .title(Span::styled(
            " 🗺️  Space Map ",
            Style::default()
                .fg(Color::Rgb(200, 180, 255))
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width < 4 || inner.height < 2 {
        return;
    }

    let parent_size: u64 = app.entries.iter().map(|e| e.size).sum();
    if parent_size == 0 {
        return;
    }

    let bar_width = inner.width as usize;

    // Build treemap segments
    let mut segments: Vec<(String, usize, Color)> = Vec::new();
    let mut used_width = 0;

    for (i, entry) in app.entries.iter().take(12).enumerate() {
        let ratio = entry.size as f64 / parent_size as f64;
        let width = (ratio * bar_width as f64).round() as usize;
        if width == 0 {
            continue;
        }
        let width = width.min(bar_width - used_width);
        if width == 0 {
            break;
        }

        let color = TREEMAP_COLORS[i % TREEMAP_COLORS.len()];
        let label = format!(
            "{} {} {}",
            entry.category.emoji(),
            if entry.name.len() > 12 {
                &entry.name[..12]
            } else {
                &entry.name
            },
            ByteSize(entry.size)
        );

        segments.push((label, width, color));
        used_width += width;
    }

    // Render bar (top line - colored blocks)
    let mut bar_spans: Vec<Span> = Vec::new();
    for (_, width, color) in &segments {
        let block_char = "█";
        bar_spans.push(Span::styled(
            block_char.repeat(*width),
            Style::default().fg(*color),
        ));
    }
    if used_width < bar_width {
        bar_spans.push(Span::styled(
            "░".repeat(bar_width - used_width),
            Style::default().fg(Color::Rgb(40, 40, 60)),
        ));
    }
    let bar_line = Paragraph::new(Line::from(bar_spans));
    f.render_widget(bar_line, Rect::new(inner.x, inner.y, inner.width, 1));

    // Render labels (bottom line)
    if inner.height >= 2 {
        let mut label_spans: Vec<Span> = Vec::new();
        for (label, width, color) in &segments {
            let display = if label.len() > *width {
                if *width > 3 {
                    format!("{:.w$}", label, w = width - 1)
                } else {
                    " ".repeat(*width)
                }
            } else {
                format!("{:<w$}", label, w = width)
            };
            label_spans.push(Span::styled(
                display,
                Style::default().fg(*color).add_modifier(Modifier::DIM),
            ));
        }
        let label_line = Paragraph::new(Line::from(label_spans));
        f.render_widget(
            label_line,
            Rect::new(inner.x, inner.y + 1, inner.width, 1),
        );
    }
}

fn render_tree_view(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    let parent_size: u64 = app.entries.iter().map(|e| e.size).sum();
    let bar_max_width: usize = 14;

    let items: Vec<ListItem> = if app.search_mode && !app.search_query.is_empty() {
        app.filtered_indices
            .iter()
            .map(|&i| render_entry_item(&app.entries[i], parent_size, bar_max_width, area.width))
            .collect()
    } else {
        app.entries
            .iter()
            .map(|e| render_entry_item(e, parent_size, bar_max_width, area.width))
            .collect()
    };

    // Build clearable-space summary for the title bar
    let clearable_summary = app.scan_result.as_ref().map(|r| {
        let clearable: u64 = r
            .category_sizes
            .iter()
            .filter(|(label, _)| {
                Category::all_clearable()
                    .iter()
                    .any(|c| c.label() == label.as_str())
            })
            .map(|(_, s)| *s)
            .sum();
        if clearable > 0 {
            format!("  🧹 {} clearable", ByteSize(clearable))
        } else {
            String::new()
        }
    }).unwrap_or_default();

    let mut title = format!(
        " 📂 {}{}",
        app.current_path.to_string_lossy(),
        clearable_summary
    );
    if !app.path_stack.is_empty() {
        title = format!(
            " 📂 {} (⌫ back){}",
            app.current_path.to_string_lossy(),
            clearable_summary
        );
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(80, 80, 120)))
        .title(Span::styled(
            title,
            Style::default()
                .fg(Color::Rgb(120, 200, 255))
                .add_modifier(Modifier::BOLD),
        ));

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(40, 50, 80))
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    f.render_stateful_widget(list, area, &mut app.list_state);
}

fn render_entry_item(entry: &DirEntry, parent_size: u64, bar_max_width: usize, total_width: u16) -> ListItem<'static> {
    let ratio = if parent_size > 0 {
        entry.size as f64 / parent_size as f64
    } else {
        0.0
    };
    let pct = (ratio * 100.0) as u8;

    // Emoji for the entry
    let emoji = if entry.is_dir {
        if entry.category != Category::None {
            entry.category.emoji()
        } else {
            "📁"
        }
    } else {
        file_emoji(&entry.name)
    };

    // Size with color
    let size_str = format!("{}", ByteSize(entry.size));
    let s_color = size_color(entry.size);

    // Bar
    let bar_filled = (ratio * bar_max_width as f64).round() as usize;
    let bar_filled = bar_filled.min(bar_max_width);
    let bar_empty = bar_max_width - bar_filled;
    let b_color = bar_color(ratio);

    // Category badge
    let cat = entry.category;

    // Build the line
    let name_display = if entry.is_dir {
        format!("{}/", entry.name)
    } else {
        entry.name.clone()
    };

    // Truncate name if needed
    let available = total_width as usize;
    let name_max = available.saturating_sub(45); // space for size + bar + pct + badge
    let name_truncated = if name_display.len() > name_max && name_max > 3 {
        format!("{}…", &name_display[..name_max - 1])
    } else {
        name_display
    };

    let mut spans = vec![
        Span::styled(
            format!("{} ", emoji),
            Style::default(),
        ),
        Span::styled(
            format!("{:<width$}", name_truncated, width = name_max.min(30)),
            Style::default()
                .fg(if entry.is_dir {
                    Color::Rgb(120, 180, 255)
                } else {
                    Color::Rgb(200, 200, 200)
                })
                .add_modifier(if entry.is_dir {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ),
        Span::styled(
            format!("{:>10} ", size_str),
            Style::default().fg(s_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "█".repeat(bar_filled),
            Style::default().fg(b_color),
        ),
        Span::styled(
            "░".repeat(bar_empty),
            Style::default().fg(Color::Rgb(40, 40, 60)),
        ),
        Span::styled(
            format!(" {:>3}%", pct),
            Style::default().fg(Color::Rgb(140, 140, 160)),
        ),
    ];

    // Add category badge
    if cat != Category::None {
        spans.push(Span::styled(
            format!(" [{}]", cat.label()),
            Style::default()
                .fg(Color::Rgb(30, 30, 30))
                .bg(cat.color()),
        ));
    }

    // Add error indicator
    if entry.error {
        spans.push(Span::styled(
            " ⚠️ ",
            Style::default().fg(Color::Yellow),
        ));
    }

    ListItem::new(Line::from(spans))
}

fn file_emoji(name: &str) -> &'static str {
    let lower = name.to_lowercase();
    if lower.ends_with(".zip") || lower.ends_with(".tar") || lower.ends_with(".gz") || lower.ends_with(".7z") || lower.ends_with(".rar") {
        "📦"
    } else if lower.ends_with(".dmg") || lower.ends_with(".iso") || lower.ends_with(".img") {
        "💿"
    } else if lower.ends_with(".mp4") || lower.ends_with(".mov") || lower.ends_with(".avi") || lower.ends_with(".mkv") {
        "🎬"
    } else if lower.ends_with(".mp3") || lower.ends_with(".wav") || lower.ends_with(".flac") || lower.ends_with(".aac") {
        "🎵"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") || lower.ends_with(".png") || lower.ends_with(".gif") || lower.ends_with(".webp") || lower.ends_with(".heic") {
        "🖼️ "
    } else if lower.ends_with(".pdf") {
        "📄"
    } else if lower.ends_with(".app") {
        "🚀"
    } else if lower.ends_with(".log") {
        "📝"
    } else if lower.ends_with(".db") || lower.ends_with(".sqlite") || lower.ends_with(".sqlite3") {
        "🗃️ "
    } else {
        "📄"
    }
}

fn render_top_files(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    let items: Vec<ListItem> = if let Some(ref result) = app.scan_result {
        result
            .largest_files
            .iter()
            .take(50)
            .map(|(path, size)| {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                let dir = path
                    .parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();

                let emoji = file_emoji(&name);
                let s_color = size_color(*size);

                let max_dir = area.width as usize - 40;
                let dir_display = if dir.len() > max_dir && max_dir > 3 {
                    format!("…{}", &dir[dir.len() - max_dir + 1..])
                } else {
                    dir
                };

                ListItem::new(Line::from(vec![
                    Span::styled(format!("{} ", emoji), Style::default()),
                    Span::styled(
                        format!("{:>10}", format!("{}", ByteSize(*size))),
                        Style::default().fg(s_color).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("  ", Style::default()),
                    Span::styled(
                        name.to_string(),
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("  {}", dir_display),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]))
            })
            .collect()
    } else {
        vec![ListItem::new(Span::styled(
            "  Scanning...",
            Style::default().fg(Color::Yellow),
        ))]
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(80, 80, 120)))
        .title(Span::styled(
            " 📊 Largest Files (>10MB) ",
            Style::default()
                .fg(Color::Rgb(255, 200, 120))
                .add_modifier(Modifier::BOLD),
        ));

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(40, 50, 80))
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    f.render_stateful_widget(list, area, &mut app.file_list_state);
}

fn render_categories(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    let items: Vec<ListItem> = if let Some(ref result) = app.scan_result {
        let mut cats: Vec<(&String, &u64)> = result.category_sizes.iter().collect();
        cats.sort_by(|a, b| b.1.cmp(a.1));

        let total_categorized: u64 = cats.iter().map(|(_, s)| **s).sum();

        let mut list_items: Vec<ListItem> = Vec::new();

        // Summary header
        list_items.push(ListItem::new(Line::from(vec![
            Span::styled(
                format!(
                    "  📊 Total categorized: {}  ({:.1}% of scanned)",
                    ByteSize(total_categorized),
                    if result.total_size > 0 {
                        total_categorized as f64 / result.total_size as f64 * 100.0
                    } else {
                        0.0
                    }
                ),
                Style::default()
                    .fg(Color::Rgb(180, 220, 255))
                    .add_modifier(Modifier::BOLD),
            ),
        ])));

        // Clearable space
        let clearable: u64 = cats
            .iter()
            .filter(|(name, _)| {
                Category::all_clearable()
                    .iter()
                    .any(|c| c.label() == name.as_str())
            })
            .map(|(_, s)| **s)
            .sum();

        if clearable > 0 {
            list_items.push(ListItem::new(Line::from(vec![
                Span::styled(
                    format!("  🧹 Potentially clearable: {} ", ByteSize(clearable)),
                    Style::default()
                        .fg(Color::Rgb(100, 255, 100))
                        .add_modifier(Modifier::BOLD),
                ),
            ])));
        }

        list_items.push(ListItem::new(Line::from(vec![Span::raw("")])));

        for (name, size) in &cats {
            // Find matching category
            let cat = [
                Category::Cache,
                Category::BuildArtifact,
                Category::Docker,
                Category::Simulator,
                Category::Messages,
                Category::ICloud,
                Category::AppData,
                Category::OrphanedAppData,
                Category::Downloads,
                Category::Trash,
                Category::NodeModules,
                Category::PackageCache,
                Category::Xcode,
                Category::Logs,
                Category::VirtualMachine,
                Category::BrowserData,
                Category::Mail,
                Category::Photos,
                Category::MusicMedia,
                Category::Backup,
                Category::Python,
                Category::AndroidSdk,
            ]
            .iter()
            .find(|c| c.label() == name.as_str())
            .copied()
            .unwrap_or(Category::None);

            let ratio = if result.total_size > 0 {
                **size as f64 / result.total_size as f64
            } else {
                0.0
            };

            let bar_width = 20;
            let filled = (ratio * bar_width as f64).round() as usize;
            let filled = filled.min(bar_width);

            let mut spans = vec![
                Span::styled(format!("  {} ", cat.emoji()), Style::default()),
                Span::styled(
                    format!("{:<15}", name),
                    Style::default()
                        .fg(cat.color())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{:>10}", format!("{}", ByteSize(**size))),
                    Style::default()
                        .fg(size_color(**size))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("  ", Style::default()),
                Span::styled(
                    "█".repeat(filled),
                    Style::default().fg(cat.color()),
                ),
                Span::styled(
                    "░".repeat(bar_width - filled),
                    Style::default().fg(Color::Rgb(40, 40, 60)),
                ),
                Span::styled(
                    format!("  {:.1}%", ratio * 100.0),
                    Style::default().fg(Color::Rgb(140, 140, 160)),
                ),
            ];

            let hint = cat.hint();
            if !hint.is_empty() {
                spans.push(Span::styled(
                    format!("  {}", hint),
                    Style::default().fg(Color::Rgb(100, 200, 100)).add_modifier(Modifier::DIM),
                ));
            }

            list_items.push(ListItem::new(Line::from(spans)));
        }

        list_items
    } else {
        vec![ListItem::new(Span::styled(
            "  Scanning...",
            Style::default().fg(Color::Yellow),
        ))]
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(80, 80, 120)))
        .title(Span::styled(
            " 🏷️  Categories — Where Your Bytes Went ",
            Style::default()
                .fg(Color::Rgb(200, 160, 255))
                .add_modifier(Modifier::BOLD),
        ));

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(40, 50, 80))
                .fg(Color::White),
        )
        .highlight_symbol("▸ ");

    f.render_stateful_widget(list, area, &mut app.cat_list_state);
}

fn render_search_bar(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let spans = vec![
        Span::styled(
            " 🔍 Search: ",
            Style::default()
                .fg(Color::Rgb(255, 200, 60))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            &app.search_query,
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "█",
            Style::default()
                .fg(Color::Rgb(255, 200, 60))
                .add_modifier(Modifier::SLOW_BLINK),
        ),
        Span::styled(
            format!("  ({} matches)", app.filtered_indices.len()),
            Style::default().fg(Color::DarkGray),
        ),
    ];

    let search = Paragraph::new(Line::from(spans)).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Rgb(255, 200, 60))),
    );

    f.render_widget(search, area);
}

fn render_footer(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let nav = match app.active_tab {
        ActiveTab::Tree => {
            vec![
                Span::styled(" ⬆⬇ ", Style::default().fg(Color::Rgb(120, 200, 255)).add_modifier(Modifier::BOLD)),
                Span::styled("Navigate  ", Style::default().fg(Color::DarkGray)),
                Span::styled("↩ ", Style::default().fg(Color::Rgb(120, 255, 120)).add_modifier(Modifier::BOLD)),
                Span::styled("Enter dir  ", Style::default().fg(Color::DarkGray)),
                Span::styled("⌫ ", Style::default().fg(Color::Rgb(255, 180, 80)).add_modifier(Modifier::BOLD)),
                Span::styled("Back  ", Style::default().fg(Color::DarkGray)),
                Span::styled("/ ", Style::default().fg(Color::Rgb(255, 200, 60)).add_modifier(Modifier::BOLD)),
                Span::styled("Search  ", Style::default().fg(Color::DarkGray)),
                Span::styled("Tab ", Style::default().fg(Color::Rgb(200, 160, 255)).add_modifier(Modifier::BOLD)),
                Span::styled("Switch view  ", Style::default().fg(Color::DarkGray)),
                Span::styled("r ", Style::default().fg(Color::Rgb(255, 120, 120)).add_modifier(Modifier::BOLD)),
                Span::styled("Rescan  ", Style::default().fg(Color::DarkGray)),
                Span::styled("q ", Style::default().fg(Color::Rgb(255, 100, 100)).add_modifier(Modifier::BOLD)),
                Span::styled("Quit", Style::default().fg(Color::DarkGray)),
            ]
        }
        _ => {
            vec![
                Span::styled(" ⬆⬇ ", Style::default().fg(Color::Rgb(120, 200, 255)).add_modifier(Modifier::BOLD)),
                Span::styled("Navigate  ", Style::default().fg(Color::DarkGray)),
                Span::styled("Tab ", Style::default().fg(Color::Rgb(200, 160, 255)).add_modifier(Modifier::BOLD)),
                Span::styled("Switch view  ", Style::default().fg(Color::DarkGray)),
                Span::styled("q ", Style::default().fg(Color::Rgb(255, 100, 100)).add_modifier(Modifier::BOLD)),
                Span::styled("Quit", Style::default().fg(Color::DarkGray)),
            ]
        }
    };

    let footer = Paragraph::new(Line::from(nav)).block(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(Color::Rgb(60, 60, 80))),
    );
    f.render_widget(footer, area);
}

fn ui(f: &mut ratatui::Frame, app: &mut App) {
    let has_search = app.search_mode;

    // Layout
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Length(3),  // Header
                Constraint::Length(1),  // Tab bar
                Constraint::Length(4),  // Treemap
                if has_search {
                    Constraint::Length(3) // Search bar
                } else {
                    Constraint::Length(0)
                },
                Constraint::Min(8),     // Main content
                Constraint::Length(2),  // Footer
            ]
            .as_ref(),
        )
        .split(f.area());

    // Header
    render_header(f, main_chunks[0], app);

    // Tab bar
    let tab_titles = vec![
        Line::from(Span::styled("🌳 Tree", Style::default())),
        Line::from(Span::styled("📊 Top Files", Style::default())),
        Line::from(Span::styled("🏷️  Categories", Style::default())),
    ];
    let selected_tab = match app.active_tab {
        ActiveTab::Tree => 0,
        ActiveTab::TopFiles => 1,
        ActiveTab::Categories => 2,
    };
    let tabs = Tabs::new(tab_titles)
        .select(selected_tab)
        .style(Style::default().fg(Color::Rgb(120, 120, 150)))
        .highlight_style(
            Style::default()
                .fg(Color::Rgb(255, 220, 120))
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::UNDERLINED),
        )
        .divider(Span::styled(" │ ", Style::default().fg(Color::Rgb(60, 60, 80))));
    f.render_widget(tabs, main_chunks[1]);

    // Treemap bar
    render_treemap_bar(f, main_chunks[2], app);

    // Search bar (if active)
    if has_search {
        render_search_bar(f, main_chunks[3], app);
    }

    // Main content
    match app.active_tab {
        ActiveTab::Tree => render_tree_view(f, main_chunks[4], app),
        ActiveTab::TopFiles => render_top_files(f, main_chunks[4], app),
        ActiveTab::Categories => render_categories(f, main_chunks[4], app),
    }

    // Footer
    render_footer(f, main_chunks[5], app);
}

// ── Main ─────────────────────────────────────────────────────

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();

    // Handle --help / --version before touching the terminal
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("diskspace — TUI disk space analyzer for macOS\n");
        println!("USAGE:");
        println!("  diskspace [PATH]         scan PATH (default: $HOME)");
        println!("  diskspace --help         show this help");
        println!("  diskspace --version      show version\n");
        println!("NAVIGATION:");
        println!("  ↑/↓  j/k    navigate      Enter/→/l  drill in");
        println!("  ←/h  ⌫     go back        Tab        switch view");
        println!("  /           search         r          rescan");
        println!("  g/G         top/bottom     q          quit\n");
        println!("VIEWS:  🌳 Tree  |  📊 Top Files  |  🏷️  Categories");
        return Ok(());
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("diskspace {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let root = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .filter(|a| *a != args.first().unwrap_or(a))
        .map(PathBuf::from)
        .unwrap_or_else(dirs_home);

    let root = fs::canonicalize(&root).unwrap_or(root);

    // Install panic hook to restore terminal state on crash
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(panic_info);
    }));

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(root);

    loop {
        // Check if scan completed
        app.check_scan_update();

        // Draw
        terminal.draw(|f| ui(f, &mut app))?;

        // Handle input with timeout for scan updates
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                // Search mode input handling
                if app.search_mode {
                    match key.code {
                        KeyCode::Esc => {
                            app.search_mode = false;
                            app.search_query.clear();
                            app.filtered_indices.clear();
                            if !app.entries.is_empty() {
                                app.list_state.select(Some(0));
                            }
                        }
                        KeyCode::Enter => {
                            app.search_mode = false;
                            // Keep filter active if there are results
                            if app.filtered_indices.is_empty() {
                                app.search_query.clear();
                            }
                        }
                        KeyCode::Backspace => {
                            app.search_query.pop();
                            if app.search_query.is_empty() {
                                app.filtered_indices.clear();
                                if !app.entries.is_empty() {
                                    app.list_state.select(Some(0));
                                }
                            } else {
                                app.update_search();
                            }
                        }
                        KeyCode::Char(c) => {
                            app.search_query.push(c);
                            app.update_search();
                        }
                        KeyCode::Up => {
                            let len = app.visible_entries_len();
                            if len > 0 {
                                let i = app.list_state.selected().unwrap_or(0);
                                app.list_state.select(Some(if i > 0 { i - 1 } else { len - 1 }));
                            }
                        }
                        KeyCode::Down => {
                            let len = app.visible_entries_len();
                            if len > 0 {
                                let i = app.list_state.selected().unwrap_or(0);
                                app.list_state.select(Some(if i < len - 1 { i + 1 } else { 0 }));
                            }
                        }
                        _ => {}
                    }
                    continue;
                }

                // Normal mode
                match key.code {
                    KeyCode::Char('q') | KeyCode::Char('Q') => break,
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                    KeyCode::Up | KeyCode::Char('k') => {
                        match app.active_tab {
                            ActiveTab::Tree => {
                                let len = app.visible_entries_len();
                                if len > 0 {
                                    let i = app.list_state.selected().unwrap_or(0);
                                    app.list_state.select(Some(if i > 0 { i - 1 } else { len - 1 }));
                                }
                            }
                            ActiveTab::TopFiles => {
                                let len = app.scan_result.as_ref().map(|r| r.largest_files.len().min(50)).unwrap_or(0);
                                if len > 0 {
                                    let i = app.file_list_state.selected().unwrap_or(0);
                                    app.file_list_state.select(Some(if i > 0 { i - 1 } else { len - 1 }));
                                }
                            }
                            ActiveTab::Categories => {
                                let len = app.scan_result.as_ref().map(|r| r.category_sizes.len() + 3).unwrap_or(0);
                                if len > 0 {
                                    let i = app.cat_list_state.selected().unwrap_or(0);
                                    app.cat_list_state.select(Some(if i > 0 { i - 1 } else { len - 1 }));
                                }
                            }
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        match app.active_tab {
                            ActiveTab::Tree => {
                                let len = app.visible_entries_len();
                                if len > 0 {
                                    let i = app.list_state.selected().unwrap_or(0);
                                    app.list_state.select(Some(if i < len - 1 { i + 1 } else { 0 }));
                                }
                            }
                            ActiveTab::TopFiles => {
                                let len = app.scan_result.as_ref().map(|r| r.largest_files.len().min(50)).unwrap_or(0);
                                if len > 0 {
                                    let i = app.file_list_state.selected().unwrap_or(0);
                                    app.file_list_state.select(Some(if i < len - 1 { i + 1 } else { 0 }));
                                }
                            }
                            ActiveTab::Categories => {
                                let len = app.scan_result.as_ref().map(|r| r.category_sizes.len() + 3).unwrap_or(0);
                                if len > 0 {
                                    let i = app.cat_list_state.selected().unwrap_or(0);
                                    app.cat_list_state.select(Some(if i < len - 1 { i + 1 } else { 0 }));
                                }
                            }
                        }
                    }
                    KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                        if matches!(app.active_tab, ActiveTab::Tree) {
                            app.enter_directory();
                        }
                    }
                    KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => {
                        if matches!(app.active_tab, ActiveTab::Tree) {
                            app.go_back();
                        }
                    }
                    KeyCode::Tab => {
                        app.active_tab = match app.active_tab {
                            ActiveTab::Tree => ActiveTab::TopFiles,
                            ActiveTab::TopFiles => ActiveTab::Categories,
                            ActiveTab::Categories => ActiveTab::Tree,
                        };
                    }
                    KeyCode::BackTab => {
                        app.active_tab = match app.active_tab {
                            ActiveTab::Tree => ActiveTab::Categories,
                            ActiveTab::TopFiles => ActiveTab::Tree,
                            ActiveTab::Categories => ActiveTab::TopFiles,
                        };
                    }
                    KeyCode::Char('/') => {
                        if matches!(app.active_tab, ActiveTab::Tree) {
                            app.search_mode = true;
                            app.search_query.clear();
                            app.filtered_indices.clear();
                        }
                    }
                    KeyCode::Char('r') | KeyCode::Char('R') => {
                        // Rescan
                        let path = app.current_path.clone();
                        app.scan_result = None;
                        app.entries.clear();
                        app.path_stack.clear();
                        app.list_state.select(None);
                        app.start_scan(path);
                    }
                    KeyCode::Home | KeyCode::Char('g') => {
                        match app.active_tab {
                            ActiveTab::Tree => app.list_state.select(Some(0)),
                            ActiveTab::TopFiles => app.file_list_state.select(Some(0)),
                            ActiveTab::Categories => app.cat_list_state.select(Some(0)),
                        }
                    }
                    KeyCode::End | KeyCode::Char('G') => {
                        match app.active_tab {
                            ActiveTab::Tree => {
                                let len = app.visible_entries_len();
                                if len > 0 {
                                    app.list_state.select(Some(len - 1));
                                }
                            }
                            ActiveTab::TopFiles => {
                                let len = app.scan_result.as_ref().map(|r| r.largest_files.len().min(50)).unwrap_or(0);
                                if len > 0 {
                                    app.file_list_state.select(Some(len - 1));
                                }
                            }
                            ActiveTab::Categories => {
                                let len = app.scan_result.as_ref().map(|r| r.category_sizes.len() + 3).unwrap_or(0);
                                if len > 0 {
                                    app.cat_list_state.select(Some(len - 1));
                                }
                            }
                        }
                    }
                    KeyCode::PageDown => {
                        match app.active_tab {
                            ActiveTab::Tree => {
                                let len = app.visible_entries_len();
                                if len > 0 {
                                    let i = app.list_state.selected().unwrap_or(0);
                                    app.list_state.select(Some((i + 20).min(len - 1)));
                                }
                            }
                            ActiveTab::TopFiles => {
                                let len = app.scan_result.as_ref().map(|r| r.largest_files.len().min(50)).unwrap_or(0);
                                if len > 0 {
                                    let i = app.file_list_state.selected().unwrap_or(0);
                                    app.file_list_state.select(Some((i + 20).min(len - 1)));
                                }
                            }
                            _ => {}
                        }
                    }
                    KeyCode::PageUp => {
                        match app.active_tab {
                            ActiveTab::Tree => {
                                let len = app.visible_entries_len();
                                if len > 0 {
                                    let i = app.list_state.selected().unwrap_or(0);
                                    app.list_state.select(Some(i.saturating_sub(20)));
                                }
                            }
                            ActiveTab::TopFiles => {
                                let len = app.scan_result.as_ref().map(|r| r.largest_files.len().min(50)).unwrap_or(0);
                                if len > 0 {
                                    let i = app.file_list_state.selected().unwrap_or(0);
                                    app.file_list_state.select(Some(i.saturating_sub(20)));
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Cleanup
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}

fn dirs_home() -> PathBuf {
    env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/"))
}

// We need libc for statfs
#[cfg(unix)]
extern crate libc;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn empty_installed() -> InstalledApps {
        InstalledApps {
            names: HashSet::new(),
            bundle_ids: HashSet::new(),
        }
    }

    fn installed_with(names: &[&str], bundle_ids: &[&str]) -> InstalledApps {
        InstalledApps {
            names: names.iter().map(|s| s.to_lowercase()).collect(),
            bundle_ids: bundle_ids.iter().map(|s| s.to_lowercase()).collect(),
        }
    }

    // ── looks_like_bundle_id ─────────────────────────────────

    #[test]
    fn test_looks_like_bundle_id_valid() {
        assert!(looks_like_bundle_id("com.apple.mail"));
        assert!(looks_like_bundle_id("com.tinyspeck.slackmacgap"));
        assert!(looks_like_bundle_id("org.mozilla.firefox"));
    }

    #[test]
    fn test_looks_like_bundle_id_invalid() {
        assert!(!looks_like_bundle_id("Slack"));
        assert!(!looks_like_bundle_id("a.b"));
        assert!(!looks_like_bundle_id("short.id"));
        assert!(!looks_like_bundle_id("has spaces.in.it"));
    }

    // ── is_apple_system_bundle ───────────────────────────────

    #[test]
    fn test_apple_system_bundle() {
        assert!(is_apple_system_bundle("com.apple.mail"));
        assert!(is_apple_system_bundle("com.apple.dt.Xcode"));
        assert!(is_apple_system_bundle("COM.APPLE.Safari"));
    }

    #[test]
    fn test_not_apple_system_bundle() {
        assert!(!is_apple_system_bundle("com.tinyspeck.slackmacgap"));
        assert!(!is_apple_system_bundle("org.mozilla.firefox"));
    }

    // ── is_apple_system_folder ───────────────────────────────

    #[test]
    fn test_apple_system_folder() {
        assert!(is_apple_system_folder("Apple"));
        assert!(is_apple_system_folder("Dock"));
        assert!(is_apple_system_folder("Safari"));
        assert!(is_apple_system_folder("com.apple.something"));
    }

    #[test]
    fn test_not_apple_system_folder() {
        assert!(!is_apple_system_folder("Slack"));
        assert!(!is_apple_system_folder("Firefox"));
    }

    // ── should_skip_path ─────────────────────────────────────

    #[test]
    fn test_skip_system_volumes() {
        let root = Path::new("/");
        assert!(should_skip_path(Path::new("/System/Volumes/Data"), root));
        assert!(should_skip_path(Path::new("/System/Volumes/VM"), root));
        assert!(should_skip_path(Path::new("/dev"), root));
        assert!(should_skip_path(Path::new("/proc"), root));
    }

    #[test]
    fn test_no_skip_under_home() {
        let root = Path::new("/Users/testuser");
        assert!(!should_skip_path(Path::new("/Users/testuser/Library"), root));
    }

    #[test]
    fn test_skip_time_machine() {
        let root = Path::new("/");
        assert!(should_skip_path(
            Path::new("/Volumes/com.apple.TimeMachine.localsnapshots"),
            root
        ));
    }

    // ── classify_path ────────────────────────────────────────

    #[test]
    fn test_classify_trash() {
        let installed = empty_installed();
        assert_eq!(
            classify_path(Path::new("/Users/me/.Trash"), ".Trash", &installed),
            Category::Trash
        );
    }

    #[test]
    fn test_classify_grails_cache() {
        let installed = empty_installed();
        assert_eq!(
            classify_path(Path::new("/Users/me/.grails"), ".grails", &installed),
            Category::BuildArtifact
        );
    }

    #[test]
    fn test_classify_node_modules() {
        let installed = empty_installed();
        assert_eq!(
            classify_path(
                Path::new("/Users/me/project/node_modules"),
                "node_modules",
                &installed
            ),
            Category::NodeModules
        );
    }

    #[test]
    fn test_classify_nvm_cache() {
        let installed = empty_installed();
        assert_eq!(
            classify_path(
                Path::new("/Users/me/.nvm/versions/node/v20"),
                "v20",
                &installed
            ),
            Category::PackageCache
        );
    }

    #[test]
    fn test_classify_bun_cache() {
        let installed = empty_installed();
        assert_eq!(
            classify_path(
                Path::new("/Users/me/.bun/install/cache"),
                "cache",
                &installed
            ),
            Category::PackageCache
        );
    }

    #[test]
    fn test_classify_cache() {
        let installed = empty_installed();
        assert_eq!(
            classify_path(
                Path::new("/Users/me/Library/Caches/something"),
                "something",
                &installed
            ),
            Category::Cache
        );
    }

    #[test]
    fn test_classify_docker() {
        let installed = empty_installed();
        assert_eq!(
            classify_path(Path::new("/Users/me/.docker"), ".docker", &installed),
            Category::Docker
        );
    }

    #[test]
    fn test_classify_ios_simulator() {
        let installed = empty_installed();
        assert_eq!(
            classify_path(
                Path::new("/Users/me/Library/Developer/CoreSimulator/Devices"),
                "Devices",
                &installed
            ),
            Category::Simulator
        );
    }

    #[test]
    fn test_classify_android_sdk() {
        let installed = empty_installed();
        assert_eq!(
            classify_path(
                Path::new("/Users/me/Library/Android/sdk"),
                "sdk",
                &installed
            ),
            Category::AndroidSdk
        );
    }

    #[test]
    fn test_classify_android_avd() {
        let installed = empty_installed();
        assert_eq!(
            classify_path(
                Path::new("/Users/me/.android/avd/Pixel_9_Pro.avd"),
                "Pixel_9_Pro.avd",
                &installed
            ),
            Category::AndroidSdk
        );
    }

    #[test]
    fn test_classify_xcode() {
        let installed = empty_installed();
        assert_eq!(
            classify_path(
                Path::new("/Users/me/Library/Developer/Xcode"),
                "Xcode",
                &installed
            ),
            Category::Xcode
        );
    }

    #[test]
    fn test_classify_vm_files() {
        let installed = empty_installed();
        assert_eq!(
            classify_path(
                Path::new("/Users/me/VMs/test.vmwarevm"),
                "test.vmwarevm",
                &installed
            ),
            Category::VirtualMachine
        );
        assert_eq!(
            classify_path(
                Path::new("/Users/me/VMs/test.qcow2"),
                "test.qcow2",
                &installed
            ),
            Category::VirtualMachine
        );
    }

    #[test]
    fn test_classify_browser_data() {
        let installed = empty_installed();
        assert_eq!(
            classify_path(
                Path::new("/Users/me/Library/Application Support/Google/Chrome"),
                "Chrome",
                &installed
            ),
            Category::BrowserData
        );
    }

    #[test]
    fn test_classify_icloud() {
        let installed = empty_installed();
        assert_eq!(
            classify_path(
                Path::new("/Users/me/Library/Mobile Documents"),
                "Mobile Documents",
                &installed
            ),
            Category::ICloud
        );
    }

    #[test]
    fn test_classify_downloads() {
        let installed = empty_installed();
        assert_eq!(
            classify_path(
                Path::new("/Users/me/Downloads"),
                "Downloads",
                &installed
            ),
            Category::Downloads
        );
    }

    #[test]
    fn test_classify_backup() {
        let installed = empty_installed();
        assert_eq!(
            classify_path(
                Path::new("/Users/me/Library/Application Support/MobileSync/Backup"),
                "Backup",
                &installed
            ),
            Category::Backup
        );
    }

    #[test]
    fn test_classify_messages() {
        let installed = empty_installed();
        assert_eq!(
            classify_path(
                Path::new("/Users/me/Library/Messages/Attachments"),
                "Attachments",
                &installed
            ),
            Category::Messages
        );
    }

    #[test]
    fn test_classify_logs() {
        let installed = empty_installed();
        assert_eq!(
            classify_path(
                Path::new("/Users/me/Library/Logs"),
                "Logs",
                &installed
            ),
            Category::Logs
        );
    }

    #[test]
    fn test_classify_uncategorized() {
        let installed = empty_installed();
        assert_eq!(
            classify_path(
                Path::new("/Users/me/Documents"),
                "Documents",
                &installed
            ),
            Category::None
        );
    }

    // ── InstalledApps ────────────────────────────────────────

    #[test]
    fn test_installed_apps_has_name() {
        let apps = installed_with(&["slack", "zoom"], &[]);
        assert!(apps.has_name("Slack"));
        assert!(apps.has_name("slack"));
        assert!(!apps.has_name("Firefox"));
    }

    #[test]
    fn test_installed_apps_has_bundle_id() {
        let apps = installed_with(&[], &["com.tinyspeck.slackmacgap"]);
        assert!(apps.has_bundle_id("com.tinyspeck.slackmacgap"));
        assert!(apps.has_bundle_id("COM.TINYSPECK.SLACKMACGAP"));
        assert!(!apps.has_bundle_id("org.mozilla.firefox"));
    }

    // ── Category methods ─────────────────────────────────────

    #[test]
    fn test_all_categories_have_labels() {
        let categories = [
            Category::Cache,
            Category::BuildArtifact,
            Category::Docker,
            Category::Simulator,
            Category::Messages,
            Category::ICloud,
            Category::AppData,
            Category::OrphanedAppData,
            Category::Downloads,
            Category::Trash,
            Category::NodeModules,
            Category::PackageCache,
            Category::Xcode,
            Category::Logs,
            Category::VirtualMachine,
            Category::BrowserData,
            Category::Mail,
            Category::Photos,
            Category::MusicMedia,
            Category::Backup,
            Category::Python,
            Category::AndroidSdk,
        ];
        for cat in &categories {
            assert!(!cat.emoji().is_empty(), "Missing emoji for {:?}", cat);
            assert!(!cat.label().is_empty(), "Missing label for {:?}", cat);
        }
    }

    #[test]
    fn test_category_none_has_empty_label() {
        assert_eq!(Category::None.label(), "");
    }

    // ── file_emoji ───────────────────────────────────────────

    #[test]
    fn test_file_emoji() {
        assert_eq!(file_emoji("archive.zip"), "📦");
        assert_eq!(file_emoji("disk.dmg"), "💿");
        assert_eq!(file_emoji("movie.mp4"), "🎬");
        assert_eq!(file_emoji("song.mp3"), "🎵");
        assert_eq!(file_emoji("photo.jpg"), "🖼️ ");
        assert_eq!(file_emoji("document.pdf"), "📄");
        assert_eq!(file_emoji("unknown.xyz"), "📄");
    }
}
