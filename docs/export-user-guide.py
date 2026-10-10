#!/usr/bin/env python3
"""Turn docs/UserGuide.md into docs/UserGuide.pdf.

The Markdown file is the master. This script is the print export: letter
paper, a title page, a numbered table of contents, and a running header.
It understands the headings, lists, tables, notes, and fenced commands
used in that master. It is not a general Markdown engine.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

from reportlab.lib import colors
from reportlab.lib.enums import TA_CENTER, TA_JUSTIFY, TA_LEFT, TA_RIGHT
from reportlab.lib.pagesizes import letter
from reportlab.lib.styles import ParagraphStyle
from reportlab.lib.units import inch
from reportlab.platypus import (
    BaseDocTemplate,
    CondPageBreak,
    Frame,
    HRFlowable,
    KeepTogether,
    ListFlowable,
    ListItem,
    NextPageTemplate,
    PageBreak,
    PageTemplate,
    Paragraph,
    Preformatted,
    Spacer,
    Table,
    TableStyle,
)

ROOT = Path(__file__).resolve().parent
MASTER = ROOT / "UserGuide.md"
OUTPUT = ROOT / "UserGuide.pdf"

PAGE_W, PAGE_H = letter
LEFT = 0.85 * inch
RIGHT = 0.75 * inch
TOP = 0.85 * inch
BOTTOM = 0.72 * inch

INK = colors.HexColor("#1c1c1c")
RULE = colors.HexColor("#1c1c1c")
RULE_LIGHT = colors.HexColor("#b8b8b8")
NOTE_BG = colors.HexColor("#f4f1e8")
CODE_BG = colors.HexColor("#f3f3f3")
HEADER = colors.HexColor("#333333")


def ascii_punct(text: str) -> str:
    """Standard PDF fonts have no em dash and no typographic quotes."""
    return (
        text.replace("\u2014", " - ")
        .replace("\u2013", "-")
        .replace("\u2018", "'")
        .replace("\u2019", "'")
        .replace("\u201c", '"')
        .replace("\u201d", '"')
        .replace("\u00a0", " ")
    )


def _xml(text: str) -> str:
    return text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def inline(text: str) -> str:
    """Code spans are protected so a `*` inside them is not an italic mark."""
    text = ascii_punct(text)
    parts: list[str] = []
    pos = 0
    for match in re.finditer(r"`([^`]+)`", text):
        parts.append(_markup(_xml(text[pos : match.start()])))
        parts.append(f'<font face="Courier" size="9">{_xml(match.group(1))}</font>')
        pos = match.end()
    parts.append(_markup(_xml(text[pos:])))
    return "".join(parts)


def _markup(text: str) -> str:
    text = re.sub(r"\*\*([^*]+)\*\*", r"<b>\1</b>", text)
    text = re.sub(r"(?<!\*)\*([^*]+)\*(?!\*)", r"<i>\1</i>", text)
    return text


def styles() -> dict[str, ParagraphStyle]:
    base = ParagraphStyle(
        "Body",
        fontName="Times-Roman",
        fontSize=11,
        leading=14,
        textColor=INK,
        alignment=TA_JUSTIFY,
        spaceAfter=8,
    )
    return {
        "cover_kicker": ParagraphStyle(
            "CoverKicker",
            fontName="Times-Bold",
            fontSize=12,
            leading=15,
            alignment=TA_CENTER,
            textColor=INK,
            spaceAfter=6,
            tracking=1.2,
        ),
        "cover_title": ParagraphStyle(
            "CoverTitle",
            fontName="Times-Bold",
            fontSize=28,
            leading=32,
            alignment=TA_CENTER,
            textColor=INK,
            spaceAfter=8,
        ),
        "cover_sub": ParagraphStyle(
            "CoverSub",
            fontName="Times-Roman",
            fontSize=16,
            leading=20,
            alignment=TA_CENTER,
            textColor=INK,
            spaceBefore=4,
            spaceAfter=4,
        ),
        "cover_meta": ParagraphStyle(
            "CoverMeta",
            fontName="Times-Roman",
            fontSize=12,
            leading=16,
            alignment=TA_CENTER,
            textColor=INK,
            spaceBefore=2,
        ),
        "toc_title": ParagraphStyle(
            "TocTitle",
            fontName="Times-Bold",
            fontSize=18,
            leading=22,
            alignment=TA_CENTER,
            textColor=INK,
            spaceAfter=14,
        ),
        "toc_chapter": ParagraphStyle(
            "TocChapter",
            fontName="Times-Bold",
            fontSize=11,
            leading=16,
            textColor=INK,
        ),
        "chapter": ParagraphStyle(
            "Chapter",
            fontName="Times-Bold",
            fontSize=18,
            leading=22,
            textColor=INK,
            spaceBefore=0,
            spaceAfter=12,
        ),
        "section": ParagraphStyle(
            "Section",
            fontName="Times-Bold",
            fontSize=13,
            leading=16,
            textColor=INK,
            spaceBefore=12,
            spaceAfter=6,
        ),
        "body": base,
        "note": ParagraphStyle(
            "Note",
            parent=base,
            fontName="Times-Italic",
            fontSize=10.5,
            leading=13.5,
            alignment=TA_LEFT,
            leftIndent=4,
            rightIndent=4,
            spaceBefore=2,
            spaceAfter=2,
        ),
        "bullet": ParagraphStyle(
            "BulletBody",
            parent=base,
            alignment=TA_LEFT,
            leftIndent=0,
            spaceAfter=2,
        ),
        "code": ParagraphStyle(
            "Code",
            fontName="Courier",
            fontSize=8.5,
            leading=11,
            textColor=INK,
            alignment=TA_LEFT,
            leftIndent=6,
            rightIndent=6,
        ),
        "table": ParagraphStyle(
            "TableCell",
            fontName="Times-Roman",
            fontSize=9,
            leading=11.5,
            textColor=INK,
            alignment=TA_LEFT,
        ),
        "table_head": ParagraphStyle(
            "TableHead",
            fontName="Times-Bold",
            fontSize=9,
            leading=11.5,
            textColor=INK,
            alignment=TA_LEFT,
        ),
        "footer": ParagraphStyle(
            "Footer",
            fontName="Times-Roman",
            fontSize=9,
            leading=11,
            textColor=HEADER,
        ),
    }


class GuideDoc(BaseDocTemplate):
    def __init__(self, path: Path, story_styles: dict[str, ParagraphStyle]):
        super().__init__(
            str(path),
            pagesize=letter,
            title="Hamspot Commander User Guide",
            author="Hamspot",
            subject="Operations manual for Hamspot Commander 0.6.6",
        )
        self.guide_styles = story_styles
        self.chapter = ""
        frame = Frame(
            LEFT,
            BOTTOM,
            PAGE_W - LEFT - RIGHT,
            PAGE_H - TOP - BOTTOM,
            id="body",
            showBoundary=0,
        )
        cover_frame = Frame(
            LEFT,
            BOTTOM,
            PAGE_W - LEFT - RIGHT,
            PAGE_H - 0.7 * inch - BOTTOM,
            id="cover",
            showBoundary=0,
        )
        self.addPageTemplates(
            [
                PageTemplate(id="cover", frames=[cover_frame], onPage=self.draw_cover),
                PageTemplate(
                    id="body",
                    frames=[frame],
                    onPage=self.draw_header,
                    onPageEnd=self.draw_footer,
                ),
            ]
        )

    def afterFlowable(self, flowable) -> None:
        if isinstance(flowable, Paragraph) and flowable.style.name == "Chapter":
            text = flowable.getPlainText()
            self.chapter = text
            self.canv.bookmarkPage(text)
            self.canv.addOutlineEntry(text, text, level=0, closed=False)

    def draw_cover(self, canv, doc) -> None:
        canv.saveState()
        canv.setStrokeColor(RULE)
        canv.setLineWidth(1.25)
        canv.line(LEFT, PAGE_H - 0.55 * inch, PAGE_W - RIGHT, PAGE_H - 0.55 * inch)
        canv.setLineWidth(0.4)
        canv.line(LEFT, 0.48 * inch, PAGE_W - RIGHT, 0.48 * inch)
        canv.setFont("Times-Roman", 9)
        canv.setFillColor(HEADER)
        canv.drawString(LEFT, 0.30 * inch, "Hamspot Commander")
        canv.drawRightString(PAGE_W - RIGHT, 0.30 * inch, "User Guide 0.6.6")
        canv.restoreState()

    def draw_header(self, canv, doc) -> None:
        canv.saveState()
        canv.setStrokeColor(RULE)
        canv.setLineWidth(0.6)
        canv.line(LEFT, PAGE_H - 0.48 * inch, PAGE_W - RIGHT, PAGE_H - 0.48 * inch)
        canv.setFillColor(HEADER)
        canv.setFont("Times-Roman", 9)
        canv.drawString(LEFT, PAGE_H - 0.40 * inch, "Hamspot Commander")
        canv.drawRightString(PAGE_W - RIGHT, PAGE_H - 0.40 * inch, "User Guide 0.6.6")
        canv.restoreState()

    def draw_footer(self, canv, doc) -> None:
        # Drawn after the flowables so a chapter that starts on this page is
        # already the running title. onPage runs too early for that.
        canv.saveState()
        canv.setStrokeColor(RULE)
        canv.setLineWidth(0.4)
        canv.line(LEFT, 0.48 * inch, PAGE_W - RIGHT, 0.48 * inch)
        canv.setFillColor(HEADER)
        canv.setFont("Times-Roman", 9)
        left = self.chapter or "Contents"
        canv.drawString(LEFT, 0.30 * inch, left[:78])
        canv.drawRightString(PAGE_W - RIGHT, 0.30 * inch, str(doc.page))
        canv.restoreState()


def parse_blocks(text: str) -> list[dict]:
    lines = text.splitlines()
    blocks: list[dict] = []
    i = 0
    while i < len(lines):
        line = lines[i]
        if not line.strip():
            i += 1
            continue
        if line.startswith("```"):
            fence = []
            i += 1
            while i < len(lines) and not lines[i].startswith("```"):
                fence.append(lines[i])
                i += 1
            i += 1
            blocks.append({"kind": "code", "text": "\n".join(fence)})
            continue
        if line.startswith("|"):
            rows = []
            while i < len(lines) and lines[i].startswith("|"):
                rows.append(lines[i])
                i += 1
            blocks.append({"kind": "table", "rows": rows})
            continue
        if line.startswith("#"):
            level = len(line) - len(line.lstrip("#"))
            blocks.append({"kind": "heading", "level": level, "text": line[level:].strip()})
            i += 1
            continue
        if line.strip() == "---":
            blocks.append({"kind": "rule"})
            i += 1
            continue
        if line.startswith("> "):
            quote = []
            while i < len(lines) and lines[i].startswith("> "):
                quote.append(lines[i][2:])
                i += 1
            blocks.append({"kind": "note", "text": " ".join(quote)})
            continue
        if re.match(r"^(\d+)\.\s+", line) or line.startswith("- "):
            ordered = bool(re.match(r"^\d+\.\s+", line))
            items = []
            while i < len(lines) and (
                re.match(r"^\d+\.\s+", lines[i]) or lines[i].startswith("- ")
            ):
                item = re.sub(r"^\d+\.\s+", "", lines[i])
                item = re.sub(r"^-\s+", "", item)
                i += 1
                while i < len(lines) and lines[i].startswith("   "):
                    item += " " + lines[i].strip()
                    i += 1
                items.append(item)
            blocks.append({"kind": "list", "ordered": ordered, "items": items})
            continue
        para = [line.strip()]
        i += 1
        while i < len(lines) and lines[i].strip() and not _breaks(lines[i]):
            para.append(lines[i].strip())
            i += 1
        blocks.append({"kind": "para", "text": " ".join(para)})
    return blocks


def _breaks(line: str) -> bool:
    return (
        line.startswith("#")
        or line.startswith("|")
        or line.startswith("```")
        or line.startswith("> ")
        or line.startswith("- ")
        or line.strip() == "---"
        or bool(re.match(r"^\d+\.\s+", line))
    )


def split_table(rows: list[str]) -> list[list[str]]:
    parsed = []
    for row in rows:
        cells = [cell.strip() for cell in row.strip().strip("|").split("|")]
        if cells and all(re.fullmatch(r":?-{3,}:?", cell) for cell in cells):
            continue
        parsed.append(cells)
    return parsed


def build_story(blocks: list[dict], st: dict[str, ParagraphStyle]):
    story = []
    # Cover consumes the title block through the first horizontal rule.
    story.append(Spacer(1, 1.7 * inch))
    story.append(Paragraph("Preliminary", st["cover_kicker"]))
    story.append(Paragraph("Hamspot Commander", st["cover_title"]))
    story.append(Spacer(1, 0.15 * inch))
    story.append(HRFlowable(width="40%", thickness=0.8, color=RULE, spaceBefore=2, spaceAfter=10, hAlign="CENTER"))
    story.append(Paragraph("User Guide", st["cover_sub"]))
    story.append(Paragraph("Version 0.6.6", st["cover_meta"]))
    story.append(Paragraph("10 October 2026", st["cover_meta"]))
    story.append(Spacer(1, 0.45 * inch))
    story.append(Paragraph("Hamspot", st["cover_sub"]))
    story.append(NextPageTemplate("body"))
    story.append(PageBreak())

    # Front matter before the first rule is the preface. The Markdown table
    # of contents is replaced by the printed one, so it is skipped through
    # the following rule. Chapter headings start the body.
    preface: list[dict] = []
    rest: list[dict] = []
    phase = "preface"
    for block in blocks:
        if phase == "preface":
            if block["kind"] == "heading" and block["level"] == 1:
                continue
            if block["kind"] == "rule":
                phase = "skip-toc"
                continue
            plain = re.sub(r"[*_]", "", block.get("text", "")).strip()
            if block["kind"] == "para" and plain in {
                "User Guide",
                "Version 0.6.6",
                "10 October 2026",
                "Hamspot",
            }:
                continue
            preface.append(block)
            continue
        if phase == "skip-toc":
            if block["kind"] == "rule":
                phase = "body"
            continue
        rest.append(block)

    for block in preface:
        if block["kind"] == "para":
            story.append(Paragraph(inline(block["text"]), st["body"]))
    if preface:
        story.append(PageBreak())

    toc_entries: list[tuple[str, str]] = []
    body: list = []
    first_chapter = True
    for block in rest:
        kind = block["kind"]
        if kind == "heading" and block["level"] == 1:
            if not first_chapter:
                body.append(PageBreak())
            first_chapter = False
            title = block["text"]
            body.append(Paragraph(inline(title), st["chapter"]))
            body.append(HRFlowable(width="100%", thickness=1.0, color=RULE, spaceBefore=0, spaceAfter=10))
            toc_entries.append(("chapter", title))
            continue
        if kind == "heading" and block["level"] == 2:
            title = block["text"]
            # The Markdown contents list is replaced by the printed contents.
            if title[:1].isdigit() and ". " in title[:4]:
                continue
            body.append(Paragraph(inline(title), st["section"]))
            body.append(HRFlowable(width="100%", thickness=0.3, color=RULE_LIGHT, spaceBefore=0, spaceAfter=6))
            toc_entries.append(("section", title))
            continue
        if kind == "para":
            # Drop the hand-written contents introduction; the printed page replaces it.
            if block["text"].startswith("The printed pages number"):
                continue
            if re.match(r"^\d+\.\s+", block["text"]) and len(block["text"]) < 80:
                continue
            body.append(Paragraph(inline(block["text"]), st["body"]))
            continue
        if kind == "note":
            note = Paragraph(inline(block["text"]), st["note"])
            wrap = Table([[note]], colWidths=[PAGE_W - LEFT - RIGHT])
            wrap.setStyle(
                TableStyle(
                    [
                        ("BACKGROUND", (0, 0), (-1, -1), NOTE_BG),
                        ("LEFTPADDING", (0, 0), (-1, -1), 8),
                        ("RIGHTPADDING", (0, 0), (-1, -1), 8),
                        ("TOPPADDING", (0, 0), (-1, -1), 6),
                        ("BOTTOMPADDING", (0, 0), (-1, -1), 6),
                        ("BOX", (0, 0), (-1, -1), 0.4, RULE_LIGHT),
                    ]
                )
            )
            body.append(Spacer(1, 4))
            body.append(wrap)
            body.append(Spacer(1, 8))
            continue
        if kind == "list":
            items = []
            for item in block["items"]:
                items.append(
                    ListItem(Paragraph(inline(item), st["bullet"]), leftIndent=12, value=None)
                )
            body.append(
                ListFlowable(
                    items,
                    bulletType="1" if block["ordered"] else "bullet",
                    start="1",
                    leftIndent=18,
                    bulletFontName="Times-Roman",
                    bulletFontSize=11,
                    spaceBefore=2,
                    spaceAfter=8,
                )
            )
            continue
        if kind == "code":
            raw = ascii_punct(block["text"]).replace("\t", "    ")
            pre = Preformatted(raw, st["code"])
            wrap = Table([[pre]], colWidths=[PAGE_W - LEFT - RIGHT])
            wrap.setStyle(
                TableStyle(
                    [
                        ("BACKGROUND", (0, 0), (-1, -1), CODE_BG),
                        ("LEFTPADDING", (0, 0), (-1, -1), 6),
                        ("RIGHTPADDING", (0, 0), (-1, -1), 6),
                        ("TOPPADDING", (0, 0), (-1, -1), 4),
                        ("BOTTOMPADDING", (0, 0), (-1, -1), 4),
                        ("BOX", (0, 0), (-1, -1), 0.4, RULE_LIGHT),
                    ]
                )
            )
            body.append(Spacer(1, 4))
            body.append(wrap)
            body.append(Spacer(1, 8))
            continue
        if kind == "table":
            parsed = split_table(block["rows"])
            if not parsed:
                continue
            head, *rest = parsed
            data = [[Paragraph(inline(cell), st["table_head"]) for cell in head]]
            for row in rest:
                while len(row) < len(head):
                    row.append("")
                data.append([Paragraph(inline(cell), st["table"]) for cell in row[: len(head)]])
            usable = PAGE_W - LEFT - RIGHT
            # Give the last column the remaining width. Short leading columns
            # stay narrow enough that a prose cell can wrap.
            if len(head) == 2:
                widths = [1.7 * inch, usable - 1.7 * inch]
            elif len(head) == 3:
                widths = [1.55 * inch, 1.45 * inch, usable - 3.0 * inch]
            else:
                widths = [usable / len(head)] * len(head)
            table = Table(data, colWidths=widths, repeatRows=1, hAlign="LEFT")
            table.setStyle(
                TableStyle(
                    [
                        ("BACKGROUND", (0, 0), (-1, 0), colors.HexColor("#ececec")),
                        ("VALIGN", (0, 0), (-1, -1), "TOP"),
                        ("GRID", (0, 0), (-1, -1), 0.3, RULE_LIGHT),
                        ("LEFTPADDING", (0, 0), (-1, -1), 4),
                        ("RIGHTPADDING", (0, 0), (-1, -1), 4),
                        ("TOPPADDING", (0, 0), (-1, -1), 3),
                        ("BOTTOMPADDING", (0, 0), (-1, -1), 3),
                    ]
                )
            )
            body.append(Spacer(1, 4))
            body.append(table)
            body.append(Spacer(1, 8))
            continue
        if kind == "rule":
            body.append(Spacer(1, 6))

    # Printed contents, with the chapter titles. Page numbers are filled
    # after the body is built once. See fill_contents.
    story.append(Paragraph("Table of Contents", st["toc_title"]))
    story.append(HRFlowable(width="100%", thickness=1.0, color=RULE, spaceBefore=0, spaceAfter=12))
    story.append(CondPageBreak(1))  # marker slot replaced below
    contents_at = len(story) - 1
    story.append(PageBreak())
    story.extend(body)
    return story, contents_at, toc_entries


class NumberedDoc(GuideDoc):
    """Records the page of each chapter and section paragraph."""

    def __init__(self, path, story_styles):
        super().__init__(path, story_styles)
        self.pages: list[tuple[str, str, int]] = []

    def afterFlowable(self, flowable) -> None:
        super().afterFlowable(flowable)
        if isinstance(flowable, Paragraph) and flowable.style.name in {"Chapter", "Section"}:
            kind = "chapter" if flowable.style.name == "Chapter" else "section"
            self.pages.append((kind, flowable.getPlainText(), self.page))


def contents_flowables(entries: list[tuple[str, str, int]], st) -> list:
    usable = PAGE_W - LEFT - RIGHT
    rows = []
    for kind, title, page in entries:
        style = st["toc_chapter"] if kind == "chapter" else ParagraphStyle(
            "TocSection",
            parent=st["body"],
            fontName="Times-Roman",
            fontSize=11,
            leading=14,
            alignment=TA_LEFT,
            leftIndent=16,
            spaceAfter=0,
        )
        label = Paragraph(inline(title), style)
        number = Paragraph(str(page), ParagraphStyle(
            "TocPage",
            fontName="Times-Roman",
            fontSize=11,
            leading=14,
            alignment=TA_RIGHT,
            textColor=INK,
        ))
        rows.append([label, number])
    if not rows:
        return []
    table = Table(rows, colWidths=[usable - 0.5 * inch, 0.5 * inch])
    table.setStyle(
        TableStyle(
            [
                ("VALIGN", (0, 0), (-1, -1), "BOTTOM"),
                ("LEFTPADDING", (0, 0), (-1, -1), 0),
                ("RIGHTPADDING", (0, 0), (-1, -1), 0),
                ("TOPPADDING", (0, 0), (-1, -1), 2),
                ("BOTTOMPADDING", (0, 0), (-1, -1), 2),
                ("LINEBELOW", (0, 0), (-1, -1), 0.2, colors.HexColor("#dddddd")),
            ]
        )
    )
    return [table]


def main() -> int:
    if not MASTER.is_file():
        print(f"missing {MASTER}", file=sys.stderr)
        return 1
    st = styles()
    blocks = parse_blocks(MASTER.read_text(encoding="utf-8"))
    story, contents_at, _entries = build_story(blocks, st)

    probe_path = OUTPUT.with_suffix(".probe.pdf")
    probe = NumberedDoc(probe_path, st)
    # First pass uses an empty contents slot so page numbers match the
    # second pass as long as the contents table stays on one page. If it
    # spills, the second pass shifts. We rebuild once more when the
    # contents page count changes.
    story[contents_at] = Spacer(1, 1)
    probe.build(story)

    def render(pages: list[tuple[str, str, int]], dest: Path) -> int:
        st_local = styles()
        blocks_local = parse_blocks(MASTER.read_text(encoding="utf-8"))
        story_local, at, _ = build_story(blocks_local, st_local)
        flow = contents_flowables(pages, st_local)
        story_local[at : at + 1] = flow
        doc = NumberedDoc(dest, st_local)
        doc.build(story_local)
        return len(doc.pages)

    render(probe.pages, OUTPUT)
    # Second measurement, in case the contents table grew onto another page
    # and moved the chapters. One correction is enough for this guide.
    check = NumberedDoc(probe_path, styles())
    # Rebuild probe from the finished PDF's outline by re-reading is harder
    # than rendering to the probe path with the same contents and comparing.
    blocks2 = parse_blocks(MASTER.read_text(encoding="utf-8"))
    story2, at2, _ = build_story(blocks2, styles())
    story2[at2 : at2 + 1] = contents_flowables(probe.pages, styles())
    check.build(story2)
    if [(k, t, p) for k, t, p in check.pages] != [(k, t, p) for k, t, p in probe.pages]:
        render(check.pages, OUTPUT)
    probe_path.unlink(missing_ok=True)
    print(f"wrote {OUTPUT}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
