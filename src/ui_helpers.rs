use eframe::egui::{Context, FontData, FontDefinitions, FontFamily};
use std::fs;

pub fn install_thai_font(ctx: &Context) {
    let Some(bytes) = thai_font_bytes() else {
        return;
    };

    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert("thai_ui".to_owned(), FontData::from_owned(bytes).into());

    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "thai_ui".to_owned());
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .push("thai_ui".to_owned());

    ctx.set_fonts(fonts);
}

fn thai_font_bytes() -> Option<Vec<u8>> {
    let candidates = [
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        "/System/Library/Fonts/Supplemental/NotoSansThai-Regular.ttf",
        "/System/Library/Fonts/Supplemental/Noto Sans Thai.ttf",
        "/Library/Fonts/NotoSansThai-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoSansThai-Regular.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "C:\\Windows\\Fonts\\tahoma.ttf",
        "C:\\Windows\\Fonts\\segoeui.ttf",
    ];

    candidates
        .iter()
        .find_map(|path| fs::read(path).ok().filter(|bytes| !bytes.is_empty()))
}
