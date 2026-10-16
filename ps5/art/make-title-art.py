#!/usr/bin/env python3
# Azeroth Survivors for the PlayStation 5 - the title's home-screen art, drawn here.
#
#   python3 ps5/art/make-title-art.py [FONT.ttf]
#
#   ps5/sce_sys/icon0.png   the home-screen tile, 512x512
#   ps5/art/pic.png         the background behind the title and its start-up splash, 3840x2160,
#                           turned into pic0.dds and pic1.dds by ps5/tools/png-to-title-dds.py
#
# Nothing in either picture is from the game: a dusk sky, a ring of foes' eyes closing in on a
# lone glow, and the title. Needs Pillow and numpy.
#
# SPDX-License-Identifier: MIT
import math
import random
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont

ps5 = Path(__file__).resolve().parent.parent
FONTS = [
    "/usr/share/fonts/truetype/dejavu/DejaVuSerif-Bold.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSerif-Bold.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
]


def font(size):
    for path in sys.argv[1:] + FONTS:
        if Path(path).is_file():
            return ImageFont.truetype(path, size)
    return ImageFont.load_default(size)


def scene(w, h, title_size, subtitle):
    rng = random.Random(1236)
    img = Image.new("RGB", (w, h))
    px = img.load()
    top, mid, bottom = (12, 10, 34), (70, 28, 60), (190, 92, 40)
    for y in range(h):
        t = y / (h - 1)
        a, b, k = (top, mid, t / 0.6) if t < 0.6 else (mid, bottom, (t - 0.6) / 0.4)
        row = tuple(int(a[i] + (b[i] - a[i]) * k) for i in range(3))
        for x in range(w):
            px[x, y] = row
    d = ImageDraw.Draw(img)
    # Stars in the upper sky.
    for _ in range(int(w * h / 9000)):
        x, y = rng.randrange(w), rng.randrange(int(h * 0.55))
        r = max(1, w // 1600) * rng.choice([1, 1, 1, 2])
        c = rng.randrange(150, 255)
        d.ellipse((x - r, y - r, x + r, y + r), fill=(c, c, min(255, c + 30)))
    # The hills, dark against the dusk.
    ground = int(h * 0.78)
    for layer, (shade, amp, freq) in enumerate([((40, 18, 40), 0.06, 3.0), ((18, 8, 22), 0.04, 5.0)]):
        pts = [(0, h)]
        for x in range(0, w + 1, max(1, w // 200)):
            y = ground + layer * h * 0.05 - amp * h * (math.sin(x / w * freq * math.pi + layer) * 0.5 + 0.5)
            pts.append((x, y))
        pts.append((w, h))
        d.polygon(pts, fill=shade)
    # The hero's glow, and the eyes of what surrounds it.
    cx, cy = w // 2, int(h * 0.80)
    glow = Image.new("L", (w, h), 0)
    ImageDraw.Draw(glow).ellipse((cx - w // 9, cy - w // 9, cx + w // 9, cy + w // 9), fill=255)
    glow = glow.filter(ImageFilter.GaussianBlur(w // 25))
    img = Image.composite(Image.new("RGB", (w, h), (255, 200, 110)), img, glow.point(lambda v: v * 0.75))
    d = ImageDraw.Draw(img)
    for i in range(26):
        ang = i / 26 * 2 * math.pi + rng.uniform(-0.1, 0.1)
        dist = rng.uniform(0.18, 0.34) * w
        ex, ey = cx + math.cos(ang) * dist, cy + math.sin(ang) * dist * 0.35
        if ey > h * 0.98:
            continue
        s = w / 900
        for off in (-3 * s, 3 * s):
            d.ellipse((ex + off - 1.6 * s, ey - 1.0 * s, ex + off + 1.6 * s, ey + 1.0 * s), fill=(255, 60, 30))
    # The title.
    f = font(title_size)
    text = "AZEROTH\nSURVIVORS"
    box = d.multiline_textbbox((0, 0), text, font=f, align="center", spacing=title_size // 6)
    tx, ty = (w - (box[2] - box[0])) // 2, int(h * 0.12)
    shadow = Image.new("L", (w, h), 0)
    ImageDraw.Draw(shadow).multiline_text((tx, ty), text, font=f, fill=255, align="center", spacing=title_size // 6)
    shadow = shadow.filter(ImageFilter.GaussianBlur(title_size // 10))
    img = Image.composite(Image.new("RGB", (w, h), (0, 0, 0)), img, shadow)
    d = ImageDraw.Draw(img)
    d.multiline_text((tx, ty), text, font=f, fill=(250, 214, 120), align="center",
                     spacing=title_size // 6, stroke_width=max(2, title_size // 28), stroke_fill=(70, 30, 10))
    if subtitle:
        sf = font(title_size // 4)
        sb = d.textbbox((0, 0), subtitle, font=sf)
        d.text(((w - (sb[2] - sb[0])) // 2, ty + box[3] + title_size // 3), subtitle, font=sf,
               fill=(235, 220, 200))
    return img


scene(512, 512, 70, None).save(ps5 / "sce_sys" / "icon0.png")
scene(3840, 2160, 300, "a survivors roguelite in the old world").save(ps5 / "art" / "pic.png")
print("wrote", ps5 / "sce_sys" / "icon0.png", "and", ps5 / "art" / "pic.png")
