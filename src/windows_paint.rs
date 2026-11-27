//! Antialiased native Windows drawing. No added GUI runtime or external fonts.
use std::ptr::null_mut;
use windows_sys::Win32::{
    Foundation::RECT,
    Graphics::{Gdi::HDC, GdiPlus::*},
};

pub(super) struct Session(usize);
impl Session {
    pub(super) fn start() -> Option<Self> {
        let mut token = 0;
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            DebugEventCallback: 0,
            SuppressBackgroundThread: 0,
            SuppressExternalCodecs: 1,
        };
        (unsafe { GdiplusStartup(&mut token, &input, null_mut()) } == 0).then(|| Self(token))
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            GdiplusShutdown(self.0);
        }
    }
}

fn argb(bgr: u32) -> u32 {
    0xff000000 | ((bgr & 0xff) << 16) | (bgr & 0xff00) | ((bgr >> 16) & 0xff)
}

pub(super) struct Canvas(*mut GpGraphics);
impl Canvas {
    pub(super) unsafe fn new(dc: HDC) -> Option<Self> {
        let mut g = null_mut();
        if GdipCreateFromHDC(dc, &mut g) != 0 {
            return None;
        }
        GdipSetSmoothingMode(g, SmoothingModeAntiAlias8x8);
        GdipSetPixelOffsetMode(g, PixelOffsetModeHalf);
        GdipSetTextRenderingHint(g, TextRenderingHintAntiAliasGridFit);
        Some(Self(g))
    }

    pub(super) fn rounded(&self, r: RECT, color: u32, diameter: i32) -> bool {
        let (x, y, w, h) = (
            r.left as f32,
            r.top as f32,
            (r.right - r.left) as f32,
            (r.bottom - r.top) as f32,
        );
        if w <= 0.0 || h <= 0.0 {
            return false;
        }
        let d = (diameter as f32).max(1.0).min(w).min(h);
        unsafe {
            let mut brush = null_mut();
            if GdipCreateSolidFill(argb(color), &mut brush) != 0 {
                return false;
            }
            let mut path = null_mut();
            let ok = if GdipCreatePath(FillModeWinding, &mut path) == 0 {
                let arcs = [
                    (x, y, 180.0),
                    (x + w - d, y, 270.0),
                    (x + w - d, y + h - d, 0.0),
                    (x, y + h - d, 90.0),
                ];
                let ok = arcs
                    .iter()
                    .all(|&(a, b, angle)| GdipAddPathArc(path, a, b, d, d, angle, 90.0) == 0)
                    && GdipClosePathFigure(path) == 0
                    && GdipFillPath(self.0, brush.cast(), path) == 0;
                GdipDeletePath(path);
                ok
            } else {
                false
            };
            GdipDeleteBrush(brush.cast());
            ok
        }
    }

