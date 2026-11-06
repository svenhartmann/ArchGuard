//! The World's keys (the canvas' `keydown` in `viewer_world.js` and the
//! details overlay's Enter and Esc in `viewer.js`).

use std::f64::consts::PI;

/// What a key on the stage does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum KeyAction {
    /// Zoom by a factor about the camera.
    Zoom(f64),
    /// Fit: centre at zoom 1, rotation kept.
    Fit,
    /// Turn by radians.
    Turn(f64),
    /// Move the camera by canvas pixels.
    Pan(f64, f64),
    /// Show the details of the selection.
    Details,
    /// Close the details; the selection stays.
    Close,
}

/// Slint's key texts for the arrows, Enter and Escape.
const LEFT: &str = "\u{F702}";
const RIGHT: &str = "\u{F703}";
const UP: &str = "\u{F700}";
const DOWN: &str = "\u{F701}";

/// The action of a key event's text; `None` passes the key on.
pub fn action(text: &str) -> Option<KeyAction> {
    Some(match text {
        "+" | "=" => KeyAction::Zoom(1.25),
        "-" => KeyAction::Zoom(0.8),
        "0" => KeyAction::Fit,
        "q" | "Q" => KeyAction::Turn(-PI / 12.0),
        "e" | "E" => KeyAction::Turn(PI / 12.0),
        LEFT => KeyAction::Pan(40.0, 0.0),
        RIGHT => KeyAction::Pan(-40.0, 0.0),
        UP => KeyAction::Pan(0.0, 40.0),
        DOWN => KeyAction::Pan(0.0, -40.0),
        "\n" | "\r" => KeyAction::Details,
        "\u{1b}" => KeyAction::Close,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_map_to_the_webs_moves() {
        assert_eq!(action("+"), Some(KeyAction::Zoom(1.25)));
        assert_eq!(action("="), action("+"));
        assert_eq!(action("-"), Some(KeyAction::Zoom(0.8)));
        assert_eq!(action("0"), Some(KeyAction::Fit));
        assert_eq!(action("Q"), Some(KeyAction::Turn(-PI / 12.0)));
        assert_eq!(action("e"), Some(KeyAction::Turn(PI / 12.0)));
        // The arrows move the World the way they point the camera.
        assert_eq!(action(LEFT), Some(KeyAction::Pan(40.0, 0.0)));
        assert_eq!(action(DOWN), Some(KeyAction::Pan(0.0, -40.0)));
        assert_eq!(action("\n"), Some(KeyAction::Details));
        assert_eq!(action("\u{1b}"), Some(KeyAction::Close));
        assert_eq!(action("x"), None);
        assert_eq!(action(""), None);
    }
}
