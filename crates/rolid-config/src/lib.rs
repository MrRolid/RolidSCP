//! Persistent configuration: saved server profiles, workspace (tabs) and secrets.
//!
//! Files live in the platform config dir (`~/.config/rolidscp/` on Linux):
//! - `profiles.toml`  – saved servers (never contains passwords)
//! - `workspace.toml` – open tabs, colors and locations, restored on start
//!
//! - `vault.bin`      – server passwords, encrypted with the master password ([`vault`])
//!
//! Versions 0.3/0.4 kept passwords in the system keyring; [`secrets`] is only used
//! to read (and, after migration, delete) those old entries.

pub mod strength;
pub mod vault;

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_path: Option<String>,
    /// Tab color `#RRGGBB` used whenever this server is opened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_path: Option<String>,
    /// Production server: extra warnings before destructive operations.
    #[serde(default)]
    pub production: bool,
    /// The password is saved (see `password_store` for where).
    #[serde(default)]
    pub save_password: bool,
    /// Where the saved password lives. Profiles written by 0.3/0.4 have no such
    /// key and mean the system keyring.
    #[serde(default = "PasswordStore::legacy")]
    pub password_store: PasswordStore,
}

impl Profile {
    /// Password saved in the system keyring by an older version (to be migrated).
    pub fn password_in_keyring(&self) -> bool {
        self.save_password && self.password_store == PasswordStore::Keyring
    }

    /// Password saved in the vault (protected by the master password).
    pub fn password_in_vault(&self) -> bool {
        self.save_password && self.password_store == PasswordStore::Vault
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum PasswordStore {
    /// Encrypted vault (0.4.2+).
    #[default]
    Vault,
    /// System keyring (0.3, 0.4.0, 0.4.1).
    Keyring,
}

impl PasswordStore {
    fn legacy() -> Self {
        PasswordStore::Keyring
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Profiles {
    #[serde(default, rename = "profile")]
    pub list: Vec<Profile>,
}

impl Profiles {
    pub fn get(&self, id: &str) -> Option<&Profile> {
        self.list.iter().find(|p| p.id == id)
    }

    /// Insert or replace (by id), keeping the list sorted by name.
    pub fn upsert(&mut self, p: Profile) {
        match self.list.iter_mut().find(|x| x.id == p.id) {
            Some(x) => *x = p,
            None => self.list.push(p),
        }
        self.list.sort_by_key(|p| p.name.to_lowercase());
    }

    pub fn remove(&mut self, id: &str) {
        self.list.retain(|p| p.id != id);
    }
}

/// Remote endpoint of a panel (no secrets).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RemoteTarget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    pub host: String,
    pub port: u16,
    pub user: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_path: Option<String>,
}

impl RemoteTarget {
    pub fn key(&self) -> String {
        format!("{}@{}:{}", self.user, self.host, self.port)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Location {
    Local { path: String },
    Remote { target: RemoteTarget, path: String },
}

/// Legacy (0.1–0.3) tab: a pair of panels sharing one color. Only read, for migration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TabState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub color: String,
    #[serde(default)]
    pub active: usize,
    pub panes: Vec<Location>,
}

/// One tab of a panel: a location with its own color and optional name.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PanelTab {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub color: String,
    pub location: Location,
    /// Listing style of this tab (`None` = detailed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewStyle>,
}

/// How a panel lists its files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ViewStyle {
    /// Today's columns: name, size, date, permissions.
    #[default]
    Full,
    /// Icon and name only, in several columns (filled column by column).
    Brief,
    /// A grid of image previews.
    Thumbs,
}

/// Size of thumbnails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThumbSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl ThumbSize {
    pub const ALL: [ThumbSize; 3] = [ThumbSize::Small, ThumbSize::Medium, ThumbSize::Large];

    /// Edge of the preview square in points.
    pub fn px(self) -> u32 {
        match self {
            ThumbSize::Small => 64,
            ThumbSize::Medium => 112,
            ThumbSize::Large => 176,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ThumbSize::Small => rolid_text::nc!("thumbnail", "small"),
            ThumbSize::Medium => rolid_text::nc!("thumbnail", "medium"),
            ThumbSize::Large => rolid_text::nc!("thumbnail", "large"),
        }
    }
}

/// Tabs of the left or the right panel.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct SideState {
    #[serde(default)]
    pub active: usize,
    #[serde(default, rename = "tab")]
    pub tabs: Vec<PanelTab>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Workspace {
    /// 0 = left panel, 1 = right panel.
    #[serde(default)]
    pub active_side: usize,
    #[serde(default)]
    pub left: SideState,
    #[serde(default)]
    pub right: SideState,
    /// Favorite folders (Ctrl+D), in the user's order.
    #[serde(default, rename = "favorite", skip_serializing_if = "Vec::is_empty")]
    pub favorites: Vec<Favorite>,
    /// Recently shown places, newest first (at most [`RECENT_MAX`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent: Vec<Location>,
    /// Recent targets of copying and moving, newest first (at most [`RECENT_MAX`]).
    #[serde(default, rename = "target", skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<Location>,
    /// Old format (shared tabs with two panels each), converted by [`Workspace::migrate`].
    #[serde(default, rename = "tab", skip_serializing)]
    pub legacy_tabs: Vec<TabState>,
}

/// The vault locks this many seconds after its last use by default.
pub const VAULT_LOCK_DEFAULT_SECS: u32 = 60;

/// How many recent places are kept.
pub const RECENT_MAX: usize = 15;

/// A favorite folder: local, or on a server (bound to its profile).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Favorite {
    /// Own name (`None` = shown by its path).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub location: Location,
}

