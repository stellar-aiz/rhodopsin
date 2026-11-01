"""Independent fixture: Japanese families at 12/18/24 pt with default line spacing.

Reproduces the deck measured in PowerPoint for Mac 16.105.3 on 2026-10-20 (Issue #89).
PowerPoint used the Meiryo UI, Yu Gothic, and MS PGothic fonts it bundles; the renderer
substitutes them on systems without those families. Preserve the content so the
recorded measurements in docs/rendering.md stay reproducible.
"""
import sys
from pptx import Presentation
from pptx.util import Emu, Inches, Pt

FAMILIES = ["Meiryo UI", "游ゴシック", "ＭＳ Ｐゴシック"]
SIZES = [12, 18, 24]

p = Presentation()
p.slide_width, p.slide_height = Emu(12191695), Emu(6858000)
s = p.slides.add_slide(p.slide_layouts[6])
for column, family in enumerate(FAMILIES):
    for row, size in enumerate(SIZES):
        box = s.shapes.add_textbox(
            Inches(0.3 + 4.35 * column), Inches(0.3 + 2.4 * row), Inches(4.2), Inches(2.3)
        )
        tf = box.text_frame
        tf.word_wrap = True
        for line in range(1, 6):
            par = tf.paragraphs[0] if line == 1 else tf.add_paragraph()
            run = par.add_run()
            run.text = f"{family} {size}pt 行{line} あいう漢字ABC"
            run.font.size = Pt(size)
            run.font.name = family
            rpr = run._r.get_or_add_rPr()
            ea = rpr.makeelement("{http://schemas.openxmlformats.org/drawingml/2006/main}ea", {"typeface": family})
            rpr.append(ea)
p.save(sys.argv[1])
