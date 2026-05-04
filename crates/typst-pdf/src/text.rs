use std::ops::Range;
use std::sync::Arc;

use bytemuck::TransparentWrapper;
use krilla::geom::PathBuilder;
use krilla::num::NormalizedF32;
use krilla::surface::{Location, Surface};
use krilla::text::GlyphId;
use ttf_parser::OutlineBuilder;
use typst_library::diag::{SourceResult, bail};
use typst_library::text::{
    Font, Glyph, GlyphOutline, TextItem, synthetic_weight_overlay,
};
use typst_library::visualize::FillRule;
use typst_syntax::Span;
use typst_utils::defer;

use crate::convert::{FrameContext, GlobalContext};
use crate::util::{AbsExt, TransformExt, display_font};
use crate::{paint, tags};

#[typst_macros::time(name = "handle text")]
pub(crate) fn handle_text(
    fc: &mut FrameContext,
    t: &TextItem,
    surface: &mut Surface,
    gc: &mut GlobalContext,
) -> SourceResult<()> {
    let fill = paint::convert_fill(
        gc,
        &t.fill,
        FillRule::NonZero,
        true,
        surface,
        fc.state(),
        None,
    )?;
    let stroke = if let Some(stroke) = t.stroke.as_ref() {
        Some(paint::convert_stroke(gc, stroke, true, surface, fc.state(), None)?)
    } else {
        None
    };
    let text = t.text.as_str();
    let size = t.size;
    let glyphs: &[PdfGlyph] = TransparentWrapper::wrap_slice(t.glyphs.as_slice());
    let synthesize_weight = t.synthesize.embolden.is_some();
    let font = convert_font(gc, t.font.clone())?;

    surface.push_transform(&fc.state().transform().to_krilla());
    let mut surface = defer(surface, |s| s.pop());
    if !synthesize_weight {
        surface.set_fill(Some(fill));
        surface.set_stroke(stroke);
        let mut handle = tags::text(gc, fc, &mut surface, t);
        let surface = handle.surface();
        surface.draw_glyphs(
            krilla::geom::Point::from_xy(0.0, 0.0),
            glyphs,
            font,
            text,
            size.to_f32(),
            false,
        );
    } else {
        {
            let mut hidden_fill = fill.clone();
            hidden_fill.opacity = NormalizedF32::ZERO;
            surface.set_fill(Some(hidden_fill));
            surface.set_stroke(None);

            let mut handle = tags::text(gc, fc, &mut surface, t);
            let surface = handle.surface();
            surface.draw_glyphs(
                krilla::geom::Point::from_xy(0.0, 0.0),
                glyphs,
                font,
                text,
                size.to_f32(),
                false,
            );
        }

        tags::artifact(gc, &mut surface, |surface| {
            surface.set_fill(Some(fill));
            surface.set_stroke(stroke);
            draw_glyphs_with_synthetic_weight(t, surface);
        });
    }

    Ok(())
}

fn draw_glyphs_with_synthetic_weight(t: &TextItem, surface: &mut Surface) -> Option<()> {
    let mut x = 0.0;
    let y = 0.0;
    let font_size = t.size.to_f32();
    let scale = font_size / t.font.units_per_em() as f32;
    let embolden = t.synthesize.embolden?;
    let mut glyphs = KrillaPathBuilder(PathBuilder::new());
    let mut overlay = KrillaPathBuilder(PathBuilder::new());
    let font_bbox = t.font.ttf().global_bounding_box();
    let pad = embolden.to_f32().max(1.0);
    let mut left = f32::INFINITY;
    let mut top = f32::INFINITY;
    let mut right = f32::NEG_INFINITY;
    let mut bottom = f32::NEG_INFINITY;

    for glyph in &t.glyphs {
        let mut outline = GlyphOutline::new();
        t.font
            .ttf()
            .outline_glyph(ttf_parser::GlyphId(glyph.id), &mut outline)?;
        let emboldened = synthetic_weight_overlay(&t.font, t.size, glyph, embolden)?;

        let tx = x + glyph.x_offset.at(t.size).to_f32();
        let ty = y - glyph.y_offset.at(t.size).to_f32();

        left = left.min(tx + font_bbox.x_min as f32 * scale - pad);
        top = top.min(ty - font_bbox.y_max as f32 * scale - pad);
        right = right.max(tx + font_bbox.x_max as f32 * scale + pad);
        bottom = bottom.max(ty - font_bbox.y_min as f32 * scale + pad);

        let mut glyph_builder = TransformedPathBuilder { inner: &mut glyphs, scale, tx, ty };
        outline.emit(&mut glyph_builder);

        let mut overlay_builder = TransformedPathBuilder { inner: &mut overlay, scale, tx, ty };
        GlyphOutline::emit_path(&emboldened, &mut overlay_builder);

        x += glyph.x_advance.at(t.size).to_f32();
    }

    // Krilla maps gradients for paths through the path bbox. Add the same
    // zero-area line to both paths so the glyphs and overlay share a bbox,
    // without merging their contours and changing fill winding behavior.
    expand_path_bbox(&mut glyphs, left, top, right, bottom);
    expand_path_bbox(&mut overlay, left, top, right, bottom);

    surface.draw_path(&glyphs.0.finish()?);
    surface.draw_path(&overlay.0.finish()?);

    Some(())
}

