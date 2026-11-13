//! Icons: Phosphor Icons 2.1 (regular), MIT license, © 2023 Phosphor Icons
//! (<https://phosphoricons.com>, license in `assets/phosphor/LICENSE`).
//!
//! The font is embedded in the binary and registered as its own font family
//! ([`FAMILY`]) – it also contains plain letters, so it must not become a
//! fallback of the text font. Draw icons with [`icon`] (color by role).
//!
//! The set already covers the toolbar (refresh, terminal,
//! settings, view modes, history, selection, archives, search…).
#![allow(dead_code)] // part of the set is for later toolbar buttons

use eframe::egui::{self, Color32, FontFamily, RichText};

pub const FAMILY: &str = "phosphor";
pub const FONT: &[u8] = include_bytes!("../assets/phosphor/Phosphor.ttf");
pub const SET: &str = "Phosphor Icons 2.1 (MIT, © 2023 Phosphor Icons)";

// Dialog actions.
pub const COPY: &str = "\u{E1CA}";
pub const UPLOAD: &str = "\u{E4C0}";
pub const DOWNLOAD: &str = "\u{E20C}";
pub const MOVE: &str = "\u{E0A4}";
pub const TRASH: &str = "\u{E4A6}";
pub const X: &str = "\u{E4F6}";
pub const X_CIRCLE: &str = "\u{E4F8}";
pub const CHECK: &str = "\u{E182}";
pub const CHECK_CIRCLE: &str = "\u{E184}";
pub const PLUG: &str = "\u{E946}";
pub const PLUGS_CONNECTED: &str = "\u{EB5A}";
pub const WARNING: &str = "\u{E4E0}";
pub const FLOPPY: &str = "\u{E248}";
pub const LOCK: &str = "\u{E2FE}";
pub const LOCK_OPEN: &str = "\u{E300}";
pub const SHIELD_CHECK: &str = "\u{E40C}";
pub const FOLDER_OPEN: &str = "\u{E256}";
pub const FOLDER_PLUS: &str = "\u{E258}";
pub const APP_WINDOW: &str = "\u{E5DA}";
pub const OPEN_EXTERNAL: &str = "\u{E5DE}";
pub const INFO: &str = "\u{E2CE}";
pub const KEY: &str = "\u{E2D6}";
pub const PENCIL: &str = "\u{E3B4}";
pub const PENCIL_LINE: &str = "\u{E3B2}";
pub const SEND: &str = "\u{E076}";
pub const FILE: &str = "\u{E230}";
pub const PLAY: &str = "\u{E3D0}";
pub const CLIPBOARD: &str = "\u{E196}";
pub const CLIPBOARD_TEXT: &str = "\u{E198}";

// Toolbar.
pub const REFRESH: &str = "\u{E094}";
pub const TERMINAL: &str = "\u{EAE8}";
pub const GEAR: &str = "\u{E270}";
pub const VIEW_BRIEF: &str = "\u{E546}";
pub const VIEW_FULL: &str = "\u{E2F2}";
pub const VIEW_THUMBS: &str = "\u{E464}";
pub const BACK: &str = "\u{E058}";
pub const FORWARD: &str = "\u{E06C}";
pub const SELECT_GROUP: &str = "\u{E69C}";
pub const UNSELECT_GROUP: &str = "\u{E69E}";
pub const INVERT_SELECTION: &str = "\u{E744}";
pub const ARCHIVE: &str = "\u{E00C}";
pub const PACKAGE: &str = "\u{E390}";
/// Box with an arrow down (extract).
pub const UNPACK: &str = "\u{E00E}";
pub const FILE_ZIP: &str = "\u{E958}";
pub const SEARCH: &str = "\u{E30C}";
pub const RENAME_MANY: &str = "\u{EB0A}";
pub const SYNC: &str = "\u{E0A0}";
pub const SERVERS: &str = "\u{E67C}";
pub const FOLDER_SIZE: &str = "\u{E158}";
pub const CHECKSUM: &str = "\u{E2A2}";

// File types and path buttons.
pub const FOLDER: &str = "\u{E24A}";
pub const LINK: &str = "\u{E2E2}";
pub const FILE_IMAGE: &str = "\u{EA24}";
pub const FILE_CODE: &str = "\u{E914}";
pub const FILE_TEXT: &str = "\u{E23A}";
pub const FILE_DOC: &str = "\u{EB1E}";
pub const FILE_AUDIO: &str = "\u{EA20}";
pub const FILE_VIDEO: &str = "\u{EA22}";
pub const ARROW_UP: &str = "\u{E08E}";
pub const HOUSE: &str = "\u{E2C2}";
pub const HOUSE_SIMPLE: &str = "\u{E2C6}";
pub const FOLDER_SIMPLE: &str = "\u{E25A}";
pub const STAR: &str = "\u{E46A}";
pub const CARET_DOWN: &str = "\u{E136}";
pub const HARD_DRIVES: &str = "\u{E2A0}";
pub const HEARTBEAT: &str = "\u{E2AC}";
pub const HISTORY: &str = "\u{E1A0}";

/// Add the icon font to the font definitions (see `fonts::apply`).
pub fn add_to(defs: &mut egui::FontDefinitions) {
    defs.font_data.insert(FAMILY.into(), std::sync::Arc::new(egui::FontData::from_static(FONT)));
    defs.families.insert(FontFamily::Name(FAMILY.into()), vec![FAMILY.into()]);
}

/// An icon glyph as text (in the icon font).
/// Font of the icons at `size` (for painting).
pub fn font(size: f32) -> egui::FontId {
    egui::FontId::new(size, FontFamily::Name(FAMILY.into()))
}

pub fn icon(glyph: &str, color: Color32, size: f32) -> RichText {
    RichText::new(glyph).family(FontFamily::Name(FAMILY.into())).color(color).size(size)
}
