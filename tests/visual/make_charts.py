"""Independent chart fixtures written with python-pptx's chart API (no source documents).

examples/chart-bar.pptx: a clustered column chart with two series, a title, data labels,
gridlines, and a bottom legend; a second slide holds a stacked horizontal bar chart.
examples/chart-line.pptx: a line chart with markers, gridlines, and a right legend.
examples/chart-pie.pptx: a pie chart with per-slice colours and percentage labels.
examples/chart-unsupported.pptx: a radar chart, which rhodopsin omits with one diagnostic.
"""
from pathlib import Path
import sys

from pptx import Presentation
from pptx.chart.data import CategoryChartData
from pptx.enum.chart import XL_CHART_TYPE, XL_LEGEND_POSITION, XL_LABEL_POSITION
from pptx.util import Inches, Pt

CATEGORIES = ['Q1', 'Q2', 'Q3', 'Q4']


def deck():
    presentation = Presentation()
    presentation.slide_width = Inches(10)
    presentation.slide_height = Inches(5.625)
    return presentation


def add(presentation, kind, series, title=None, legend=XL_LEGEND_POSITION.BOTTOM):
    slide = presentation.slides.add_slide(presentation.slide_layouts[6])
    data = CategoryChartData()
    data.categories = CATEGORIES
    for name, values in series:
        data.add_series(name, values)
    frame = slide.shapes.add_chart(kind, Inches(1), Inches(0.5), Inches(8), Inches(4.6), data)
    chart = frame.chart
    chart.has_title = title is not None
    if title:
        chart.chart_title.text_frame.text = title
    chart.has_legend = legend is not None
    if legend is not None:
        chart.legend.position = legend
        chart.legend.include_in_layout = False
    chart.font.size = Pt(12)
    return chart


def main(out: Path):
    bar = deck()
    chart = add(bar, XL_CHART_TYPE.COLUMN_CLUSTERED, [('East', (120, 135, 150, 170)), ('West', (90, 110, 95, 130))], 'Revenue')
    plot = chart.plots[0]
    plot.has_data_labels = True
    plot.data_labels.number_format = '#,##0'
    plot.data_labels.number_format_is_linked = False
    plot.data_labels.position = XL_LABEL_POSITION.OUTSIDE_END
    chart.value_axis.has_major_gridlines = True
    stacked = add(bar, XL_CHART_TYPE.BAR_STACKED, [('Product', (40, 55, 30, 60)), ('Service', (20, 25, 35, 30))], 'Mix')
    stacked.value_axis.has_major_gridlines = True
    bar.save(out / 'chart-bar.pptx')

    line = deck()
    chart = add(line, XL_CHART_TYPE.LINE_MARKERS, [('Plan', (100, 120, 140, 160)), ('Actual', (95, 125, 130, 175))], 'Trend', XL_LEGEND_POSITION.RIGHT)
    chart.value_axis.has_major_gridlines = True
    chart.value_axis.minimum_scale = 80
    chart.value_axis.maximum_scale = 180
    chart.value_axis.major_unit = 20
    line.save(out / 'chart-line.pptx')

    pie = deck()
    chart = add(pie, XL_CHART_TYPE.PIE, [('Share', (45, 25, 20, 10))], 'Share', XL_LEGEND_POSITION.RIGHT)
    plot = chart.plots[0]
    plot.has_data_labels = True
    plot.data_labels.show_percentage = True
    plot.data_labels.show_value = False
    plot.data_labels.number_format = '0%'
    plot.data_labels.number_format_is_linked = False
    pie.save(out / 'chart-pie.pptx')

    radar = deck()
    add(radar, XL_CHART_TYPE.RADAR, [('Score', (3, 4, 2, 5))], 'Radar')
    radar.save(out / 'chart-unsupported.pptx')


if __name__ == '__main__':
    main(Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parents[2] / 'examples')
