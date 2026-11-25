#!/usr/bin/env python3
"""Export the acclaim bundle (data/acclaim.tsv) from a deadwax database.

    python3 tools/export_acclaim.py [path/to/deadwax.sqlite]

deadwax fits a one-factor latent model over ~96 critics (Wikipedia ratings boxes,
Pitchfork, MusicBrainz users). Each album gets a latent score `theta` with a posterior
sd. We ship, per album with >= MIN_CRITICS critics:

  z      (theta - mean) / sd over all scored albums: how many standard deviations above
         the average album the critics put it
  lb     z minus one posterior sd: a conservative score ("84% sure it is at least this
         good"). Sorting by lb instead of z keeps thinly-reviewed albums from topping lists
  pct    percentile among all 64k scored albums;  gpct: within its genre
  n      number of critics;  split: how much the critics disagree (sd of their z's)
  lists  how many "greatest albums" lists it is on;  prize: W won / N nominated / A other award
"""
import os, sqlite3, statistics, sys

MIN_CRITICS = 3
HERE = os.path.dirname(os.path.abspath(__file__))
DB = sys.argv[1] if len(sys.argv) > 1 else os.path.expanduser("~/repos/deadwax/data/deadwax.sqlite")
OUT = os.path.join(HERE, "..", "data", "acclaim.tsv")

c = sqlite3.connect(DB)
th = [r[0] for r in c.execute("select theta from score")]
mu, sd = statistics.mean(th), statistics.pstdev(th)
zcols = [r[1] for r in c.execute("pragma table_info(score)") if r[1].startswith("z_")]

lists = dict(c.execute("select rgid, count(distinct list) from accolade group by 1"))
prize = {}
for rgid, a in c.execute("select rgid, award from award"):
    low = a.lower()
    if any(w in low for w in ("snep", "platinum", "gold record", "silver record", "diamond", "certif")):
        continue
    kind = "N" if "nominee" in low else ("W" if any(w in low for w in ("album of the year", "mercury prize", "british album", "hall of fame")) else "A")
    if rgid not in prize or "WNA".index(kind) < "WNA".index(prize[rgid]):
        prize[rgid] = kind

q = f"""select r.rgid, r.artist, r.title, r.year, s.genre, s.theta, s.sd, s.n_raters, s.pct, s.gpct, {",".join("s."+z for z in zcols)}
        from score s join rg r using(rgid)
        where s.n_raters >= ? and r.kind in ('studio','ep') order by s.theta desc"""
rows = []
for r in c.execute(q, (MIN_CRITICS,)):
    rgid, artist, title, year, genre, theta, tsd, n, pct, gpct = r[:10]
    zs = [z for z in r[10:] if z is not None]
    split = statistics.pstdev(zs) if len(zs) >= 4 else 0.0
    z = (theta - mu) / sd
    lb = (theta - tsd - mu) / sd
    clean = lambda s: (s or "").replace("\t", " ").replace("\n", " ").strip()
    rows.append((clean(artist), clean(title), int(year or 0), clean(genre), round(z, 2), round(lb, 2), round(pct, 2), round(gpct or 0, 1), n, round(split, 2), lists.get(rgid, 0), prize.get(rgid, "-")))

# distribution of z over ALL scored albums (40 bins of 0.2 from -4 to +4), for the card's histogram
bins = [0] * 40
for t in th:
    bins[min(39, max(0, int(((t - mu) / sd + 4) / 0.2)))] += 1

with open(OUT, "w", encoding="utf-8") as f:
    f.write("#hist\t" + ",".join(map(str, bins)) + "\n")
    f.write("# artist\ttitle\tyear\tgenre\tz\tlb\tpct\tgpct\tn\tsplit\tlists\tprize  (deadwax latent model; z over %d albums, mu=%.3f sd=%.3f)\n" % (len(th), mu, sd))
    for r in rows:
        f.write("\t".join(map(str, r)) + "\n")
print(f"{len(rows)} albums -> {OUT} ({os.path.getsize(OUT)//1024} KB)")