    /// Uniform 24-unit line icons, all drawn from the same stroke system.
    pub(super) fn icon(&self, name: &str, r: RECT, color: u32) {
        let size = (r.right - r.left).min(r.bottom - r.top) as f32;
        let s = size / 24.0;
        let x = r.left as f32;
        let y = r.top as f32;
        unsafe {
            let mut pen = null_mut();
            if GdipCreatePen1(argb(color), 1.65 * s, UnitPixel, &mut pen) != 0 {
                return;
            }
            GdipSetPenStartCap(pen, LineCapRound);
            GdipSetPenEndCap(pen, LineCapRound);
            GdipSetPenLineJoin(pen, LineJoinRound);
            let lines = |p: &[(f32, f32)]| {
                let pts: Vec<PointF> = p
                    .iter()
                    .map(|&(a, b)| PointF {
                        X: x + a * s,
                        Y: y + b * s,
                    })
                    .collect();
                GdipDrawLines(self.0, pen, pts.as_ptr(), pts.len() as i32);
            };
            let rect = |a: f32, b: f32, c: f32, d: f32| {
                GdipDrawRectangle(self.0, pen, x + a * s, y + b * s, c * s, d * s);
            };
            let circle = |a: f32, b: f32, d: f32| {
                GdipDrawEllipse(self.0, pen, x + a * s, y + b * s, d * s, d * s);
            };
            match name {
                "server" => {
                    rect(3., 4., 18., 6.);
                    rect(3., 14., 18., 6.);
                    lines(&[(6., 7.), (8., 7.)]);
                    lines(&[(6., 17.), (8., 17.)]);
                }
                "game" => {
                    lines(&[
                        (7., 6.),
                        (17., 6.),
                        (20., 9.),
                        (22., 17.),
                        (19., 19.),
                        (15., 15.),
                        (9., 15.),
                        (5., 19.),
                        (2., 17.),
                        (4., 9.),
                        (7., 6.),
                    ]);
                    lines(&[(6., 10.), (12., 10.)]);
                    lines(&[(9., 7.5), (9., 12.5)]);
                    circle(16., 9., 1.5);
                    circle(18.5, 11.5, 1.5);
                }
                "shield" => {
                    lines(&[
                        (12., 2.),
                        (20., 5.),
                        (20., 12.),
                        (17., 18.),
                        (12., 22.),
                        (7., 18.),
                        (4., 12.),
                        (4., 5.),
                        (12., 2.),
                    ]);
                    lines(&[(8., 12.), (11., 15.), (16., 9.)]);
                }
                "key" => {
                    circle(3., 3., 10.);
                    lines(&[
                        (11., 11.),
                        (21., 21.),
                        (18., 21.),
                        (18., 18.),
                        (15., 18.),
                        (15., 15.),
                    ]);
                }
                "link" => {
                    lines(&[
                        (10., 14.),
                        (7., 17.),
                        (4., 17.),
                        (2., 14.),
                        (2., 11.),
                        (7., 6.),
                        (10., 6.),
                        (12., 8.),
                    ]);
                    lines(&[
                        (12., 16.),
                        (14., 18.),
                        (17., 18.),
                        (22., 13.),
                        (22., 10.),
                        (19., 7.),
                        (16., 7.),
                        (13., 10.),
                    ]);
                    lines(&[(8., 16.), (16., 8.)]);
                }
                "external" => {
                    lines(&[(13., 3.), (21., 3.), (21., 11.)]);
                    lines(&[(21., 3.), (10., 14.)]);
                    lines(&[(10., 5.), (4., 5.), (4., 20.), (19., 20.), (19., 14.)]);
                }
                "power" => {
                    GdipDrawArc(
                        self.0,
                        pen,
                        x + 3. * s,
                        y + 4. * s,
                        18. * s,
                        18. * s,
                        -55.,
                        290.,
                    );
                    lines(&[(12., 2.), (12., 12.)]);
                }
                "minus" => {
                    lines(&[(5., 12.), (19., 12.)]);
                }
                "close" => {
                    lines(&[(6., 6.), (18., 18.)]);
                    lines(&[(18., 6.), (6., 18.)]);
                }
                "code" => {
                    lines(&[(8., 6.), (2., 12.), (8., 18.)]);
                    lines(&[(16., 6.), (22., 12.), (16., 18.)]);
                    lines(&[(14., 4.), (10., 20.)]);
                }
                "trash" => {
                    lines(&[(4., 6.), (20., 6.)]);
                    lines(&[(9., 6.), (9., 3.), (15., 3.), (15., 6.)]);
                    lines(&[(6., 6.), (7., 21.), (17., 21.), (18., 6.)]);
                    lines(&[(10., 10.), (10., 17.)]);
                    lines(&[(14., 10.), (14., 17.)]);
                }
                _ => {}
            }
            GdipDeletePen(pen);
        }
    }

    pub(super) fn text(
        &self,
        label: &[u16],
        r: RECT,
        color: u32,
        size: i32,
        style: (bool, bool, bool),
    ) -> bool {
        let (bold, center, wrap) = style;
        let name: Vec<u16> = "Segoe UI".encode_utf16().chain(Some(0)).collect();
        unsafe {
            let mut family = null_mut();
            if GdipCreateFontFamilyFromName(name.as_ptr(), null_mut(), &mut family) != 0 {
                return false;
            }
            let mut font = null_mut();
            let status = GdipCreateFont(
                family,
                size as f32,
                if bold {
                    FontStyleBold
                } else {
                    FontStyleRegular
                },
                UnitPixel,
                &mut font,
            );
            GdipDeleteFontFamily(family);
            if status != 0 {
                return false;
            }
            let mut format = null_mut();
            let mut brush = null_mut();
            let ok = if GdipCreateStringFormat(
                if wrap { 0 } else { StringFormatFlagsNoWrap },
                0,
                &mut format,
            ) == 0
            {
                GdipSetStringFormatAlign(
                    format,
                    if center {
                        StringAlignmentCenter
                    } else {
                        StringAlignmentNear
                    },
                );
                GdipSetStringFormatLineAlign(
                    format,
                    if wrap {
                        StringAlignmentNear
                    } else {
                        StringAlignmentCenter
                    },
                );
                GdipSetStringFormatTrimming(format, StringTrimmingEllipsisCharacter);
                let ok = if GdipCreateSolidFill(argb(color), &mut brush) == 0 {
                    let rect = RectF {
                        X: r.left as f32,
                        Y: r.top as f32,
                        Width: (r.right - r.left) as f32,
                        Height: (r.bottom - r.top) as f32,
                    };
                    let ok = GdipDrawString(
                        self.0,
                        label.as_ptr(),
                        label.len() as i32,
                        font,
                        &rect,
                        format,
                        brush.cast(),
                    ) == 0;
                    GdipDeleteBrush(brush.cast());
                    ok
                } else {
                    false
                };
                GdipDeleteStringFormat(format);
                ok
            } else {
                false
            };
            GdipDeleteFont(font);
            ok
        }
    }
}
impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe {
            GdipDeleteGraphics(self.0);
        }
    }
}
