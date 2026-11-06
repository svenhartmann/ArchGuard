//! What differs per operating system in the look: the platform's type
//! scale and control metrics (`Theme.platform`, values in `ui/theme.slint`),
//! the UI and monospace fonts and, on macOS, the renderer that can draw
//! them.

use slint::ComponentHandle;

use crate::ui::{AppWindow, Theme};

/// The desktop platforms with their own look, as `Theme.platform` numbers
/// them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    MacOs = 0,
    Windows = 1,
    Linux = 2,
}

impl Platform {
    /// The platform this binary was built for. Snapshot builds take
    /// `ARCHGUARD_DESKTOP_PLATFORM=macos|windows|linux` to preview another
    /// platform's scale (with that platform's fonts only where installed).
    pub fn current() -> Self {
        #[cfg(feature = "snapshot")]
        if let Some(p) = std::env::var("ARCHGUARD_DESKTOP_PLATFORM")
            .ok()
            .and_then(|v| Self::named(&v))
        {
            return p;
        }
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Linux
        }
    }

    #[cfg(any(test, feature = "snapshot"))]
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "macos" => Some(Self::MacOs),
            "windows" => Some(Self::Windows),
            "linux" => Some(Self::Linux),
            _ => None,
        }
    }

    /// The family of the platform's UI font, as the font collection names
    /// it; empty keeps Slint's default (fontconfig's sans-serif on Linux).
    fn ui_font(self) -> &'static str {
        match self {
            // San Francisco: /System/Library/Fonts/SFNS.ttf.
            Self::MacOs => "System Font",
            Self::Windows => "Segoe UI",
            Self::Linux => "",
        }
    }

    /// The monospace family. macOS: SF Mono
    /// (/System/Library/Fonts/SFNSMono.ttf, a hidden family). Windows:
    /// Consolas, shipped with every Windows since Vista (Cascadia Mono only
    /// with newer Windows 11 builds). Linux: fontconfig's alias.
    fn mono_font(self) -> &'static str {
        match self {
            Self::MacOs => ".SF NS Mono",
            Self::Windows => "Consolas",
            Self::Linux => "monospace",
        }
    }
}

/// The UI font family; `ARCHGUARD_DESKTOP_FONT` overrides it
/// (`ARCHGUARD_DESKTOP_MONO_FONT` the monospace one).
pub fn font_family() -> String {
    std::env::var("ARCHGUARD_DESKTOP_FONT").unwrap_or_else(|_| Platform::current().ui_font().into())
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
    let platform = Platform::current();
    let theme = ui.global::<Theme>();
    theme.set_platform(platform as i32);
    theme.set_font_family(font_family().into());
    let mono = std::env::var("ARCHGUARD_DESKTOP_MONO_FONT")
        .unwrap_or_else(|_| platform.mono_font().into());
    theme.set_mono_family(mono.into());
}

#[cfg(test)]
mod tests {
    use super::Platform;

    #[test]
    fn each_desktop_platform_names_its_ui_font() {
        let family = super::font_family();
        if cfg!(any(target_os = "macos", target_os = "windows")) {
            assert!(!family.is_empty());
        }
    }

    #[test]
    fn every_platform_names_a_monospace_font() {
        for p in [Platform::MacOs, Platform::Windows, Platform::Linux] {
            assert!(!p.mono_font().is_empty(), "{p:?}");
            assert_eq!(Platform::named(&format!("{p:?}").to_lowercase()), Some(p));
        }
    }
}
