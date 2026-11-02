import sys
from openpyxl import Workbook
from openpyxl.styles import Alignment, Font
from openpyxl.utils.indexed_list import IndexedList
F="Noto Sans JP"
b=Workbook(); b._fonts=IndexedList([Font(name=F,size=11,scheme=None)]); b._named_styles["Normal"].font=Font(name=F,size=11,scheme=None)
s=b.active; s.page_setup.orientation="portrait"; s.page_setup.paperSize=9
for k,v in dict(left=0.7,right=0.7,top=0.75,bottom=0.75,header=0.3,footer=0.3).items(): setattr(s.page_margins,k,v)
for c,w in zip("ABCD",(12,12,12,30)): s.column_dimensions[c].width=w
sizes=[8,9,10,11,12,14,16,18,20,24,28]
for i,z in enumerate(sizes, start=1):
    s.row_dimensions[i].height=48 if z<20 else 60
    for c,v in zip("ABC",("top","center","bottom")):
        x=s[f"{c}{i}"]; x.value="Hg"+str(z); x.font=Font(name=F,size=z,scheme=None); x.alignment=Alignment(vertical=v,horizontal=["left","center","right"]["ABC".index(c)])
    x=s[f"D{i}"]; x.value=1234567.891 if i%2 else "あいう ABC 123"; x.number_format="#,##0.00"; x.font=Font(name=F,size=z,scheme=None); x.alignment=Alignment(horizontal="right",vertical="bottom")
s.print_area=f"A1:D{len(sizes)}"
b.save(sys.argv[1])
