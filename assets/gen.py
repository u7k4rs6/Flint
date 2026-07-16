#!/usr/bin/env python3
"""Generates the animated SVG assets for the Flint README.

Everything is pure SVG + CSS keyframes: no JavaScript, no external fetches,
so it survives GitHub's <img> sandbox and still animates.
"""
import os

OUT = os.path.dirname(os.path.abspath(__file__))
os.makedirs(OUT, exist_ok=True)

# ---------------------------------------------------------------- tokens
INK    = "#0a0c10"   # board black
PANEL  = "#11151c"   # inert block
RULE   = "#232a35"   # hairline
BONE   = "#cfd8e3"   # console text
DIM    = "#69737f"   # secondary text
EMBER  = "#ff7a2f"   # ring 0 / live
EMBER2 = "#ffb45c"   # ring 0 highlight
VIOLET = "#a78bfa"   # ring 3
MINT   = "#59c07f"   # ok
HW     = "#5b6672"   # hardware, outside the kernel
LIT    = "#1c1610"   # ember block fill
LITV   = "#171425"   # violet block fill

MONO = ("ui-monospace,SFMono-Regular,'SF Mono',Menlo,Consolas,"
        "'DejaVu Sans Mono','Liberation Mono',monospace")


def esc(s):
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def txt(x, y, s, fill=BONE, size=13.5, anchor="start", cls=None, ls=None,
        weight=None, tl=None):
    a = [f'x="{x}" y="{y}"', f'fill="{fill}"', f'font-size="{size}"']
    if anchor != "start":
        a.append(f'text-anchor="{anchor}"')
    if cls:
        a.append(f'class="{cls}"')
    if ls:
        a.append(f'letter-spacing="{ls}"')
    if weight:
        a.append(f'font-weight="{weight}"')
    if tl:
        a.append(f'textLength="{tl:.2f}" lengthAdjust="spacingAndGlyphs"')
    return f'<text {" ".join(a)}>{esc(s)}</text>'


def rect(x, y, w, h, fill="none", stroke="none", rx=0, cls=None, extra=""):
    a = [f'x="{x}" y="{y}" width="{w}" height="{h}"',
         f'fill="{fill}"', f'stroke="{stroke}"']
    if rx:
        a.append(f'rx="{rx}"')
    if cls:
        a.append(f'class="{cls}"')
    if extra:
        a.append(extra)
    return f'<rect {" ".join(a)}/>'


def pct(t, T):
    return max(0.0, min(100.0, t / T * 100.0))


def kf_show(name, t, T, fade=0.12):
    """opacity 0 -> 1 at t, held to the end of the cycle."""
    a, b = pct(t, T), pct(t + fade, T)
    return (f"@keyframes {name}{{0%,{a:.2f}%{{opacity:0}}"
            f"{b:.2f}%,100%{{opacity:1}}}}")


def kf_window(name, t0, t1, T, fade=0.1):
    """opacity 0 -> 1 for [t0,t1) -> 0."""
    a, b = pct(t0, T), pct(t0 + fade, T)
    c, d = pct(t1, T), pct(t1 + fade, T)
    return (f"@keyframes {name}{{0%,{a:.2f}%{{opacity:0}}"
            f"{b:.2f}%,{c:.2f}%{{opacity:1}}{d:.2f}%,100%{{opacity:0}}}}")


def head(w, h, title, css, defs=""):
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" '
            f'width="{w}" height="{h}" font-family="{MONO}" role="img">'
            f"<title>{esc(title)}</title>"
            f"<defs>{defs}</defs>"
            f"<style><![CDATA[{css}]]></style>")


TAIL = "</svg>"

# Reduced motion: freeze on the last frame instead of looping.
REDUCE = ("@media (prefers-reduced-motion:reduce){"
          "*{animation:none!important}"
          ".fx{opacity:1!important}"
          ".transient{opacity:0!important}}")


def write(name, body):
    with open(os.path.join(OUT, name), "w") as f:
        f.write(body)
    print(name, len(body), "bytes")


