// Light markdown for agent answers, built as DOM nodes from text, never HTML,
// so an answer can't inject markup.

/** Fill `el` with `text` rendered as light markdown. */
export function renderProse(el: HTMLElement, text: string) {
  el.replaceChildren();
  el.classList.add("prose");
  let list: HTMLElement | null = null;
  let para: string[] = [];
  const flushPara = () => {
    if (!para.length) return;
    const p = document.createElement("p");
    inline(p, para.join(" "));
    el.appendChild(p);
    para = [];
  };
  for (const raw of text.split("\n")) {
    const line = raw.trimEnd();
    const item = /^\s*(?:[-*•]|\d+[.)])\s+(.*)$/.exec(line);
    const head = /^#{1,6}\s+(.*)$/.exec(line);
    if (item) {
      flushPara();
      if (!list) {
        list = document.createElement(/^\s*\d/.test(line) ? "ol" : "ul");
        el.appendChild(list);
      }
      const li = document.createElement("li");
      inline(li, item[1]);
      list.appendChild(li);
      continue;
    }
    list = null;
    if (head) {
      flushPara();
      const h = document.createElement("div");
      h.className = "prose-head";
      inline(h, head[1]);
      el.appendChild(h);
    } else if (line.trim() === "") {
      flushPara();
    } else {
      para.push(line.trim());
    }
  }
  flushPara();
}

/** **bold**, *italic* and `code` inside one line. */
function inline(el: HTMLElement, text: string) {
  const re = /(\*\*[^*]+\*\*|`[^`]+`|\*[^*\s][^*]*\*)/g;
  let at = 0;
  for (const m of text.matchAll(re)) {
    if (m.index! > at) el.appendChild(document.createTextNode(text.slice(at, m.index)));
    const t = m[0];
    const node = document.createElement(t.startsWith("**") ? "strong" : t.startsWith("`") ? "code" : "em");
    node.textContent = t.startsWith("**") ? t.slice(2, -2) : t.slice(1, -1);
    el.appendChild(node);
    at = m.index! + t.length;
  }
  if (at < text.length) el.appendChild(document.createTextNode(text.slice(at)));
}

/** One line of an answer without its markdown marks, for a cell's face. */
export function plainLine(line: string): string {
  return line
    .replace(/^\s*#{1,6}\s+/, "")
    .replace(/^\s*(?:[-*•]|\d+[.)])\s+/, "")
    .replace(/\*\*([^*]+)\*\*/g, "$1")
    .replace(/`([^`]+)`/g, "$1");
}
