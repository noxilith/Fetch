//! Shading ramps and sub-cell glyph tables.
//!
//! Coverage is sampled on a grid finer than the character cell and collapsed
//! into one glyph: 1x1 is the plain ramp, 2x2 picks a quadrant, 2x3 a block
//! sextant.

pub const MAX_SUB_ROWS: usize = 3;
pub const MAX_SUB_COLS: usize = 2;
const MAX_SHADING: usize = 64;

// The ASCII ramp bottoms out at '.' and ',', so dimly lit parts of the logo
// turn into scattered specks. The block ramp keeps ink in every cell it covers.
const RAMP_ASCII: &str = ".,-~:;=!*#$@";
const RAMP_BLOCKS: &str = "░▒▓█";

/// Quadrants by coverage mask, bit 0 top-left through bit 3 bottom-right.
pub const QUADRANT_GLYPHS: [&str; 16] = [
    " ", "▘", "▝", "▀", "▖", "▌", "▞", "▛", "▗", "▚", "▐", "▜", "▄", "▙", "▟", "█",
];

pub struct Shading {
    pub chars: Vec<String>,
    pub sub_rows: usize,
    pub sub_cols: usize,
    /// Sextants by coverage mask (bit 0 top-left through bit 5 bottom-right).
    pub sextants: Vec<String>,
}

/// Sextants run U+1FB00..U+1FB3B in mask order, skipping the two masks
/// Unicode already had as half blocks.
fn build_sextant_glyphs() -> Vec<String> {
    let mut g = vec![String::new(); 64];
    for mask in 1u32..63 {
        g[mask as usize] = if mask == 21 {
            "▌".into()
        } else if mask == 42 {
            "▐".into()
        } else {
            let cp = 0x1FB00 + mask - 1 - u32::from(mask > 21) - u32::from(mask > 42);
            char::from_u32(cp).map(String::from).unwrap_or_default()
        };
    }
    g
}

fn parse_ramp(s: &str) -> Vec<String> {
    let mut v: Vec<String> = s.chars().take(MAX_SHADING).map(String::from).collect();
    if v.is_empty() {
        v.push(".".into());
    }
    v
}

impl Shading {
    /// `mode` is ascii/blocks/sextants, or `None` for the ascii default; `chars`
    /// overrides the mode's ramp. Returns `None` on an unknown mode.
    pub fn select(mode: Option<&str>, chars: Option<&str>) -> Option<Shading> {
        let mode = mode.unwrap_or("ascii");
        let (sub_cols, sub_rows, sextants) = match mode {
            "sextants" => (2, 3, build_sextant_glyphs()),
            "blocks" => (2, 2, Vec::new()),
            "ascii" => (1, 1, Vec::new()),
            _ => return None,
        };
        let ramp = match chars {
            Some(c) => parse_ramp(c),
            None => parse_ramp(if mode == "ascii" { RAMP_ASCII } else { RAMP_BLOCKS }),
        };
        Some(Shading {
            chars: ramp,
            sub_rows,
            sub_cols,
            sextants,
        })
    }
}
