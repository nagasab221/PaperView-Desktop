"""Generate PaperView's app icon.

Design: a soft paper-white page with a folded corner in the app's chip-blue,
three muted text bars, and a blue "PDF" badge overlapping the left edge —
the classic PDF-icon language, decluttered, in PaperView's palette.

Outputs (into assets/):
- paperview.ico   multi-size Windows icon (16..256), embedded into the exe
- icon_128.rgba   raw RGBA bytes, embedded as the runtime window icon
- icon_preview.png  256px preview for humans

Run:  py scripts/make_icon.py
"""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont

S = 1024  # master canvas

# Palette — matches the app's dark-theme chrome and its selection blue.
PAGE = (238, 242, 246, 255)        # paper
PAGE_EDGE = (183, 192, 204, 255)   # subtle rim
BARS = (194, 203, 216, 255)        # text hint
BLUE = (45, 125, 210, 255)         # app chip-blue (badge)
BLUE_DARK = (31, 95, 168, 255)     # fold flap
SHADOW = (0, 0, 0, 60)

PAGE_L, PAGE_T, PAGE_R, PAGE_B = 232, 64, 824, 960
FOLD = 210
RADIUS = 72


def page_mask() -> Image.Image:
    """Rounded page silhouette with the top-right corner cut off."""
    m = Image.new("L", (S, S), 0)
    d = ImageDraw.Draw(m)
    d.rounded_rectangle((PAGE_L, PAGE_T, PAGE_R, PAGE_B), RADIUS, fill=255)
    # Cut the fold corner (overshoot outward so the rounding is removed too).
    d.polygon(
        [
            (PAGE_R - FOLD, PAGE_T - 8),
            (PAGE_R + 8, PAGE_T - 8),
            (PAGE_R + 8, PAGE_T + FOLD),
        ],
        fill=0,
    )
    return m


def build() -> Image.Image:
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))

    # Soft drop shadow so the light page reads on any background.
    sh = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    shm = page_mask()
    sh.paste(Image.new("RGBA", (S, S), SHADOW), (0, 18), shm)
    sh = sh.filter(ImageFilter.GaussianBlur(18))
    img = Image.alpha_composite(img, sh)

    # Page (with a hairline rim: edge color under, page color inset over).
    mask = page_mask()
    rim = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    rim.paste(Image.new("RGBA", (S, S), PAGE_EDGE), (0, 0), mask)
    img = Image.alpha_composite(img, rim)
    inset = mask.filter(ImageFilter.MinFilter(7))  # shrink ~3px
    page = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    page.paste(Image.new("RGBA", (S, S), PAGE), (0, 0), inset)
    img = Image.alpha_composite(img, page)

    d = ImageDraw.Draw(img)

    # Folded flap: triangle tucked into the cut corner.
    d.polygon(
        [
            (PAGE_R - FOLD, PAGE_T),
            (PAGE_R - FOLD, PAGE_T + FOLD),
            (PAGE_R, PAGE_T + FOLD),
        ],
        fill=BLUE_DARK,
    )

    # Text hint bars.
    bar_x0, bar_x1 = 320, 660
    for i, (y, x1) in enumerate([(330, bar_x1), (424, bar_x1), (518, 560)]):
        d.rounded_rectangle((bar_x0, y, x1, y + 46), 23, fill=BARS)

    # Badge, overlapping the page's left edge.
    bx0, by0, bx1, by1 = 152, 632, 700, 838
    badge_shadow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    bd = ImageDraw.Draw(badge_shadow)
    bd.rounded_rectangle((bx0, by0 + 12, bx1, by1 + 12), 56, fill=(0, 0, 0, 70))
    img = Image.alpha_composite(img, badge_shadow.filter(ImageFilter.GaussianBlur(12)))
    d = ImageDraw.Draw(img)
    d.rounded_rectangle((bx0, by0, bx1, by1), 56, fill=BLUE)

    # "PDF" in a solid geometric bold.
    font = None
    for cand in (r"C:\Windows\Fonts\segoeuib.ttf", r"C:\Windows\Fonts\arialbd.ttf"):
        try:
            font = ImageFont.truetype(cand, 148)
            break
        except OSError:
            continue
    if font is not None:
        text = "PDF"
        tb = d.textbbox((0, 0), text, font=font)
        tw, th = tb[2] - tb[0], tb[3] - tb[1]
        tx = bx0 + ((bx1 - bx0) - tw) / 2 - tb[0]
        ty = by0 + ((by1 - by0) - th) / 2 - tb[1]
        d.text((tx, ty), text, font=font, fill=(255, 255, 255, 255))

    return img


def main() -> None:
    out_dir = Path(__file__).resolve().parent.parent / "assets"
    out_dir.mkdir(exist_ok=True)
    master = build()

    sizes = [16, 24, 32, 48, 64, 128, 256]
    frames = [master.resize((s, s), Image.LANCZOS) for s in sizes]
    ico = out_dir / "paperview.ico"
    frames[-1].save(ico, format="ICO", sizes=[(s, s) for s in sizes])
    print(f"wrote {ico}")

    icon128 = master.resize((128, 128), Image.LANCZOS)
    (out_dir / "icon_128.rgba").write_bytes(icon128.tobytes())
    print(f"wrote {out_dir / 'icon_128.rgba'} ({128 * 128 * 4} bytes)")

    preview = master.resize((256, 256), Image.LANCZOS)
    preview.save(out_dir / "icon_preview.png")
    print(f"wrote {out_dir / 'icon_preview.png'}")


if __name__ == "__main__":
    main()