impl Location {
    pub fn path(&self) -> &str {
        match self {
            Location::Local { path } | Location::Remote { path, .. } => path,
        }
    }

    /// The same folder: the same path on the same disk or server (a server is
    /// the same profile, or the same user@host:port when there is none).
    pub fn same_place(&self, other: &Location) -> bool {
        fn norm(p: &str) -> &str {
            let t = p.trim_end_matches('/');
            if t.is_empty() { p } else { t }
        }
        self.same_disk(other) && norm(self.path()) == norm(other.path())
    }

    /// Both on the local disk, or both on the same server.
    pub fn same_disk(&self, other: &Location) -> bool {
        match (self, other) {
            (Location::Local { .. }, Location::Local { .. }) => true,
            (Location::Remote { target: ta, .. }, Location::Remote { target: tb, .. }) => match (&ta.profile, &tb.profile) {
                (Some(x), Some(y)) => x == y,
                _ => ta.key() == tb.key(),
            },
            _ => false,
        }
    }
}

/// Favorite folders and recent places (kept in the workspace file).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Places {
    pub favorites: Vec<Favorite>,
    pub recent: Vec<Location>,
    pub targets: Vec<Location>,
}

impl Places {
    /// Ctrl+D: add the folder to the favorites, or remove it when it is there.
    /// Returns true when it was added.
    pub fn toggle_favorite(&mut self, location: Location) -> bool {
        match self.favorites.iter().position(|f| f.location.same_place(&location)) {
            Some(i) => {
                self.favorites.remove(i);
                false
            }
            None => {
                self.favorites.push(Favorite { name: None, location });
                true
            }
        }
    }

    pub fn is_favorite(&self, location: &Location) -> bool {
        self.favorites.iter().any(|f| f.location.same_place(location))
    }

    /// Move favorite `i` up (`-1`) or down (`1`); returns its new index.
    /// Add the folder to the favorites (not a toggle). False when it is already there.
    pub fn add_favorite(&mut self, location: Location) -> bool {
        if self.favorites.iter().any(|f| f.location.same_place(&location)) {
            return false;
        }
        self.favorites.push(Favorite { name: None, location });
        true
    }

    /// Remove favorite `i` (if there is one).
    pub fn remove_favorite(&mut self, i: usize) -> Option<Favorite> {
        (i < self.favorites.len()).then(|| self.favorites.remove(i))
    }

    /// Name favorite `i`; an empty name shows the path again.
    pub fn rename_favorite(&mut self, i: usize, name: &str) {
        if let Some(f) = self.favorites.get_mut(i) {
            let n = name.trim();
            f.name = (!n.is_empty()).then(|| n.to_string());
        }
    }

    pub fn move_favorite(&mut self, i: usize, delta: isize) -> usize {
        let j = i as isize + delta;
        if i >= self.favorites.len() || j < 0 || j as usize >= self.favorites.len() {
            return i;
        }
        self.favorites.swap(i, j as usize);
        j as usize
    }

    /// A place was shown: it goes first in the recent places.
    pub fn remember(&mut self, location: Location) {
        self.recent.retain(|l| !l.same_place(&location));
        self.recent.insert(0, location);
        self.recent.truncate(RECENT_MAX);
    }

    /// A copy or move went to this folder: it goes first in the targets.
    pub fn remember_target(&mut self, location: Location) {
        self.targets.retain(|l| !l.same_place(&location));
        self.targets.insert(0, location);
        self.targets.truncate(RECENT_MAX);
    }

    /// Recent targets on the disk or server of `like`, newest first.
    pub fn targets_on(&self, like: &Location) -> Vec<String> {
        self.targets.iter().filter(|l| l.same_disk(like)).map(|l| l.path().to_string()).collect()
    }
}

