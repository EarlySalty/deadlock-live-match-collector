//! Pixel geometry shared by the native UI and platform-independent layout tests.
pub(crate) fn scale(value: i32, dpi: u32) -> i32 {
    (value * dpi as i32 + 48) / 96
}

/// Center a fixed-size span in the actual pixel bounds (at most one pixel of rounding).
pub(crate) fn centered(start: i32, end: i32, size: i32) -> (i32, i32) {
    let first = start + (end - start - size) / 2;
    (first, first + size)
}

/// Baseline for visible glyph ink, not for the taller typographic line box.
pub(crate) fn ink_baseline(top: i32, bottom: i32, above: i32, below: i32) -> i32 {
    centered(top, bottom, above + below).0 + above
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_track_thumb_and_label_share_the_button_center_at_common_dpis() {
        for dpi in [96, 120, 144, 168, 192, 240, 288] {
            for top in [0, 1, 5] {
                let bottom = top + scale(538, dpi) - scale(502, dpi);
                let track = centered(top, bottom, scale(20, dpi));
                let thumb = centered(top, bottom, scale(14, dpi));
                let above = scale(11, dpi);
                let below = scale(3, dpi);
                let baseline = ink_baseline(top, bottom, above, below);
                let axis = top + bottom;
                for sum in [
                    track.0 + track.1,
                    thumb.0 + thumb.1,
                    2 * baseline - above + below,
                ] {
                    assert!((sum - axis).abs() <= 1, "dpi={dpi}, sum={sum}, axis={axis}");
                }
                assert!(thumb.0 >= track.0 && thumb.1 <= track.1);
            }
        }
    }

    #[test]
    fn header_icon_and_edit_inner_bounds_stay_centered() {
        for dpi in [96, 120, 144, 168, 192, 240, 288] {
            for (top, bottom, size) in [(20, 76, 42), (305, 334, 20)] {
                let (a, b) = (scale(top, dpi), scale(bottom, dpi));
                let span = centered(a, b, scale(size, dpi));
                assert!((span.0 + span.1 - a - b).abs() <= 1);
                assert!(span.0 >= a && span.1 <= b);
            }
        }
    }
}
