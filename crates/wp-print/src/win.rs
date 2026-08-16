//! The Windows printer, through the dialog everyone knows.
//!
//! GDI printing is drawing: a printer device context accepts the same calls a
//! screen one does, at the printer's resolution. So this walks the same
//! [`crate::ops`] the PDF writer walks and issues rectangles, pens and text —
//! with one deliberate difference in how text is placed. The PDF embeds our
//! font bytes; GDI is instead *named* a face and does its own rasterising, so
//! every character is pinned to the layout's advance with `lpDx`, the argument
//! that exists for exactly this. Whatever GDI thinks a string measures, each
//! character lands where the screen put it, and the printed line breaks where
//! the screen's did.
//!
//! The dialog is `PrintDlgW` — old, small, and the one that returns a ready
//! device context. Orientation follows the document's pages through
//! `ResetDCW`, per page, because a landscape section in a portrait document is
//! a thing Word documents do.

use std::collections::HashMap;

use wp_layout::block::Page;

use crate::ops::{draw_charts, flatten, Charts, Op};
use crate::Raster;

use windows_sys::Win32::Foundation::{GlobalFree, HGLOBAL, HWND, POINT};
use windows_sys::Win32::Graphics::Gdi::ExtTextOutW;
use windows_sys::Win32::Graphics::Gdi::Polygon;
use windows_sys::Win32::Graphics::Gdi::{
    CreateDCW, CreateFontW, CreatePen, CreateSolidBrush, DeleteDC, DeleteObject, FillRect,
    GetDeviceCaps, LineTo, MoveToEx, ResetDCW, SelectObject, SetBkMode, SetBrushOrgEx,
    SetStretchBltMode, SetTextAlign, SetTextColor, StretchDIBits, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEVMODEW, DIB_RGB_COLORS,
    DMORIENT_LANDSCAPE, DMORIENT_PORTRAIT, DM_ORIENTATION, FW_BOLD, FW_NORMAL, HALFTONE, HDC,
    HGDIOBJ, LOGPIXELSX, LOGPIXELSY, OUT_DEFAULT_PRECIS, PHYSICALOFFSETX, PHYSICALOFFSETY,
    PS_SOLID, SRCCOPY, TA_BASELINE, TA_LEFT, TRANSPARENT,
};
use windows_sys::Win32::Storage::Xps::{AbortDoc, EndDoc, EndPage, StartDocW, StartPage, DOCINFOW};
use windows_sys::Win32::System::Memory::{GlobalLock, GlobalUnlock};
use windows_sys::Win32::UI::Controls::Dialogs::{
    PrintDlgW, PD_HIDEPRINTTOFILE, PD_NOSELECTION, PD_PAGENUMS, PD_RETURNDC,
    PD_USEDEVMODECOPIESANDCOLLATE, PRINTDLGW,
};

/// Shows the print dialog and, if the user says print, renders the pages.
///
/// `Ok(false)` is a cancel, which is not an error. `gdi_family` maps a
/// document's face name to the family GDI should be asked for — the same
/// substitution the screen made, so unknown names print in the face they were
/// shown in.
pub fn print(
    pages: &[Page],
    images: &HashMap<String, Raster>,
    charts: Option<&mut Charts>,
    document_name: &str,
    gdi_family: &dyn Fn(&str) -> String,
) -> Result<bool, String> {
    let mut dialog: PRINTDLGW = unsafe { std::mem::zeroed() };
    dialog.lStructSize = std::mem::size_of::<PRINTDLGW>() as u32;
    dialog.hwndOwner = std::ptr::null_mut::<std::ffi::c_void>() as HWND;
    dialog.Flags =
        PD_RETURNDC | PD_NOSELECTION | PD_HIDEPRINTTOFILE | PD_USEDEVMODECOPIESANDCOLLATE;
    dialog.nMinPage = 1;
    dialog.nMaxPage = pages.len().max(1) as u16;
    dialog.nFromPage = 1;
    dialog.nToPage = dialog.nMaxPage;
    dialog.nCopies = 1;

    if unsafe { PrintDlgW(&mut dialog) } == 0 {
        // Cancelled — or failed, which CommDlgExtendedError would say; either
        // way there is nothing to print and nothing to report.
        free_dialog(&dialog);
        return Ok(false);
    }
    let hdc = dialog.hDC;
    if hdc.is_null() {
        free_dialog(&dialog);
        return Err("The printer did not provide a device context.".to_owned());
    }

    let chosen: Vec<usize> = if dialog.Flags & PD_PAGENUMS != 0 {
        let from = dialog.nFromPage.max(1) as usize;
        let to = (dialog.nToPage as usize).min(pages.len());
        (from..=to.max(from)).map(|n| n - 1).collect()
    } else {
        (0..pages.len()).collect()
    };
    let copies = dialog.nCopies.max(1);

    let outcome = render_job(
        hdc,
        dialog.hDevMode,
        None,
        pages,
        &chosen,
        copies,
        images,
        charts,
        document_name,
        gdi_family,
    );
    unsafe { DeleteDC(hdc) };
    free_dialog(&dialog);
    outcome.map(|()| true)
}