impl Workspace {
    /// Convert the old shared-tab format: the left panels become the left tabs,
    /// the right panels the right tabs (each keeps the old tab's color and name).
    pub fn migrate(&mut self) {
        if self.legacy_tabs.is_empty() || !self.left.tabs.is_empty() || !self.right.tabs.is_empty() {
            self.legacy_tabs.clear();
            return;
        }
        for t in std::mem::take(&mut self.legacy_tabs) {
            for (i, loc) in t.panes.into_iter().take(2).enumerate() {
                let side = if i == 0 { &mut self.left } else { &mut self.right };
                side.tabs.push(PanelTab { name: t.name.clone(), color: t.color.clone(), location: loc, view: None });
            }
        }
    }
}

/// Application settings (`settings.toml`).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Settings {
    /// Donation address shown in the About dialog (must be one of the built-in ones).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub donate_address: Option<String>,
    /// Transfer speed limit in KiB/s for new transfers; `None` = unlimited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_limit_kib: Option<u64>,
    /// Up to 0.8.7: lock the vault after this many minutes (read only, see
    /// [`Settings::vault_lock_after`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_lock_minutes: Option<u32>,
    /// Lock the vault this many seconds after it was last used; `Some(0)` =
    /// only when the application closes, `None` = the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_lock_secs: Option<u32>,
    /// Day (days since 1970) the keyring migration was last offered and declined.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migration_declined_day: Option<u64>,
    /// External editor command for F4 (`{file}` = path); `None` = detected default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor_command: Option<String>,
    /// Preselected answer when files dragged in from the system already exist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drop_conflict: Option<ConflictPreset>,
    /// Preselected answer in the F5 copy dialog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copy_conflict: Option<ConflictPreset>,
    /// Remembered "Open with" per file extension (lowercase, without the dot).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub open_with: std::collections::BTreeMap<String, OpenWith>,
    /// The toolbar under the menu is hidden.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hide_toolbar: bool,
    /// The bar above the panels (path tree, navigation buttons) is hidden.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hide_nav_bar: bool,
    /// Size of thumbnails (`None` = medium).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumb_size: Option<ThumbSize>,
    /// Thumbnails on servers (`None` = on).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbs_remote: Option<bool>,
    /// Thumbnails also on production servers (off by default).
    #[serde(default, skip_serializing_if = "is_false")]
    pub thumbs_production: bool,
    /// Largest remote file downloaded for a thumbnail, in KiB (`None` = 2048).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbs_remote_max_kib: Option<u64>,
    /// Last masks used to mark / unmark a group (newest first).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mark_masks: Vec<String>,
    /// Text colors by file type are turned off.
    #[serde(default, skip_serializing_if = "is_false")]
    pub type_colors_off: bool,
    /// Own text colors by file type ("image" = "#8a2b9a"), light theme.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub type_colors: std::collections::BTreeMap<String, String>,
    /// Folder names without the [brackets] in the full and brief listings.
    #[serde(default, skip_serializing_if = "is_false")]
    pub dir_brackets_off: bool,
    /// Marking by a mask (and inverting) includes folders by default.
    #[serde(default, skip_serializing_if = "is_false")]
    pub mask_dirs: bool,
    /// The Ext column in the detailed listing.
    #[serde(default, skip_serializing_if = "is_false")]
    pub ext_column: bool,
    /// Dates with seconds.
    #[serde(default, skip_serializing_if = "is_false")]
    pub seconds: bool,
    /// Packing: last format ("zip", "7z", "tar.gz", "tar.zst").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pack_format: Option<String>,
    /// Packing: last compression level ("store", "fast", "normal", "max").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pack_level: Option<String>,
    /// Checksums: last algorithm ("md5", "sha1", "sha256", "sha512", "blake3", "crc32").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum_algo: Option<String>,
    /// Packing: compression threads (default: cores − 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pack_threads: Option<u64>,
    /// Packing zip: compress also files that are compressed already (jpg, mp4, zip…).
    #[serde(default, skip_serializing_if = "is_false")]
    pub pack_compress_all: bool,
    /// Extraction: most bytes written in all, in GB (default 20).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unpack_max_gb: Option<u64>,
    /// Extraction: most items (default 1 000 000).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unpack_max_items: Option<u64>,
    /// Extraction: compression ratio above which to ask (default 1000).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unpack_max_ratio: Option<u64>,
    /// Largest remote archive downloaded to be opened as a folder, in MB (default 500).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_archive_max_mb: Option<u64>,
    /// Text font from the system (fontconfig) instead of the built-in Noto Sans.
    #[serde(default, skip_serializing_if = "is_false")]
    pub font_system: bool,
    /// System font family for fontconfig (`None` = the default sans-serif).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    /// Size of the file list text in whole points (default 14); the rest scales with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f32>,
    /// How hard the text is fitted to the pixel grid (`None` = the default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_hinting: Option<FontHinting>,
    /// How dark the edges of the letters are drawn (`None` = the default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_contrast: Option<FontContrast>,
    /// Language code ("cs", "de"…); `None` = from the system (LC_ALL, LC_MESSAGES, LANG).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Dates as 2026-11-08 21:08 in every language (otherwise as the language writes them).
    #[serde(default, skip_serializing_if = "is_false")]
    pub date_iso: bool,
    /// Viewer (F3): long lines are not wrapped.
    #[serde(default, skip_serializing_if = "is_false")]
    pub viewer_nowrap: bool,
    /// Viewer: no line numbers.
    #[serde(default, skip_serializing_if = "is_false")]
    pub viewer_numbers_off: bool,
    /// Viewer: text size in points (`None` = 13).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewer_font: Option<f32>,
    /// Viewer: no syntax highlighting.
    #[serde(default, skip_serializing_if = "is_false")]
    pub viewer_highlight_off: bool,
    /// Viewer: largest file highlighted, in MB (`None` = 20).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewer_highlight_max_mb: Option<u64>,
    /// Viewer: largest image downloaded from a server, in MB (`None` = 50).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewer_image_max_mb: Option<u64>,
    /// Automatic refresh of server folders every N seconds while the window is
    /// active (0 = off; `None` = 30).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_refresh_secs: Option<u32>,
    /// Column widths of the listing, by panel and view style ("left/full").
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub columns: std::collections::BTreeMap<String, ColumnWidths>,
}

