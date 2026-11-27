//! Native Direct2D/DirectWrite renderer. All coordinates are actual device
//! pixels; the render target stays at 96 DPI because layout is already scaled.
use std::{
    cell::{Cell, RefCell},
    mem::zeroed,
};
use windows::{
    core::w,
    Win32::{
        Foundation::RECT as Rect,
        Graphics::{
            Direct2D::{Common::*, *},
            DirectWrite::*,
            Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
            Gdi::HDC as Dc,
        },
    },
};
use windows_sys::Win32::{Foundation::RECT, Graphics::Gdi::HDC};

struct Factories {
    d2d: ID2D1Factory,
    write: IDWriteFactory,
    stroke: ID2D1StrokeStyle,
}
impl Factories {
    fn new() -> windows::core::Result<Self> {
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let write = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let stroke = d2d.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    dashCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    miterLimit: 10.,
                    dashStyle: D2D1_DASH_STYLE_SOLID,
                    dashOffset: 0.,
                },
                None,
            )?;
            Ok(Self { d2d, write, stroke })
        }
    }
}
thread_local! {
    static FACTORIES: RefCell<Option<Factories>> = const { RefCell::new(None) };
}

fn color(c: u32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: (c & 255) as f32 / 255.,
        g: ((c >> 8) & 255) as f32 / 255.,
        b: ((c >> 16) & 255) as f32 / 255.,
        a: 1.,
    }
}
fn rect(r: RECT) -> D2D_RECT_F {
    D2D_RECT_F {
        left: r.left as f32,
        top: r.top as f32,
        right: r.right as f32,
        bottom: r.bottom as f32,
    }
}
fn point(x: f32, y: f32) -> D2D_POINT_2F {
    D2D_POINT_2F { x, y }
}

