"""Independent PSD/PSB fixtures for layer interchange and high-depth decoding."""
import argparse
import json
import os
from pathlib import Path
import struct
import sys
import zlib

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
import build_rs


def section(data, large=False):
    return struct.pack('>Q' if large else '>I', len(data)) + data


def channel(values, w, h, depth, compression, large=False):
    if depth == 8:
        raw = bytes(values)
    elif depth == 16:
        raw = struct.pack('>' + 'H' * len(values), *values)
    else:
        raw = struct.pack('>' + 'f' * len(values), *values)
    row_size = w * depth // 8
    if compression == 0:
        packed = raw
    elif compression == 1:
        rows = [bytes([row_size - 1]) + raw[y*row_size:(y+1)*row_size] for y in range(h)]
        packed = b''.join(struct.pack('>I' if large else '>H', len(row)) for row in rows) + b''.join(rows)
    elif compression == 2:
        packed = zlib.compress(raw)
    else:
        predicted = bytearray()
        for y in range(h):
            row = raw[y*row_size:(y+1)*row_size]
            if depth == 16:
                samples = struct.unpack('>' + 'H'*w, row)
                predicted += struct.pack('>' + 'H'*w, samples[0], *((samples[i]-samples[i-1]) % 65536 for i in range(1,w)))
            else:
                if depth == 32:
                    row = b''.join(row[c::4] for c in range(4))
                predicted += bytes([row[0]]) + bytes((row[i]-row[i-1]) % 256 for i in range(1,len(row)))
        packed = zlib.compress(predicted)
    return struct.pack('>H',compression) + packed


def header(w,h,depth,large=False):
    return b'8BPS'+struct.pack('>H6xHIIHH',2 if large else 1,4,h,w,depth,3)


