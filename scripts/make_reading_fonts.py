# Static, trimmed reading fonts for PaperView: instance variable fonts at
# regular/bold, keep Latin + Greek + Cyrillic + punctuation, drop hinting.
import sys, os
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer
from fontTools import subset

src, out = sys.argv[1], sys.argv[2]
UNI = (list(range(0x20, 0x250)) + list(range(0x370, 0x400)) + list(range(0x400, 0x530))
       + list(range(0x1E00, 0x1F00)) + list(range(0x2000, 0x2070)) + list(range(0x20A0, 0x20C1))
       + list(range(0x2100, 0x2150)) + list(range(0x2150, 0x2190)) + [0x2212, 0x2022, 0x25CF, 0xFB00, 0xFB01, 0xFB02, 0xFB03, 0xFB04])

def trim(font, path):
    opts = subset.Options()
    opts.hinting = False
    opts.layout_features = ["kern", "liga"]
    opts.name_IDs = ["*"]
    opts.notdef_outline = True
    opts.glyph_names = False
    sub = subset.Subsetter(opts)
    sub.populate(unicodes=UNI)
    sub.subset(font)
    for t in ("DSIG", "STAT", "hdmx", "VDMX", "LTSH"):
        if t in font: del font[t]
    font.save(path)
    print(f"{os.path.getsize(path)//1024:5d} KB  {os.path.basename(path)}")

# (out name, regular var file, italic var file, extra axes)
VAR = [
    ("Literata", "Literata[opsz,wght].ttf", "Literata-Italic[opsz,wght].ttf", {"opsz": 12}),
    ("Merriweather", "Merriweather[opsz,wdth,wght].ttf", "Merriweather-Italic[opsz,wdth,wght].ttf", {"opsz": 12, "wdth": 100}),
    ("EBGaramond", "EBGaramond[wght].ttf", "EBGaramond-Italic[wght].ttf", {}),
    ("Lora", "Lora[wght].ttf", "Lora-Italic[wght].ttf", {}),
    ("SourceSerif4", "SourceSerif4[opsz,wght].ttf", "SourceSerif4-Italic[opsz,wght].ttf", {"opsz": 12}),
    ("NotoSans", "NotoSans[wdth,wght].ttf", "NotoSans-Italic[wdth,wght].ttf", {"wdth": 100}),
]
for name, reg, ita, extra in VAR:
    for file, italic in ((reg, False), (ita, True)):
        for wght, bold in ((400, False), (700, True)):
            f = TTFont(os.path.join(src, file))
            axes = {a.axisTag for a in f["fvar"].axes}
            loc = {"wght": wght, **{k: v for k, v in extra.items() if k in axes}}
            f = instancer.instantiateVariableFont(f, loc, updateFontNames=False)
            style = {(False, False): "Regular", (True, False): "Bold", (False, True): "Italic", (True, True): "BoldItalic"}[(bold, italic)]
            trim(f, os.path.join(out, f"{name}-{style}.ttf"))
STATIC = [("AtkinsonHyperlegible", "AtkinsonHyperlegible-{}.ttf", {"Regular": "Regular", "Bold": "Bold", "Italic": "Italic", "BoldItalic": "BoldItalic"}, "ttf"),
          ("OpenDyslexic", "OpenDyslexic-{}.otf", {"Regular": "Regular", "Bold": "Bold", "Italic": "Italic", "BoldItalic": "Bold-Italic"}, "otf")]
for name, pat, styles, ext in STATIC:
    for style, srcstyle in styles.items():
        f = TTFont(os.path.join(src, pat.format(srcstyle)))
        trim(f, os.path.join(out, f"{name}-{style}.{ext}"))
