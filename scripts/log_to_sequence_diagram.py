#!/usr/bin/env python3
"""Render an SFU chat debug log as a sequence diagram of client ↔ SFU signaling.

The chat example's signaling layer (`examples/signaling/mod.rs`) logs every SDP
message it relays between a browser and the SFU, tagged with `room=<id>
client=<id>`, followed by the full SDP on the next lines. This script extracts
those events, plus the SFU's "unknown client" drops (`src/room.rs`), and renders
a sequence diagram whose only roles are the clients (labelled by client id) and
the SFU server — no apprtc or signaling-hub hops.

Two output formats:

  --format mermaid   (default) a ```mermaid sequenceDiagram — brief events.
  --format html      a self-contained interactive page: the same brief events,
                     but every offer/answer/re-offer/re-answer has a clickable
                     [+] that expands the full SDP inline.

Usage:
    scripts/log_to_sequence_diagram.py debug.log
    scripts/log_to_sequence_diagram.py debug.log --format html -o seq.html
    scripts/log_to_sequence_diagram.py debug.log --room 30606874 --no-drops
"""

import argparse
import html
import re
import sys
from dataclasses import dataclass, field

# path:line [LEVEL] HH:MM:SS.ffffff - message  (start of a log entry)
LINE_RE = re.compile(
    r"^(?P<src>\S+?):(?P<ln>\d+) \[(?P<lvl>[A-Z]+)\] "
    r"(?P<ts>\d{2}:\d{2}:\d{2}\.\d+) - (?P<msg>.*)$"
)

# Signaling relay events. `dir` is the SDP hop between browser and SFU:
#   in  → client ->> SFU   (client-initiated)
#   out → SFU -->> client  (SFU-initiated / response)
# `sdp` marks events whose full SDP body follows on the next lines.
SIGNALING = [
    ("register", "in", False, re.compile(r"^Register\s+browser->SFU room=(\d+) client=(\d+)")),
    ("offer", "in", True, re.compile(r"^Offer\s+browser->SFU room=(\d+) client=(\d+)")),
    ("answer", "out", True, re.compile(r"^Answer\s+SFU->browser room=(\d+) client=(\d+)")),
    ("re-offer", "out", True, re.compile(r"^Re-Offer\s+SFU->browser room=(\d+) client=(\d+)")),
    ("re-answer", "in", True, re.compile(r"^Re-Answer\s+browser->SFU room=(\d+) client=(\d+)")),
    ("leave", "in", False, re.compile(r"^Leave\s+browser->SFU room=(\d+) client=(\d+)(?P<ws>.*)")),
]

# SFU dropping a relayed message whose client already left the room.
DROP_RE = re.compile(r"^Received message for unknown client (\d+)")


@dataclass
class Event:
    ts: str
    room: str
    client: str
    kind: str  # event label
    dir: str  # "in" (client->SFU), "out" (SFU->client), or "note"
    sdp: str = ""


def parse(path, room_filter, include_drops):
    with open(path, encoding="utf-8", errors="replace") as fh:
        lines = fh.read().splitlines()

    events = []
    for i, line in enumerate(lines):
        m = LINE_RE.match(line)
        if not m:
            continue
        src, ts, msg = m["src"], m["ts"], m["msg"]

        if src.endswith("signaling/mod.rs"):
            for kind, direction, has_sdp, rx in SIGNALING:
                hit = rx.match(msg)
                if not hit:
                    continue
                room, client = hit.group(1), hit.group(2)
                if room_filter and room != room_filter:
                    break
                label = kind
                if kind == "leave" and hit.groupdict().get("ws", "").strip():
                    label = "leave (ws closed)"
                sdp = _capture_sdp(lines, i) if has_sdp else ""
                events.append(Event(ts, room, client, label, direction, sdp))
                break
        elif include_drops and src.endswith("src/room.rs"):
            hit = DROP_RE.match(msg)
            if hit:
                events.append(Event(ts, "", hit.group(1), "drop msg for departed client", "note"))
    return events


