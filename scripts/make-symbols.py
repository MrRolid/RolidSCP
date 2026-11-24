#!/usr/bin/env python3
"""Build the program's symbol font from its icon font.

The window's texts use a few symbols (a warning sign, a lock, a pencil, the
house of the home folder...) that neither the text font (Noto Sans) nor the
monospace font (Hack) has. Without this font they come from the system's
fonts and are missing on a system without any. This script takes the
matching icons of Phosphor (crates/rolid-ui/assets/phosphor/Phosphor.ttf,
MIT) and puts them under the code points of those symbols, in a small font
that the program uses after its text fonts.

    python3 -m pip install fonttools==4.60.1
    python3 scripts/make-symbols.py

writes crates/rolid-ui/assets/phosphor/RolidSymbols.ttf. The output only
depends on the input font and this file. A test in crates/rolid-ui
(fonts::tests::every_symbol_of_the_texts_has_a_glyph) checks that every
character of the program's texts has a glyph in the built-in fonts.
"""

import os
import sys

from fontTools import subset
from fontTools.ttLib import TTFont

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC = os.path.join(ROOT, "crates/rolid-ui/assets/phosphor/Phosphor.ttf")
DST = os.path.join(ROOT, "crates/rolid-ui/assets/phosphor/RolidSymbols.ttf")

# Symbol -> Phosphor 2.1 icon (name: code point in Phosphor.ttf).
ICONS = {
    "arrow-bend-up-left": 0xE024,
    "arrow-clockwise": 0xE036,
    "caret-double-up": 0xE12C,
    "caret-down": 0xE136,
    "caret-up": 0xE13C,
    "check": 0xE182,
    "clipboard": 0xE196,
    "hard-drive": 0xE29E,
    "heart": 0xE2A8,
    "hourglass-medium": 0xE2B8,
    "house": 0xE2C2,
    "info": 0xE2CE,
    "key": 0xE2D6,
    "lightning": 0xE2DE,
    "lock": 0xE2FA,
    "lock-open": 0xE306,
    "magnifying-glass": 0xE30C,
    "package": 0xE390,
    "pencil": 0xE3AE,
    "pencil-simple": 0xE3B4,
    "plus": 0xE3D4,
    "star": 0xE46A,
    "trash": 0xE4A6,
    "warning": 0xE4E0,
    "x": 0xE4F6,
    "network": 0xEDDE,
}
SYMBOLS = {
    0x2302: "house",  # ⌂ HOUSE
    0x23EB: "caret-double-up",  # ⏫
    0x23F3: "hourglass-medium",  # ⏳
    0x23F6: "caret-up",  # ⏶
    0x23F7: "caret-down",  # ⏷
    0x24D8: "info",  # ⓘ
    0x2139: "info",  # ℹ
    0x2605: "star",  # ★
    0x26A0: "warning",  # ⚠
    0x26A1: "lightning",  # ⚡
    0x270E: "pencil-simple",  # ✎
    0x270F: "pencil",  # ✏
    0x2714: "check",  # ✔
    0x2715: "x",  # ✕
    0x2716: "x",  # ✖
    0x2764: "heart",  # ❤
    0x27F3: "arrow-clockwise",  # ⟳
    0x2BAA: "arrow-bend-up-left",  # ⮪
    0xFF0B: "plus",  # ＋ (the "new tab" button)
    0x1F4CB: "clipboard",  # 📋
    0x1F4E6: "package",  # 📦
    0x1F50D: "magnifying-glass",  # 🔍
    0x1F50E: "magnifying-glass",  # 🔎
    0x1F511: "key",  # 🔑
    0x1F512: "lock",  # 🔒
    0x1F513: "lock-open",  # 🔓
    0x1F5A7: "network",  # 🖧
    0x1F5B4: "hard-drive",  # 🖴
    0x1F5D1: "trash",  # 🗑
}

# A text line is taller than an icon's square: the icons are made a bit
# smaller than the letters' height and stand on the baseline like a capital.
SCALE = 0.82


def main():
    font = TTFont(SRC)
    cmap = font.getBestCmap()
    for name, cp in ICONS.items():
        if cp not in cmap:
            sys.exit(f"{name}: U+{cp:04X} is not in {SRC}")
    # Keep only the icons needed, then give them the symbols' code points.
    opts = subset.Options()
    opts.layout_features = []
    opts.name_IDs = [0, 1, 2, 3, 4, 5, 6]
    opts.notdef_outline = True
    opts.recalc_bounds = True
    opts.drop_tables += ["GSUB", "GPOS", "GDEF", "FFTM"]
    sub = subset.Subsetter(opts)
    sub.populate(unicodes=sorted(set(ICONS.values())))
    sub.subset(font)
    glyph = {name: font.getBestCmap()[cp] for name, cp in ICONS.items()}
    # Two subtables: format 4 for the Basic Multilingual Plane, format 12
    # for all (the emoji are above U+FFFF).
    from fontTools.ttLib.tables._c_m_a_p import CmapSubtable

    full = {cp: glyph[name] for cp, name in SYMBOLS.items()}
    tables = []
    for fmt, enc, cps in ((4, 1, {c: g for c, g in full.items() if c <= 0xFFFF}), (12, 10, full)):
        t = CmapSubtable.newSubtable(fmt)
        t.platformID, t.platEncID, t.language, t.cmap = 3, enc, 0, cps
        tables.append(t)
    font["cmap"].tables = tables
    scale(font, SCALE)
    # The line metrics of the text font (Noto Sans: 1.069 and 0.293 of the
    # em), so the symbols share its baseline and line height.
    em = font["head"].unitsPerEm
    asc, desc = round(1.069 * em), round(0.293 * em)
    font["hhea"].ascent, font["hhea"].descent, font["hhea"].lineGap = asc, -desc, 0
    os2 = font["OS/2"]
    os2.sTypoAscender, os2.sTypoDescender, os2.sTypoLineGap = asc, -desc, 0
    os2.usWinAscent, os2.usWinDescent = asc, desc
    for rec in font["name"].names:
        if rec.nameID in (1, 4, 6, 16):
            rec.string = "RolidSymbols"
        elif rec.nameID == 3:
            rec.string = "RolidSymbols from Phosphor Icons 2.1"
    # Reproducible: no build time in the file.
    font["head"].created = font["head"].modified = 0
    font.save(DST, reorderTables=True)
    print(f"{DST}: {len(SYMBOLS)} symbols from {len(ICONS)} icons, {os.path.getsize(DST)} bytes")


def scale(font, f):
    """Scale the outlines and advance widths by f around the origin."""
    from fontTools.pens.transformPen import TransformPen
    from fontTools.pens.ttGlyphPen import TTGlyphPen

    gs = font.getGlyphSet()
    glyf = font["glyf"]
    hmtx = font["hmtx"]
    for name in font.getGlyphOrder():
        pen = TTGlyphPen(gs)
        gs[name].draw(TransformPen(pen, (f, 0, 0, f, 0, 0)))
        glyf[name] = pen.glyph()
        adv, _ = hmtx[name]
        g = glyf[name]
        g.recalcBounds(glyf)
        hmtx[name] = (round(adv * f), getattr(g, "xMin", 0))


if __name__ == "__main__":
    main()
