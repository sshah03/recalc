// A1-notation helpers, matching model::a1_coord / a1_name / A1Ref in the core.

export function a1Coord(name: string): [number, number] | null {
  const m = /^([A-Z]+)([0-9]+)$/.exec(name);
  if (!m) return null;
  let col = 0;
  for (const ch of m[1]) col = col * 26 + (ch.charCodeAt(0) - 64);
  const row = parseInt(m[2], 10);
  return row > 0 ? [col, row] : null;
}

export function a1Name(col: number, row: number): string {
  let letters = "";
  let c = col;
  while (c > 0) {
    letters = String.fromCharCode(65 + ((c - 1) % 26)) + letters;
    c = Math.floor((c - 1) / 26);
  }
  return letters + row;
}

/** Column number → its letters (1 → "A", 27 → "AA"). */
export function colLetters(col: number): string {
  return a1Name(col, 1).replace(/1$/, "");
}

/** The rectangle two corners span, normalized: [c0, r0, c1, r1]. */
export function rect(a: string, b: string): [number, number, number, number] | null {
  const p = a1Coord(a);
  const q = a1Coord(b);
  if (!p || !q) return null;
  return [Math.min(p[0], q[0]), Math.min(p[1], q[1]), Math.max(p[0], q[0]), Math.max(p[1], q[1])];
}

/** Every cell name in a rectangle, row-major. */
export function rectNames(r: [number, number, number, number]): string[] {
  const out: string[] = [];
  for (let row = r[1]; row <= r[3]; row++) {
    for (let col = r[0]; col <= r[2]; col++) out.push(a1Name(col, row));
  }
  return out;
}

/** "B2:D4" for a rectangle, or the single name when it is one cell. */
export function rectLabel(r: [number, number, number, number]): string {
  const a = a1Name(r[0], r[1]);
  const b = a1Name(r[2], r[3]);
  return a === b ? a : `${a}:${b}`;
}

/**
 * Cycle the `$` anchors on an A1 reference the way F4 does in a spreadsheet:
 * A1 → $A$1 → A$1 → $A1 → A1. Anything that isn't an A1 ref comes back as is.
 */
export function cycleAnchors(ref: string): string {
  const m = /^(\$?)([A-Z]+)(\$?)([0-9]+)$/.exec(ref);
  if (!m) return ref;
  const [, c, letters, r, digits] = m;
  const next = c && r ? ["", "$"] : c ? ["", ""] : r ? ["$", ""] : ["$", "$"];
  return `${next[0]}${letters}${next[1]}${digits}`;
}