# ================================================================= boot.svg
def boot():
    """CRT powers on, kernel blits its log line by line, flame catches."""
    T = 14.0
    W, H = 880, 460
    FS = 13.5
    CW = FS * 0.6
    TS = "#48525e"
    css = [".fx{opacity:0}"]
    defs = [f'<clipPath id="card"><rect x="1" y="1" width="{W-2}" '
            f'height="{H-2}" rx="12"/></clipPath>',
            '<pattern id="scan" width="3" height="3" '
            'patternUnits="userSpaceOnUse">'
            '<rect width="3" height="1.2" fill="#000" opacity=".5"/></pattern>',
            '<radialGradient id="vig" cx=".5" cy=".5" r=".78">'
            '<stop offset=".6" stop-color="#000" stop-opacity="0"/>'
            '<stop offset="1" stop-color="#000" stop-opacity=".35"/>'
            '</radialGradient>',
            f'<radialGradient id="emberGlow">'
            f'<stop offset="0" stop-color="{EMBER}" stop-opacity=".5"/>'
            f'<stop offset=".5" stop-color="{EMBER}" stop-opacity=".12"/>'
            f'<stop offset="1" stop-color="{EMBER}" stop-opacity="0"/>'
            f'</radialGradient>',
            '<filter id="soft" x="-90%" y="-90%" width="280%" height="280%">'
            '<feGaussianBlur stdDeviation="3.5" result="b"/>'
            '<feMerge><feMergeNode in="b"/><feMergeNode in="b"/>'
            '<feMergeNode in="SourceGraphic"/></feMerge></filter>']
    body = []
    wipes = []

    def wipe(x, y, s, color, size, t, cid, per=0.009):
        """Blit text left to right, one glyph per step."""
        w = len(s) * (size * 0.6)
        dur = len(s) * per
        a, b = pct(t, T), pct(t + dur, T)
        css.append(f"@keyframes {cid}k{{0%,{a:.2f}%{{transform:"
                   f"translateX({-w:.2f}px)}}{b:.2f}%,100%"
                   f"{{transform:translateX(0)}}}}")
        css.append(f".{cid}k{{animation:{cid}k {T}s steps({len(s)},end) "
                   f"infinite}}")
        defs.append(f'<clipPath id="{cid}"><rect class="{cid}k" x="{x}" '
                    f'y="{y-size}" width="{w:.2f}" height="{size+5}"/>'
                    f'</clipPath>')
        return (f'<g clip-path="url(#{cid})">'
                + txt(x, y, s, color, size, tl=w) + "</g>")

    # ---- card + chrome
    body.append(rect(0.5, 0.5, W - 1, H - 1, INK, RULE, 12))
    body.append(f'<line x1="0" y1="38" x2="{W}" y2="38" stroke="{RULE}"/>')
    for cx in (24, 44, 64):
        body.append(f'<circle cx="{cx}" cy="19" r="5" fill="{RULE}"/>')
    body.append(txt(88, 24, "qemu-system-x86_64  -serial stdio  -m 128M",
                    DIM, 11.5))
    body.append(txt(834, 24, "power", TS, 11, anchor="end"))
    css.append("@keyframes led{0%,4.9%{fill:#2a3038}5%,55.9%{fill:#ff7a2f}"
               f"56%,100%{{fill:{MINT}}}}}")
    css.append(f".led{{animation:led {T}s infinite}}")
    body.append(f'<circle class="led" cx="852" cy="19" r="4.5" fill="#2a3038"/>')

    # ---- ignition column glow, behind everything on the screen
    css.append("@keyframes glow{"
               "0%,5.6%{transform:scale(.08);opacity:0}"
               "6%,15.6%{transform:scale(.2);opacity:.55}"
               "15.7%,25.6%{transform:scale(.36);opacity:.65}"
               "25.8%,35.6%{transform:scale(.58);opacity:.8}"
               "35.8%,45.6%{transform:scale(.8);opacity:.9}"
               "45.8%,100%{transform:scale(1);opacity:1}}")
    css.append(f".glow{{transform-origin:778px 250px;animation:glow {T}s "
               f"infinite}}")
    css.append("@keyframes flick{0%{opacity:.84}18%{opacity:1}36%{opacity:.9}"
               "54%{opacity:1}72%{opacity:.86}100%{opacity:1}}")
    css.append(".flick{animation:flick .55s ease-in-out infinite}")
    body.append('<g class="flick"><circle class="glow" cx="778" cy="250" '
                f'r="96" fill="url(#emberGlow)"/></g>')
    body.append(f'<line x1="676" y1="38" x2="676" y2="{H}" stroke="{RULE}"/>')

    # ---- the log
    LX, DX, OX, RX = 112, 290, 432, 660
    LEAD = "." * 16
    log = [
        # t, y, stamp, label, ok_t, ok_col, detail
        (0.80, 76, "[0.000000]", "Power applied", None, None, None),
        (1.05, 103, "[0.000318]", "POST", 1.70, HW, "128 MiB, 512 frames"),
        (2.20, 139, "[0.001204]", "Initializing GDT", 2.62, MINT,
         "5 descriptors, 1 tss"),
        (2.90, 166, "[0.001861]", "Loading IDT", 3.32, MINT, "256 vectors"),
        (3.60, 193, "[0.002450]", "Entering Long Mode", 4.02, MINT,
         "efer.lme = 1"),
        (4.30, 220, "[0.004137]", "Setting up Paging", 4.72, MINT,
         "cr3 = 0x0011_9000"),
        (5.00, 247, "[0.005902]", "Kernel Heap", 5.42, MINT, "128 KiB"),
        (5.70, 274, "[0.007744]", "Starting Scheduler", 6.12, MINT,
         "100 Hz, 4 tasks"),
        (6.40, 301, "[0.009318]", "Switching to Ring 3", 6.82, VIOLET,
         "pid 2, iretq"),
    ]
    for i, (t, y, stamp, label, ok_t, ok_c, detail) in enumerate(log):
        css.append(kf_show(f"s{i}", t, T, fade=0.02))
        css.append(f".s{i}{{animation:s{i} {T}s infinite}}")
        body.append(txt(20, y, stamp, TS, FS, cls=f"fx s{i}",
                        tl=len(stamp) * CW))
        body.append(wipe(LX, y, label, BONE if ok_t else DIM, FS, t, f"w{i}"))
        if ok_t is None:
            continue
        body.append(wipe(DX, y, LEAD, "#566270", FS, t + 0.12, f"d{i}",
                         per=0.022))
        css.append(kf_show(f"o{i}", ok_t, T, fade=0.02))
        css.append(f".o{i}{{animation:o{i} {T}s infinite}}")
        body.append(f'<g class="fx o{i}">')
        body.append(txt(OX, y, "[", "#39414c", FS))
        body.append(txt(OX + 16, y, "OK", ok_c, FS))
        body.append(txt(OX + 47, y, "]", "#39414c", FS))
        body.append("</g>")
        css.append(kf_window(f"z{i}", ok_t, ok_t + 0.16, T, fade=0.02))
        css.append(f".z{i}{{animation:z{i} {T}s infinite}}")
        body.append(txt(OX + 16, y, "OK", "#ffffff", FS,
                        cls=f"fx transient z{i}") .replace(
                        "<text ", '<text filter="url(#soft)" '))
        css.append(kf_show(f"r{i}", ok_t + 0.04, T, fade=0.06))
        css.append(f".r{i}{{animation:r{i} {T}s infinite}}")
        body.append(txt(RX, y, detail, TS, 11.5, anchor="end",
                        cls=f"fx r{i}"))

    # ---- banner, login, prompt
    css.append(kf_show("ban", 7.30, T, fade=0.03))
    css.append(f".ban{{animation:ban {T}s infinite}}")
    body.append(f'<g class="fx ban">')
    body.append(txt(20, 343, "Flint Kernel 0.1", EMBER2, 15,
                    tl=16 * 9).replace("<text ", '<text filter="url(#soft)" '))
    body.append(txt(RX, 343, "x86_64 · no_std · 0.011 s", TS, 11.5,
                    anchor="end"))
    body.append("</g>")
    body.append(wipe(20, 370, "login: root", BONE, FS, 7.80, "lg"))
    css.append(kf_show("prompt", 8.40, T, fade=0.02))
    css.append(f".prompt{{animation:prompt {T}s infinite}}")
    css.append("@keyframes blink{0%,49%{opacity:1}50%,100%{opacity:0}}")
    body.append('<g class="fx prompt">')
    body.append(txt(20, 404, "flint>", EMBER, FS, tl=6 * CW))
    body.append(rect(76, 393, 8, 14, EMBER, rx=1,
                     extra='style="animation:blink 1s steps(1) infinite"'))
    body.append("</g>")

    # ---- ignition column
    body.append(txt(778, 70, "I G N I T I O N", TS, 10, anchor="middle"))
    stages = [
        (0.80, 2.20, ".", 26, EMBER2, "cold"),
        (2.20, 3.60, "*", 30, EMBER2, "spark"),
        (3.60, 5.00, "**", 32, EMBER, "catch"),
        (5.00, 6.40, "****", 32, EMBER, "burn"),
        (6.40, T, "\u2588\u2588\u2588\u2588\u2588\u2588", 22, EMBER,
         "steady"),
    ]
    for i, (t0, t1, glyph, sz, col, cap) in enumerate(stages):
        n = f"f{i}"
        last = i == len(stages) - 1
        css.append(kf_show(n, t0, T) if last else kf_window(n, t0, t1, T))
        css.append(f".{n}{{animation:{n} {T}s infinite}}")
        cls = f"fx {n}" if last else f"fx transient {n}"
        body.append(f'<g class="{cls}">')
        body.append(f'<text x="778" y="258" fill="{col}" font-size="{sz}" '
                    f'text-anchor="middle" filter="url(#soft)">'
                    f"{esc(glyph)}</text>")
        body.append(txt(778, 312, cap, TS, 10, anchor="middle", ls="2"))
        body.append("</g>")

    # ---- embers off the flame
    import random
    random.seed(11)
    css.append(kf_show("emb", 2.20, T, fade=0.4))
    css.append(f".emb{{animation:emb {T}s infinite}}")
    body.append('<g class="fx emb">')
    for i in range(10):
        dx = random.uniform(-16, 16)
        rise = random.uniform(55, 115)
        dur = random.uniform(1.5, 2.6)
        delay = random.uniform(0, 2.6)
        sz = random.choice([1.4, 1.8, 2.2])
        css.append(f"@keyframes e{i}{{0%{{transform:translate(0,0);opacity:0}}"
                   f"14%{{opacity:.95}}66%{{opacity:.45}}"
                   f"100%{{transform:translate({dx:.1f}px,{-rise:.1f}px);"
                   f"opacity:0}}}}")
        body.append(f'<rect x="{778+random.uniform(-20,20):.1f}" '
                    f'y="{254+random.uniform(-8,4):.1f}" width="{sz}" '
                    f'height="{sz}" fill="{EMBER2}" rx=".4" '
                    f'style="animation:e{i} {dur:.2f}s linear infinite;'
                    f'animation-delay:{delay:.2f}s"/>')
    body.append("</g>")

    # ---- boot progress hairline along the bottom edge
    css.append("@keyframes bar{0%,5.7%{transform:scaleX(0)}"
               "52.2%,100%{transform:scaleX(1)}}")
    css.append(f".bar{{transform-origin:0 0;animation:bar {T}s "
               f"cubic-bezier(.4,0,.2,1) infinite}}")
    body.append(f'<g clip-path="url(#card)">'
                f'<rect x="0" y="{H-4}" width="{W}" height="4" '
                f'fill="#161b22"/>'
                f'<rect class="bar" x="0" y="{H-4}" width="{W}" height="4" '
                f'fill="{EMBER}"/></g>')

    # ---- screen treatment
    body.append(f'<g clip-path="url(#card)">'
                f'<rect x="1" y="39" width="{W-2}" height="{H-40}" '
                f'fill="url(#scan)" opacity=".4"/>'
                f'<rect x="1" y="39" width="{W-2}" height="{H-40}" '
                f'fill="url(#vig)"/></g>')

    # ---- crt power on
    css.append("@keyframes crt{0%{transform:scaleY(0);opacity:0}"
               "1%{transform:scaleY(.004);opacity:1}"
               "2.2%{transform:scaleY(.004);opacity:1}"
               "3.6%{transform:scaleY(1);opacity:.5}"
               "5.4%,100%{transform:scaleY(1);opacity:0}}")
    css.append(f".crt{{transform-origin:440px 249px;animation:crt {T}s "
               f"infinite}}")
    body.append(f'<g clip-path="url(#card)"><rect class="crt" x="1" y="39" '
                f'width="{W-2}" height="{H-40}" fill="{BONE}"/></g>')

    css.append(REDUCE)
    write("boot.svg", head(W, H, "Flint boot sequence", "".join(css),
                           "".join(defs)) + "".join(body) + TAIL)


