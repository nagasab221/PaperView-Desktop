"""Generate a small 3-page test PDF for PaperView's render tests.

Pure-stdlib, uncompressed streams, hand-computed xref — no third-party deps.
Each page: large "Page N of 3" text, a distinctly colored rectangle, footer line.
Colors differ per channel so BGRA-vs-RGBA mixups are detectable in tests.

Extras exercised by tests:
- Page 3 has an offset CropBox (coordinate correctness on print-layout PDFs).
- A nested outline (Page 2 → child pointing at Page 3).
- Page 1 carries two link annotations: an internal /Dest to page 3 and a URI.
"""

import sys

N_PAGES = 3
COLORS = ["0.85 0.20 0.20", "0.20 0.55 0.25", "0.20 0.35 0.80"]  # red, green, blue
# Page 3 gets an offset CropBox: text/search coordinates must be reported
# relative to the *displayed* (bounding) box, not the raw MediaBox.
CROPS = [None, None, "[30 40 582 752]"]


def content_stream(i: int) -> bytes:
    color = COLORS[i]
    lines = (
        f"q {color} rg 72 560 240 100 re f Q\n"
        f"BT /F1 48 Tf 72 690 Td (Page {i + 1} of {N_PAGES}) Tj ET\n"
        f"BT /F1 14 Tf 72 520 Td (PaperView render-test fixture) Tj ET\n"
    )
    return lines.encode("ascii")


def main(path: str) -> None:
    page_ids = [4 + 2 * i for i in range(N_PAGES)]      # 4, 6, 8
    content_ids = [5 + 2 * i for i in range(N_PAGES)]   # 5, 7, 9
    OUTLINES, BM1, BM2, LINK_GOTO, LINK_URI = 10, 11, 12, 13, 14
    kids = " ".join(f"{pid} 0 R" for pid in page_ids)

    objs: list[tuple[int, bytes]] = [
        (1, f"<< /Type /Catalog /Pages 2 0 R /Outlines {OUTLINES} 0 R >>".encode()),
        (2, f"<< /Type /Pages /Kids [{kids}] /Count {N_PAGES} >>".encode()),
        (3, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"),
    ]
    for i in range(N_PAGES):
        crop = f"/CropBox {CROPS[i]} " if CROPS[i] else ""
        annots = f"/Annots [{LINK_GOTO} 0 R {LINK_URI} 0 R] " if i == 0 else ""
        page = (
            f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] {crop}{annots}"
            f"/Resources << /Font << /F1 3 0 R >> >> /Contents {content_ids[i]} 0 R >>"
        ).encode()
        stream = content_stream(i)
        content = b"<< /Length %d >>\nstream\n%s\nendstream" % (len(stream), stream)
        objs.append((page_ids[i], page))
        objs.append((content_ids[i], content))

    # Nested outline: "Page 2" (top level) with child "Page 3 section".
    objs.append((
        OUTLINES,
        f"<< /Type /Outlines /First {BM1} 0 R /Last {BM1} 0 R /Count 2 >>".encode(),
    ))
    objs.append((
        BM1,
        (
            f"<< /Title (Page 2) /Parent {OUTLINES} 0 R "
            f"/First {BM2} 0 R /Last {BM2} 0 R /Count 1 "
            f"/Dest [{page_ids[1]} 0 R /XYZ null null null] >>"
        ).encode(),
    ))
    objs.append((
        BM2,
        (
            f"<< /Title (Page 3 section) /Parent {BM1} 0 R "
            f"/Dest [{page_ids[2]} 0 R /XYZ null null null] >>"
        ).encode(),
    ))
    # Links on page 1: internal jump to page 3 over y 510–540 (raw page
    # space), and a URI link over y 470–500.
    objs.append((
        LINK_GOTO,
        (
            f"<< /Type /Annot /Subtype /Link /Rect [72 510 300 540] "
            f"/Border [0 0 0] /Dest [{page_ids[2]} 0 R /XYZ null null null] >>"
        ).encode(),
    ))
    objs.append((
        LINK_URI,
        (
            b"<< /Type /Annot /Subtype /Link /Rect [72 470 300 500] "
            b"/Border [0 0 0] /A << /S /URI /URI (https://example.com/paperview) >> >>"
        ),
    ))
    objs.sort(key=lambda t: t[0])

    out = bytearray(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n")
    offsets = {}
    for oid, body in objs:
        offsets[oid] = len(out)
        out += b"%d 0 obj\n" % oid
        out += body
        out += b"\nendobj\n"
    xref_pos = len(out)
    size = len(objs) + 1
    out += b"xref\n0 %d\n" % size
    out += b"0000000000 65535 f \n"
    for oid in range(1, size):
        out += b"%010d 00000 n \n" % offsets[oid]
    out += b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (size, xref_pos)

    with open(path, "wb") as fh:
        fh.write(bytes(out))
    print(f"wrote {path} ({len(out)} bytes)")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "sample.pdf")
