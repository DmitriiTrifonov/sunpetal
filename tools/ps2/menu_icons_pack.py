"""K-118: the walking menu icons (the case and the card, each a 32x32 + 8x32 pair of sprites) enlarged 4x for the 2D pack.

    python3 tools/ps2/menu_icons_pack.py

Reads the sprites dumped by `sunpetal-debug ... dump2d DIR` into build/k118/d2 (the keys are those of the EU ROM) and writes
build/k118/pack_<method>. The two parts of an icon are joined before the enlarging so that no seam shows, then cut apart again.
"""
import sys,os,subprocess,tempfile
os.chdir(os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', '..'))
sys.path.insert(0,'tools/ps2'); sys.path.insert(0,'tools')
import upscale
D='build/k118/d2/'; OUT='build/k118/pack'; os.makedirs(OUT,exist_ok=True)
pairs=[('obj1_32x32_c97cc7e54f8d96b6_255310a8ddb68a03.png','obj1_8x32_a217afc8b313d841_f27b6b8a99d571f5.png'),
       ('obj1_32x32_ded37d7f2f092376_17e222ba98519e07.png','obj1_8x32_0961b95598ea78c8_b2c05f2770bfaf12.png')]
for method in ('upscayl','lanczos-sharp'):
    for a,b in pairs:
        tmp=tempfile.mkdtemp(); comp=tmp+'/c.png'
        subprocess.run(['magick',D+a,D+b,'-background','none','+append',comp],check=True)
        big=upscale.enlarge(open(comp,'rb').read(),method,4,'magick',tmp,'never'); open(tmp+'/b.png','wb').write(big)
        d=f'{OUT}_{method}'; os.makedirs(d,exist_ok=True)
        subprocess.run(['magick',tmp+'/b.png','-crop','128x128+0+0','+repage',f'{d}/{a}'],check=True)
        subprocess.run(['magick',tmp+'/b.png','-crop','32x128+128+0','+repage',f'{d}/{b}'],check=True)
        print(method,a[:20])
