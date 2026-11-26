# SPDX-License-Identifier: LGPL-2.1-or-later
"""edid_facts_oracle.py — turns `edid-decode --skip-hex-dump` output into normalized fact lines,
the same format `cargo run --example edid_facts` prints for displaycaps-parse, so the two can be
diffed (scripts/edid-diff.sh). Lab tooling only; never shipped.

Usage: edid-decode --skip-hex-dump FILE | python3 scripts/edid_facts_oracle.py

Fact lines (sorted, de-duplicated):
  version 1.4
  mfg HEC / product 41008 / name HISENSE
  mode <src> <W>x<H>[i] <refresh Hz, 3 decimals> [pclk <kHz>]
      src: est | std | dtd | cvt3 | est3 | vic | hdmivic | did
  vic <n> / vic420only <n> / vic420cap <n>
  range v <min>-<max> h <min>-<max> pclk <MHz>
  vcdb qy <0|1> qs <0|1>
  colorimetry <name>          (xvYCC601 …, BT2020RGB, DCI-P3, ICtCp, ST2113RGB)
  eotf <sdr|hdr|pq|hlg>
  hdmi tmds <MHz> / hdmi dc <30|36|48|y444>
  hf version <n> / hf tmds <MHz> / hf frl <lanes>x<gbps> / hf allm / hf vrr <min>-<max> / hf dsc <flag>
  freesync min <Hz> / freesync max <Hz>
  dv version <n> / hdr10plus <app version>
  displayid version <x.y>
"""
import re
import sys

facts = set()
section = None
block_kind = None


def add(*parts):
    facts.add(" ".join(str(p) for p in parts))


def mode(src, line):
    m = re.search(r"(\d+)x(\d+)(i?)\s+([\d.]+) Hz", line)
    if not m:
        return
    w, h, il, hz = m.groups()
    pc = re.search(r"([\d.]+) MHz", line[m.end():])
    pclk = f" pclk {round(float(pc.group(1)) * 1000)}" if pc else ""
    add(f"mode {src} {w}x{h}{il} {float(hz):.3f}{pclk}")


EOTF = {
    "Traditional gamma - SDR luminance range": "sdr",
    "Traditional gamma - HDR luminance range": "hdr",
    "SMPTE ST2084": "pq",
    "Hybrid Log-Gamma": "hlg",
}
COLORIMETRY = {"xvYCC601", "xvYCC709", "sYCC601", "opYCC601", "opRGB", "BT2020cYCC", "BT2020YCC",
               "BT2020RGB", "DCI-P3", "ICtCp", "ST2113RGB"}

