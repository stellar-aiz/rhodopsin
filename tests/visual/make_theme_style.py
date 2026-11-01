"""Independent fixture for shape style references (`p:style`): python-pptx's default shape
style is `fillRef idx="3"` (a gradient in its theme), `lnRef idx="2"`, `effectRef idx="2"`
(a shadow), and `fontRef idx="minor"` with `lt1` text. Nothing is drawn from source
documents.

Page 1: three default-styled shapes with text (gradient fill, theme shadow, white theme
text), one shape whose style fill is changed to the solid `fillRef idx="1"`, one whose run
sets an explicit red colour, and one whose `effectRef` is index 0 (no shadow).
"""
from pathlib import Path
import sys

from pptx import Presentation
from pptx.dml.color import RGBColor
from pptx.enum.shapes import MSO_SHAPE
from pptx.util import Inches, Pt

A = 'http://schemas.openxmlformats.org/drawingml/2006/main'


def add(slide, preset, x, y, text, size=20):
    shape = slide.shapes.add_shape(preset, Inches(x), Inches(y), Inches(2.8), Inches(1.6))
    shape.text_frame.text = text
    for paragraph in shape.text_frame.paragraphs:
        paragraph.font.size = Pt(size)
    return shape


def main(out: Path):
    prs = Presentation()
    prs.slide_width, prs.slide_height = Inches(10), Inches(5.625)
    slide = prs.slides.add_slide(prs.slide_layouts[6])
    add(slide, MSO_SHAPE.ROUNDED_RECTANGLE, .4, .4, 'Gradient and shadow')
    add(slide, MSO_SHAPE.OVAL, 3.6, .4, 'Theme text')
    add(slide, MSO_SHAPE.RIGHT_ARROW, 6.8, .4, 'Arrow')
    solid = add(slide, MSO_SHAPE.RECTANGLE, .4, 2.6, 'Solid fill ref')
    solid._element.find(f'.//{{{A}}}fillRef').set('idx', '1')
    explicit = add(slide, MSO_SHAPE.RECTANGLE, 3.6, 2.6, 'Explicit red run')
    explicit.text_frame.paragraphs[0].runs[0].font.color.rgb = RGBColor(255, 0, 0)
    plain = add(slide, MSO_SHAPE.RECTANGLE, 6.8, 2.6, 'No effect')
    plain._element.find(f'.//{{{A}}}effectRef').set('idx', '0')
    prs.save(out / 'theme-style.pptx')


if __name__ == '__main__':
    main(Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parents[2] / 'examples')
