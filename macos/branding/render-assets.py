"""Render the Yas Mail SVG identity. Requires PyMuPDF, Pillow, and installed Dubai fonts."""
import argparse
import io
import os
from pathlib import Path
import shutil
import struct
import pymupdf
from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parent
PROJECT = ROOT.parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--font', type=Path, default=Path(os.environ.get('WINDIR', 'C:/Windows')) / 'Fonts/DUBAI-BOLD.TTF')
parser.add_argument('--regular-font', type=Path, default=Path(os.environ.get('WINDIR', 'C:/Windows')) / 'Fonts/DUBAI-REGULAR.TTF')
args = parser.parse_args()
if not args.font.is_file():
    raise SystemExit('Dubai Bold is not installed; pass --font with an installed font path.')
regular_font = args.regular_font if args.regular_font.is_file() else args.font
source = (ROOT / 'yas-mail-symbol.svg').read_text(encoding='utf-8')

def raster(size):
    document = pymupdf.open(stream=source.encode('utf-8'), filetype='svg')
    pixmap = document[0].get_pixmap(matrix=pymupdf.Matrix(size / 128, size / 128), alpha=True)
    return Image.open(io.BytesIO(pixmap.tobytes('png'))).convert('RGBA')

master = raster(1024)
master.save(ROOT / 'yas-mail-logo.png')
shutil.copyfile(ROOT / 'yas-mail-symbol.svg', ROOT / 'yas-mail-logo.svg')
shutil.copyfile(ROOT / 'yas-mail-symbol.svg', PROJECT / 'ui/logo.svg')
icon_root = PROJECT / 'src-tauri/icons'
icon_root.mkdir(parents=True, exist_ok=True)
entries = []
for size in (16, 20, 24, 32, 40, 48, 64, 128, 256):
    bitmap = master.resize((size, size), Image.Resampling.LANCZOS)
    # Resampling can leave trace alpha at pixels outside the rounded tile.
    for corner in ((0, 0), (size-1, 0), (0, size-1), (size-1, size-1)):
        bitmap.putpixel(corner, (0, 0, 0, 0))
    stream = io.BytesIO()
    bitmap.save(stream, format='PNG')
    entries.append((size, stream.getvalue()))
    if size == 32:
        bitmap.save(icon_root / '32x32.png')
    elif size == 128:
        bitmap.save(icon_root / '128x128.png')
    elif size == 256:
        bitmap.save(icon_root / '128x128@2x.png')
        bitmap.save(ROOT / 'yas-mail-icon.png')
offset = 6 + 16 * len(entries)
directory = bytearray()
for size, data in entries:
    encoded = 0 if size == 256 else size
    directory.extend(struct.pack('<BBBBHHII', encoded, encoded, 0, 0, 1, 32, len(data), offset))
    offset += len(data)
ico = struct.pack('<HHH', 0, 1, len(entries)) + bytes(directory) + b''.join(data for _, data in entries)
(icon_root / 'icon.ico').write_bytes(ico)
(ROOT / 'yas-mail.ico').write_bytes(ico)

font = ImageFont.truetype(str(args.font), 76)
view_width = int(155 + font.getlength('Yas Mail') + 14)
inner = source[source.index('<rect'):source.rindex('</svg>')]
wordmark = f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {view_width} 128" role="img" aria-label="Yas Mail"><title>Yas Mail</title>{inner}<text x="155" y="87" fill="#192C49" font-family="Dubai, Segoe UI, sans-serif" font-size="76" font-weight="700">Yas Mail</text></svg>'
(ROOT / 'yas-mail-wordmark.svg').write_text(wordmark, encoding='utf-8')
scale = 3
wordmark_image = Image.new('RGBA', (view_width * scale, 128 * scale), (0, 0, 0, 0))
wordmark_image.alpha_composite(raster(128 * scale), (0, 0))
ImageDraw.Draw(wordmark_image).text((155 * scale, 87 * scale), 'Yas Mail', font=ImageFont.truetype(str(args.font), 76 * scale), fill='#192C49', anchor='ls')
wordmark_image.resize((view_width * 2, 256), Image.Resampling.LANCZOS).save(ROOT / 'yas-mail-wordmark.png')

board = Image.new('RGB', (1200, 780), '#F7F9FC')
draw = ImageDraw.Draw(board)
draw.text((50, 30), 'Yas Mail', font=ImageFont.truetype(str(args.font), 40), fill='#192C49')
draw.text((50, 88), 'Wing Letter · Mail, made clear.', font=ImageFont.truetype(str(regular_font), 24), fill='#5F708A')
large = raster(380)
board.paste(large, (60, 170), large)
preview = wordmark_image.resize((int(view_width * 1.38), 177), Image.Resampling.LANCZOS)
board.paste(preview, (500, 190), preview)
draw.text((500, 400), 'Royal blue  #2563EB', font=ImageFont.truetype(str(args.font), 26), fill='#192C49')
draw.text((500, 444), 'White envelope · swept wing · sky-blue tail', font=ImageFont.truetype(str(regular_font), 22), fill='#5F708A')
draw.rounded_rectangle((40, 602, 1160, 748), radius=18, fill='#192C49')
x = 85
for size in (16, 20, 24, 32, 48, 64, 96):
    icon = master.resize((size, size), Image.Resampling.LANCZOS)
    board.paste(icon, (x, 620 + (96-size)//2), icon)
    draw.text((x, 719), str(size), font=ImageFont.truetype(str(regular_font), 16), fill='#FFFFFF')
    x += size + 55
board.save(ROOT / 'yas-mail-brand-preview.png')
print('Rendered Yas Mail Wing Letter: transparent master, 9 Windows icon sizes, wordmark, and preview.')
