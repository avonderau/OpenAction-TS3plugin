#!/usr/bin/env python3
"""Draws the key and plugin icons into com.avonderau.ts3deck.sdPlugin/imgs.

Run from the repository root: python3 scripts/make-icons.py (needs Pillow).
Each image is written at 72 px (name.png) and 144 px (name@2x.png).
"""

from pathlib import Path

from PIL import Image, ImageDraw

OUT = Path(__file__).resolve().parent.parent / "com.avonderau.ts3deck.sdPlugin" / "imgs"

BG = (24, 27, 33, 255)
FG = (230, 233, 238, 255)
BLUE = (46, 134, 222, 255)
GREEN = (46, 204, 113, 255)
RED = (231, 76, 60, 255)
AMBER = (241, 196, 15, 255)
DIM = (120, 126, 136, 255)

S = 576  # draw big, then downscale for smooth edges


def canvas(bg=BG):
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.rounded_rectangle((0, 0, S - 1, S - 1), radius=72, fill=bg)
    return img, d


def mic(d, color, cx=S // 2, cy=230, scale=1.0):
    w, h = int(90 * scale), int(160 * scale)
    d.rounded_rectangle((cx - w // 2, cy - h // 2, cx + w // 2, cy + h // 2), radius=w // 2, fill=color)
    r = int(95 * scale)
    lw = int(22 * scale)
    d.arc((cx - r, cy - r + int(20 * scale), cx + r, cy + r + int(20 * scale)), 20, 160, fill=color, width=lw)
    d.line((cx, cy + r + int(20 * scale), cx, cy + r + int(70 * scale)), fill=color, width=lw)
    d.line((cx - int(55 * scale), cy + r + int(70 * scale), cx + int(55 * scale), cy + r + int(70 * scale)), fill=color, width=lw)


def slash(d, color=RED):
    d.line((150, 120, 426, 440), fill=color, width=36)


def speaker(d, color):
    d.polygon([(150, 230), (220, 230), (310, 150), (310, 420), (220, 340), (150, 340)], fill=color)
    for r in (70, 130):
        d.arc((310 - r, 285 - r, 310 + r, 285 + r), -50, 50, fill=color, width=26)


def moon(d, color, bg=BG):
    d.ellipse((150, 120, 430, 400), fill=color)
    d.ellipse((230, 90, 480, 340), fill=bg)


def dot(d, color):
    d.ellipse((190, 150, 390, 350), fill=color)


def waves(d, color):
    for r in (150, 200):
        d.arc((288 - r, 240 - r, 288 + r, 240 + r), -40, 40, fill=color, width=20)
        d.arc((288 - r, 240 - r, 288 + r, 240 + r), 140, 220, fill=color, width=20)


def hashsign(d, color):
    for x in (230, 340):
        d.line((x + 20, 130, x - 20, 400), fill=color, width=34)
    for y in (210, 320):
        d.line((150, y, 430, y), fill=color, width=34)


def people(d, color):
    for cx, cy, r in ((210, 200, 55), (366, 200, 55)):
        d.ellipse((cx - r, cy - r, cx + r, cy + r), fill=color)
        d.pieslice((cx - 95, cy + 70, cx + 95, cy + 260), 180, 360, fill=color)


def save(name, img):
    OUT.mkdir(parents=True, exist_ok=True)
    img.resize((144, 144), Image.LANCZOS).save(OUT / f"{name}@2x.png")
    img.resize((72, 72), Image.LANCZOS).save(OUT / f"{name}.png")


def main():
    img, d = canvas(); mic(d, FG); save("mic-on", img)
    img, d = canvas(); mic(d, DIM); slash(d); save("mic-off", img)
    img, d = canvas(); speaker(d, FG); save("speakers-on", img)
    img, d = canvas(); speaker(d, DIM); slash(d); save("speakers-off", img)
    img, d = canvas(); dot(d, GREEN); save("away-off", img)
    img, d = canvas(); moon(d, AMBER); save("away-on", img)
    img, d = canvas(); mic(d, DIM, scale=0.8); save("ptt-idle", img)
    img, d = canvas(); mic(d, GREEN, scale=0.8); waves(d, GREEN); save("ptt-live", img)
    img, d = canvas(); hashsign(d, FG); save("channel-off", img)
    img, d = canvas(); hashsign(d, BLUE); save("channel-on", img)
    img, d = canvas(); people(d, FG); save("status-idle", img)
    img, d = canvas(); people(d, GREEN); save("status-talking", img)
    img, d = canvas(BLUE); mic(d, FG); save("plugin", img)


if __name__ == "__main__":
    main()