# ============================================================= chain helper
def node(x, y, w, h, name, sub, cls):
    o = [f'<g class="{cls}">']
    o.append(rect(x, y, w, h, PANEL, RULE, 6, cls="nb"))
    o.append(txt(x + w / 2, y + 26, name, BONE, 13.5, anchor="middle",
                 cls="nt"))
    o.append(txt(x + w / 2, y + 45, sub, DIM, 10.5, anchor="middle"))
    o.append("</g>")
    return "".join(o)


# ============================================================ pagewalk.svg
def pagewalk():
    T = 9.0
    W, H = 880, 320
    xs = [20, 190, 360, 530, 700]
    NW, NH, NY = 140, 64, 176
    css = [".fx{opacity:0}",
           f".nb{{stroke:{RULE};fill:{PANEL}}}",
           f".nt{{fill:{BONE}}}"]
    body = [rect(0.5, 0.5, W - 1, H - 1, INK, RULE, 12)]
    body.append(txt(20, 34, "TRANSLATING", DIM, 10, ls="2"))
    body.append(txt(122, 35, "0xffff_8000_0021_3040", EMBER2, 14))
    body.append(txt(W - 20, 34, "cr3 -> 4 KiB page", DIM, 11, anchor="end"))

    fields = [("[47:39]", "0x100"), ("[38:30]", "0x000"),
              ("[29:21]", "0x001"), ("[20:12]", "0x013"),
              ("[11:0]", "0x040")]
    names = [("PML4", "512 entries"), ("PDPT", "1 GiB each"),
             ("PD", "2 MiB each"), ("PT", "4 KiB each"),
             ("frame", "phys 0x1_2000")]

    for i in range(5):
        x = xs[i]
        t0 = 0.8 + i * 1.2
        t1 = t0 + 1.2
        n = f"n{i}"
        a, b, c, d = pct(t0, T), pct(t0 + .08, T), pct(t1, T), pct(t1 + .08, T)
        settle = EMBER if i == 4 else "#3a2a1c"
        stxt = EMBER2 if i == 4 else DIM
        css.append(
            f"@keyframes {n}b{{0%,{a:.2f}%{{stroke:{RULE};fill:{PANEL}}}"
            f"{b:.2f}%,{c:.2f}%{{stroke:{EMBER};fill:{LIT}}}"
            f"{d:.2f}%,100%{{stroke:{settle};fill:{LIT}}}}}")
        css.append(
            f"@keyframes {n}t{{0%,{a:.2f}%{{fill:{DIM}}}"
            f"{b:.2f}%,{c:.2f}%{{fill:{EMBER2}}}"
            f"{d:.2f}%,100%{{fill:{stxt}}}}}")
        css.append(f".{n} .nb{{animation:{n}b {T}s infinite}}")
        css.append(f".{n} .nt{{animation:{n}t {T}s infinite}}")
        # index field chip
        css.append(kf_show(f"{n}f", t0, T))
        css.append(f".{n}f{{animation:{n}f {T}s infinite}}")
        body.append(rect(xs[i], 84, NW, 30, PANEL, RULE, 4))
        body.append(txt(xs[i] + 10, 104, fields[i][0], DIM, 11))
        body.append(txt(xs[i] + NW - 10, 104, fields[i][1], EMBER2, 11.5,
                        anchor="end", cls=f"fx {n}f"))
        body.append(node(x, NY, NW, NH, names[i][0], names[i][1], n))
        body.append(f'<line x1="{x+NW/2}" y1="114" x2="{x+NW/2}" y2="{NY}" '
                    f'stroke="{RULE}" stroke-dasharray="2 4"/>')

    for i in range(4):
        x0, x1 = xs[i] + NW, xs[i + 1]
        y = NY + NH / 2
        body.append(f'<line x1="{x0}" y1="{y}" x2="{x1-6}" y2="{y}" '
                    f'stroke="{RULE}"/>')
        body.append(f'<path d="M{x1-6} {y-4} L{x1} {y} L{x1-6} {y+4} Z" '
                    f'fill="{RULE}"/>')
        n = f"p{i}"
        t0 = 0.8 + i * 1.2 + 0.9
        a, b = pct(t0, T), pct(t0 + .3, T)
        css.append(f"@keyframes {n}{{0%,{a:.2f}%{{opacity:0;transform:"
                   f"translateX(0)}}{a+0.01:.2f}%{{opacity:1}}"
                   f"{b:.2f}%{{opacity:1;transform:translateX({x1-x0-6}px)}}"
                   f"{b+0.01:.2f}%,100%{{opacity:0;transform:"
                   f"translateX({x1-x0-6}px)}}}}")
        css.append(f".{n}{{animation:{n} {T}s infinite}}")
        body.append(f'<circle class="fx {n} transient" cx="{x0}" cy="{y}" '
                    f'r="3.5" fill="{EMBER}"/>')

    css.append(kf_show("res", 5.8, T))
    css.append(f".res{{animation:res {T}s infinite}}")
    body.append(txt(20, 288, "hit", MINT, 11.5, cls="fx res"))
    body.append(txt(52, 288,
                    "0x0000_0001_2000 + 0x040   ->   0x0000_0001_2040",
                    DIM, 11.5, cls="fx res"))
    body.append(txt(W - 20, 288, "4 loads on a TLB miss, 0 on a hit",
                    DIM, 11, anchor="end", cls="fx res"))
    css.append(REDUCE)
    write("pagewalk.svg", head(W, H, "Four level page walk", "".join(css))
          + "".join(body) + TAIL)


