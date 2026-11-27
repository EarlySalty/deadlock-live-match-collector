//! Antialiased native Windows drawing. No added GUI runtime or external fonts.
use std::ptr::null_mut;
use windows_sys::Win32::{
    Foundation::RECT,
    Graphics::{Gdi::*, GdiPlus::*},
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
            if GdipCreatePen1(argb(color), 2.1 * s, UnitPixel, &mut pen) != 0 {
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
            let curve = |points: &[(f32, f32)]| {
                let pts: Vec<PointF> = points
                    .iter()
                    .map(|&(a, b)| PointF {
                        X: x + a * s,
                        Y: y + b * s,
                    })
                    .collect();
                GdipDrawBeziers(self.0, pen, pts.as_ptr(), pts.len() as i32);
            };
            let circle = |a: f32, b: f32, d: f32| {
                GdipDrawEllipse(self.0, pen, x + a * s, y + b * s, d * s, d * s);
            };
            let box_outline = |a: f32, b: f32, c: f32, d: f32| {
                let mut path = null_mut();
                if GdipCreatePath(FillModeWinding, &mut path) == 0 {
                    let radius = 3.0 * s;
                    let (xx, yy, ww, hh) = (x + a * s, y + b * s, c * s, d * s);
                    for (px, py, angle) in [
                        (xx, yy, 180.),
                        (xx + ww - radius, yy, 270.),
                        (xx + ww - radius, yy + hh - radius, 0.),
                        (xx, yy + hh - radius, 90.),
                    ] {
                        GdipAddPathArc(path, px, py, radius, radius, angle, 90.);
                    }
                    GdipClosePathFigure(path);
                    GdipDrawPath(self.0, pen, path);
                    GdipDeletePath(path);
                }
            };
            match name {
                "server" => {
                    box_outline(3., 3., 18., 7.);
                    box_outline(3., 14., 18., 7.);
                    lines(&[(6.5, 6.5), (8., 6.5)]);
                    lines(&[(6.5, 17.5), (8., 17.5)]);
                    lines(&[(17., 6.5), (18., 6.5)]);
                    lines(&[(17., 17.5), (18., 17.5)]);
                }
                "game" => {
                    curve(&[
                        (7., 6.),
                        (4., 6.),
                        (3., 9.),
                        (2.8, 14.),
                        (2., 18.),
                        (4.5, 20.),
                        (7., 17.),
                        (9., 14.),
                        (15., 14.),
                        (17., 17.),
                        (19.5, 20.),
                        (22., 18.),
                        (21.2, 14.),
                        (21., 9.),
                        (20., 6.),
                        (17., 6.),
                        (14., 6.),
                        (10., 6.),
                        (7., 6.),
                    ]);
                    lines(&[(6., 10.), (11., 10.)]);
                    lines(&[(8.5, 7.5), (8.5, 12.5)]);
                    circle(15., 8., 1.8);
                    circle(18., 11., 1.8);
                }
                "shield" => {
                    curve(&[
                        (12., 3.),
                        (15., 4.5),
                        (18., 5.),
                        (20., 5.5),
                        (20., 13.),
                        (18., 18.),
                        (12., 21.),
                        (6., 18.),
                        (4., 13.),
                        (4., 5.5),
                        (6., 5.),
                        (9., 4.5),
                        (12., 3.),
                    ]);
                    lines(&[(8.5, 12.), (11., 14.5), (15.5, 10.)]);
                }
                "key" => {
                    circle(3., 3., 10.);
                    lines(&[
                        (11.5, 11.5),
                        (20.5, 20.5),
                        (20.5, 16.5),
                        (17.5, 16.5),
                        (17.5, 13.5),
                    ]);
                }
                "link" => {
                    curve(&[
                        (10., 14.),
                        (8., 16.),
                        (5., 17.),
                        (3.5, 15.5),
                        (1.5, 13.5),
                        (2., 11.),
                        (4., 9.),
                        (6., 7.),
                        (9., 6.),
                        (11., 8.),
                    ]);
                    curve(&[
                        (14., 10.),
                        (16., 8.),
                        (19., 7.),
                        (20.5, 8.5),
                        (22.5, 10.5),
                        (22., 13.),
                        (20., 15.),
                        (18., 17.),
                        (15., 18.),
                        (13., 16.),
                    ]);
                    lines(&[(8.5, 15.5), (15.5, 8.5)]);
                }
                "external" => {
                    lines(&[(13., 3.5), (20.5, 3.5), (20.5, 11.)]);
                    lines(&[(20.5, 3.5), (10., 14.)]);
                    lines(&[(10., 5.5), (5., 5.5), (5., 19.5), (18.5, 19.5), (18.5, 14.)]);
                }
                "power" => {
                    GdipDrawArc(
                        self.0,
                        pen,
                        x + 3.5 * s,
                        y + 4.5 * s,
                        17. * s,
                        17. * s,
                        -49.,
                        278.,
                    );
                    lines(&[(12., 3.), (12., 11.5)]);
                }
                "minus" => {
                    lines(&[(6., 12.), (18., 12.)]);
                }
                "close" => {
                    lines(&[(6.5, 6.5), (17.5, 17.5)]);
                    lines(&[(17.5, 6.5), (6.5, 17.5)]);
                }
                "repo" => {
                    // Repository/branch symbol, not a misleading generic code bracket.
                    circle(4., 3., 5.);
                    circle(4., 16., 5.);
                    circle(15., 3., 5.);
                    lines(&[(6.5, 8.), (6.5, 16.)]);
                    curve(&[(17.5, 8.), (17.5, 12.), (13., 13.), (6.5, 13.)]);
                }
                "trash" => {
                    lines(&[(4., 6.5), (20., 6.5)]);
                    lines(&[(9., 6.5), (9., 3.5), (15., 3.5), (15., 6.5)]);
                    lines(&[(6., 6.5), (7., 20.5), (17., 20.5), (18., 6.5)]);
                    lines(&[(10., 10.), (10., 16.5)]);
                    lines(&[(14., 10.), (14., 16.5)]);
                }
                _ => {}
            }
            GdipDeletePen(pen);
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

// Render every opaque owner-drawn control into a device-compatible bitmap,
// then present it with one blit. GDI text stays on native device pixels.
pub(super) struct Buffer {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
}
impl Buffer {
    pub(super) unsafe fn new(target: HDC, r: RECT) -> Option<Self> {
        if r.right <= 0 || r.bottom <= 0 {
            return None;
        }
        let dc = CreateCompatibleDC(target);
        if dc.is_null() {
            return None;
        }
        let bitmap = CreateCompatibleBitmap(target, r.right, r.bottom);
        if bitmap.is_null() {
            DeleteDC(dc);
            return None;
        }
        let old = SelectObject(dc, bitmap);
        if old.is_null() || old as isize == -1 {
            DeleteObject(bitmap);
            DeleteDC(dc);
            return None;
        }
        Some(Self { dc, bitmap, old })
    }
    pub(super) fn dc(&self) -> HDC {
        self.dc
    }
    pub(super) unsafe fn present(&self, target: HDC, r: RECT) {
        BitBlt(
            target,
            r.left,
            r.top,
            r.right - r.left,
            r.bottom - r.top,
            self.dc,
            r.left,
            r.top,
            SRCCOPY,
        );
    }
}
impl Drop for Buffer {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}
