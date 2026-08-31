# Fits Word's line-pitch law to a dumpall.ps1 CSV.
#
# The model, established by ADR 0001: lines are laid at a constant base pitch
# B; an accumulator tracks (ideal - B) per line; the line that tips the debt
# to half a point is laid half a point taller (or shorter). This fits B and
# the correction size D by least squares given the jump positions read off
# the residuals, and reports how well the model reproduces every measured
# position (max residual <= 0.51tw means: to within Word's own reporting).
#
# Usage: python fit.py lines-verdana-20.csv [more.csv ...]

import csv
import sys


def series(name, page=1):
    ys = []
    for row in csv.reader(open(name)):
        # y_points, in_table, font, size[, page]
        in_table = row[1] == "1"
        on_page = len(row) < 5 or int(row[4]) == page
        if not in_table and on_page:
            ys.append(round(float(row[0]) * 20))  # twips
    return ys


def lstsq(ys, jumps_at):
    n = len(ys)
    X = [(1.0, float(i), float(sum(1 for j in jumps_at if j <= i))) for i in range(n)]
    ATA = [[sum(x[r] * x[c] for x in X) for c in range(3)] for r in range(3)]
    ATy = [sum(X[i][r] * ys[i] for i in range(n)) for r in range(3)]
    M = [row[:] + [ATy[r]] for r, row in enumerate(ATA)]
    for col in range(3):
        piv = max(range(col, 3), key=lambda r: abs(M[r][col]))
        M[col], M[piv] = M[piv], M[col]
        for r in range(3):
            if r != col:
                f = M[r][col] / M[col][col]
                M[r] = [a - f * b for a, b in zip(M[r], M[col])]
    a, B, D = (M[i][3] / M[i][i] for i in range(3))
    resid = max(abs(ys[i] - (a + B * i + D * X[i][2])) for i in range(n))
    return B, D, resid


def fit(name):
    ys = series(name)
    if len(ys) > 2:
        ys = ys[:-1]  # the document-final paragraph mark is not a probe line
    diffs = [b - a for a, b in zip(ys, ys[1:])]
    med = sorted(diffs)[len(diffs) // 2]
    jumps = [i + 1 for i, d in enumerate(diffs) if abs(d - med) > 5]
    B, D, resid = lstsq(ys, jumps)
    print(
        f"{name}: B={B:.4f}tw = {B / 20:.5f}pt   D={D / 20:+.3f}pt   "
        f"maxresid={resid:.2f}tw   jumps at {jumps}"
    )


if __name__ == "__main__":
    for name in sys.argv[1:]:
        fit(name)