/// Widths of the listing columns set by dragging, in points at text size 1:1
/// (`None` = the default). The Name column takes the rest of the panel.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ColumnWidths {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ext: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<f32>,
}

impl ColumnWidths {
    pub fn is_default(&self) -> bool {
        *self == ColumnWidths::default()
    }
}

/// The key of a panel's column widths: `left/full`, `right/full`…
pub fn columns_key(right: bool, style: &str) -> String {
    format!("{}/{style}", if right { "right" } else { "left" })
}

/// Fitting the letters to the pixel grid (hinting).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FontHinting {
    /// The sharpest stems: the auto-hinter puts them and their widths on
    /// whole pixels (the font's own instructions are not used).
    #[default]
    Strong,
    /// The font's own hinting instructions (vertical fitting only).
    Normal,
    /// No hinting: the shapes as drawn, the softest letters.
    Off,
}

impl FontHinting {
    pub const ALL: [FontHinting; 3] = [FontHinting::Strong, FontHinting::Normal, FontHinting::Off];

    pub fn label(self) -> &'static str {
        match self {
            FontHinting::Strong => rolid_text::nc!("hinting", "Strong"),
            FontHinting::Normal => rolid_text::nc!("hinting", "Medium"),
            FontHinting::Off => rolid_text::nc!("hinting", "Off"),
        }
    }
}

/// How the edge pixels of the letters are blended (darker = bolder look).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FontContrast {
    /// As the font rasterizer gives them (in the dark theme slightly
    /// brightened, like egui does).
    Normal,
    /// Edges a bit darker: the default, it makes up for the thinner stems of
    /// the strong hinting.
    #[default]
    Higher,
    /// Edges much darker: the letters look bolder.
    Highest,
}

impl FontContrast {
    pub const ALL: [FontContrast; 3] = [FontContrast::Normal, FontContrast::Higher, FontContrast::Highest];

    pub fn label(self) -> &'static str {
        match self {
            FontContrast::Normal => rolid_text::nc!("contrast", "Normal"),
            FontContrast::Higher => rolid_text::nc!("contrast", "Higher"),
            FontContrast::Highest => rolid_text::nc!("contrast", "Highest"),
        }
    }
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Which answer a conflict dialog has preselected (Enter confirms it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConflictPreset {
    /// Skip files that already exist (Enter never overwrites).
    Skip,
    /// Overwrite only older files.
    Newer,
    Overwrite,
    /// Nothing preselected: the user has to choose (Enter does nothing until then).
    Ask,
}

impl ConflictPreset {
    pub const ALL: [ConflictPreset; 4] =
        [ConflictPreset::Skip, ConflictPreset::Newer, ConflictPreset::Overwrite, ConflictPreset::Ask];

    pub fn label(self) -> &'static str {
        match self {
            ConflictPreset::Skip => rolid_text::n!("Skip existing"),
            ConflictPreset::Newer => rolid_text::n!("Overwrite only older"),
            ConflictPreset::Overwrite => rolid_text::n!("Overwrite"),
            ConflictPreset::Ask => rolid_text::n!("No preset (choose manually)"),
        }
    }
}

impl Settings {
    /// Drag and drop: skip existing files unless set otherwise.
    /// Seconds after the last use when the vault locks (`None` = only when
    /// the application closes). A setting from before 0.8.8 (minutes) stays.
    pub fn vault_lock_after(&self) -> Option<u32> {
        match (self.vault_lock_secs, self.vault_lock_minutes) {
            (Some(0), _) => None,
            (Some(s), _) => Some(s),
            (None, Some(m)) => Some(m.saturating_mul(60)),
            (None, None) => Some(VAULT_LOCK_DEFAULT_SECS),
        }
    }