# =========================================================== interrupt.svg
def interrupt():
    T = 6.0
    W, H = 880, 250
    xs = [20, 190, 360, 530, 700]
    NW, NH, NY = 140, 64, 110
    names = [("keypress", "ps/2 scancode"), ("IDT[0x21]", "vector -> stub"),
             ("isr_keyboard", "push regs, eoi"), ("scheduler", "unblock pid 2"),
             ("shell", "ring 3 resumes")]
    rings = [HW, EMBER, EMBER, EMBER, VIOLET]
    css = [".fx{opacity:0}", f".nb{{stroke:{RULE};fill:{PANEL}}}",
           f".nt{{fill:{BONE}}}"]
    body = [rect(0.5, 0.5, W - 1, H - 1, INK, RULE, 12)]
    body.append(txt(20, 34, "INTERRUPT PATH", DIM, 10, ls="2"))
    body.append(txt(W - 20, 34, "one keystroke, 5 stops, ~2 us", DIM, 11,
                    anchor="end"))

    for i in range(5):
        x = xs[i]
        t0 = 0.5 + i * 0.85
        t1 = t0 + 1.1
        n = f"n{i}"
        col = rings[i]
        fillc = LITV if col == VIOLET else (PANEL if col == HW else LIT)
        a, b, c, d = pct(t0, T), pct(t0 + .06, T), pct(t1, T), pct(t1 + .3, T)
        css.append(
            f"@keyframes {n}b{{0%,{a:.2f}%{{stroke:{RULE};fill:{PANEL}}}"
            f"{b:.2f}%,{c:.2f}%{{stroke:{col};fill:{fillc}}}"
            f"{d:.2f}%,100%{{stroke:{RULE};fill:{PANEL}}}}}")
        css.append(
            f"@keyframes {n}t{{0%,{a:.2f}%{{fill:{DIM}}}"
            f"{b:.2f}%,{c:.2f}%{{fill:{col}}}"
            f"{d:.2f}%,100%{{fill:{DIM}}}}}")
        css.append(f".{n} .nb{{animation:{n}b {T}s infinite}}")
        css.append(f".{n} .nt{{animation:{n}t {T}s infinite}}")
        body.append(node(x, NY, NW, NH, names[i][0], names[i][1], n))
        lab = ["hardware", "ring 0", "ring 0", "ring 0", "ring 3"][i]
        body.append(txt(x + NW / 2, NY + NH + 22, lab,
                        HW if i == 0 else (VIOLET if i == 4 else "#4a3a2c"),
                        9.5, anchor="middle", ls="1"))

    for i in range(4):
        x0, x1 = xs[i] + NW, xs[i + 1]
        y = NY + NH / 2
        body.append(f'<line x1="{x0}" y1="{y}" x2="{x1-6}" y2="{y}" '
                    f'stroke="{RULE}"/>')
        body.append(f'<path d="M{x1-6} {y-4} L{x1} {y} L{x1-6} {y+4} Z" '
                    f'fill="{RULE}"/>')
        n = f"p{i}"
        t0 = 0.5 + i * 0.85 + 0.6
        a, b = pct(t0, T), pct(t0 + .25, T)
        css.append(f"@keyframes {n}{{0%,{a:.2f}%{{opacity:0;transform:"
                   f"translateX(0)}}{a+0.01:.2f}%{{opacity:1}}"
                   f"{b:.2f}%{{opacity:1;transform:translateX({x1-x0-6}px)}}"
                   f"{b+0.01:.2f}%,100%{{opacity:0;transform:"
                   f"translateX({x1-x0-6}px)}}}}")
        css.append(f".{n}{{animation:{n} {T}s infinite}}")
        body.append(f'<circle class="fx {n} transient" cx="{x0}" cy="{y}" '
                    f'r="3.5" fill="{EMBER}"/>')

    css.append(kf_window("echo", 4.4, 5.6, T))
    css.append(f".echo{{animation:echo {T}s infinite}}")
    body.append(txt(770, 216, "flint> l", BONE, 13, anchor="middle",
                    cls="fx transient echo"))
    css.append(REDUCE)
    write("interrupt.svg", head(W, H, "Interrupt path", "".join(css))
          + "".join(body) + TAIL)