for raw in sys.stdin:
    line = raw.rstrip("\n")
    s = line.strip()
    if re.match(r"Block \d+, ", s):
        block_kind = "displayid" if "DisplayID" in s else "cta" if "CTA-861" in s else "base"
        section = None
        continue
    if s.startswith("EDID Structure Version & Revision:"):
        add("version", s.split(":")[1].strip())
    elif s.startswith("Manufacturer:") and block_kind == "base":
        add("mfg", s.split(":", 1)[1].strip())
    elif s.startswith("Model:") and block_kind == "base":
        add("product", s.split(":", 1)[1].strip())
    elif s.startswith("Display Product Name:"):
        add("name", s.split(":", 1)[1].strip().strip("'").strip())
    elif re.match(r"Version: \d+\.\d+$", s) and block_kind == "displayid":
        add("displayid version", s.split(":")[1].strip())
    # Section headers
    if s.endswith(":") and not s.startswith(("DTD", "VIC", "DMT")):
        section = s[:-1]
    # Modes
    if block_kind == "base" and section == "Established Timings I & II" and re.match(r"(DMT|IBM|Apple|VESA)\b", s):
        mode("est", s)
    elif block_kind == "base" and section == "Standard Timings" and re.match(r"(DMT|CVT|GTF|Unknown)", s):
        mode("std", s)
    elif s.startswith("DTD") and block_kind in ("base", "cta"):
        mode("dtd", s)
        if block_kind == "base" and s.startswith("DTD 1:"):
            m = re.search(r"(\d+)x(\d+)(i?)\s+([\d.]+) Hz", s)
            add(f"preferred {m.group(1)}x{m.group(2)}{m.group(3)} {float(m.group(4)):.3f}")
    elif section and "CVT 3 Byte" in section and re.match(r"CVT", s):
        mode("cvt3", s)
    elif section and "Established timings III" in section and s.startswith("DMT"):
        mode("est3", s)
    elif re.match(r"VIC\s+\d+", s) and section and "Video Data Block" in section and "4:2:0" not in section:
        n = int(re.match(r"VIC\s+(\d+)", s).group(1))
        add("vic", n)
        if s.endswith("(native)"):
            add("vic native", n)
        mode("vic", s)
    elif re.match(r"VIC\s+\d+", s) and section == "YCbCr 4:2:0 Video Data Block":
        add("vic420only", int(re.match(r"VIC\s+(\d+)", s).group(1)))
    elif re.match(r"VIC\s+\d+", s) and section == "YCbCr 4:2:0 Capability Map Data Block":
        add("vic420cap", int(re.match(r"VIC\s+(\d+)", s).group(1)))
    elif s.startswith("HDMI VIC"):
        mode("hdmivic", s)
    elif block_kind == "displayid" and re.match(r"(DTD|VIC|DMT|CVT|Type)", s) and " Hz" in s:
        mode("did", s)
    # Range limits
    m = re.search(r"Monitor ranges \(.*\): (\d+)-(\d+) Hz V, (\d+)-(\d+) kHz H(?:, max dotclock (\d+) MHz)?", s)
    if m:
        a, b, c, d, e = m.groups()
        add(f"range v {a}-{b} h {c}-{d}" + (f" pclk {e}" if e else ""))
    # Video capability
    if s.startswith("YCbCr quantization:"):
        add("vcdb qy", 1 if "Selectable" in s else 0)
    elif s.startswith("RGB quantization:"):
        add("vcdb qs", 1 if "Selectable" in s else 0)
    if section == "Colorimetry Data Block" and s in COLORIMETRY:
        add("colorimetry", s)
    if s in EOTF:
        add("eotf", EOTF[s])
    m = re.match(r"Desired content (max|max frame-average|min) luminance: (\d+) ", s)
    if m:
        add("hdr lum", {"max": "max", "max frame-average": "fall", "min": "min"}[m.group(1)], m.group(2))
    # HDMI and HDMI Forum
    if section and "OUI 00-0C-03" in section:
        if s.startswith("Maximum TMDS clock:"):
            add("hdmi tmds", re.search(r"(\d+) MHz", s).group(1))
        elif s in ("DC_48bit", "DC_36bit", "DC_30bit"):
            add("hdmi dc", s[3:5])
        elif s == "DC_Y444":
            add("hdmi dc y444")
    if section and ("OUI C4-5D-D8" in section or "Sink Capability" in section):
        if s.startswith("Version:"):
            add("hf version", s.split(":")[1].strip())
        elif s.startswith("Maximum TMDS Character Rate:"):
            add("hf tmds", re.search(r"(\d+) MHz", s).group(1))
        elif s.startswith("Max Fixed Rate Link:") and "Not Supported" not in s:
            # e.g. "3 and 6 Gbps per lane on 3 lanes, 6, 8, 10 and 12 Gbps on 4 lanes": the last
            # rate on the last lane count is the maximum.
            lanes = re.findall(r"on (\d) lanes", s)
            rates = re.findall(r"(\d+) Gbps", s)
            if lanes and rates:
                add(f"hf frl {lanes[-1]}x{rates[-1]}")
        elif s == "Supports Auto Low-Latency Mode":
            add("hf allm")
        elif re.match(r"Supports (\d+)-bits/component Deep Color 4:2:0", s):
            add("hf dc420", re.match(r"Supports (\d+)", s).group(1))
        elif re.match(r"Supports (\d+) bpc Compressed Video Transport", s):
            add("hf dsc bpc", re.match(r"Supports (\d+)", s).group(1))
        elif s.startswith("DSC Max Slices:"):
            add("hf dsc slices", re.search(r"up to (\d+) slices", s).group(1))
        elif s.startswith("DSC Max Fixed Rate Link:"):
            lanes = re.findall(r"on (\d) lanes", s)
            rates = re.findall(r"(\d+) Gbps", s)
            if lanes and rates:
                add(f"hf dscfrl {lanes[-1]}x{rates[-1]}")
        elif s.startswith("VRRmin:"):
            vmin = re.search(r"(\d+) Hz", s).group(1)
            facts.add(f"hf vrrmin {vmin}")
        elif s.startswith("VRRmax:"):
            facts.add(f"hf vrrmax {re.search(r'(\d+) Hz', s).group(1)}")
        elif s.startswith("Supports VESA DSC 1.2a"):
            add("hf dsc 1.2a")
        elif s.startswith("Supports Compressed Video Transport for 4:2:0"):
            add("hf dsc native420")
    if section and "OUI 00-00-1A" in section:
        if s.startswith("Minimum Refresh Rate:"):
            add("freesync min", re.search(r"(\d+) Hz", s).group(1))
        elif s.startswith("Maximum Refresh Rate:"):
            add("freesync max", re.search(r"(\d+) Hz", s).group(1))
    if section and "Video Data Block (Dolby)" in section and s.startswith("Version:"):
        add("dv version", re.match(r"Version: (\d+)", s).group(1))
    if section and "Video Data Block (Dolby)" in section:
        m = re.match(r"Target (Max|Min) PQ v\d: (\d+)", s)
        if m:
            add("dv pq", m.group(1).lower(), m.group(2))
    if section and "OUI 90-84-8B" in section and s.startswith("Application Version:"):
        add("hdr10plus", s.split(":")[1].strip())
    if section and "OUI 90-84-8B" in section:
        m = re.match(r"(Full Frame Peak|Peak) Luminance Index: (\d+)", s)
        if m:
            add("hdr10plus", "ffpeak" if m.group(1).startswith("Full") else "peak", m.group(2))

for f in sorted(facts):
    print(f)