    pub fn set_vault_lock_after(&mut self, secs: Option<u32>) {
        self.vault_lock_secs = Some(secs.unwrap_or(0));
        self.vault_lock_minutes = None;
    }

    pub fn drop_preset(&self) -> ConflictPreset {
        self.drop_conflict.unwrap_or(ConflictPreset::Skip)
    }

    /// Extraction limit on bytes written (before the free-space cap).
    pub fn unpack_max_bytes(&self) -> u64 {
        self.unpack_max_gb.unwrap_or(20).max(1).saturating_mul(1_000_000_000)
    }

    pub fn unpack_items(&self) -> u64 {
        self.unpack_max_items.unwrap_or(1_000_000).max(1)
    }

    pub fn unpack_ratio(&self) -> u64 {
        self.unpack_max_ratio.unwrap_or(1000).max(2)
    }

    /// Compression threads by default: all cores but one.
    pub fn default_pack_threads() -> u64 {
        std::thread::available_parallelism().map_or(1, |n| n.get() as u64 - 1).max(1)
    }

    /// Compression threads (setting or the default).
    pub fn pack_threads(&self) -> u64 {
        self.pack_threads.filter(|&n| n > 0).unwrap_or_else(Settings::default_pack_threads)
    }

    pub fn remote_archive_max(&self) -> u64 {
        self.remote_archive_max_mb.unwrap_or(500).saturating_mul(1_000_000)
    }

    /// Viewer: largest file highlighted, in bytes.
    pub fn viewer_highlight_max(&self) -> u64 {
        self.viewer_highlight_max_mb.unwrap_or(20).saturating_mul(1 << 20)
    }

    /// Viewer: largest image downloaded from a server, in bytes.
    pub fn viewer_image_max(&self) -> u64 {
        self.viewer_image_max_mb.unwrap_or(50).saturating_mul(1 << 20)
    }

    /// Viewer: text size in points.
    pub fn viewer_font_size(&self) -> f32 {
        self.viewer_font.filter(|v| v.is_finite()).unwrap_or(13.0).clamp(8.0, 32.0)
    }

    pub fn thumb_px(&self) -> u32 {
        self.thumb_size.unwrap_or_default().px()
    }

    pub fn thumbs_on_servers(&self) -> bool {
        self.thumbs_remote.unwrap_or(true)
    }

    /// Largest remote file for a thumbnail, in bytes.
    pub fn thumbs_remote_max(&self) -> u64 {
        self.thumbs_remote_max_kib.unwrap_or(2048) * 1024
    }

    /// Remember a mask (newest first, no duplicates, at most 10).
    pub fn remember_mask(&mut self, mask: &str) {
        let m = mask.trim().to_string();
        if m.is_empty() {
            return;
        }
        self.mark_masks.retain(|x| *x != m);
        self.mark_masks.insert(0, m);
        self.mark_masks.truncate(10);
    }

    /// F5: overwrite, as before, unless set otherwise.
    pub fn copy_preset(&self) -> ConflictPreset {
        self.copy_conflict.unwrap_or(ConflictPreset::Overwrite)
    }
}

/// A remembered application for "Open with".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpenWith {
    /// A desktop entry ID, e.g. `org.gnome.gedit.desktop`.
    Desktop(String),
    /// A command with `{file}`.
    Command(String),
}

/// Today as days since 1970 (UTC), for "at most once a day" reminders.
pub fn today() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0)
}

pub fn config_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("ROLIDSCP_CONFIG_DIR") {
        return PathBuf::from(d);
    }
    dirs::config_dir()
        .unwrap_or_else(|| std::env::home_dir().unwrap_or_default().join(".config"))
        .join("rolidscp")
}

/// Program state on disk (`~/.local/state/rolidscp` on Linux): the log, the
/// journal of unfinished replacements on servers. `ROLIDSCP_STATE_DIR` overrides it.
pub fn state_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("ROLIDSCP_STATE_DIR") {
        return PathBuf::from(d);
    }
    dirs::state_dir().or_else(dirs::cache_dir).unwrap_or_else(std::env::temp_dir).join("rolidscp")
}

/// Log file (`~/.local/state/rolidscp/rolidscp.log` on Linux). `ROLIDSCP_LOG_DIR` overrides the directory.
pub fn log_path() -> PathBuf {
    if let Some(d) = std::env::var_os("ROLIDSCP_LOG_DIR") {
        return PathBuf::from(d).join("rolidscp.log");
    }
    dirs::state_dir()
        .or_else(dirs::cache_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("rolidscp")
        .join("rolidscp.log")
}

fn load<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> Result<T, String> {
    match std::fs::read_to_string(path) {
        Ok(s) => toml::from_str(&s).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Write atomically (temp file in the same directory + rename), readable only by the owner.
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<(), String> {
    let dir = path.parent().ok_or("bad path")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut tmp_name = path.file_name().ok_or("bad path")?.to_os_string();
    tmp_name.push(".tmp");
    let tmp = path.with_file_name(tmp_name);
    // A leftover temp file could have other permissions: start fresh.
    let _ = std::fs::remove_file(&tmp);
    let r = (|| {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
        f.write_all(data).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
        drop(f);
        std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))?;
        // Make the rename durable.
        #[cfg(unix)]
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
        Ok(())
    })();
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}

fn save<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let text = toml::to_string_pretty(value).map_err(|e| e.to_string())?;
    write_atomic(path, text.as_bytes())
}