# =========================================================== scheduler.svg
def scheduler():
    T = 8.8
    W, H = 880, 300
    PH = 2.0
    procs = [("idle", "pid 0", 0), ("init", "pid 1", 0),
             ("shell", "pid 2", 3), ("ticker", "pid 3", 3)]
    css = [".fx{opacity:0}"]
    body = [rect(0.5, 0.5, W - 1, H - 1, INK, RULE, 12)]
    body.append(txt(20, 34, "SCHEDULER", DIM, 10, ls="2"))
    body.append(txt(W - 20, 34, "round robin, preemptive, one tick quantum",
                    DIM, 11, anchor="end"))

    # cpu panel
    body.append(rect(20, 56, 230, 220, PANEL, RULE, 8))
    body.append(txt(38, 82, "CPU 0", BONE, 13))
    body.append(txt(232, 82, "100 Hz", DIM, 11, anchor="end"))
    body.append(f'<line x1="20" y1="96" x2="250" y2="96" stroke="{RULE}"/>')
    body.append(txt(38, 124, "current", DIM, 11))
    body.append(txt(38, 178, "rsp", DIM, 11))
    body.append(txt(38, 206, "cr3", DIM, 11))
    body.append(txt(38, 234, "ring", DIM, 11))

    for p in range(4):
        t0, t1 = 0.4 + p * PH, 0.4 + (p + 1) * PH
        n = f"c{p}"
        css.append(kf_window(n, t0, t1, T))
        css.append(f".{n}{{animation:{n} {T}s infinite}}")
        name, pid, ring = procs[p]
        col = VIOLET if ring == 3 else EMBER
        body.append(f'<g class="fx transient {n}">')
        body.append(txt(38, 152, name, col, 20))
        body.append(txt(232, 152, pid, DIM, 12, anchor="end"))
        body.append(txt(232, 178, f"0x{0x7fff_0000 + p*0x1000:08x}", BONE, 11.5,
                        anchor="end"))
        body.append(txt(232, 206, f"0x{0x0011_9000 + p*0x1000:08x}", BONE, 11.5,
                        anchor="end"))
        body.append(txt(232, 234, str(ring), col, 11.5, anchor="end"))
        body.append("</g>")

    # queue panel
    body.append(rect(330, 56, 530, 220, PANEL, RULE, 8))
    body.append(txt(350, 82, "run queue", BONE, 13))
    body.append(txt(840, 82, "state", DIM, 11, anchor="end"))
    body.append(f'<line x1="330" y1="96" x2="860" y2="96" stroke="{RULE}"/>')

    rows = [128, 168, 208, 248]
    for i, (name, pid, ring) in enumerate(procs):
        y = rows[i]
        col = VIOLET if ring == 3 else EMBER
        body.append(f'<circle cx="356" cy="{y-5}" r="5" fill="{INK}" '
                    f'stroke="{col}"/>')
        body.append(txt(376, y, name, BONE, 13.5))
        body.append(txt(462, y, pid, DIM, 11.5))
        body.append(txt(522, y, f"ring {ring}", col if ring == 3 else DIM, 11))
        for p in range(4):
            t0, t1 = 0.4 + p * PH, 0.4 + (p + 1) * PH
            run = (p == i)
            n = f"s{i}{p}"
            css.append(kf_window(n, t0, t1, T))
            css.append(f".{n}{{animation:{n} {T}s infinite}}")
            body.append(f'<g class="fx transient {n}">')
            if run:
                body.append(f'<circle cx="356" cy="{y-5}" r="5" fill="{col}"/>')
                body.append(txt(840, y, "running", col, 12, anchor="end"))
                body.append(rect(600, y - 13, 180, 10, "none", RULE, 2))
                body.append(rect(600, y - 13, 180, 10, col, rx=2,
                                 extra=f'style="transform-origin:600px 0;'
                                       f'animation:q{p} {T}s linear infinite"'))
            else:
                body.append(txt(840, y, "ready", DIM, 12, anchor="end"))
            body.append("</g>")
    for p in range(4):
        t0, t1 = 0.4 + p * PH, 0.4 + (p + 1) * PH
        a, b = pct(t0, T), pct(t1 - 0.35, T)
        css.append(f"@keyframes q{p}{{0%,{a:.2f}%{{transform:scaleX(0)}}"
                   f"{b:.2f}%,100%{{transform:scaleX(1)}}}}")

    # timer interrupt flashes at each boundary
    for p in range(4):
        t0 = 0.4 + (p + 1) * PH - 0.35
        n = f"irq{p}"
        css.append(kf_window(n, t0, t0 + 0.35, T, fade=0.04))
        css.append(f".{n}{{animation:{n} {T}s infinite}}")
        body.append(txt(290, 172, "irq 0x20", EMBER2, 10.5, anchor="middle",
                        cls=f"fx transient {n}"))
        body.append(txt(290, 186, "preempt", DIM, 9.5, anchor="middle",
                        cls=f"fx transient {n}"))
    css.append(REDUCE)
    write("scheduler.svg", head(W, H, "Round robin scheduler", "".join(css))
          + "".join(body) + TAIL)


