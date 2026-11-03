mod delta;

use crate::alpha;

pub fn combine() -> String {
    format!("{}+{}", alpha::render(), delta::tag())
}
