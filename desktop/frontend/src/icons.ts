// 16px stroke icons as inline SVG: they take the text color and need no font or network.
// Names say what an icon means here, not what it depicts.

const PATHS: Record<string, string> = {
  run: '<path d="M5 3.5v9l7.5-4.5z" fill="currentColor" stroke="none"/>',
  run_all:
    '<path d="M3 3.5v9l6-4.5z" fill="currentColor" stroke="none"/><path d="M10.5 4v8" /><path d="M13.5 4v8" />',
  terminal: '<path d="M3 4.5l4 3.5-4 3.5" /><path d="M8 12h5" />',
  check: '<path d="M3 8.5l3 3 7-7" />',
  flag: '<path d="M3.5 14V2.5" /><path d="M3.5 3h7.5l-1.5 3 1.5 3H3.5" />',
  spark: '<path d="M2 11l3-5 3 3 2-2 4 4" />',
  agent:
    '<rect x="3" y="5" width="10" height="8" rx="2" /><path d="M8 2.5v2.5" /><path d="M6 9h.01M10 9h.01" /><path d="M1.5 8.5v2M14.5 8.5v2" />',
  session: '<rect x="2" y="3" width="12" height="10" rx="2" /><path d="M5 7l2 1.5L5 10" /><path d="M8.5 10h2.5" />',
  template: '<rect x="2.5" y="2.5" width="11" height="11" rx="1.5" /><path d="M2.5 6.5h11M6.5 6.5v7" />',
  palette: '<rect x="2" y="3" width="12" height="10" rx="2" /><path d="M5 6.5h6M5 9.5h3.5" />',
  folder: '<path d="M2 4.5A1.5 1.5 0 0 1 3.5 3h3l1.5 1.5H12.5A1.5 1.5 0 0 1 14 6v6a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 2 12z" />',
  bolt: '<path d="M9 2L4 9h4l-1 5 5-7H8z" fill="currentColor" stroke="none"/>',
  bell: '<path d="M4 11V7.5a4 4 0 0 1 8 0V11l1 1.5H3z" /><path d="M6.5 14.5h3" />',
  chevron: '<path d="M6 4l4 4-4 4" />',
  bold: '<path d="M5 2.5h4.5a2.75 2.75 0 0 1 0 5.5H5z" /><path d="M5 8h5.25a2.75 2.75 0 0 1 0 5.5H5z" />',
  italic: '<path d="M10 2.5h3M3 13.5h3M9.5 2.5l-3 11" />',
  format: '<path d="M3 3.5h10M3 8h6M3 12.5h10" /><circle cx="12" cy="8" r="1.4" fill="currentColor" stroke="none"/>',
  sort: '<path d="M4 3v10M4 13l-2-2M4 13l2-2" /><path d="M8 4h6M8 8h4M8 12h2" />',
  driver:
    '<path d="M2.5 3.5h11v7.5H7.5L4.5 13.5v-2.5h-2z" /><path d="M6 7h4" />',
  close: '<path d="M4.5 4.5l7 7M11.5 4.5l-7 7" />',
  column: '<rect x="2.5" y="2.5" width="11" height="11" rx="1.5" /><path d="M9.5 2.5v11M2.5 6h11" /><path d="M11.5 8.5v3M10 10h3" />',
};

/** An inline 16px icon. */
export function icon(name: string, size = 16): SVGSVGElement {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 16 16");
  svg.setAttribute("width", String(size));
  svg.setAttribute("height", String(size));
  svg.setAttribute("fill", "none");
  svg.setAttribute("stroke", "currentColor");
  svg.setAttribute("stroke-width", "1.6");
  svg.setAttribute("stroke-linecap", "round");
  svg.setAttribute("stroke-linejoin", "round");
  svg.setAttribute("aria-hidden", "true");
  svg.classList.add("icon");
  svg.innerHTML = PATHS[name] ?? "";
  return svg;
}
