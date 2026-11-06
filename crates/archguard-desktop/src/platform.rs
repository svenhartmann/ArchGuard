//! What differs per operating system in the look: the UI font and, on
//! macOS, the renderer that can draw it.

use slint::ComponentHandle;

use crate::ui::{AppWindow, Theme};

/// The family of the platform's UI font, as the font collection names it;
/// empty keeps Slint's default (fontconfig's sans-serif on Linux).
/// `ARCHGUARD_DESKTOP_FONT` overrides it.
pub fn font_family() -> String {
    if let Ok(name) = std::env::var("ARCHGUARD_DESKTOP_FONT") {
        return name;
    }
    if cfg!(target_os = "macos") {
        // San Francisco: /System/Library/Fonts/SFNS.ttf.
        "System Font".into()
    } else if cfg!(target_os = "windows") {
        "Segoe UI".into()
    } else {
        String::new()
    }
}

/// On macOS the window renders with Skia, so text uses the system font
/// with its weights. `SLINT_BACKEND` set by the user wins.
pub fn select_renderer() -> Result<(), slint::PlatformError> {
    if cfg!(target_os = "macos") && std::env::var_os("SLINT_BACKEND").is_none() {
        return slint::BackendSelector::new()
            .renderer_name("skia".into())
            .select();
    }
    Ok(())
}

pub fn apply(ui: &AppWindow) {
    ui.global::<Theme>().set_font_family(font_family().into());
}

#[cfg(test)]
mod tests {
    #[test]
    fn each_desktop_platform_names_its_ui_font() {
        let family = super::font_family();
        if cfg!(any(target_os = "macos", target_os = "windows")) {
            assert!(!family.is_empty());
        }
    }
}