def _capture_sdp(lines, i):
    """Collect the raw SDP body that follows log line `i` (until the next log entry)."""
    body = []
    for line in lines[i + 1 :]:
        if LINE_RE.match(line):
            break
        body.append(line)
    while body and not body[-1].strip():
        body.pop()
    return "\n".join(body)


def participants(events):
    """SFU-implicit; clients in first-seen order."""
    seen = []
    for ev in events:
        if ev.client and ev.client not in seen:
            seen.append(ev.client)
    return seen


# ─────────────────────────────── mermaid ────────────────────────────────


def render_mermaid(events, room_filter):
    out = ["```mermaid", "sequenceDiagram", "    autonumber", "    participant SFU as SFU server"]
    for c in participants(events):
        out.append(f"    participant c{c} as {c}")

    rooms = room_filter or ", ".join(sorted({ev.room for ev in events if ev.room}))
    span = f"{events[0].ts}–{events[-1].ts}" if events else "—"
    out.append(f"    note over SFU: room {rooms} · {len(events)} events · {span}")

    for ev in events:
        cid = f"c{ev.client}"
        if ev.dir == "in":
            out.append(f"    {cid} ->> SFU: {ev.kind}")
        elif ev.dir == "out":
            out.append(f"    SFU -->> {cid}: {ev.kind}")
        else:
            out.append(f"    note over SFU: {ev.kind} {ev.client}")
        if ev.kind == "register":
            out.append(f"    note over {cid},SFU: ▶ client {ev.client} JOINS")
        elif ev.kind.startswith("leave"):
            out.append(f"    note over {cid},SFU: ⏹ client {ev.client} LEAVES")

    out.append("```")
    return "\n".join(out)


# ──────────────────────────────── html ──────────────────────────────────

