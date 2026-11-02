"""Generate the synthetic workbooks compared against Excel for Mac (issue #137).

Run with: uv run --with openpyxl==3.1.5 python3 make_fixtures.py OUT_DIR
Every font is named explicitly (no theme scheme), every number has an explicit
format code, and no cell holds a formula, so the workbooks stay inside the
first-class scope described in docs/xlsx.md.
"""

import datetime
import sys
from pathlib import Path

from openpyxl import Workbook
from openpyxl.styles import Alignment, Border, Color, Font, PatternFill, Side
from openpyxl.utils.indexed_list import IndexedList
from openpyxl.worksheet.pagebreak import Break

FAMILY = "Noto Sans JP"
FIXED = datetime.datetime(2026, 11, 2, 0, 0, 0)


def font(size=11, bold=False, color="FF000000"):
    return Font(name=FAMILY, size=size, bold=bold, color=color, scheme=None)


def workbook():
    book = Workbook()
    book.properties.created = FIXED
    book.properties.modified = FIXED
    book.properties.creator = "rhodopsin"
    # openpyxl's default font is Calibri with the minor theme scheme; name the
    # fixture family explicitly for the default cell format and the Normal style.
    book._fonts = IndexedList([font()])
    book._named_styles["Normal"].font = font()
    return book


def page(sheet, orientation="portrait", paper=9):
    sheet.page_setup.orientation = orientation
    sheet.page_setup.paperSize = paper
    sheet.page_margins.left = 0.7
    sheet.page_margins.right = 0.7
    sheet.page_margins.top = 0.75
    sheet.page_margins.bottom = 0.75
    sheet.page_margins.header = 0.3
    sheet.page_margins.footer = 0.3


THIN = Side(style="thin", color="FF000000")
MEDIUM = Side(style="medium", color="FF000000")
THICK = Side(style="thick", color="FF000000")


def quote(path):
    book = workbook()
    sheet = book.active
    sheet.title = "Quote"
    page(sheet)
    for column, width in zip("ABCD", (30, 10, 14, 16)):
        sheet.column_dimensions[column].width = width
    sheet.row_dimensions[1].height = 30
    sheet.merge_cells("A1:D1")
    title = sheet["A1"]
    title.value = "御見積書"
    title.font = font(16, bold=True)
    title.alignment = Alignment(horizontal="center", vertical="center")
    title.fill = PatternFill("solid", fgColor="FFDDEBF7")
    sheet["A3"] = "株式会社サンプル 御中"
    sheet["A3"].font = font(12)
    sheet["C3"] = "発行日"
    sheet["C3"].font = font()
    sheet["D3"] = datetime.date(2026, 11, 2)
    sheet["D3"].number_format = 'yyyy"年"m"月"d"日"'
    sheet["D3"].font = font()
    sheet["D3"].alignment = Alignment(horizontal="right")
    header = ("品名", "数量", "単価", "金額")
    for column, text in zip("ABCD", header):
        cell = sheet[f"{column}5"]
        cell.value = text
        cell.font = font(bold=True)
        cell.alignment = Alignment(horizontal="center", vertical="center")
        cell.fill = PatternFill("solid", fgColor="FFF2F2F2")
        cell.border = Border(left=THIN, right=THIN, top=THIN, bottom=THIN)
    items = (("コンサルティング業務", 1, 1200000), ("資料作成", 3, 85000), ("交通費 Travel", 2, 12345))
    for row, (name, quantity, price) in enumerate(items, start=6):
        values = (name, quantity, price, quantity * price)
        formats = ("@", "#,##0", "¥#,##0", "¥#,##0")
        for column, value, code in zip("ABCD", values, formats):
            cell = sheet[f"{column}{row}"]
            cell.value = value
            cell.number_format = code
            cell.font = font()
            cell.border = Border(left=THIN, right=THIN, top=THIN, bottom=THIN)
    sheet["A9"] = "合計"
    sheet["A9"].font = font(bold=True)
    sheet["A9"].alignment = Alignment(horizontal="right")
    sheet["D9"] = sum(q * p for _, q, p in items)
    sheet["D9"].number_format = "¥#,##0"
    sheet["D9"].font = font(bold=True)
    sheet["D9"].border = Border(left=THICK, right=THICK, top=THICK, bottom=THICK)
    for column in "ABC":
        sheet[f"{column}9"].border = Border(top=MEDIUM)
    sheet["D10"] = 0.1
    sheet["D10"].number_format = "0.0%"
    sheet["D10"].font = font()
    sheet["C10"] = "消費税率"
    sheet["C10"].font = font()
    sheet["A11"] = "お支払い条件：納品月末締め翌月末払い（銀行振込）"
    sheet["A11"].font = Font(name=FAMILY, size=10, color=Color(auto=True), scheme=None)
    sheet.row_dimensions[12].height = 24
    sheet["D12"] = "以上"
    sheet["D12"].font = font()
    sheet["D12"].alignment = Alignment(horizontal="right", vertical="bottom")
    sheet.print_area = "A1:D12"
    book.save(path)


def form(path):
    book = workbook()
    sheet = book.active
    sheet.title = "Form"
    page(sheet)
    for column, width in zip("ABC", (8, 40, 16)):
        sheet.column_dimensions[column].width = width
    for column, text in zip("ABC", ("No.", "項目", "金額")):
        cell = sheet[f"{column}1"]
        cell.value = text
        cell.font = font(bold=True)
        cell.fill = PatternFill("solid", fgColor="FFE2EFDA")
        cell.border = Border(bottom=THIN)
    for row in range(2, 41):
        number = row - 1
        sheet[f"A{row}"] = number
        sheet[f"A{row}"].number_format = "0"
        sheet[f"A{row}"].font = font()
        sheet[f"B{row}"] = f"明細 {number:02d} 行目"
        sheet[f"B{row}"].font = font()
        sheet[f"C{row}"] = number * 1000 + 0.5
        sheet[f"C{row}"].number_format = "#,##0.00"
        sheet[f"C{row}"].font = font()
    sheet.row_breaks.append(Break(id=20))
    sheet.print_title_rows = "1:1"
    sheet.print_area = "A1:C40"
    book.save(path)


def landscape(path):
    book = workbook()
    sheet = book.active
    sheet.title = "Wide"
    page(sheet, orientation="landscape")
    sheet.sheet_properties.pageSetUpPr.fitToPage = True
    sheet.page_setup.fitToWidth = 1
    sheet.page_setup.fitToHeight = 1
    columns = [chr(ord("A") + i) for i in range(14)]
    for column in columns:
        sheet.column_dimensions[column].width = 14
    for index, column in enumerate(columns):
        cell = sheet[f"{column}1"]
        cell.value = f"{index + 1}月"
        cell.font = font(bold=True)
        cell.alignment = Alignment(horizontal="center")
        cell.border = Border(bottom=MEDIUM)
    for row in range(2, 31):
        for index, column in enumerate(columns):
            cell = sheet[f"{column}{row}"]
            cell.value = (row * 37 + index * 101) * 10
            cell.number_format = "#,##0"
            cell.font = font()
    sheet.print_area = f"A1:{columns[-1]}30"
    book.save(path)


def main():
    out = Path(sys.argv[1])
    for name, build in (("quote", quote), ("form", form), ("landscape", landscape)):
        (out / name).mkdir(parents=True, exist_ok=True)
        build(out / name / "input.xlsx")


if __name__ == "__main__":
    main()