/// Load any TOML file from the config dir (missing file = default).
pub fn load_file<T: for<'de> Deserialize<'de> + Default>(name: &str) -> Result<T, String> {
    load(&config_dir().join(name))
}

/// Save any TOML file into the config dir (atomic, 0600).
pub fn save_file<T: Serialize>(name: &str, value: &T) -> Result<(), String> {
    save(&config_dir().join(name), value)
}

pub fn load_profiles() -> Result<Profiles, String> {
    load(&config_dir().join("profiles.toml"))
}

pub fn save_profiles(p: &Profiles) -> Result<(), String> {
    save(&config_dir().join("profiles.toml"), p)
}

pub fn load_workspace() -> Result<Workspace, String> {
    let mut w: Workspace = load(&config_dir().join("workspace.toml"))?;
    w.migrate();
    Ok(w)
}

pub fn save_workspace(w: &Workspace) -> Result<(), String> {
    save(&config_dir().join("workspace.toml"), w)
}

/// Serialized form, used to detect changes cheaply.
pub fn workspace_fingerprint(w: &Workspace) -> String {
    toml::to_string(w).unwrap_or_default()
}

pub fn new_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros())
        .unwrap_or(0);
    format!("{t:x}{:02x}", N.fetch_add(1, Ordering::Relaxed) & 0xff)
}

/// System keyring access. All calls block (D-Bus etc.): call them off the UI thread.
pub mod secrets {
    const SERVICE: &str = "rolidscp";

    fn entry(profile_id: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(SERVICE, &format!("profile:{profile_id}")).map_err(|e| e.to_string())
    }