HTML_TEMPLATE = """<!doctype html>
<html lang="en"><head>
<meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>SFU signaling — room {room}</title>
<style>
  :root {{ --line:#94a3b8; --in:#2563eb; --out:#7c3aed; }}
  * {{ box-sizing: border-box; }}
  body {{ margin: 0; padding: 20px; font: 14px/1.4 system-ui, sans-serif; color:#0f172a; background:#f8fafc; }}
  h1 {{ font-size: 1.15rem; margin: 0 0 2px; }}
  .meta {{ color:#64748b; font-size: 12px; margin-bottom: 14px; }}
  .scroll {{ overflow-x: auto; }}
  .diagram {{ --cols: repeat({n}, minmax(150px, 1fr)); min-width: {minw}px; }}
  .headers {{ position: sticky; top: 0; z-index: 3; display: grid; grid-template-columns: var(--cols);
             background: #f8fafc; padding: 8px 0; border-bottom: 1px solid #e2e8f0; }}
  .lane {{ text-align:center; }}
  .lane b {{ display:inline-block; padding:4px 10px; border-radius:8px; font-size:12.5px; font-weight:700;
            background:#fff; border:1.5px solid var(--c, #cbd5e1); color: var(--c, #334155); }}
  .lane.sfu b {{ background:#0f172a; color:#fff; border-color:#0f172a; }}
  .board {{ position: relative; }}
  .lifelines {{ position:absolute; inset:0; display:grid; grid-template-columns: var(--cols); z-index:0; }}
  .lifeline {{ justify-self:center; border-left:1.5px dashed var(--c, #cbd5e1); opacity:.55; }}
  .steps {{ position: relative; z-index:1; }}
  .step {{ display:grid; grid-template-columns: var(--cols); align-items:center; }}
  .arrow {{ display:flex; align-items:center; padding:5px 0; min-height:26px; }}
  .arrow .stalk {{ flex:1 1 auto; border-top:1.5px solid var(--c, var(--line)); }}
  .arrow.out .stalk {{ border-top-style: dashed; }}
  .arrow .head {{ flex:0 0 auto; width:0; height:0; border:5px solid transparent; }}
  .arrow .head.right {{ border-left-color: var(--c, var(--line)); }}
  .arrow .head.left {{ border-right-color: var(--c, var(--line)); }}
  .arrow .label {{ flex:0 0 auto; padding:1px 8px; white-space:nowrap; background:#f8fafc;
                  border-radius:6px; font-size:12.5px; box-shadow: inset 0 -2px 0 var(--c, var(--line)); }}
  .seq {{ color:#94a3b8; font-variant-numeric: tabular-nums; margin-right:6px; }}
  .toggle {{ cursor:pointer; user-select:none; border:1px solid #cbd5e1; background:#fff; color:#334155;
            border-radius:4px; padding:0 5px; font: inherit; font-size:11px; line-height:16px; margin-left:6px; }}
  .toggle:hover {{ background:#eef2ff; }}
  .sdp {{ grid-column: 1 / -1; display:none; }}
  .sdp.open {{ display:block; }}
  .sdp pre {{ margin:2px 0 10px; max-height:320px; overflow:auto; background:#0f172a; color:#e2e8f0;
             padding:10px 12px; border-radius:8px; font: 11.5px/1.45 ui-monospace, monospace; }}
  .note {{ text-align:center; margin:6px 0; }}
  .note span {{ display:inline-block; padding:2px 10px; border-radius:9999px; font-size:12px;
               white-space:nowrap; color: var(--c, #334155); border:1.5px solid var(--c, #cbd5e1);
               background:#fff; background: color-mix(in srgb, var(--c, #94a3b8) 14%, #fff); }}
  .controls {{ margin: 4px 0 12px; }}
  .controls button {{ font: inherit; font-size:12px; padding:3px 10px; border-radius:6px;
                     border:1px solid #cbd5e1; background:#fff; cursor:pointer; }}
</style></head>
<body>
<h1>SFU signaling — room {room}</h1>
<div class="meta">{count} events · {span} · clients labelled by id · solid = client→SFU, dashed = SFU→client</div>
<div class="controls"><button onclick="allSdp(true)">Expand all SDP</button>
<button onclick="allSdp(false)">Collapse all</button></div>
<div class="scroll"><div class="diagram">
  <div class="headers">{heads}</div>
  <div class="board">
    <div class="lifelines">{lifelines}</div>
    <div class="steps">{steps}</div>
  </div>
</div></div>
<script>
  document.querySelectorAll('.toggle').forEach(function (t) {{
    t.addEventListener('click', function () {{
      var box = document.getElementById(t.dataset.target);
      var open = box.classList.toggle('open');
      t.textContent = open ? '[−]' : '[+]';
    }});
  }});
  function allSdp(open) {{
    document.querySelectorAll('.sdp').forEach(function (b) {{ b.classList.toggle('open', open); }});
    document.querySelectorAll('.toggle').forEach(function (t) {{ t.textContent = open ? '[−]' : '[+]'; }});
  }}
</script>
</body></html>
"""


# Distinct per-client colors (cycled if there are more clients than entries).
PALETTE = [
    "#2563eb", "#dc2626", "#059669", "#d97706", "#7c3aed",
    "#0891b2", "#db2777", "#65a30d", "#ea580c", "#4f46e5",
]


