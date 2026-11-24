#!/usr/bin/env python3
"""Build data/awards.tsv (award, year, title, artist) from Wikipedia's winner tables.

rola embeds the result with include_str!, so the app never scrapes at runtime.
Re-run this once a year after the Grammys:   python3 tools/gen_awards.py

Grammy tables list every nominee; the winner of each year is the row with the
gold background (#FAEB86). `year` is the ceremony year as Wikipedia lists it
(e.g. 2020 = awards held in 2020 for music released in 2019).
Source text: https://en.wikipedia.org (CC BY-SA).
"""
import html, json, re, sys, urllib.request

def page(title):
    url = f"https://en.wikipedia.org/w/api.php?action=parse&page={title}&prop=text&format=json&formatversion=2"
    req = urllib.request.Request(url, headers={"User-Agent": "rola-data-generator/1.0"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.load(r)["parse"]["text"]

def text(cell):
    t = re.sub(r"<sup.*?</sup>", "", cell, flags=re.S)       # footnote markers
    t = html.unescape(re.sub(r"<[^>]+>", "", t))
    t = re.sub(r"\[[^\]]*\]", "", t)
    return re.sub(r"\s+", " ", t).strip().strip('"“”').strip()

def rows(h):
    for table in re.findall(r'<table class="wikitable.*?</table>', h, flags=re.S):
        for tr in re.findall(r"<tr.*?</tr>", table, flags=re.S):
            yield tr

def grammy(title, award, title_col, artist_col):
    """title_col/artist_col: index among the <td> cells of the winner row."""
    out, year = [], None
    for tr in rows(page(title)):
        ycell = re.search(r"<t[hd][^>]*rowspan[^>]*>(.*?)</t[hd]>", tr, flags=re.S)
        if ycell:
            m = re.search(r"\b(19|20)\d\d\b", text(ycell.group(1)))
            if m:
                year = int(m.group(0))
        if "FAEB86" not in tr.split(">")[0].upper() and "FAEB86" not in tr[:80].upper():
            continue
        cells = re.findall(r"<td[^>]*>(.*?)</td>", tr, flags=re.S)
        if year and len(cells) > max(title_col, artist_col):
            out.append((award, year, text(cells[title_col]), text(cells[artist_col])))
    return out

def mercury():
    out = []
    for tr in rows(page("Mercury_Prize")):
        cells = re.findall(r"<t[hd][^>]*>(.*?)</t[hd]>", tr, flags=re.S)
        if len(cells) < 2:
            continue
        m = re.match(r"(\d{4})", text(cells[0]))
        w = text(cells[1])
        if m and " – " in w:
            artist, album = w.split(" – ", 1)
            out.append(("Mercury Prize", int(m.group(1)), album.strip(), artist.strip()))
    return out

if __name__ == "__main__":
    data = []
    data += grammy("Grammy_Award_for_Album_of_the_Year", "grammy-album", 0, 1)
    data += grammy("Grammy_Award_for_Record_of_the_Year", "grammy-record", 0, 1)
    # Song of the Year columns: song, songwriter(s), performing artist(s)
    data += grammy("Grammy_Award_for_Song_of_the_Year", "grammy-song", 0, 2)
    data += mercury()
    with open("data/awards.tsv", "w", encoding="utf-8") as f:
        for a in data:
            f.write("\t".join(str(x).replace("\t", " ") for x in a) + "\n")
    from collections import Counter
    print(Counter(a[0] for a in data), file=sys.stderr)
