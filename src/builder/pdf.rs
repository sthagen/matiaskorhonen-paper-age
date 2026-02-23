//! Native printpdf QR code renderer.
//!
//! Implements the [`qrcode::render::Pixel`] and [`qrcode::render::Canvas`] traits
//! so that QR codes are drawn directly as `printpdf` [`Op::DrawRectangle`] operations,
//! eliminating the SVG intermediate format entirely.

use log::{debug, info};
use printpdf::{Color, Op, PaintMode, Pt, Rect, Rgb, WindingOrder};
use qrcode::{
    render::{Canvas as RenderCanvas, Pixel},
    types::Color as ModuleColor,
    types::QrError,
    EcLevel, QrCode,
};

use crate::page::PageSize;

// ---------------------------------------------------------------------------
// Pixel / Canvas implementation
// ---------------------------------------------------------------------------

/// A pixel type for the printpdf QR renderer.
///
/// The `Image` associated type is a flat list of dark module rectangles in
/// module-unit coordinates `(left, top, width, height)`.  The caller converts
/// those to PDF point coordinates after the renderer finishes.
#[derive(Copy, Clone, Default, PartialEq, Eq)]
pub struct PdfPixel {
    is_dark: bool,
}

impl Pixel for PdfPixel {
    /// Module rectangles in module-unit coordinates: (left, top, width, height).
    type Image = Vec<(u32, u32, u32, u32)>;
    type Canvas = PdfCanvas;

    /// Use a 1×1 module size so that the coordinates coming out of the renderer
    /// are plain module indices, making the point-conversion math trivial.
    fn default_unit_size() -> (u32, u32) {
        (1, 1)
    }

    fn default_color(color: ModuleColor) -> Self {
        PdfPixel {
            is_dark: color == ModuleColor::Dark,
        }
    }
}

/// Canvas that collects dark module rectangles during rendering.
pub struct PdfCanvas {
    dark_rects: Vec<(u32, u32, u32, u32)>,
}

impl RenderCanvas for PdfCanvas {
    type Pixel = PdfPixel;
    type Image = Vec<(u32, u32, u32, u32)>;

    fn new(_width: u32, _height: u32, _dark_pixel: PdfPixel, _light_pixel: PdfPixel) -> Self {
        PdfCanvas {
            dark_rects: Vec::new(),
        }
    }

    fn draw_dark_pixel(&mut self, x: u32, y: u32) {
        self.draw_dark_rect(x, y, 1, 1);
    }

    fn draw_dark_rect(&mut self, left: u32, top: u32, width: u32, height: u32) {
        self.dark_rects.push((left, top, width, height));
    }

    fn into_image(self) -> Vec<(u32, u32, u32, u32)> {
        self.dark_rects
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Build a QR code and return the `printpdf` drawing operations for it.
///
/// The QR code is positioned and sized for the given `page_size`:
/// - Centered horizontally.
/// - Filling `page_size.qrcode_size()` in the upper half of the page,
///   inset by the standard margin.
///
/// The error correction level is chosen automatically (H → Q → M → L),
/// returning the highest level that fits the data.
pub fn qrcode(text: String, page_size: &PageSize) -> Result<Vec<Op>, QrError> {
    // Error Correction Capability (approx.): H 30% / Q 25% / M 15% / L 7%
    let levels = [EcLevel::H, EcLevel::Q, EcLevel::M, EcLevel::L];

    let mut result: Result<QrCode, QrError> = Err(QrError::DataTooLong);
    for &ec_level in &levels {
        debug!("Trying EC level {:?}", ec_level);
        result = QrCode::with_error_correction_level(text.clone(), ec_level);
        if result.is_ok() {
            break;
        }
    }
    let code = result?;

    info!("QR code EC level: {:?}", code.error_correction_level());
    info!("QR code version: {:?}", code.version());

    // Render to module-unit rectangles (no quiet zone, 1×1 module size).
    let dark_rects: Vec<(u32, u32, u32, u32)> = code.render::<PdfPixel>().quiet_zone(false).build();

    let modules_count = code.width() as u32;

    // --- Coordinate calculations ---
    // Desired QR code size in PDF points.
    let desired_pt = page_size.qrcode_size().into_pt().0;
    // Size of a single module in PDF points.
    let module_pt = desired_pt / modules_count as f32;
    // Actual rendered size (may differ slightly from desired due to rounding).
    let qr_size_pt = module_pt * modules_count as f32;

    let page_width_pt = page_size.dimensions().width.into_pt().0;
    let page_height_pt = page_size.dimensions().height.into_pt().0;
    let margin_pt = page_size.dimensions().margin.into_pt().0;

    // Bottom-left origin of the QR code in PDF coordinates (y-axis up).
    let origin_x = (page_width_pt - qr_size_pt) / 2.0;
    let origin_y = page_height_pt - qr_size_pt - margin_pt * 2.0;

    // --- Build ops ---
    let mut ops = vec![Op::SetFillColor {
        col: Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None)),
    }];

