use crate::layout::Abs;
use crate::text::{Font, Glyph};
use kurbo::{BezPath, PathEl};
use ttf_parser::OutlineBuilder;

/// A recorded glyph outline whose closed contours can be geometrically
/// emboldened before emission.
#[derive(Debug, Clone)]
pub struct GlyphOutline {
    path: BezPath,
    contours: Vec<Contour>,
    open: Option<OpenContour>,
    current: Option<kurbo::Point>,
}

#[derive(Debug, Clone)]
struct Contour {
    elements: Vec<PathEl>,
    closed: bool,
}

#[derive(Debug, Clone)]
struct OpenContour {
    elements: Vec<PathEl>,
    start: kurbo::Point,
}

impl GlyphOutline {
    /// Create an empty outline.
    pub fn new() -> Self {
        Self {
            path: BezPath::new(),
            contours: vec![],
            open: None,
            current: None,
        }
    }

    /// Return the synthetic weight overlay for this outline.
    pub fn synthetic_weight_overlay(&mut self, strength: f64) -> BezPath {
        self.finish_open_contour(false);

        let mut emboldened = BezPath::new();
        if strength == 0.0 {
            return emboldened;
        }

        let width = strength.abs();
        let stroke = kurbo::Stroke::new(width)
            .with_caps(kurbo::Cap::Butt)
            .with_join(kurbo::Join::Miter)
            .with_miter_limit(4.0);
        let opts = kurbo::StrokeOpts::default();

        for contour in &self.contours {
            if !contour.closed {
                continue;
            }

            let path = BezPath::from_vec(contour.elements.clone());
            let widened =
                kurbo::stroke(path.elements().iter().copied(), &stroke, &opts, 0.01);
            emboldened.extend(widened.elements().iter().copied());
        }

        emboldened
    }

    /// Emit the outline to a path builder.
    pub fn emit(&self, builder: &mut impl OutlineBuilder) {
        Self::emit_path(&self.path, builder);
    }

    /// Emit a Kurbo path to a path builder.
    pub fn emit_path(path: &BezPath, builder: &mut impl OutlineBuilder) {
        for element in path.elements() {
            match *element {
                PathEl::MoveTo(p) => builder.move_to(p.x as f32, p.y as f32),
                PathEl::LineTo(p) => builder.line_to(p.x as f32, p.y as f32),
                PathEl::QuadTo(p1, p2) => {
                    builder.quad_to(p1.x as f32, p1.y as f32, p2.x as f32, p2.y as f32)
                }
                PathEl::CurveTo(p1, p2, p3) => builder.curve_to(
                    p1.x as f32,
                    p1.y as f32,
                    p2.x as f32,
                    p2.y as f32,
                    p3.x as f32,
                    p3.y as f32,
                ),
                PathEl::ClosePath => builder.close(),
            }
        }
    }

    fn finish_open_contour(&mut self, closed: bool) {
        if let Some(mut open) = self.open.take() {
            if closed {
                open.elements.push(PathEl::ClosePath);
            }

            if open.elements.len() > 1 {
                self.contours.push(Contour { elements: open.elements, closed });
            }
        }
    }

    fn push(&mut self, element: PathEl) {
        self.path.push(element);
        if let Some(open) = &mut self.open {
            open.elements.push(element);
        }

        self.current = match element {
            PathEl::MoveTo(p)
            | PathEl::LineTo(p)
            | PathEl::QuadTo(_, p)
            | PathEl::CurveTo(_, _, p) => Some(p),
            PathEl::ClosePath => self.open.as_ref().map(|open| open.start),
        };
    }
}

/// Build the synthetic weight overlay for a glyph.
pub fn synthetic_weight_overlay(
    font: &Font,
    size: Abs,
    glyph: &Glyph,
    embolden: Abs,
) -> Option<BezPath> {
    let mut outline = GlyphOutline::new();
    font.ttf()
        .outline_glyph(ttf_parser::GlyphId(glyph.id), &mut outline)?;
    let strength = embolden.to_pt() / size.to_pt() * font.units_per_em() as f64;
    Some(outline.synthetic_weight_overlay(strength))
}

impl Default for GlyphOutline {
    fn default() -> Self {
        Self::new()
    }
}

impl OutlineBuilder for GlyphOutline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.finish_open_contour(false);
        let point = kurbo::Point::new(x as f64, y as f64);
        self.open = Some(OpenContour {
            elements: vec![PathEl::MoveTo(point)],
            start: point,
        });
        self.path.move_to(point);
        self.current = Some(point);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.push(PathEl::LineTo(kurbo::Point::new(x as f64, y as f64)));
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.push(PathEl::QuadTo(
            kurbo::Point::new(x1 as f64, y1 as f64),
            kurbo::Point::new(x as f64, y as f64),
        ));
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.push(PathEl::CurveTo(
            kurbo::Point::new(x1 as f64, y1 as f64),
            kurbo::Point::new(x2 as f64, y2 as f64),
            kurbo::Point::new(x as f64, y as f64),
        ));
    }

    fn close(&mut self) {
        self.path.close_path();
        self.finish_open_contour(true);
        self.current = None;
    }
}
