// A minimal sparkline as an inline SVG polyline, scaled to the value range.

export function sparkline(values: number[], w = 90, h = 16): SVGSVGElement | null {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  if (values.length < 2) return null;
  const min = Math.min(...values);
  const max = Math.max(...values);
  const range = max - min || 1;
  const step = w / (values.length - 1);
  const points = values
    .map((v, i) => {
      const x = i * step;
      const y = h - 1 - ((v - min) / range) * (h - 2);
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
  svg.setAttribute("class", "spark");
  svg.setAttribute("width", String(w));
  svg.setAttribute("height", String(h));
  svg.setAttribute("viewBox", `0 0 ${w} ${h}`);
  const line = document.createElementNS("http://www.w3.org/2000/svg", "polyline");
  line.setAttribute("points", points);
  svg.appendChild(line);
  return svg;
}
