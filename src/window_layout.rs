//! Pixel geometry shared by the native UI and platform-independent layout tests.
pub(crate) type Bounds = (i32, i32, i32, i32);
pub(crate) const CLIENT_W: i32 = 632;
pub(crate) const CLIENT_H: i32 = 628;
pub(crate) const CAPTION_H: i32 = 36;
pub(crate) const HEADER_TOP: i32 = 51;
pub(crate) const HEADER_BOTTOM: i32 = 114;
pub(crate) const EDIT_BOUNDS: Bounds = (40, 330, 552, 42);
pub(crate) const AUTO_BOUNDS: Bounds = (536, 509, 56, 38);
pub(crate) const CARDS: [Bounds; 4] = [
    (24, 130, 286, 118),
    (322, 130, 286, 118),
    (24, 262, 584, 212),
    (24, 488, 584, 92),
];

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

/// Align the icon and its measured label as a single centered unit.
pub(crate) fn icon_label_group(
    left: i32,
    right: i32,
    icon: i32,
    gap: i32,
    text: i32,
) -> (i32, i32, i32) {
    let text = text.max(0).min((right - left - icon - gap).max(0));
    let (start, end) = centered(left, right, icon + gap + text);
    (start, start + icon + gap, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_and_label_are_centered_as_one_group_without_overflow() {
        for dpi in [96, 120, 144, 168, 192, 240, 288] {
            for (w, text) in [(110, 45), (120, 48), (270, 100), (186, 134), (110, 200)] {
                let l = scale(8, dpi);
                let r = scale(w, dpi) - l;
                let i = scale(16, dpi);
                let gap = scale(8, dpi);
                let (a, b, c) = icon_label_group(l, r, i, gap, scale(text, dpi));
                assert!(a >= l && c <= r);
                assert_eq!(b - a, i + gap);
                assert!((a + c - l - r).abs() <= 1);
            }
        }
    }

    #[test]
    fn toggle_track_thumb_and_label_share_the_button_center_at_common_dpis() {
        for dpi in [96, 120, 144, 168, 192, 240, 288] {
            for top in [0, 1, 5] {
                let bottom =
                    top + scale(AUTO_BOUNDS.1 + AUTO_BOUNDS.3, dpi) - scale(AUTO_BOUNDS.1, dpi);
                let track = centered(top, bottom, scale(24, dpi));
                let thumb = centered(top, bottom, scale(18, dpi));
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
    fn premium_cards_and_controls_fit_at_all_dpis() {
        for dpi in [96, 120, 144, 168, 192, 240, 288] {
            let edges = |(x, y, w, h): Bounds| {
                (
                    scale(x, dpi),
                    scale(y, dpi),
                    scale(x + w, dpi),
                    scale(y + h, dpi),
                )
            };
            let contains = |outer: Bounds, inner: Bounds| {
                let (a, b, c, d) = edges(outer);
                let (e, f, g, h) = edges(inner);
                a <= e && b <= f && c >= g && d >= h
            };
            for card in CARDS {
                assert!(contains(
                    (0, CAPTION_H, CLIENT_W, CLIENT_H - CAPTION_H),
                    card
                ));
            }
            for (card, control) in [
                (CARDS[2], EDIT_BOUNDS),
                (CARDS[3], AUTO_BOUNDS),
                (CARDS[0], (40, 184, 254, 46)),
                (CARDS[1], (338, 184, 254, 46)),
                (CARDS[2], (40, 382, 270, 38)),
                (CARDS[2], (322, 382, 270, 38)),
                (CARDS[2], (238, 430, 354, 32)),
            ] {
                assert!(contains(card, control));
            }
            assert!(contains((0, 0, CLIENT_W, CAPTION_H), (588, 3, 40, 30)));
            assert!(contains((0, 0, CLIENT_W, CLIENT_H), (488, 595, 120, 30)));
        }
    }

    #[test]
    fn header_icon_and_edit_inner_bounds_stay_centered() {
        for dpi in [96, 120, 144, 168, 192, 240, 288] {
            for (top, bottom, size) in [
                (HEADER_TOP, HEADER_BOTTOM, 54),
                (EDIT_BOUNDS.1, EDIT_BOUNDS.1 + EDIT_BOUNDS.3, 20),
            ] {
                let (a, b) = (scale(top, dpi), scale(bottom, dpi));
                let span = centered(a, b, scale(size, dpi));
                assert!((span.0 + span.1 - a - b).abs() <= 1);
                assert!(span.0 >= a && span.1 <= b);
            }
        }
    }
}
