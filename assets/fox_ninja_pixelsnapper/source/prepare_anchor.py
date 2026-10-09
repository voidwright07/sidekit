from pathlib import Path
from PIL import Image, ImageDraw
import json

ROOT = Path(__file__).resolve().parent.parent
im = Image.open(ROOT/'reference/anchor-snapped-40.png').convert('RGBA')
im.putalpha(im.getchannel('A').point(lambda a: 255 if a >= 128 else 0))
bounds = im.getbbox()
cropped = im.crop(bounds)
anchor = Image.new('RGBA', (32, 32))
anchor.alpha_composite(cropped, (1, 31-cropped.height))
anchor.save(ROOT/'reference/anchor-32.png')
anchor.resize((1024,1024), Image.Resampling.NEAREST).save(ROOT/'reference/anchor-1024.png')

# Same pixel-scale reference idea as the linked video: 4 by 3 pose regions,
# each 512x512, each region carrying a 32x32 logical checkerboard.
guide=Image.new('RGB',(2048,1536),'white')
d=ImageDraw.Draw(guide)
for y in range(0,1536,16):
    for x in range(0,2048,16):
        if (x//16+y//16)%2==0: d.rectangle((x,y,x+15,y+15),fill='#171717')
for x in range(0,2049,512):d.line((x,0,x,1535),fill='#a0a0a0',width=2)
for y in range(0,1537,512):d.line((0,y,2047,y),fill='#a0a0a0',width=2)
guide.save(ROOT/'reference/pose-board-guide.png')
colours = sorted({p[:3] for p in anchor.getdata() if p[3]})
(ROOT/'source/palette.json').write_text(json.dumps(colours,indent=2)+'\n')
(ROOT/'source/anchor-settings.json').write_text(json.dumps({'tool':'Sprite Fusion Pixel Snapper','version':'1.0.0','pixelSize':40,'colors':16,'input':'reference/anchor-generated.png','snapOutput':'reference/anchor-snapped-40.png','snapDimensions':[31,32],'crop':bounds,'placement':[1,31-cropped.height],'finalDimensions':[32,32],'alphaThreshold':128},indent=2)+'\n')
print('Prepared 32x32 snapped anchor, 1024x1024 nearest-neighbor reference and 2048x1536 pose guide.')