    pub fn get(profile_id: &str) -> Result<Option<String>, String> {
        match entry(profile_id)?.get_password() {
            Ok(p) => Ok(Some(p)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn set(profile_id: &str, password: &str) -> Result<(), String> {
        entry(profile_id)?
            .set_password(password)
            .map_err(|e| e.to_string())
    }

    pub fn delete(profile_id: &str) -> Result<(), String> {
        match entry(profile_id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn conflict_presets_default_and_round_trip() {
        let s = Settings::default();
        assert_eq!(s.drop_preset(), ConflictPreset::Skip, "dragging never overwrites by default");
        assert_eq!(s.copy_preset(), ConflictPreset::Overwrite, "F5 stays as it was");
        let mut s = Settings {
            drop_conflict: Some(ConflictPreset::Ask),
            copy_conflict: Some(ConflictPreset::Newer),
            ..Default::default()
        };
        s.open_with.insert("log".into(), OpenWith::Desktop("org.gnome.TextEditor.desktop".into()));
        let text = toml::to_string_pretty(&s).unwrap();
        assert!(text.contains("drop_conflict = \"ask\""), "{text}");
        assert!(text.contains("copy_conflict = \"newer\""), "{text}");
        let back: Settings = toml::from_str(&text).unwrap();
        assert_eq!(back.drop_preset(), ConflictPreset::Ask);
        assert_eq!(back.copy_preset(), ConflictPreset::Newer);
        assert_eq!(back.open_with.get("log"), Some(&OpenWith::Desktop("org.gnome.TextEditor.desktop".into())));
        // An old settings.toml without the keys still loads.
        let old: Settings = toml::from_str("speed_limit_kib = 4096\n").unwrap();
        assert_eq!(old.drop_preset(), ConflictPreset::Skip);
    }

    #[test]
    fn hidden_nav_bar_is_remembered() {
        let s = Settings::default();
        assert!(!s.hide_nav_bar, "the bar is shown by default");
        assert!(!toml::to_string_pretty(&s).unwrap().contains("hide_nav_bar"), "nothing written while shown");
        let s = Settings { hide_nav_bar: true, ..Default::default() };
        let text = toml::to_string_pretty(&s).unwrap();
        assert!(text.contains("hide_nav_bar = true"), "{text}");
        let back: Settings = toml::from_str(&text).unwrap();
        assert!(back.hide_nav_bar);
        // Settings from before 0.8.12: the bar is shown.
        let old: Settings = toml::from_str("hide_toolbar = true\n").unwrap();
        assert!(!old.hide_nav_bar && old.hide_toolbar);
    }

    #[test]
    fn workspace_migration_from_shared_tabs() {
        let old = r##"
active_tab = 0
[[tab]]
name = "web"
color = "#FFD1DC"
active = 1
[[tab.panes]]
kind = "local"
path = "/home/a"
[[tab.panes]]
kind = "local"
path = "/srv"
"##;
        let mut w: super::Workspace = toml::from_str(old).unwrap();
        w.migrate();
        assert_eq!(w.left.tabs.len(), 1);
        assert_eq!(w.right.tabs.len(), 1);
        assert_eq!(w.right.tabs[0].location, super::Location::Local { path: "/srv".into() });
        assert_eq!(w.left.tabs[0].color, "#FFD1DC");
        // The new format round-trips and does not write the legacy key.
        let text = toml::to_string(&w).unwrap();
        assert!(!text.contains("panes"), "{text}");
        let back: super::Workspace = toml::from_str(&text).unwrap();
        assert_eq!(back.left, w.left);
    }

    use super::*;

    #[test]
    fn old_profiles_mean_keyring_new_ones_the_vault() {
        let old = "[[profile]]\nid = \"a\"\nname = \"A\"\nhost = \"h\"\nport = 22\nuser = \"u\"\nsave_password = true\n";
        let p: Profiles = toml::from_str(old).unwrap();
        assert!(p.list[0].password_in_keyring());
        assert!(!p.list[0].password_in_vault());
        let new = Profile { save_password: true, ..Default::default() };
        assert!(new.password_in_vault());
        let text = toml::to_string(&Profiles { list: vec![new] }).unwrap();
        assert!(text.contains("password_store = \"vault\""), "{text}");
        let back: Profiles = toml::from_str(&text).unwrap();
        assert!(back.list[0].password_in_vault());
    }

    fn remote(profile: Option<&str>, host: &str, path: &str) -> Location {
        Location::Remote {
            target: RemoteTarget { profile: profile.map(Into::into), host: host.into(), port: 22, user: "u".into(), key_path: None },
            path: path.into(),
        }
    }

    #[test]
    fn favorites_add_remove_order_and_save() {
        let mut ws = Places::default();
        let a = Location::Local { path: "/home/u/a".into() };
        assert!(ws.toggle_favorite(a.clone()));
        assert!(ws.is_favorite(&Location::Local { path: "/home/u/a/".into() }), "a trailing slash is the same folder");
        // A remote folder is bound to its profile, not to the host name.
        assert!(ws.toggle_favorite(remote(Some("p1"), "old.example", "/srv")));
        assert!(ws.is_favorite(&remote(Some("p1"), "new.example", "/srv")));
        assert!(!ws.is_favorite(&remote(Some("p2"), "old.example", "/srv")));
        assert!(!ws.is_favorite(&Location::Local { path: "/srv".into() }));
        assert!(ws.toggle_favorite(Location::Local { path: "/c".into() }));
        assert_eq!(ws.move_favorite(2, -1), 1);
        assert_eq!(ws.move_favorite(0, -1), 0, "the first one stays");
        assert_eq!(ws.move_favorite(2, 1), 2, "the last one stays");
        let paths: Vec<&str> = ws.favorites.iter().map(|f| f.location.path()).collect();
        assert_eq!(paths, ["/home/u/a", "/c", "/srv"]);
        ws.favorites[1].name = Some("Drive C".into());
        let w = Workspace { favorites: ws.favorites.clone(), ..Default::default() };
        let text = toml::to_string(&w).unwrap();
        let back: Workspace = toml::from_str(&text).unwrap();
        assert_eq!(back.favorites, ws.favorites);
        assert!(!ws.toggle_favorite(a.clone()), "Ctrl+D again removes it");
        assert_eq!(ws.favorites.len(), 2);
        assert!(!ws.is_favorite(&a));
        // "Add the current folder" never removes it.
        assert!(ws.add_favorite(a.clone()));
        assert!(!ws.add_favorite(Location::Local { path: "/home/u/a/".into() }), "already there");
        assert_eq!(ws.favorites.len(), 3);
        ws.rename_favorite(2, "  Projects  ");
        assert_eq!(ws.favorites[2].name.as_deref(), Some("Projects"));
        ws.rename_favorite(2, " ");
        assert_eq!(ws.favorites[2].name, None, "an empty name shows the path");
        assert!(ws.remove_favorite(2).is_some_and(|f| f.location == a));
        assert!(ws.remove_favorite(9).is_none());
        assert!(!ws.is_favorite(&a));
    }

    #[test]
    fn vault_lock_keeps_the_old_setting_and_has_a_new_default() {
        let s = Settings::default();
        assert_eq!(s.vault_lock_after(), Some(60), "the default is 60 s");
        let old: Settings = toml::from_str("vault_lock_minutes = 15").unwrap();
        assert_eq!(old.vault_lock_after(), Some(15 * 60), "a setting from 0.8.7 stays");
        let mut s = old.clone();
        s.set_vault_lock_after(Some(30));
        assert_eq!(s.vault_lock_after(), Some(30));
        s.set_vault_lock_after(None);
        assert_eq!(s.vault_lock_after(), None, "never");
        let text = toml::to_string(&s).unwrap();
        assert!(!text.contains("vault_lock_minutes"), "{text}");
        let back: Settings = toml::from_str(&text).unwrap();
        assert_eq!(back.vault_lock_after(), None, "never is saved, not mistaken for the default");
    }

    #[test]
    fn copy_targets_are_kept_per_disk() {
        let mut p = Places::default();
        let srv = |path: &str| remote(Some("p"), "h", path);
        p.remember_target(Location::Local { path: "/a".into() });
        p.remember_target(srv("/srv"));
        p.remember_target(Location::Local { path: "/b".into() });
        p.remember_target(Location::Local { path: "/a/".into() });
        assert_eq!(p.targets_on(&Location::Local { path: "/x".into() }), vec!["/a/", "/b"]);
        assert_eq!(p.targets_on(&srv("/")), vec!["/srv"]);
    }

    #[test]
    fn recent_places_newest_first_without_duplicates() {
        let mut ws = Places::default();
        for i in 0..20 {
            ws.remember(Location::Local { path: format!("/d{i}") });
        }
        ws.remember(Location::Local { path: "/d17/".into() });
        ws.remember(remote(Some("p1"), "h", "/srv"));
        assert_eq!(ws.recent.len(), RECENT_MAX);
        assert_eq!(ws.recent[0].path(), "/srv");
        assert_eq!(ws.recent[1].path(), "/d17/");
        assert_eq!(ws.recent.iter().filter(|l| l.path().starts_with("/d17")).count(), 1);
        assert_eq!(ws.recent[2].path(), "/d19");
    }

    #[test]
    fn column_widths_are_saved_per_panel_and_style() {
        let mut s = Settings::default();
        assert!(!toml::to_string(&s).unwrap().contains("columns"), "nothing saved by default");
        s.columns.insert(columns_key(false, "full"), ColumnWidths { ext: Some(80.0), ..Default::default() });
        s.columns.insert(columns_key(true, "full"), ColumnWidths { size: Some(120.5), mode: Some(40.0), ..Default::default() });
        let text = toml::to_string_pretty(&s).unwrap();
        let back: Settings = toml::from_str(&text).unwrap();
        assert_eq!(back.columns, s.columns);
        assert_eq!(back.columns["left/full"].ext, Some(80.0));
        assert_eq!(back.columns["left/full"].size, None);
        assert_eq!(back.columns["right/full"].size, Some(120.5));
        assert!(ColumnWidths::default().is_default());
        // An older file without the key loads with the defaults.
        let old: Settings = toml::from_str("").unwrap();
        assert!(old.columns.is_empty());
    }

    #[test]
    fn roundtrip_files() {
        let dir = std::env::temp_dir().join(format!("rolid-cfg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut profiles = Profiles::default();
        profiles.upsert(Profile {
            id: new_id(),
            name: "Production".into(),
            host: "example.org".into(),
            port: 22,
            user: "deploy".into(),
            color: Some("#FFD1DC".into()),
            production: true,
            ..Default::default()
        });
        let ws = Workspace {
            active_side: 1,
            left: SideState {
                active: 0,
                tabs: vec![PanelTab {
                    name: None,
                    color: "#C1F0D6".into(),
                    location: Location::Local {
                        path: "/home/u".into(),
                    },
                    view: Some(ViewStyle::Brief),
                }],
            },
            right: SideState {
                active: 0,
                tabs: vec![PanelTab {
                    name: Some("Web".into()),
                    color: "#FFD1DC".into(),
                    location: Location::Remote {
                        target: RemoteTarget {
                            profile: Some(profiles.list[0].id.clone()),
                            host: "example.org".into(),
                            port: 22,
                            user: "deploy".into(),
                            key_path: None,
                        },
                        path: "/var/www".into(),
                    },
                    view: None,
                }],
            },
            favorites: vec![Favorite { name: Some("Projects".into()), location: Location::Local { path: "/home/u/p".into() } }],
            recent: vec![Location::Local { path: "/tmp".into() }],
            targets: vec![Location::Local { path: "/srv/www".into() }],
            legacy_tabs: vec![],
        };
        let pp = dir.join("profiles.toml");
        let wp = dir.join("workspace.toml");
        save(&pp, &profiles).unwrap();
        save(&wp, &ws).unwrap();
        assert_eq!(load::<Profiles>(&pp).unwrap(), profiles);
        assert_eq!(load::<Workspace>(&wp).unwrap(), ws);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&pp).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        // Missing file -> defaults
        assert_eq!(
            load::<Workspace>(&dir.join("nope.toml")).unwrap(),
            Workspace::default()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
