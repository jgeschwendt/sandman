// The `chart` verb's own output as a timeline: one bar per active session (start to end, or on to `at` while open),
// one dot per taken transcript coloured by state, one dot per memory file coloured by source. Data: docs/data/chart.json,
// the sample the test suite proves is `sandman chart`'s output over its fixture.
import { d3, hover, load, mount, redraw } from "./lib.js";

const root = document.getElementById("timeline");
const data = await load("chart.json");

const LABEL = 90;
const LANE = 14;
const DOT = 26;
const STATE = { archived: "var(--kind-7)", dreamed: "var(--kind-3)", waiting: "var(--kind-1)" };
const SOURCE = { dream: "var(--kind-2)", other: "var(--kind-6)", reflect: "var(--kind-4)", remember: "var(--kind-5)" };
const time = d3.utcParse("%Y-%m-%dT%H:%M:%SZ");
const short = (id) => id.slice(-4);
const memories = data.memories.filter((m) => m.created);
const times = [data.at, ...data.active.flatMap((a) => [a.start, a.end]), ...data.taken.map((t) => t.taken), ...memories.map((m) => m.created)];

const { frame, hide, show } = mount(root, {
  head: ["row", "when", "what"],
  legend: [
    { color: "var(--ui-fg-muted)", label: "active" },
    ...Object.entries(STATE).map(([label, color]) => ({ color, label: `taken · ${label}`, mark: "dot" })),
    ...Object.entries(SOURCE).map(([label, color]) => ({ color, label: `memory · ${label}`, mark: "dot" })),
  ],
  rows: [
    ...data.active.map((a) => ["active", `${a.start ?? "…"} → ${a.end ?? "open"}`, `${a.session} · ${a.machine} · ${a.cwd}`]),
    ...data.taken.map((t) => ["taken", t.taken, `${t.session} · ${t.state} · ${t.title ?? "untitled"}`]),
    ...data.memories.map((m) => ["memories", m.created ?? "no created (not drawn)", `${m.bank}/${m.file} · ${m.source}`]),
  ],
});

redraw(root, frame, 560, (width) => {
  const x = d3.scaleUtc().domain(d3.extent(times.filter(Boolean), time)).range([LABEL, width - 24]).nice();
  const top = data.active.length * LANE + 12;
  const rows = [["active", 0], ["taken", top + DOT / 2], ["memories", top + DOT * 1.5]];
  const height = top + DOT * 2 + 28;
  const svg = frame.append("svg").attr("width", width).attr("height", height).attr("role", "img").attr("aria-label", "sample chart output as a timeline");
  svg.append("g").attr("class", "chart-heads").selectAll("text").data(rows).join("text").attr("y", ([, y], i) => (i ? y + 4 : 14)).text(([name]) => name);
  svg.append("g").attr("class", "chart-axis").attr("transform", `translate(0,${height - 22})`).call(d3.axisBottom(x).ticks(width / 110));
  svg.append("line").attr("class", "chart-rule").attr("x1", x(time(data.at))).attr("x2", x(time(data.at))).attr("y1", 0).attr("y2", height - 22);

  const bars = svg.append("g").selectAll("rect").data(data.active).join("rect").attr("class", "chart-bar-mark").classed("is-open", (a) => !a.end).attr("tabindex", 0)
    .attr("x", (a) => x(time(a.start ?? a.end))).attr("y", (_, i) => 4 + i * LANE).attr("height", LANE - 4).attr("rx", 3)
    .attr("width", (a) => Math.max(4, x(time(a.end ?? data.at)) - x(time(a.start ?? a.end))));
  const dots = (items, y, when, color) => svg.append("g").selectAll("circle").data(items).join("circle").attr("class", "chart-dot").attr("tabindex", 0)
    .attr("cx", (d) => x(time(when(d)))).attr("cy", y).attr("r", 6).style("fill", color);
  const taken = dots(data.taken, rows[1][1], (t) => t.taken, (t) => STATE[t.state]);
  const banked = dots(memories, rows[2][1], (m) => m.created, (m) => SOURCE[m.source]);

  hover(bars, (event, a) => show(event.currentTarget, `session …${short(a.session)}`, [["start", a.start ?? "no start mark"], ["end", a.end ?? "open"], ["machine", a.machine], ["cwd", a.cwd ?? "—"]]), hide);
  hover(taken, (event, t) => show(event.currentTarget, t.title ?? `session …${short(t.session)}`, [["taken", t.taken], ["state", t.state], ["bytes", String(t.bytes)], ["highlights", String(t.highlights)], ["memories", String(t.memories)], ["dreamed", t.dreamed ?? "—"]]), hide);
  hover(banked, (event, m) => show(event.currentTarget, m.name, [["bank", m.bank], ["file", m.file], ["source", m.source], ["session", m.session ? `…${short(m.session)}` : "—"], ["created", m.created]]), hide);
});