# ================================================================ heap.svg
def heap():
    T = 10.0
    W, H = 880, 240
    COLS, ROWS = 32, 3
    CW, CH, GX, GY = 22, 16, 4, 8
    X0, Y0 = 26, 84
    css = [".fx{opacity:0}"]
    body = [rect(0.5, 0.5, W - 1, H - 1, INK, RULE, 12)]
    body.append(txt(20, 34, "PHYSICAL FRAMES", DIM, 10, ls="2"))
    body.append(txt(W - 20, 34, "96 of 512 shown, 4 KiB each", DIM, 11,
                    anchor="end"))

    # allocation phases: (t, count, colour, label)
    phases = [
        (0.6, 14, "#4a3323", "kernel image"),
        (1.6, 8, "#5a3d24", "page tables"),
        (2.6, 24, EMBER, "kernel heap"),
        (4.0, 18, VIOLET, "user process, pid 2"),
        (5.4, 10, VIOLET, "user stacks"),
    ]
    free_at = 7.0     # last 10 frames go back
    cells = []
    idx = 0
    for t, cnt, col, lab in phases:
        for k in range(cnt):
            cells.append((idx, t + k * 0.028, col))
            idx += 1
    total = idx

    for i in range(COLS * ROWS):
        c, r = i % COLS, i // COLS
        x = X0 + c * (CW + GX)
        y = Y0 + r * (CH + GY)
        body.append(rect(x, y, CW, CH, "#141922", "#1d232d", 2))
    for (i, t, col) in cells:
        c, r = i % COLS, i // COLS
        x = X0 + c * (CW + GX)
        y = Y0 + r * (CH + GY)
        n = f"m{i}"
        if i >= total - 10:
            css.append(kf_window(n, t, free_at + (i - (total - 10)) * 0.03, T,
                                 fade=0.05))
            cls = f"fx transient {n}"
        else:
            css.append(kf_show(n, t, T, fade=0.05))
            cls = f"fx {n}"
        css.append(f".{n}{{animation:{n} {T}s infinite}}")
        body.append(rect(x, y, CW, CH, col, "none", 2, cls=cls))

    # legend
    leg = [("#4a3323", "kernel image"), (EMBER, "kernel heap"),
           (VIOLET, "user pages"), ("#141922", "free")]
    lx = 26
    for col, lab in leg:
        body.append(rect(lx, 190, 12, 12, col, "#1d232d", 2))
        body.append(txt(lx + 20, 200, lab, DIM, 11))
        lx += 30 + len(lab) * 7.2

    counters = [(0.6, "used   14"), (1.6, "used   22"), (2.9, "used   46"),
                (4.3, "used   64"), (5.7, "used   74"),
                (7.3, "used   64   free() returned 10 frames")]
    for i, (t, s) in enumerate(counters):
        n = f"k{i}"
        t1 = counters[i + 1][0] if i + 1 < len(counters) else T
        css.append(kf_window(n, t, t1, T, fade=0.05))
        css.append(f".{n}{{animation:{n} {T}s infinite}}")
        last = i == len(counters) - 1
        body.append(txt(W - 26, 200, s, EMBER2 if last else DIM, 11.5,
                        anchor="end",
                        cls=f"fx {n}" if last else f"fx transient {n}"))
    css.append(REDUCE)
    write("heap.svg", head(W, H, "Frame allocator", "".join(css))
          + "".join(body) + TAIL)