def render_html(events, room_filter):
    clients = participants(events)
    # Grid columns: SFU first (col 1), then clients.
    col = {"SFU": 1}
    for idx, c in enumerate(clients):
        col[c] = idx + 2
    n = len(col)
    # A stable color per client — used for its lane, lifeline, and every arrow it takes part in.
    color_of = {c: PALETTE[i % len(PALETTE)] for i, c in enumerate(clients)}

    heads = ['<div class="lane sfu"><b>SFU server</b></div>']
    for c in clients:
        heads.append(f'<div class="lane" style="--c:{color_of[c]}"><b>{html.escape(c)}</b></div>')
    lifelines = "".join(
        ['<div class="lifeline"></div>']
        + [f'<div class="lifeline" style="--c:{color_of[c]}"></div>' for c in clients]
    )

    steps = []
    for seq, ev in enumerate(events, 1):
        if ev.dir == "note":
            # Placed under the departed client's own lane (and tinted its color), not centered.
            steps.append(
                f'<div class="step"><div class="note drop" '
                f'style="grid-column:{col[ev.client]};--c:{color_of[ev.client]}">'
                f"<span>{html.escape(ev.kind)} {html.escape(ev.client)}</span></div></div>"
            )
            continue

        src, dst = ("SFU", ev.client) if ev.dir == "out" else (ev.client, "SFU")
        cs, cd = col[src], col[dst]
        lo, hi = min(cs, cd), max(cs, cd)
        head_side = "right" if cd > cs else "left"
        # The arrow grid-item spans whole lanes (lo..hi), but the lifelines sit at lane centers.
        # Inset the arrow by half a lane on each side so it runs center-to-center of the two lanes.
        inset = 50.0 / (hi - lo + 1)

        label = f'<span class="seq">{seq}</span>{html.escape(ev.kind)}'
        if ev.sdp:
            sdp_id = f"sdp{seq}"
            label += f'<button class="toggle" data-target="{sdp_id}">[+]</button>'

        head = f'<span class="head {head_side}"></span>'
        stalk = '<span class="stalk"></span>'
        inner = f'{stalk}<span class="label">{label}</span>{stalk}'
        arrow = f'{head}{inner}' if head_side == "left" else f'{inner}{head}'

        row = [f'<div class="step">']
        row.append(
            f'<div class="arrow {ev.dir}" '
            f'style="grid-column:{lo} / {hi + 1};padding-left:{inset:.3f}%;padding-right:{inset:.3f}%;'
            f'--c:{color_of[ev.client]}">'
            f"{arrow}</div>"
        )
        if ev.sdp:
            row.append(f'<pre class="sdp" id="sdp{seq}">{html.escape(ev.sdp)}</pre>')
        row.append("</div>")
        steps.append("".join(row))

        # Membership dividers — placed under the client's own lane and tinted its color.
        style = f'style="grid-column:{col[ev.client]};--c:{color_of[ev.client]}"'
        if ev.kind == "register":
            steps.append(f'<div class="step"><div class="note join" {style}><span>▶ client {html.escape(ev.client)} joins</span></div></div>')
        elif ev.kind.startswith("leave"):
            steps.append(f'<div class="step"><div class="note leave" {style}><span>⏹ client {html.escape(ev.client)} leaves</span></div></div>')

    rooms = room_filter or ", ".join(sorted({ev.room for ev in events if ev.room}))
    span = f"{events[0].ts}–{events[-1].ts}" if events else "—"
    return HTML_TEMPLATE.format(
        room=html.escape(rooms),
        n=n,
        minw=n * 150,
        count=len(events),
        span=span,
        heads="".join(heads),
        lifelines=lifelines,
        steps="".join(steps),
    )


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("log", help="path to the SFU chat debug log")
    ap.add_argument("-f", "--format", choices=["mermaid", "html"], default="mermaid")
    ap.add_argument("-o", "--out", help="write here (default: stdout)")
    ap.add_argument("--room", help="only include this room id")
    ap.add_argument("--no-drops", action="store_true", help="omit SFU 'unknown client' drop notes")
    args = ap.parse_args()

    events = parse(args.log, args.room, include_drops=not args.no_drops)
    if not events:
        sys.exit("no client/SFU signaling events found (wrong log, or --room filtered everything out)")

    diagram = render_html(events, args.room) if args.format == "html" else render_mermaid(events, args.room)
    if args.out:
        with open(args.out, "w", encoding="utf-8") as fh:
            fh.write(diagram + "\n")
        print(f"wrote {len(events)} events ({args.format}) to {args.out}", file=sys.stderr)
    else:
        print(diagram)


if __name__ == "__main__":
    main()