fn expand_path_bbox(
    builder: &mut KrillaPathBuilder,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
) {
    builder.move_to(left, top);
    builder.line_to(right, bottom);
}

struct TransformedPathBuilder<'a> {
    inner: &'a mut KrillaPathBuilder,
    scale: f32,
    tx: f32,
    ty: f32,
}

impl TransformedPathBuilder<'_> {
    fn transform(&self, x: f32, y: f32) -> (f32, f32) {
        (self.tx + x * self.scale, self.ty - y * self.scale)
    }
}

impl OutlineBuilder for TransformedPathBuilder<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.transform(x, y);
        self.inner.move_to(x, y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.transform(x, y);
        self.inner.line_to(x, y);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (x1, y1) = self.transform(x1, y1);
        let (x, y) = self.transform(x, y);
        self.inner.quad_to(x1, y1, x, y);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (x1, y1) = self.transform(x1, y1);
        let (x2, y2) = self.transform(x2, y2);
        let (x, y) = self.transform(x, y);
        self.inner.curve_to(x1, y1, x2, y2, x, y);
    }

    fn close(&mut self) {
        self.inner.close();
    }
}

struct KrillaPathBuilder(PathBuilder);

impl OutlineBuilder for KrillaPathBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, y);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.0.quad_to(x1, y1, x, y);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.0.cubic_to(x1, y1, x2, y2, x, y);
    }

    fn close(&mut self) {
        self.0.close();
    }
}

fn convert_font(
    gc: &mut GlobalContext,
    typst_font: Font,
) -> SourceResult<krilla::text::Font> {
    if let Some(font) = gc.fonts_forward.get(&typst_font) {
        Ok(font.clone())
    } else {
        let font = build_font(typst_font.clone())?;

        gc.fonts_forward.insert(typst_font.clone(), font.clone());
        gc.fonts_backward.insert(font.clone(), typst_font.clone());

        Ok(font)
    }
}

#[comemo::memoize]
fn build_font(typst_font: Font) -> SourceResult<krilla::text::Font> {
    let font_data: Arc<dyn AsRef<[u8]> + Send + Sync> =
        Arc::new(typst_font.data().clone());

    match krilla::text::Font::new(font_data.into(), typst_font.index()) {
        Some(f) => Ok(f),
        None => {
            bail!(
                Span::detached(),
                "failed to process {}",
                display_font(Some(&typst_font)),
            )
        }
    }
}

#[derive(Debug, TransparentWrapper)]
#[repr(transparent)]
struct PdfGlyph(Glyph);

impl krilla::text::Glyph for PdfGlyph {
    #[inline(always)]
    fn glyph_id(&self) -> GlyphId {
        GlyphId::new(self.0.id as u32)
    }

    #[inline(always)]
    fn text_range(&self) -> Range<usize> {
        self.0.range.start as usize..self.0.range.end as usize
    }

    #[inline(always)]
    fn x_advance(&self, size: f32) -> f32 {
        // Don't use `Em::at`, because it contains an expensive check whether the result is finite.
        self.0.x_advance.get() as f32 * size
    }

    #[inline(always)]
    fn x_offset(&self, size: f32) -> f32 {
        // Don't use `Em::at`, because it contains an expensive check whether the result is finite.
        self.0.x_offset.get() as f32 * size
    }

    #[inline(always)]
    fn y_offset(&self, size: f32) -> f32 {
        // Don't use `Em::at`, because it contains an expensive check whether the result is finite.
        self.0.y_offset.get() as f32 * size
    }

    #[inline(always)]
    fn y_advance(&self, size: f32) -> f32 {
        // Don't use `Em::at`, because it contains an expensive check whether the result is finite.
        self.0.y_advance.get() as f32 * size
    }

    fn location(&self) -> Option<Location> {
        Some(self.0.span.0.into_raw())
    }
}
