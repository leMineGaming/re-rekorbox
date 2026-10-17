"""Draws the program icon (a platter in the UI colours) and writes
ui/assets/icon.ico (16-256 px, for the .exe) and ui/assets/icon-64.rgba (raw RGBA for the window).

    python packaging/make_icon.py
"""
import os

from PIL import Image, ImageDraw

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, '..', 'ui', 'assets')


def draw(size):
    s = 4 * size  # draw large, scale down for smooth edges
    img = Image.new('RGBA', (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    r = s * 0.18
    d.rounded_rectangle([0, 0, s - 1, s - 1], radius=r, fill=(22, 24, 28, 255))
    c = s / 2
    # platter with grooves
    rad = s * 0.42
    d.ellipse([c - rad, c - rad, c + rad, c + rad], fill=(12, 12, 14, 255), outline=(0, 160, 255, 255), width=max(2, int(s * 0.035)))
    for k in range(4):
        g = rad * (0.82 - k * 0.11)
        d.ellipse([c - g, c - g, c + g, c + g], outline=(48, 52, 58, 255), width=max(1, int(s * 0.008)))
    # label and spindle
    lab = rad * 0.34
    d.ellipse([c - lab, c - lab, c + lab, c + lab], fill=(255, 130, 0, 255))
    sp = rad * 0.06
    d.ellipse([c - sp, c - sp, c + sp, c + sp], fill=(22, 24, 28, 255))
    # play-head marker
    w = s * 0.03
    d.rectangle([c - w / 2, c - rad - s * 0.02, c + w / 2, c - rad * 0.5], fill=(255, 255, 255, 255))
    return img.resize((size, size), Image.LANCZOS)


def main():
    os.makedirs(OUT, exist_ok=True)
    big = draw(256)
    big.save(os.path.join(OUT, 'icon.ico'), sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
    big.save(os.path.join(OUT, 'icon.png'))
    with open(os.path.join(OUT, 'icon-64.rgba'), 'wb') as f:
        f.write(draw(64).tobytes())
    print('written to', os.path.normpath(OUT))


if __name__ == '__main__':
    main()
