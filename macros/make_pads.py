#!/usr/bin/env python3
"""Generate pads.csv (simulation pad table) from a TPCReco geometry .dat.

All geometry numbers (angles, pad size, reference point, strip rows) are read
from the .dat, so this script contains no detector data and is safe to commit;
the OUTPUT is derived from the real geometry and must stay local (gitignored).

Conventions ported from TPCReco GeometryTPC.cpp (reference/TPCReco/latest):
  pad_pitch = a*sqrt(3), strip_pitch = a*1.5
  pitch_U = -(u_W + u_V)/|..|, pitch_V = (u_U + u_W)/|..|, pitch_W = (u_V - u_U)/|..|
  centre of pad k on a strip = REF + so*strip_pitch*pitch + (po + k)*pad_pitch*u
Simulation frame (agreed 2026-09-07): x_sim = x_det,
  z_sim = -250 + d + (y_edge - y_det), y_edge = true upstream pad-vertex edge,
  d = entrance window -> upstream pad edge [mm] (default 0 until measured).
Usage: make_pads.py <geometry.dat> <out.csv> [d_mm] [channel_map.csv-for-oracle]
"""
import math, sys, hashlib

def unit(v):
    n = math.hypot(*v)
    return (v[0] / n, v[1] / n)

def main():
    datpath, outpath = sys.argv[1], sys.argv[2]
    d_window = float(sys.argv[3]) if len(sys.argv) > 3 else 0.0
    cmap_path = sys.argv[4] if len(sys.argv) > 4 else None

    angles, ref, a = None, None, None
    strips = []
    for line in open(datpath):
        f = line.split()
        if not f:
            continue
        if f[0] == 'ANGLES:':
            angles = dict(zip('UVW', map(float, f[1:4])))
        elif line.startswith('DIAMOND SIZE:'):
            a = float(line.split(':')[1])
        elif line.startswith('REFERENCE POINT:'):
            ref = tuple(map(float, line.split(':')[1].split()[:2]))
        elif f[0] in ('U', 'V', 'W') and len(f) >= 10:
            strips.append((f[0], int(f[2]), int(f[5]), int(f[6]),
                           float(f[7]), float(f[8]), int(f[9])))
    assert angles and ref and a and strips, "geometry .dat parse failed"

    pad_p, strip_p = a * math.sqrt(3.0), a * 1.5
    u = {k: (math.cos(math.radians(t)), math.sin(math.radians(t)))
         for k, t in angles.items()}
    p = {'U': unit((-(u['W'][0] + u['V'][0]), -(u['W'][1] + u['V'][1]))),
         'V': unit((u['U'][0] + u['W'][0], u['U'][1] + u['W'][1])),
         'W': unit((u['V'][0] - u['U'][0], u['V'][1] - u['U'][1]))}
    FPN = {11, 22, 45, 56}
    geo2raw = [r for r in range(68) if r not in FPN]
    VIDX = {'U': 0, 'V': 1, 'W': 2}

    if cmap_path:  # oracle: mapping + strip position vs the validated map
        cmap = {}
        for line in open(cmap_path):
            line = line.strip()
            if not line or line.startswith('#'):
                continue
            ag, ch, v, st, s = line.split(',')
            cmap[(int(ag), int(ch))] = (int(v), int(st), float(s))
        for dd, num, aget, chg, po, so, n in strips:
            c1 = (ref[0] + so * strip_p * p[dd][0], ref[1] + so * strip_p * p[dd][1])
            s_pred = c1[0] * p[dd][0] + c1[1] * p[dd][1]
            got = cmap[(aget, geo2raw[chg])]
            assert got[0] == VIDX[dd] and got[1] == num, f"map mismatch {dd}{num}"
            assert abs(s_pred - got[2]) < 1e-3, f"s mismatch {dd}{num}"

    pads = []
    for dd, num, aget, chg, po, so, n in sorted(strips, key=lambda r: (VIDX[r[0]], r[1])):
        ud, pd_ = u[dd], p[dd]
        perp = (-ud[1], ud[0])
        for k in range(n):
            cx = ref[0] + so * strip_p * pd_[0] + (po + k) * pad_p * ud[0]
            cy = ref[1] + so * strip_p * pd_[1] + (po + k) * pad_p * ud[1]
            verts = [(cx - pad_p / 2 * ud[0], cy - pad_p / 2 * ud[1]),
                     (cx + a / 2 * perp[0], cy + a / 2 * perp[1]),
                     (cx + pad_p / 2 * ud[0], cy + pad_p / 2 * ud[1]),
                     (cx - a / 2 * perp[0], cy - a / 2 * perp[1])]
            pads.append((dd, num, k, aget, geo2raw[chg], chg, cx, cy, verts))

    y_edge = max(v[1] for pd in pads for v in pd[8])
    md5 = hashlib.md5(open(datpath, 'rb').read()).hexdigest()[:8]
    sim = lambda pt: (pt[0], -250.0 + d_window + (y_edge - pt[1]))
    with open(outpath, 'w') as out:
        out.write(f"""# pads.csv  units: mm
# frame: gas-volume centre origin, x horizontal, z alpha-source axis, readout plane y = -100 (x-z plane)
#   x_sim = x_det ; z_sim = -250 + d + ({y_edge:.3f} - y_det)
#   {y_edge:.3f} = true upstream pad-vertex edge, so d = 0 puts the first pad edge exactly at the
#   entrance plane z = -250; set d = (entrance window -> upstream pad edge) [mm] when measured (d = {d_window})
# NOTE: the axis mapped to z here is the ALPHA-SOURCE axis (-y_det on the PCB drawing);
#       the accelerator BEAM axis on that drawing is +x_det (maps to sim +x).
# pad = rhombus, edge {a} mm, 3 orientations (long diagonal sqrt(3)*a along the strip); vertices CCW in (x, z)
# ch_graw = raw GRAW channel 0-67 (as in data), ch_geom = geometry channel 0-63 (FPN 11/22/45/56 removed)
# provenance: {datpath.split('/')[-1]} (md5 {md5}) + TPCReco GeometryTPC conventions
pad_id,strip_dir,strip_no,pad_no,aget,ch_graw,ch_geom,cx,cz,x0,z0,x1,z1,x2,z2,x3,z3
""")
        for pid, (dd, num, k, aget, raw, chg, cx, cy, verts) in enumerate(pads):
            scx, scz = sim((cx, cy))
            sv = sorted((sim(v) for v in verts),
                        key=lambda q: math.atan2(q[1] - scz, q[0] - scx))
            out.write(f"{pid},{dd},{num},{k},{aget},{raw},{chg},{scx:.3f},{scz:.3f}," +
                      ",".join(f"{v[0]:.3f},{v[1]:.3f}" for v in sv) + "\n")
    print(f"{outpath}: {len(pads)} pads, upstream edge y_det = {y_edge:.3f}, d = {d_window}")

if __name__ == '__main__':
    main()
