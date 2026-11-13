//! Text fields for secrets (server password, key passphrase, master password,
//! keyboard-interactive answers).
//!
//! - The buffer is a `Zeroizing<String>`: overwritten with zeros when dropped or wiped.
//! - Capacity is reserved up front and input is limited, so typing never reallocates
//!   (a reallocation would leave an unzeroed copy in freed memory).
//! - egui keeps an undo history with copies of the text; it is cleared after every
//!   frame. Password mode also renders only bullets and blocks copying.

use eframe::egui;
use zeroize::Zeroizing;

/// Reserved bytes per field (4 bytes per character at most).
const CAP_BYTES: usize = 1024;
/// Maximum characters in a secret field.
pub const MAX_CHARS: usize = CAP_BYTES / 4;

/// Make sure the buffer will not reallocate while typing. Only grows an empty buffer
/// (growing a filled one would copy the secret).
pub fn prepare(buf: &mut Zeroizing<String>) {
    if buf.is_empty() && buf.capacity() < CAP_BYTES {
        **buf = String::with_capacity(CAP_BYTES);
    }
}

/// Single-line secret field; `masked = false` shows the text (echo prompts).
pub fn edit(
    ui: &mut egui::Ui,
    buf: &mut Zeroizing<String>,
    masked: bool,
    hint: &str,
    width: f32,
) -> egui::Response {
    prepare(buf);
    let r = ui.add(
        egui::TextEdit::singleline(&mut **buf)
            .password(masked)
            .char_limit(MAX_CHARS)
            .hint_text(hint)
            .desired_width(width),
    );
    forget_history(ui.ctx(), r.id);
    r
}

/// Drop egui's undo history (copies of the text) for one field.
pub fn forget_history(ctx: &egui::Context, id: egui::Id) {
    if let Some(mut st) = egui::text_edit::TextEditState::load(ctx, id) {
        st.set_undoer(Default::default());
        st.store(ctx, id);
    }
}

/// Overwrite with zeros, then empty (keeps nothing readable in the old buffer).
pub fn wipe(buf: &mut Zeroizing<String>) {
    use zeroize::Zeroize;
    buf.zeroize();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepared_buffer_does_not_reallocate_while_typing() {
        let mut b: Zeroizing<String> = Zeroizing::default();
        prepare(&mut b);
        let ptr = b.as_ptr();
        for _ in 0..MAX_CHARS {
            b.push('😀'); // 4 bytes, the worst case
        }
        assert_eq!(b.as_ptr(), ptr, "no reallocation within the character limit");
        wipe(&mut b);
        assert!(b.is_empty());
    }
}
