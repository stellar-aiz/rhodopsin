import sys
from openpyxl import Workbook
from openpyxl.styles import Font, Alignment
from openpyxl.utils.indexed_list import IndexedList
F="Noto Sans JP"
b=Workbook(); b._fonts=IndexedList([Font(name=F,size=11,scheme=None)]); b._named_styles["Normal"].font=Font(name=F,size=11,scheme=None)
s=b.active; s.page_setup.orientation="portrait"; s.page_setup.paperSize=9
for k,v in dict(left=0.7,right=0.7,top=0.75,bottom=0.75,header=0.3,footer=0.3).items(): setattr(s.page_margins,k,v)
sizes=list(range(6,41))
for i,z in enumerate(sizes, start=1):
    x=s[f"A{i}"]; x.value="Hg"; x.font=Font(name=F,size=z,scheme=None)
    x=s[f"B{i}"]; x.value="Hg"; x.font=Font(name=F,size=z,scheme=None); x.alignment=Alignment(vertical="top")
s.print_area=f"A1:B{len(sizes)}"
b.save(sys.argv[1])