pub(super) struct VectorFrame {
    target: ID2D1DCRenderTarget,
    factory: ID2D1Factory,
    write: IDWriteFactory,
    stroke: ID2D1StrokeStyle,
    failed: Cell<bool>,
    ended: bool,
}
impl VectorFrame {
    pub(super) fn begin(dc: HDC, r: RECT) -> Option<Self> {
        if r.left != 0 || r.top != 0 || r.right <= 0 || r.bottom <= 0 {
            return None;
        }
        FACTORIES.with(|f| {
            let mut f = f.borrow_mut();
            if f.is_none() {
                *f = Factories::new().ok();
            }
            let f = f.as_ref()?;
            unsafe {
                let target = f
                    .d2d
                    .CreateDCRenderTarget(&D2D1_RENDER_TARGET_PROPERTIES {
                        r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                        pixelFormat: D2D1_PIXEL_FORMAT {
                            format: DXGI_FORMAT_B8G8R8A8_UNORM,
                            alphaMode: D2D1_ALPHA_MODE_IGNORE,
                        },
                        dpiX: 96.,
                        dpiY: 96.,
                        usage: D2D1_RENDER_TARGET_USAGE_NONE,
                        minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
                    })
                    .ok()?;
                target
                    .BindDC(
                        Dc(dc as isize),
                        &Rect {
                            left: r.left,
                            top: r.top,
                            right: r.right,
                            bottom: r.bottom,
                        },
                    )
                    .ok()?;
                target.SetDpi(96., 96.);
                target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
                // Flat pixel geometry and zero ClearType level prevent RGB fringes.
                if let Ok(params) = f.write.CreateCustomRenderingParams(
                    2.2,
                    0.5,
                    0.,
                    DWRITE_PIXEL_GEOMETRY_FLAT,
                    DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC,
                ) {
                    target.SetTextRenderingParams(&params);
                }
                target.BeginDraw();
                Some(Self {
                    target,
                    factory: f.d2d.clone(),
                    write: f.write.clone(),
                    stroke: f.stroke.clone(),
                    failed: Cell::new(false),
                    ended: false,
                })
            }
        })
    }
    pub(super) fn finish(mut self) -> bool {
        self.ended = true;
        unsafe { self.target.EndDraw(None, None).is_ok() && !self.failed.get() }
    }
    fn brush(&self, c: u32) -> Option<ID2D1SolidColorBrush> {
        unsafe {
            self.target
                .CreateSolidColorBrush(&color(c), None)
                .map_err(|_| self.failed.set(true))
                .ok()
        }
    }
    pub(super) fn fill(&self, r: RECT, c: u32) {
        if let Some(b) = self.brush(c) {
            unsafe {
                self.target.FillRectangle(&rect(r), &b);
            }
        }
    }
    pub(super) fn rounded(&self, r: RECT, c: u32, diameter: i32) {
        let radius = (diameter as f32 / 2.)
            .min((r.right - r.left) as f32 / 2.)
            .min((r.bottom - r.top) as f32 / 2.)
            .max(0.);
        if let Some(b) = self.brush(c) {
            unsafe {
                self.target.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: rect(r),
                        radiusX: radius,
                        radiusY: radius,
                    },
                    &b,
                );
            }
        }
    }
    fn layout(
        &self,
        label: &[u16],
        size: i32,
        weight: i32,
        bounds: (f32, f32),
        style: (bool, bool),
    ) -> windows::core::Result<IDWriteTextLayout> {
        let (w, h) = bounds;
        let (center, wrap) = style;
        unsafe {
            let format = self.write.CreateTextFormat(
                w!("Segoe UI"),
                None,
                DWRITE_FONT_WEIGHT(weight),
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size as f32,
                w!("de-de"),
            )?;
            format.SetTextAlignment(if center {
                DWRITE_TEXT_ALIGNMENT_CENTER
            } else {
                DWRITE_TEXT_ALIGNMENT_LEADING
            })?;
            format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            format.SetWordWrapping(if wrap {
                DWRITE_WORD_WRAPPING_WRAP
            } else {
                DWRITE_WORD_WRAPPING_NO_WRAP
            })?;
            let sign = self.write.CreateEllipsisTrimmingSign(&format)?;
            format.SetTrimming(
                &DWRITE_TRIMMING {
                    granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                    delimiter: 0,
                    delimiterCount: 0,
                },
                &sign,
            )?;
            self.write
                .CreateTextLayout(label, &format, w.max(1.), h.max(1.))
        }
    }
    pub(super) fn text_width(&self, label: &[u16], size: i32, weight: i32) -> Option<i32> {
        let layout = self
            .layout(label, size, weight, (16384., 1024.), (false, false))
            .ok()?;
        let mut m: DWRITE_TEXT_METRICS = unsafe { zeroed() };
        unsafe {
            layout.GetMetrics(&mut m).ok()?;
        }
        Some(m.widthIncludingTrailingWhitespace.ceil() as i32 + 2)
    }
    pub(super) fn text(
        &self,
        label: &[u16],
        r: RECT,
        c: u32,
        size: i32,
        weight: i32,
        style: (bool, bool),
    ) {
        if label.is_empty() {
            return;
        }
        let (center, wrap) = style;
        let Ok(layout) = self.layout(
            label,
            size,
            weight,
            ((r.right - r.left) as f32, (r.bottom - r.top) as f32),
            (center, wrap),
        ) else {
            self.failed.set(true);
            return;
        };
        if let Some(b) = self.brush(c) {
            unsafe {
                self.target.DrawTextLayout(
                    point(r.left as f32, r.top as f32),
                    &layout,
                    &b,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                );
            }
        }
    }

    pub(super) fn icon(&self, name: &str, r: RECT, c: u32) {
        let Some(b) = self.brush(c) else {
            return;
        };
        let s = (r.right - r.left).min(r.bottom - r.top) as f32 / 24.;
        let x = r.left as f32;
        let y = r.top as f32;
        let p = |a: f32, b: f32| point(x + a * s, y + b * s);
        let width = 2.1 * s;
        let path = |pts: &[(f32, f32)], curved: bool| {
            if pts.len() < 2 {
                return;
            }
            unsafe {
                let Ok(g) = self.factory.CreatePathGeometry() else {
                    self.failed.set(true);
                    return;
                };
                let Ok(sink) = g.Open() else {
                    self.failed.set(true);
                    return;
                };
                sink.BeginFigure(p(pts[0].0, pts[0].1), D2D1_FIGURE_BEGIN_HOLLOW);
                if curved {
                    let segments = pts[1..]
                        .as_chunks::<3>()
                        .0
                        .iter()
                        .map(|a| D2D1_BEZIER_SEGMENT {
                            point1: p(a[0].0, a[0].1),
                            point2: p(a[1].0, a[1].1),
                            point3: p(a[2].0, a[2].1),
                        })
                        .collect::<Vec<_>>();
                    sink.AddBeziers(&segments);
                } else {
                    let points = pts[1..].iter().map(|&(a, b)| p(a, b)).collect::<Vec<_>>();
                    sink.AddLines(&points);
                }
                sink.EndFigure(D2D1_FIGURE_END_OPEN);
                if sink.Close().is_err() {
                    self.failed.set(true);
                    return;
                }
                self.target.DrawGeometry(&g, &b, width, &self.stroke);
            }
        };
        let lines = |pts: &[(f32, f32)]| path(pts, false);
        let curve = |pts: &[(f32, f32)]| path(pts, true);
        let circle = |a: f32, d: f32, size: f32| unsafe {
            self.target.DrawEllipse(
                &D2D1_ELLIPSE {
                    point: p(a + size / 2., d + size / 2.),
                    radiusX: size * s / 2.,
                    radiusY: size * s / 2.,
                },
                &b,
                width,
                &self.stroke,
            );
        };
        let box_outline = |a: f32, d: f32, w: f32, h: f32| unsafe {
            self.target.DrawRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: x + a * s,
                        top: y + d * s,
                        right: x + (a + w) * s,
                        bottom: y + (d + h) * s,
                    },
                    radiusX: 1.5 * s,
                    radiusY: 1.5 * s,
                },
                &b,
                width,
                &self.stroke,
            );
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
                curve(&[
                    (7., 5.5),
                    (3.8, 7.5),
                    (2.5, 11.),
                    (4., 15.5),
                    (6., 22.),
                    (18., 22.),
                    (20., 15.5),
                    (21.5, 11.),
                    (20.2, 7.5),
                    (17., 5.5),
                ]);
                lines(&[(12., 3.), (12., 11.5)]);
            }
            "minus" => lines(&[(6., 12.), (18., 12.)]),
            "close" => {
                lines(&[(6.5, 6.5), (17.5, 17.5)]);
                lines(&[(17.5, 6.5), (6.5, 17.5)]);
            }
            "repo" => {
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
    }
    pub(super) fn logo(&self, r: RECT) {
        // Original 192px PNG converted to premultiplied BGRA,
        // prefiltered for common physical logo sizes (not a low-res icon stretch).
        let desired = (r.right - r.left).max(1) as u32;
        let variants: [(u32, &[u8]); 7] = [
            (64, include_bytes!("../packaging/logo-pixels/logo64.bgra")),
            (80, include_bytes!("../packaging/logo-pixels/logo80.bgra")),
            (96, include_bytes!("../packaging/logo-pixels/logo96.bgra")),
            (112, include_bytes!("../packaging/logo-pixels/logo112.bgra")),
            (128, include_bytes!("../packaging/logo-pixels/logo128.bgra")),
            (160, include_bytes!("../packaging/logo-pixels/logo160.bgra")),
            (192, include_bytes!("../packaging/logo-pixels/logo192.bgra")),
        ];
        let (size, data) = variants
            .iter()
            .min_by_key(|(size, _)| size.abs_diff(desired))
            .unwrap();
        unsafe {
            let bitmap = self.target.CreateBitmap(
                D2D_SIZE_U {
                    width: *size,
                    height: *size,
                },
                Some(data.as_ptr().cast()),
                size * 4,
                &D2D1_BITMAP_PROPERTIES {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                    },
                    dpiX: 96.,
                    dpiY: 96.,
                },
            );
            match bitmap {
                Ok(bitmap) => self.target.DrawBitmap(
                    &bitmap,
                    Some(&rect(r)),
                    1.,
                    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                    None,
                ),
                Err(_) => self.failed.set(true),
            }
        }
    }
}
impl Drop for VectorFrame {
    fn drop(&mut self) {
        if !self.ended {
            unsafe {
                let _ = self.target.EndDraw(None, None);
            }
        }
    }
}
