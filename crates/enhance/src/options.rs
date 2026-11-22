//! What Enhance does to a photo (Lightroom Classic's Enhance dialog: Denoise with an amount, Raw Details, Super
//! Resolution), with Lightroom's file-name suffixes and a stable cache key.

use serde::{Deserialize, Serialize};

/// What to do (Lightroom's Enhance dialog).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Options {
    /// AI Denoise with this amount, 1..100 (0: off).
    pub denoise: f32,
    /// Raw Details: the finer demosaic (implied by Denoise, which demosaics itself).
    pub raw_details: bool,
    /// Super Resolution: twice the width and height.
    pub super_resolution: bool,
}

impl Options {
    pub fn any(&self) -> bool {
        self.denoise > 0.0 || self.raw_details || self.super_resolution
    }

    /// Lightroom's file name suffix: `Enhanced-NR`, `-RD`, `-SR`, `-NR-SR` …
    pub fn suffix(&self) -> String {
        let mut s = String::from("Enhanced");
        if self.denoise > 0.0 {
            s.push_str("-NR");
        } else if self.raw_details {
            s.push_str("-RD");
        }
        if self.super_resolution {
            s.push_str("-SR");
        }
        s
    }

    /// A stable text naming these options (cache keys).
    pub fn key(&self) -> String {
        format!("nr{:.0}-rd{}-sr{}", self.denoise.clamp(0.0, 100.0), u8::from(self.raw_details), u8::from(self.super_resolution))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suffixes_and_keys_follow_lightroom() {
        let o = |denoise, raw_details, super_resolution| Options { denoise, raw_details, super_resolution };
        assert_eq!(o(50.0, false, false).suffix(), "Enhanced-NR");
        assert_eq!(o(0.0, true, false).suffix(), "Enhanced-RD");
        assert_eq!(o(0.0, false, true).suffix(), "Enhanced-SR");
        assert_eq!(o(50.0, true, true).suffix(), "Enhanced-NR-SR");
        assert_ne!(o(50.0, false, false).key(), o(60.0, false, false).key());
        assert!(!o(0.0, false, false).any());
    }
}
