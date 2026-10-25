"""Original blue/white crop-corner mark, shared by macOS, Windows and the tray."""
from pathlib import Path
import struct
import zlib
root = Path(__file__).resolve().parent.parent
segments = [(494,324,324,324),(324,324,324,584),(530,700,700,700),(700,700,700,440)]
def render(size):
    rows=[]
    for y in range(size):
        row=bytearray()
        for x in range(size):
            fx=(x+.5)*1024/size;fy=(y+.5)*1024/size
            cx=min(max(fx,264),760);cy=min(max(fy,264),760)
            inside=64<=fx<960 and 64<=fy<960 and (fx-cx)**2+(fy-cy)**2<=200**2
            def stroke(ax,ay,bx,by):
                dx=bx-ax;dy=by-ay
                t=max(0,min(1,((fx-ax)*dx+(fy-ay)*dy)/(dx*dx+dy*dy)))
                return (fx-ax-t*dx)**2+(fy-ay-t*dy)**2<=35**2
            mark=inside and any(stroke(*s) for s in segments)
            row.extend((255,255,255,255) if mark else (23,105,224,255) if inside else (0,0,0,0))
        rows.append(b'\0'+row)
    def chunk(t,data):return struct.pack('>I',len(data))+t+data+struct.pack('>I',zlib.crc32(t+data)&0xffffffff)
    return b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',size,size,8,6,0,0,0))+chunk(b'IDAT',zlib.compress(b''.join(rows)))+chunk(b'IEND',b'')
(root/'assets/icons/app.png').write_bytes(render(1024))
small=render(256)
(root/'assets/icons/app.ico').write_bytes(struct.pack('<HHH',0,1,1)+struct.pack('<BBBBHHII',0,0,0,0,1,32,len(small),22)+small)
