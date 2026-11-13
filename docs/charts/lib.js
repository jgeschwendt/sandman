// The charts' shared chrome: the data fetch, the legend and table toggle, the tooltip, the redraw on resize. Every chart
// mounts into a `.chart` root and draws into its frame; text goes in through d3's .text (textContent), never as HTML.
export const d3 = globalThis.d3;

export const load = async (name) => (await fetch(new URL(`../data/${name}`, import.meta.url))).json();

// mounts the chrome into `root`: a legend of { label, color, mark: "dot" | "dash" | "rect" }, the table toggle, the frame, a
// tooltip, the table of `head` × `rows`; returns the frame, show(element, title, pairs) and hide()
export const mount = (root, { head, legend, rows }) => {
  root.replaceChildren();
  const top = d3.select(root);
  const bar = top.append("div").attr("class", "chart-bar");
  const list = bar.append("ul").attr("class", "chart-legend").attr("aria-label", "legend");
  for (const { color, label, mark } of legend) {
    const li = list.append("li");
    const svg = li.append("svg").attr("class", "chart-key").attr("width", 16).attr("height", 12).attr("aria-hidden", "true");
    if (mark === "dot") svg.append("circle").attr("cx", 8).attr("cy", 6).attr("r", 4).style("fill", color);
    else if (mark === "dash") svg.append("rect").attr("class", "chart-key-dash").attr("x", 1).attr("y", 2).attr("width", 14).attr("height", 8).attr("rx", 2).style("stroke", color);
    else svg.append("rect").attr("x", 2).attr("y", 3).attr("width", 12).attr("height", 6).attr("rx", 2).style("fill", color);
    li.append("span").text(label);
  }
  const toggle = bar.append("button").attr("type", "button").attr("class", "chart-toggle").attr("aria-pressed", "false").text("table");
  const frame = top.append("div").attr("class", "chart-frame");
  const tip = top.append("div").attr("class", "chart-tip").attr("role", "tooltip").attr("hidden", "");
  const wrap = top.append("div").attr("class", "chart-table").attr("hidden", "");
  const table = wrap.append("table");
  table.append("thead").append("tr").selectAll("th").data(head).join("th").text((d) => d);
  table.append("tbody").selectAll("tr").data(rows).join("tr").selectAll("td").data((r) => r).join("td").text((d) => d);
  const hide = () => tip.attr("hidden", "");
  toggle.on("click", () => {
    const isTable = toggle.attr("aria-pressed") !== "true";
    toggle.attr("aria-pressed", String(isTable)).text(isTable ? "chart" : "table");
    frame.attr("hidden", isTable ? "" : null);
    wrap.attr("hidden", isTable ? null : "");
    hide();
  });
  const show = (element, title, pairs) => {
    tip.selectAll("*").remove();
    tip.append("strong").text(title);
    const dl = tip.append("dl");
    for (const [k, v] of pairs) {
      dl.append("dt").text(k);
      dl.append("dd").text(v);
    }
    tip.attr("hidden", null);
    const box = root.getBoundingClientRect();
    const at = element.getBoundingClientRect();
    const left = Math.min(Math.max(0, at.left - box.left), box.width - tip.node().offsetWidth);
    tip.style("left", `${left}px`).style("top", `${at.bottom - box.top + 6}px`);
  };
  return { frame, hide, show };
};

// draw at the frame's content width (its 16px padding off both sides, at least `min`), and again on every width change
export const redraw = (root, frame, min, draw) => {
  let last;
  new ResizeObserver(() => {
    if (root.clientWidth === last) return;
    last = root.clientWidth;
    frame.selectAll("*").remove();
    draw(Math.max(min, frame.node().clientWidth - 32));
  }).observe(root);
};

// hover and focus on a selection: enter(event, d), then leave
export const hover = (sel, enter, leave) =>
  sel.on("pointerenter focus", (event, d) => enter(event, d)).on("pointerleave blur", leave);