# ================================================================ chip.svg
def chip():
    T = 7.5
    W, H = 880, 430
    css = [".fx{opacity:0}", f".cb{{stroke:{RULE};fill:{PANEL}}}",
           f".ct{{fill:{DIM}}}"]
    body = []
    # package
    body.append(rect(90, 20, 700, 390, "#0c1016", RULE, 14))
    for i in range(14):
        y = 46 + i * 26
        body.append(rect(74, y, 16, 6, "#2c333d", rx=1))
        body.append(rect(790, y, 16, 6, "#2c333d", rx=1))
    body.append(f'<circle cx="112" cy="42" r="5" fill="none" stroke="{RULE}"/>')
    body.append(txt(440, 46, "F L I N T   ·   x86_64   ·   no_std rust",
                    DIM, 10, anchor="middle", ls="1"))

    cols = [110, 280, 450, 620]
    rows = [64, 174, 284]
    BW, BH = 150, 94
    blocks = [
        ("boot", "bios -> bootimage", HW),
        ("gdt + tss", "ring 0 / ring 3", EMBER),
        ("idt", "256 vectors", EMBER),
        ("long mode", "cr0, cr4, efer", EMBER),
        ("frames", "bump + free list", EMBER),
        ("paging", "4 level, PML4", EMBER),
        ("kernel heap", "linked list alloc", EMBER),
        ("syscall", "int 0x80 gate", EMBER),
        ("scheduler", "round robin", EMBER),
        ("context switch", "swap rsp, cr3", EMBER),
        ("shell", "ring 3, pid 2", VIOLET),
        ("uart 16550", "serial console", HW),
    ]
    for i, (name, sub, col) in enumerate(blocks):
        c, r = i % 4, i // 4
        x, y = cols[c], rows[r]
        centre = x + BW / 2
        t0 = (centre - 60) / 880 * T
        fillc = LITV if col == VIOLET else (LIT if col == EMBER else "#151a21")
        n = f"b{i}"
        a, b = pct(t0, T), pct(t0 + 0.25, T)
        css.append(f"@keyframes {n}b{{0%,{a:.2f}%{{stroke:{RULE};fill:{PANEL}}}"
                   f"{b:.2f}%,100%{{stroke:{col};fill:{fillc}}}}}")
        css.append(f"@keyframes {n}t{{0%,{a:.2f}%{{fill:{DIM}}}"
                   f"{b:.2f}%,100%{{fill:{col if col != HW else BONE}}}}}")
        css.append(f".{n} .cb{{animation:{n}b {T}s infinite}}")
        css.append(f".{n} .ct{{animation:{n}t {T}s infinite}}")
        body.append(f'<g class="{n}">')
        body.append(rect(x, y, BW, BH, PANEL, RULE, 6, cls="cb"))
        body.append(txt(x + 14, y + 34, name, DIM, 13.5, cls="ct"))
        body.append(txt(x + 14, y + 54, sub, DIM, 10.5))
        body.append(txt(x + BW - 14, y + BH - 12,
                        "hw" if col == HW else ("ring 3" if col == VIOLET
                                                else "ring 0"),
                        HW if col == HW else ("#4b3d6b" if col == VIOLET
                                              else "#4a3a2c"),
                        9.5, anchor="end"))
        body.append("</g>")

    css.append("@keyframes sweep{0%{transform:translateX(-120px)}"
               "100%{transform:translateX(880px)}}")
    css.append(f".sweep{{animation:sweep {T}s linear infinite}}")
    body.append(f'<rect class="sweep" x="0" y="20" width="120" height="390" '
                f'fill="url(#sw)"/>')
    defs = (f'<linearGradient id="sw">'
            f'<stop offset="0" stop-color="{EMBER}" stop-opacity="0"/>'
            f'<stop offset="1" stop-color="{EMBER}" stop-opacity=".14"/>'
            f'</linearGradient>')
    css.append(REDUCE)
    write("chip.svg", head(W, H, "Flint subsystem map", "".join(css), defs)
          + "".join(body) + TAIL)