    for (left, top, width, height) in dark_rects {
        // PDF x: origin_x + column_offset
        let x = origin_x + left as f32 * module_pt;
        // PDF y for the bottom edge of this rect (y-axis up, row 0 is the QR
        // code's top row which sits at the highest y value).
        let y = origin_y + (modules_count - top - height) as f32 * module_pt;

        ops.push(Op::DrawPolygon {
            polygon: Rect {
                x: Pt(x),
                y: Pt(y),
                width: Pt(width as f32 * module_pt),
                height: Pt(height as f32 * module_pt),
                mode: Some(PaintMode::Fill),
                winding_order: Some(WindingOrder::NonZero),
            }
            .to_polygon(),
        });
    }

    Ok(ops)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::{PageSize, A4_PAGE};

    #[test]
    fn test_pdf_qrcode_ok() {
        let ops = qrcode(String::from("Some value"), &PageSize::A4).unwrap();
        // First op is SetFillColor, followed by at least one DrawRectangle.
        assert!(ops.len() > 1);
        assert!(matches!(ops[0], Op::SetFillColor { .. }));
        assert!(ops[1..]
            .iter()
            .all(|op| matches!(op, Op::DrawPolygon { .. })));
    }

    #[test]
    fn test_pdf_qrcode_rect_count() {
        let ops = qrcode(String::from("test"), &PageSize::A4).unwrap();
        // Subtract the leading SetFillColor op.
        let rect_count = ops.len() - 1;
        // A minimal QR code (version 1 = 21×21 = 441 modules) will have at
        // least a few dozen dark modules, and at most 441.
        assert!(
            rect_count >= 50,
            "Expected ≥50 dark modules, got {rect_count}"
        );
        assert!(
            rect_count <= 441,
            "Expected ≤441 modules for version 1, got {rect_count}"
        );
    }

    #[test]
    fn test_pdf_qrcode_letter() {
        let ops = qrcode(String::from("Some value"), &PageSize::Letter).unwrap();
        assert!(ops.len() > 1);
    }

    #[test]
    fn test_pdf_qrcode_origin_x_centered() {
        // For A4 the QR code should be horizontally centered.
        let ops = qrcode(String::from("hi"), &PageSize::A4).unwrap();
        let page_width_pt = A4_PAGE.width.into_pt().0;
        let desired_pt = PageSize::A4.qrcode_size().into_pt().0;

        let expected_origin_x = (page_width_pt - desired_pt) / 2.0;

        // First DrawPolygon should have its leftmost point at origin_x (the
        // first dark module of a QR finder pattern is at column 0).
        if let Op::DrawPolygon { polygon } = &ops[1] {
            let x = polygon.rings[0]
                .points
                .iter()
                .map(|lp| lp.p.x.0)
                .fold(f32::INFINITY, f32::min);
            let diff = (x - expected_origin_x).abs();
            assert!(diff < 0.01, "Expected x ≈ {expected_origin_x}, got {x}");
        } else {
            panic!("Expected a DrawPolygon op");
        }
    }

    #[test]
    fn test_pdf_qrcode_too_large() {
        let result = qrcode(
            String::from(include_str!("../../tests/data/too_large.txt")),
            &PageSize::A4,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err() == QrError::DataTooLong);
    }
}
