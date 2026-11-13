// What the earlier recall reaches from the start directory: one strip of every memory (sent, reachable but cut, out of
// reach) over one bar per bank sized by its memory count. Data: docs/data/recall-reach.json, counts only, banks unnamed.
import { d3, hover, load, mount, redraw } from "./lib.js";

const root = document.getElementById("reach");
const data = await load("recall-reach.json");

const REACH = { below: "below the start directory", elsewhere: "elsewhere", reachable: "reachable" };
const COLOR = { below: "var(--kind-7)", elsewhere: "var(--kind-2)", reachable: "var(--kind-3)" };
const sum = (reach) => d3.sum(data.banks.filter((b) => b.reach === reach), (b) => b.memories);
const reachable = sum("reachable");
const strip = [
  { color: COLOR.reachable, label: `${data.sent} sent`, n: data.sent, open: false },
  { color: COLOR.reachable, label: `${reachable - data.sent} cut`, n: reachable - data.sent, open: true },
  { color: COLOR.below, label: `${sum("below")} below`, n: sum("below"), open: false },
  { color: COLOR.elsewhere, label: `${sum("elsewhere")} elsewhere`, n: sum("elsewhere"), open: false },
];
const never = data.total - data.sent;

const { frame, hide, show } = mount(root, {
  head: ["bank", "memories", "reach"],
  legend: [
    { color: COLOR.reachable, label: "reachable · sent" },
    { color: COLOR.reachable, label: "reachable · cut", mark: "dash" },
    { color: COLOR.below, label: REACH.below },
    { color: COLOR.elsewhere, label: REACH.elsewhere },
  ],
  rows: data.banks.map((b) => [b.bank, String(b.memories), REACH[b.reach]]),
});

redraw(root, frame, 560, (width) => {
  const left = 32;
  const svg = frame.append("svg").attr("width", width).attr("height", 300).attr("role", "img")
    .attr("aria-label", `${never} of ${data.total} memories never reach a session; the earlier recall reaches 2 of ${data.banks.length} banks`);
  svg.append("text").attr("class", "reach-head").attr("y", 14).text(`${never} of ${data.total} memories (${Math.round((100 * never) / data.total)} %) never reach a session`);

  const x = d3.scaleLinear().domain([0, data.total]).range([0, width]);
  let at = 0;
  const parts = strip.map((s) => ({ ...s, x: at, end: (at += s.n) }));
  const seg = svg.append("g").attr("transform", "translate(0,44)").selectAll("rect").data(parts).join("rect")
    .attr("class", (s) => (s.open ? "chart-bar-mark reach-open" : "chart-bar-mark")).attr("tabindex", 0)
    .attr("x", (s) => x(s.x) + 0.5).attr("width", (s) => Math.max(1, x(s.end) - x(s.x) - 1)).attr("height", 22).attr("rx", 2)
    .style("fill", (s) => (s.open ? "none" : s.color)).style("stroke", (s) => (s.open ? s.color : null));
  // the sent label rides above the strip, the rest below, so a thin segment never crowds its neighbour's label
  svg.append("g").attr("class", "reach-label").selectAll("text").data(parts).join("text")
    .attr("x", (s) => x(s.x)).attr("y", (_, i) => (i ? 84 : 38)).text((s) => s.label);

  const top = 112;
  const bottom = 278;
  const bx = d3.scaleBand().domain(data.banks.map((b) => b.bank)).range([left, width]).padding(0.18);
  const by = d3.scaleLinear().domain([0, d3.max(data.banks, (b) => b.memories)]).range([bottom, top]).nice();
  svg.append("g").attr("class", "chart-axis").attr("transform", `translate(${left - 4},0)`).call(d3.axisLeft(by).ticks(4));
  svg.append("text").attr("class", "reach-label").attr("x", width).attr("y", 296).attr("text-anchor", "end").text(`${data.banks.length} banks, by memory count`);
  const bars = svg.append("g").selectAll("rect").data(data.banks).join("rect")
    .attr("class", (b) => (b.reach === "reachable" ? "chart-bar-mark reach-open" : "chart-bar-mark")).attr("tabindex", 0)
    .attr("x", (b) => bx(b.bank)).attr("width", bx.bandwidth()).attr("y", (b) => by(b.memories)).attr("height", (b) => bottom - by(b.memories)).attr("rx", 1)
    .style("fill", (b) => (b.reach === "reachable" ? "none" : COLOR[b.reach])).style("stroke", (b) => (b.reach === "reachable" ? COLOR.reachable : null));

  hover(seg, (event, s) => show(event.currentTarget, s.label, [["memories", String(s.n)], ["share", `${Math.round((100 * s.n) / data.total)} %`]]), hide);
  hover(bars, (event, b) => show(event.currentTarget, b.bank, [["memories", String(b.memories)], ["reach", REACH[b.reach]]]), hide);
});