def layer_record(name, rect, rgba, opacity=255, visible=True, compression=0, mask=None, extra=b'', depth=8, large=False):
    y,x,b,r = rect
    w,h = r-x,b-y
    rgba = [[v if depth==8 else v*257 if depth==16 else v/255.0 for v in plane] for plane in rgba]
    channels = [(-1, channel(rgba[3],w,h,depth,compression,large)),
                *[(c,channel(rgba[c],w,h,depth,compression,large)) for c in range(3)]]
    if mask is not None:
        channels.append((-2,channel(mask,w,h,8,compression)))
        mask_data = struct.pack('>4i2B2x',*rect,255,0)
    else:
        mask_data = b''
    encoded_name = name.encode('utf-16-be')
    extra = section(mask_data)+section(b'')+b'\x01X\0\0'+b'8BIMluni'+section(struct.pack('>I',len(encoded_name)//2)+encoded_name)+extra
    record = struct.pack('>4iH',*rect,len(channels))
    record += b''.join(struct.pack('>hQ' if large else '>hI',c,len(data)) for c,data in channels)
    record += b'8BIMnorm'+bytes([opacity,0,8 if visible else 10,0])+section(extra)
    return record, b''.join(data for _,data in channels)


def layered(compression=0, mask=False, unsupported=False, depth=8, large=False):
    extra = b'8BIMlsct'+section(struct.pack('>I',1)) if unsupported else b''
    top = layer_record('Ä Ebene 🖌',(1,1,3,3),[[0]*4,[0]*4,[255]*4,[128]*4],180,False,compression,
                       [0,255,128,255] if mask else None,extra,depth,large)
    bottom = layer_record('Untergrund',(0,0,3,4),[[255]*12,[0]*12,[0]*12,[255]*12],compression=compression,depth=depth,large=large)
    info = struct.pack('>h',-2)+top[0]+bottom[0]+top[1]+bottom[1]
    info += bytes(len(info)%2)
    # Visible composite is the red bottom layer; the blue top layer is hidden.
    if depth==8:
        layers=section(info,large)+bytes(4)
    else:
        layers=section(b'',large)+bytes(4)+b'8BIM'+(b'Lr16' if depth==16 else b'Lr32')+section(info,large)
    values=[255]*12+[0]*24+[255]*12
    values=[v if depth==8 else v*257 if depth==16 else v/255.0 for v in values]
    return header(4,3,depth,large)+bytes(8)+section(layers,large)+channel(values,4,12,depth,0,large)


def main():
    parser=argparse.ArgumentParser(__doc__)
    parser.add_argument('--out',default='build/paintcraft-next')
    parser.add_argument('--target',choices=['win','linux','both'],default='both')
    args=parser.parse_args()
    os.chdir(ROOT)
    out=Path(args.out).resolve()
    cases=out/'psd-check'
    cases.mkdir(parents=True,exist_ok=True)
    results=[]
    def rel(path): return path.relative_to(ROOT).as_posix()
    def run(target,name,data,script,extension="psd"):
        source=cases/(name+'.'+extension)
        source.write_bytes(data)
        test=cases/(name+'-'+target+'.uis')
        test.write_text('size 1280 800\ntheme light\n'+script,encoding='utf-8')
        exe=out/target/('PaintCraft.exe' if target=='win' else 'PaintCraft')
        r=build_rs.run_exe(target,exe,['--open',rel(source),'--config',rel(cases/('config-'+target)),
                                     '--script',rel(test),'--verify-dirty'],timeout=120)
        assert r.returncode==0,name+': '+r.stdout+r.stderr
        results.append(dict(target=target,case=name,passed=True))
    for target in (['win','linux'] if args.target=='both' else [args.target]):
        for compression in range(4):
            saved=cases/f'layers-out-{target}-{compression}.psd'
            saved.unlink(missing_ok=True)
            checks='''expect #9903 value 2
expect #9904 value 1
expect #9906 text "Ä Ebene 🖌"
expect #9906 value 0
expect #9907 value 180
cmd 901 65537
expect #9900 text "800000FF"
cmd 900 65537
expect #9900 text "FFFF0000"
cmd 339 0
expect #9906 text "Untergrund"
cmd 901 0
expect #9900 text "FFFF0000"
'''
            run(target,'layers-'+str(compression),layered(compression),checks+f'''cmd 103
click #81932
key ctrl+a
type "{saved.name}"
click button:"Speichern"
expect #9905 value 0
key ctrl+w
cmd 101
click #65548
key ctrl+a
type "{saved.name}"
click button:"Öffnen"
sync
'''+checks)
            exported=saved.read_bytes()
            rlen=struct.unpack_from('>I',exported,30)[0]
            base=34+rlen
            assert struct.unpack_from('>h',exported,base+8)[0]==-2
            assert 'Ä Ebene 🖌'.encode('utf-16-be') in exported
        run(target,'mask',layered(mask=True),'''expect #9903 value 2
expect #9905 value 1
cmd 901 65537
expect #9900 text "000000FF"
cmd 901 131073
expect #9900 text "400000FF"
''')
        run(target,'unsupported-group',layered(unsupported=True),'''expect #9903 value 1
expect #9905 value 1
cmd 901 0
expect #9900 text "FFFF0000"
''')
        for depth in (16,32):
            for large in (False,True):
                for compression in range(4):
                    values=([0,32768,65535] if depth==16 else [0.0,0.5,1.0])*8
                    # Four channels, two rows, three pixels; compression operates on all rows.
                    body=channel(values,3,8,depth,compression,large)
                    data=header(3,2,depth,large)+bytes(8)+(bytes(8) if large else bytes(4))+body
                    color='80808080' if depth==16 else '80BCBCBC'
                    run(target,f'depth-{depth}-{int(large)}-{compression}',data,f'''expect #9913 value 3
expect #9914 value 2
expect #9905 value 1
cmd 901 0
expect #9900 text "00000000"
cmd 901 1
expect #9900 text "{color}"
cmd 901 2
expect #9900 text "FFFFFFFF"
''')
        for depth in (16,32):
            for large in (False,True):
                for compression in range(4):
                    run(target,f'layer-depth-{depth}-{int(large)}-{compression}',layered(compression,depth=depth,large=large),checks+'''expect #9905 value 1
expect #9910 text ""
''')
        run(target,'psb-raster-layers',layered(large=True),checks+'''expect #9905 value 1
expect #9910 text ""
''')
        values=[-1.0,0.5,2.0,float('nan'),float('inf')]
        run(target,'hdr-float-conversion',header(5,1,32)+bytes(12)+channel(values*3+[1.0]*5,5,4,32,0),'''expect #9905 value 1
expect #9910 text ""
cmd 901 0
expect #9900 text "FF000000"
cmd 901 1
expect #9900 text "FFBCBCBC"
cmd 901 2
expect #9900 text "FFFFFFFF"
cmd 901 3
expect #9900 text "FF000000"
cmd 901 4
expect #9900 text "FFFFFFFF"
''')
        def chunk(tag,data): return struct.pack('>I',len(data))+tag+data+struct.pack('>I',zlib.crc32(tag+data))
        pixels=struct.pack('>12H',*([0,0,0,65535]+[32768]*3+[65535]+[65535]*4))
        png=b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',3,1,16,6,0,0,0))+chunk(b'IDAT',zlib.compress(b'\0'+pixels))+chunk(b'IEND',b'')
        run(target,'png16-source-protection',png,'''expect #9905 value 1
expect #9910 text ""
expect #9913 value 3
cmd 901 1
expect #9900 text "FF808080"
''',extension='png')
        malformed=layered()[:45]
        run(target,'truncated-layers',malformed,'expect #9913 value 800\nexpect #9903 value 1\n')
    (cases/'results.json').write_text(json.dumps(results,indent=2),encoding='utf-8')
    print(f'{len(results)} PSD regression cases passed')


if __name__=='__main__':
    main()