/// Prints without a dialog, to a named printer, spooling to `output`.
///
/// This is the whole rendering path with the one interactive step removed —
/// what a test drives through the *Microsoft Print to PDF* driver to hold the
/// printed pages in its hands, and what a future `/print` switch would call.
pub fn print_to_file(
    printer: &str,
    output: &std::path::Path,
    pages: &[Page],
    images: &HashMap<String, Raster>,
    charts: Option<&mut Charts>,
    document_name: &str,
    gdi_family: &dyn Fn(&str) -> String,
) -> Result<(), String> {
    let name = wide(printer);
    let hdc = unsafe {
        CreateDCW(
            std::ptr::null(),
            name.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if hdc.is_null() {
        return Err(format!("No printer called {printer}."));
    }
    let every: Vec<usize> = (0..pages.len()).collect();
    let outcome = render_job(
        hdc,
        std::ptr::null_mut(),
        Some(output),
        pages,
        &every,
        1,
        images,
        charts,
        document_name,
        gdi_family,
    );
    unsafe { DeleteDC(hdc) };
    outcome
}

fn free_dialog(dialog: &PRINTDLGW) {
    unsafe {
        if !dialog.hDevMode.is_null() {
            GlobalFree(dialog.hDevMode);
        }
        if !dialog.hDevNames.is_null() {
            GlobalFree(dialog.hDevNames);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_job(
    hdc: HDC,
    devmode: HGLOBAL,
    output: Option<&std::path::Path>,
    pages: &[Page],
    chosen: &[usize],
    copies: u16,
    images: &HashMap<String, Raster>,
    charts: Option<&mut Charts>,
    document_name: &str,
    gdi_family: &dyn Fn(&str) -> String,
) -> Result<(), String> {
    let name = wide(document_name);
    let output = output.map(|path| wide(&path.to_string_lossy()));
    let info = DOCINFOW {
        cbSize: std::mem::size_of::<DOCINFOW>() as i32,
        lpszDocName: name.as_ptr(),
        lpszOutput: output
            .as_ref()
            .map(|wide| wide.as_ptr())
            .unwrap_or(std::ptr::null()),
        lpszDatatype: std::ptr::null(),
        fwType: 0,
    };
    if unsafe { StartDocW(hdc, &info) } <= 0 {
        return Err("The printer refused the document.".to_owned());
    }

    let mut fonts = FontCache::default();
    let mut charts = charts;
    let mut landscape_now: Option<bool> = None;
    let mut failed = None;
    'job: for _copy in 0..copies {
        for &index in chosen {
            let page = &pages[index];
            // Turn the paper before the page starts, when the driver allows
            // the change at all.
            let landscape = page.geometry.width > page.geometry.height;
            if landscape_now != Some(landscape) {
                set_orientation(hdc, devmode, landscape);
                landscape_now = Some(landscape);
            }
            if unsafe { StartPage(hdc) } <= 0 {
                failed = Some("The printer refused a page.".to_owned());
                break 'job;
            }
            render_page(
                hdc,
                page,
                images,
                charts.as_deref_mut(),
                &mut fonts,
                gdi_family,
            );
            if unsafe { EndPage(hdc) } <= 0 {
                failed = Some("The printer stopped mid-document.".to_owned());
                break 'job;
            }
        }
    }
    fonts.clear();

    if let Some(message) = failed {
        unsafe { AbortDoc(hdc) };
        return Err(message);
    }
    unsafe { EndDoc(hdc) };
    Ok(())
}

/// Asks the driver to turn the paper — through the dialog's own DEVMODE when
/// there is one, through a minimal one of ours when printing without a dialog.
fn set_orientation(hdc: HDC, devmode: HGLOBAL, landscape: bool) {
    let orientation = if landscape {
        DMORIENT_LANDSCAPE as i16
    } else {
        DMORIENT_PORTRAIT as i16
    };
    unsafe {
        if !devmode.is_null() {
            let locked = GlobalLock(devmode) as *mut DEVMODEW;
            if !locked.is_null() {
                (*locked).Anonymous1.Anonymous1.dmOrientation = orientation;
                (*locked).dmFields |= DM_ORIENTATION;
                ResetDCW(hdc, locked);
                GlobalUnlock(devmode);
            }
            return;
        }
        let mut local: DEVMODEW = std::mem::zeroed();
        local.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
        // DM_SPECVERSION, as wingdi.h has said it since Windows 95.
        local.dmSpecVersion = 0x0401;
        local.Anonymous1.Anonymous1.dmOrientation = orientation;
        local.dmFields = DM_ORIENTATION;
        ResetDCW(hdc, &local);
    }
}

/// Fonts GDI created for this job, one per face and device size.
#[derive(Default)]
struct FontCache {
    fonts: HashMap<(String, i32, bool, bool), HGDIOBJ>,
}

impl FontCache {
    fn get(&mut self, family: &str, height: i32, bold: bool, italic: bool) -> HGDIOBJ {
        *self
            .fonts
            .entry((family.to_owned(), height, bold, italic))
            .or_insert_with(|| unsafe {
                let name = wide(family);
                CreateFontW(
                    -height,
                    0,
                    0,
                    0,
                    if bold {
                        FW_BOLD as i32
                    } else {
                        FW_NORMAL as i32
                    },
                    italic as u32,
                    0,
                    0,
                    DEFAULT_CHARSET as u32,
                    OUT_DEFAULT_PRECIS as u32,
                    CLIP_DEFAULT_PRECIS as u32,
                    CLEARTYPE_QUALITY as u32,
                    0,
                    name.as_ptr(),
                ) as HGDIOBJ
            })
    }

    fn clear(&mut self) {
        for (_, font) in self.fonts.drain() {
            unsafe { DeleteObject(font) };
        }
    }
}

fn render_page(
    hdc: HDC,
    page: &Page,
    images: &HashMap<String, Raster>,
    charts: Option<&mut Charts>,
    fonts: &mut FontCache,
    gdi_family: &dyn Fn(&str) -> String,
) {
    let sx = unsafe { GetDeviceCaps(hdc, LOGPIXELSX as i32) } as f64 / 72.0;
    let sy = unsafe { GetDeviceCaps(hdc, LOGPIXELSY as i32) } as f64 / 72.0;
    // The driver's origin is the printable corner, not the paper's; the layout
    // measures from the paper.
    let ox = unsafe { GetDeviceCaps(hdc, PHYSICALOFFSETX as i32) };
    let oy = unsafe { GetDeviceCaps(hdc, PHYSICALOFFSETY as i32) };
    let px = |x: f64| (x * sx).round() as i32 - ox;
    let py = |y: f64| (y * sy).round() as i32 - oy;

    unsafe {
        SetBkMode(hdc, TRANSPARENT as i32);
        SetTextAlign(hdc, TA_BASELINE | TA_LEFT);
        SetStretchBltMode(hdc, HALFTONE);
        SetBrushOrgEx(hdc, 0, 0, std::ptr::null_mut());
    }

    let ops = match charts {
        Some(charts) => draw_charts(flatten(page), charts),
        None => flatten(page),
    };
    for op in ops {
        match op {
            Op::Fill {
                x,
                y,
                width,
                height,
                rgb,
            } => unsafe {
                let rect = windows_sys::Win32::Foundation::RECT {
                    left: px(x),
                    top: py(y),
                    right: px(x + width).max(px(x) + 1),
                    bottom: py(y + height).max(py(y) + 1),
                };
                let brush = CreateSolidBrush(colorref(rgb));
                FillRect(hdc, &rect, brush);
                DeleteObject(brush as HGDIOBJ);
            },
            Op::Rule {
                from,
                to,
                thickness,
                rgb,
            } => unsafe {
                let pen = CreatePen(
                    PS_SOLID,
                    ((thickness * sy).round() as i32).max(1),
                    colorref(rgb),
                );
                let old = SelectObject(hdc, pen as HGDIOBJ);
                MoveToEx(hdc, px(from.0), py(from.1), std::ptr::null_mut());
                LineTo(hdc, px(to.0), py(to.1));
                SelectObject(hdc, old);
                DeleteObject(pen as HGDIOBJ);
            },
            Op::Text {
                x,
                baseline,
                text,
                advances,
                font,
                rgb,
            } => {
                let height = (font.size * sy).round() as i32;
                let family = gdi_family(&font.family);
                let handle = fonts.get(&family, height, font.bold, font.italic);
                let (units, dx) = pin_advances(&text, &advances, x, sx);
                if units.is_empty() {
                    continue;
                }
                unsafe {
                    let old = SelectObject(hdc, handle);
                    SetTextColor(hdc, colorref(rgb));
                    ExtTextOutW(
                        hdc,
                        px(x),
                        py(baseline),
                        0,
                        std::ptr::null(),
                        units.as_ptr(),
                        units.len() as u32,
                        dx.as_ptr(),
                    );
                    SelectObject(hdc, old);
                }
            }
            Op::Image {
                x,
                y,
                width,
                height,
                rel,
            } => {
                let Some(raster) = images.get(&rel) else {
                    continue;
                };
                let bits = over_white(raster);
                let header = BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: raster.width as i32,
                    // Negative height is the top-down orientation the decoded
                    // rows are already in.
                    biHeight: -(raster.height as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    biSizeImage: 0,
                    biXPelsPerMeter: 0,
                    biYPelsPerMeter: 0,
                    biClrUsed: 0,
                    biClrImportant: 0,
                };
                let info = BITMAPINFO {
                    bmiHeader: header,
                    bmiColors: [unsafe { std::mem::zeroed() }],
                };
                unsafe {
                    StretchDIBits(
                        hdc,
                        px(x),
                        py(y),
                        (width * sx).round() as i32,
                        (height * sy).round() as i32,
                        0,
                        0,
                        raster.width as i32,
                        raster.height as i32,
                        bits.as_ptr() as *const std::ffi::c_void,
                        &info,
                        DIB_RGB_COLORS,
                        SRCCOPY,
                    );
                }
            }
            Op::Poly { points, rgb } => {
                if points.len() < 3 {
                    continue;
                }
                let device: Vec<POINT> = points
                    .iter()
                    .map(|&(x, y)| POINT { x: px(x), y: py(y) })
                    .collect();
                unsafe {
                    // No outline: the fill is the shape, and a pen of the
                    // paper's colour would still leave a hairline seam
                    // between the slices of a pie.
                    let brush = CreateSolidBrush(colorref(rgb));
                    let old_brush = SelectObject(hdc, brush as HGDIOBJ);
                    let pen = CreatePen(PS_SOLID, 1, colorref(rgb));
                    let old_pen = SelectObject(hdc, pen as HGDIOBJ);
                    Polygon(hdc, device.as_ptr(), device.len() as i32);
                    SelectObject(hdc, old_pen);
                    SelectObject(hdc, old_brush);
                    DeleteObject(pen as HGDIOBJ);
                    DeleteObject(brush as HGDIOBJ);
                }
            }
            // A chart nobody expanded: its box stays empty, as it did before
            // charts were drawn at all.
            Op::Chart { .. } => {}
        }
    }
}

/// UTF-16 units and the per-unit advances that pin every character to the
/// layout's position, in device pixels with the rounding shared so error never
/// accumulates across a line.
fn pin_advances(text: &str, advances: &[f64], x: f64, sx: f64) -> (Vec<u16>, Vec<i32>) {
    let mut units = Vec::with_capacity(text.len());
    let mut dx = Vec::with_capacity(text.len());
    let mut pen = x;
    for (index, c) in text.chars().enumerate() {
        let advance = advances.get(index).copied().unwrap_or(0.0);
        let here = (pen * sx).round() as i32;
        let next = ((pen + advance) * sx).round() as i32;
        pen += advance;
        let mut encoded = [0u16; 2];
        let pair = c.encode_utf16(&mut encoded);
        units.push(pair[0]);
        dx.push(if pair.len() == 2 { 0 } else { next - here });
        if pair.len() == 2 {
            // A surrogate pair is one pen movement; GDI wants an entry per
            // code unit, so the movement rides the second.
            units.push(pair[1]);
            dx.push(next - here);
        }
    }
    (units, dx)
}

/// BGRA over white paper — printers do not blend, so this does.
fn over_white(raster: &Raster) -> Vec<u8> {
    let mut out = Vec::with_capacity(raster.rgba.len());
    for pixel in raster.rgba.chunks_exact(4) {
        let a = u16::from(pixel[3]);
        let blend = |c: u8| ((u16::from(c) * a + 255 * (255 - a)) / 255) as u8;
        out.push(blend(pixel[2]));
        out.push(blend(pixel[1]));
        out.push(blend(pixel[0]));
        out.push(255);
    }
    out
}

fn colorref([r, g, b]: [u8; 3]) -> u32 {
    u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16)
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advances_are_pinned_without_accumulating_rounding_error() {
        // Three characters of 1.5pt at 300dpi: each is 6.25px. Rounding each
        // alone gives 6+6+6 = 18; the line is really 18.75px, which is 19.
        let (units, dx) = pin_advances("abc", &[1.5, 1.5, 1.5], 0.0, 300.0 / 72.0);
        assert_eq!(units.len(), 3);
        let total: i32 = dx.iter().sum();
        assert_eq!(total, 19, "the shared rounding keeps the fourth pixel");
    }

    #[test]
    fn a_surrogate_pair_is_two_units_and_one_movement() {
        let (units, dx) = pin_advances("a𝄞b", &[6.0, 10.0, 6.0], 0.0, 1.0);
        assert_eq!(units.len(), 4, "the clef is two code units");
        assert_eq!(dx, vec![6, 0, 10, 6]);
    }

    #[test]
    fn transparency_prints_as_paper() {
        let raster = Raster {
            width: 1,
            height: 1,
            rgba: vec![0, 0, 0, 0],
            jpeg: None,
        };
        assert_eq!(over_white(&raster), vec![255, 255, 255, 255]);
    }
}