# =============================================================== shell.svg
def shell():
    T = 10.0
    W, H = 880, 440
    FS = 13.5
    CWID = FS * 0.6
    X = 28
    PROMPT = "flint>"
    PX = X + (len(PROMPT) + 1) * CWID
    css = [".fx{opacity:0}",
           f".screen{{animation:screen {T}s infinite}}",
           "@keyframes screen{0%,1.5%{opacity:0}3%,95%{opacity:1}"
           "100%{opacity:0}}"]
    body = [rect(0.5, 0.5, W - 1, H - 1, INK, RULE, 12)]
    body.append(f'<line x1="0" y1="38" x2="{W}" y2="38" stroke="{RULE}"/>')
    for cx in (24, 44, 64):
        body.append(f'<circle cx="{cx}" cy="19" r="5" fill="{RULE}"/>')
    body.append(txt(88, 24, "ttyS0  ·  ring 3  ·  pid 2", DIM, 11.5))
    body.append('<g class="screen">')

    defs = []
    cmds = [
        (0.6, 64, "help"),
        (2.6, 180, "ps"),
        (4.4, 296, "meminfo"),
        (6.4, 370, "exit"),
    ]
    for i, (t, y, cmd) in enumerate(cmds):
        n = f"c{i}"
        css.append(kf_show(n, t - 0.25, T, fade=0.05))
        css.append(f".{n}{{animation:{n} {T}s infinite}}")
        body.append(f'<g class="fx {n}">')
        body.append(txt(X, y, PROMPT, EMBER, FS,
                        tl=len(PROMPT) * CWID))
        w = len(cmd) * CWID
        dur = len(cmd) * 0.1
        a, b = pct(t, T), pct(t + dur, T)
        css.append(f"@keyframes {n}k{{0%,{a:.2f}%{{transform:"
                   f"translateX({-w:.2f}px)}}{b:.2f}%,100%"
                   f"{{transform:translateX(0)}}}}")
        css.append(f".{n}k{{animation:{n}k {T}s steps({len(cmd)},end) "
                   f"infinite}}")
        defs.append(f'<clipPath id="{n}c"><rect class="{n}k" '
                    f'x="{PX:.2f}" y="{y-FS}" width="{w:.2f}" '
                    f'height="{FS+5}"/></clipPath>')
        body.append(f'<g clip-path="url(#{n}c)">')
        body.append(txt(PX, y, cmd, BONE, FS, tl=w))
        body.append("</g>")
        # caret rides the typed text, then leaves when output lands
        css.append(f"@keyframes {n}r{{0%,{a:.2f}%{{transform:translateX(0)}}"
                   f"{b:.2f}%,100%{{transform:translateX({w:.2f}px)}}}}")
        css.append(f".{n}r{{animation:{n}r {T}s steps({len(cmd)},end) "
                   f"infinite}}")
        last = i == len(cmds) - 1
        t_off = t + dur + (T if last else 1.4)
        css.append(kf_window(f"{n}v", t - 0.25, t_off, T, fade=0.05))
        css.append(f".{n}v{{animation:{n}v {T}s infinite}}")
        body.append(f'<g class="fx {n}v"><rect class="{n}r" x="{PX:.2f}" '
                    f'y="{y-FS+1}" width="{CWID:.2f}" height="{FS+2}" '
                    f'fill="{EMBER}" opacity=".85"/></g>')
        body.append("</g>")

    out = [
        # (t, y, [(x, text, colour, size)])
        (1.3, 85, [(46, "help", BONE, FS), (150, "list commands", DIM, FS)]),
        (1.4, 106, [(46, "ps", BONE, FS), (150, "list processes", DIM, FS)]),
        (1.5, 127, [(46, "meminfo", BONE, FS),
                    (150, "frame and heap usage", DIM, FS)]),
        (1.6, 148, [(46, "exit", BONE, FS), (150, "halt the kernel", DIM, FS)]),
        (3.1, 201, [(46, "PID", DIM, 12), (110, "RING", DIM, 12),
                    (190, "STATE", DIM, 12), (300, "NAME", DIM, 12)]),
        (3.2, 222, [(46, "0", BONE, FS), (110, "0", EMBER, FS),
                    (190, "ready", DIM, FS), (300, "idle", BONE, FS)]),
        (3.3, 243, [(46, "1", BONE, FS), (110, "0", EMBER, FS),
                    (190, "ready", DIM, FS), (300, "init", BONE, FS)]),
        (3.4, 264, [(46, "2", BONE, FS), (110, "3", VIOLET, FS),
                    (190, "running", MINT, FS), (300, "shell", BONE, FS)]),
        (5.4, 317, [(46, "frames", DIM, FS), (150, "74 / 512", BONE, FS),
                    (280, "4 KiB each", DIM, FS)]),
        (5.5, 338, [(46, "heap", DIM, FS), (150, "41 / 128 KiB", BONE, FS),
                    (280, "linked list allocator", DIM, FS)]),
        (7.2, 391, [(28, "[ kernel halted ]", EMBER2, FS)]),
    ]
    for i, (t, y, parts) in enumerate(out):
        n = f"o{i}"
        css.append(kf_show(n, t, T, fade=0.06))
        css.append(f".{n}{{animation:{n} {T}s infinite}}")
        body.append(f'<g class="fx {n}">')
        for (x, s, c, sz) in parts:
            body.append(txt(x, y, s, c, sz))
        body.append("</g>")

    body.append("</g>")
    css.append(REDUCE)
    write("shell.svg", head(W, H, "Flint shell session", "".join(css),
                            "".join(defs)) + "".join(body) + TAIL)


boot()
chip()
pagewalk()
interrupt()
scheduler()
heap()
shell()
