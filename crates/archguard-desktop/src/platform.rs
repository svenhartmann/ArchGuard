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

    /// The monospace family. macOS: Menlo (/System/Library/Fonts/Menlo.ttc,
    /// shipped since 10.6, with separate upright and italic faces). Not
    /// SF Mono: its hidden family ".SF NS Mono" also holds
    /// SFNSMonoItalic.ttf, which Slint's font matcher (fontique) reads as
    /// upright and picks for upright text, and Slint cannot register a font
    /// file by path (ADR 0002, 2026-11-08). Windows: Consolas, shipped with
    /// every Windows since Vista (Cascadia Mono only with newer Windows 11
    /// builds). Linux: fontconfig's alias.
    fn mono_font(self) -> &'static str {
        match self {
            Self::MacOs => "Menlo",
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
    theme.set_mono_family(mono_family().into());
}

/// The monospace family; `ARCHGUARD_DESKTOP_MONO_FONT` overrides it.
fn mono_family() -> String {
    std::env::var("ARCHGUARD_DESKTOP_MONO_FONT")
        .unwrap_or_else(|_| Platform::current().mono_font().into())
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

    /// `font-size`/`font-weight` values in `text` that name a number
    /// instead of a `Theme` token. Scaled values (zoomed diagram fonts such
    /// as `11px * zoom`, a size relative to its control) keep theirs.
    fn literal_fonts(text: &str) -> Vec<String> {
        text.lines()
            .filter(|line| {
                line.split("font-size:")
                    .skip(1)
                    .chain(line.split("font-weight:").skip(1))
                    .any(literal_value)
            })
            .map(|line| line.trim().to_string())
            .collect()
    }

    fn literal_value(rest: &str) -> bool {
        let value = rest.split(';').next().unwrap_or(rest);
        !value.contains('*')
            && value
                .split(|c: char| !c.is_ascii_alphanumeric() && c != '.' && c != '-')
                .any(|w| {
                    let px = w
                        .strip_suffix("px")
                        .is_some_and(|n| n.parse::<f32>().is_ok());
                    // A weight (100–1000); smaller numbers are comparisons.
                    px || w.parse::<u32>().is_ok_and(|n| n >= 100)
                })
    }

    #[test]
    fn views_take_font_sizes_and_weights_from_the_theme() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
        let mut found = Vec::new();
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".slint") || name == "theme.slint" {
                continue;
            }
            let text = std::fs::read_to_string(entry.path()).unwrap();
            found.extend(
                literal_fonts(&text)
                    .into_iter()
                    .map(|l| format!("{name}: {l}")),
            );
        }
        assert!(
            found.is_empty(),
            "use Theme.font-*/weight-* tokens:\n{}",
            found.join("\n")
        );
    }

    #[test]
    fn a_literal_font_is_found_and_a_zoomed_one_allowed() {
        assert_eq!(literal_fonts("x { font-size: 12px; }").len(), 1);
        assert_eq!(
            literal_fonts("font-weight: a ? 700 : Theme.weight-regular;").len(),
            1
        );
        assert!(
            literal_fonts("font-size: Theme.font-body; font-weight: Theme.weight-strong;")
                .is_empty()
        );
        assert!(literal_fonts("font-size: max(6px, 11px * zoom);").is_empty());
    }

    slint::slint! {
        export component Mono inherits Window {
            width: 80px;
            height: 80px;
            background: black;
            in property <string> family;
            Text { x: 20px; y: 0; text: "|"; font-family: root.family; font-size: 60px; color: white; }
        }
    }

    struct Pixels(std::rc::Rc<slint::platform::software_renderer::MinimalSoftwareWindow>);

    impl slint::platform::Platform for Pixels {
        fn create_window_adapter(
            &self,
        ) -> Result<std::rc::Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
            Ok(self.0.clone())
        }
    }

    /// The red channel of a "|" drawn in `family`, 80×80 pixels.
    fn draw_bar(family: &str) -> Vec<u8> {
        use slint::ComponentHandle;
        use slint::platform::software_renderer::{
            MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
        };
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        window.set_size(slint::PhysicalSize::new(80, 80));
        slint::platform::set_platform(Box::new(Pixels(window.clone()))).expect("platform");
        let mono = Mono::new().expect("component");
        mono.set_family(family.into());
        mono.show().expect("shown");
        let mut buf = vec![PremultipliedRgbaColor::default(); 80 * 80];
        window.draw_if_needed(|r| {
            r.render(&mut buf, 80);
        });
        buf.iter().map(|p| p.red).collect()
    }

    /// The ink-weighted mean column of `rows`.
    fn mean_column(red: &[u8], rows: std::ops::Range<usize>) -> f32 {
        let (mut sum, mut weight) = (0.0, 0.0);
        for y in rows {
            for x in 0..80 {
                let v = f32::from(red[y * 80 + x]);
                sum += v * x as f32;
                weight += v;
            }
        }
        sum / weight.max(1.0)
    }

    /// The monospace face is upright: the bar's top quarter sits above its
    /// bottom quarter (".SF NS Mono" drew it 8 px slanted, see `mono_font`).
    #[test]
    fn the_monospace_font_is_upright() {
        let red = draw_bar(&super::mono_family());
        let rows: Vec<usize> = (0..80)
            .filter(|y| red[y * 80..][..80].iter().any(|&v| v > 128))
            .collect();
        let (top, bottom) = (rows[0], rows[rows.len() - 1] + 1);
        let quarter = (bottom - top) / 4;
        let slant =
            mean_column(&red, top..top + quarter) - mean_column(&red, bottom - quarter..bottom);
        assert!(slant.abs() < 1.0, "bar slants by {slant:.1} px");
    }

    #[test]
    fn every_platform_names_a_monospace_font() {
        for p in [Platform::MacOs, Platform::Windows, Platform::Linux] {
            assert!(!p.mono_font().is_empty(), "{p:?}");
            assert_eq!(Platform::named(&format!("{p:?}").to_lowercase()), Some(p));
        }
    }
}
