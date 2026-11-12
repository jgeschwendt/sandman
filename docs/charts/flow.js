// The data flow, ①–⑤ marking the order a session passes it: a session timeline down the left edge, broken before the
// later dream pass; its moments (① ② start and end) → mark → the marks files, and → the earlier sandman's take and dream
// → the archive, the pointers and the bank files (③ ④ ⑤); all of them → chart → its consumer. Nodes and edges from
// docs/data/flow.json, each node at its column and row; a moment is a dot on the timeline, every other node a box.
import { d3, hover, load, mount, redraw } from "./lib.js";

const root = document.getElementById("flow");
const data = await load("flow.json");

const COLS = [180, 120, 290, 120, 150];
const H = 32;
const STEP = 48;
const KIND = { earlier: "var(--kind-5)", file: "var(--kind-7)", moment: "var(--kind-1)", sink: "var(--kind-4)", verb: "var(--kind-3)" };
// the legend's words per kind; the earlier sandman's writers are dashed as well as coloured
const NAME = { earlier: "earlier sandman", file: "file", moment: "moment", sink: "consumer", verb: "this rewrite" };
const byId = new Map(data.nodes.map((n) => [n.id, n]));

const { frame, hide, show } = mount(root, {
  head: ["from", "to", "carries"],
  legend: Object.entries(KIND).map(([kind, color]) => ({ color, label: NAME[kind], mark: kind === "moment" && "dot" })),
  rows: data.edges.map((e) => [byId.get(e.from).label, byId.get(e.to).label, e.label]),
});

redraw(root, frame, d3.sum(COLS) + 4 * 40, (width) => {
  const gap = (width - d3.sum(COLS)) / (COLS.length - 1);
  const left = COLS.map((_, i) => d3.sum(COLS.slice(0, i)) + i * gap);
  const height = (d3.max(data.nodes, (n) => n.row) + 1) * STEP;
  const at = new Map(data.nodes.map((n) => [n.id, { x: left[n.col], y: n.row * STEP + (STEP - H) / 2 }]));
  const svg = frame.append("svg").attr("width", width).attr("height", height).attr("role", "img").attr("aria-label", "sandman data flow");

  svg.append("defs").append("marker").attr("id", "flow-head").attr("viewBox", "0 0 8 8").attr("refX", 8).attr("refY", 4)
    .attr("markerUnits", "userSpaceOnUse").attr("markerWidth", 8).attr("markerHeight", 8).attr("orient", "auto").append("path").attr("d", "M0,0L8,4L0,8Z");
  const link = d3.linkHorizontal().x((p) => p[0]).y((p) => p[1]);
  const edges = svg.append("g").attr("class", "chart-links").selectAll("path").data(data.edges).join("path").attr("marker-end", "url(#flow-head)")
    .attr("d", (e) => link({ source: [at.get(e.from).x + COLS[byId.get(e.from).col], at.get(e.from).y + H / 2], target: [at.get(e.to).x, at.get(e.to).y + H / 2] }));

  const time = svg.append("g").attr("class", "chart-time").attr("aria-hidden", "true");
  time.append("path").attr("marker-end", "url(#flow-head)").attr("d", `M6,0V${at.get("later").y - 16}M6,${at.get("later").y - 4}V${height}`);
  time.append("text").attr("x", 16).attr("y", height - 2).text("time");

  const nodes = svg.append("g").selectAll("g").data(data.nodes).join("g").attr("class", (n) => `chart-node is-${n.kind}`).attr("tabindex", 0)
    .attr("transform", (n) => `translate(${at.get(n.id).x},${at.get(n.id).y})`);
  const boxes = nodes.filter((n) => n.kind !== "moment");
  boxes.append("rect").attr("width", (n) => COLS[n.col]).attr("height", H).attr("rx", 6);
  boxes.append("rect").attr("class", "chart-kind").attr("width", 4).attr("height", H - 12).attr("x", 6).attr("y", 6).attr("rx", 2).style("fill", (n) => KIND[n.kind]);
  nodes.filter((n) => n.kind === "moment").append("circle").attr("class", "chart-dot").attr("cx", 6).attr("cy", H / 2).attr("r", 5).style("fill", KIND.moment);
  nodes.append("text").attr("x", 16).attr("y", H / 2 + 4).text((n) => n.label);

  hover(nodes, (event, n) => {
    const touches = (e) => e.from === n.id || e.to === n.id;
    svg.classed("is-focus", true);
    edges.classed("is-lit", touches);
    nodes.classed("is-lit", (m) => m === n || data.edges.some((e) => touches(e) && (e.from === m.id || e.to === m.id)));
    show(event.currentTarget, n.label, [["is", n.doc], ...data.edges.filter(touches).map((e) => [e.from === n.id ? `→ ${byId.get(e.to).label}` : `← ${byId.get(e.from).label}`, e.label])]);
  }, () => {
    svg.classed("is-focus", false);
    hide();
  });
});
