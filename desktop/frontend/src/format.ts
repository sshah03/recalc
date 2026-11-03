// Display formatting changes how a value is shown, never the value. Maps a
// cell's Format to classes and a rendered string; lists the presets offered.

import type { Format, Highlight } from "./types.js";

/** Number presets; an exemplar is shown when the selected cell is empty. */
export const NUM_PRESETS: Array<{ id: Format["num"] & string; label: string; exemplar: string }> = [
  { id: "text", label: "Plain text", exemplar: "1000.12" },
  { id: "number", label: "Number", exemplar: "1000.12" },
  { id: "integer", label: "Integer", exemplar: "1000.12" },
  { id: "currency", label: "Currency", exemplar: "1000.12" },
  { id: "percent", label: "Percent", exemplar: "0.1012" },
  { id: "date", label: "Date", exemplar: "2026-09-03" },
];

export const FILLS: Array<{ id: string; label: string }> = [
  { id: "yellow", label: "Yellow" },
  { id: "green", label: "Green" },
  { id: "blue", label: "Blue" },
  { id: "red", label: "Red" },
  { id: "purple", label: "Purple" },
  { id: "gray", label: "Gray" },
];

/** Currencies offered in the Format panel; a cell can hold any ISO code. */
export const CURRENCIES = ["USD", "EUR", "GBP", "JPY", "CAD", "AUD", "INR", "CNY", "CHF", "KRW", "MXN", "BRL"];

/** Does a highlight rule match this value? Text is case-insensitive; above
 * and below compare numbers. Mirrors model::Highlight::matches. */
export function highlightMatches(h: Highlight, value: string): boolean {
  const v = value.trim();
  const want = (h.value ?? "").trim();
  const num = (s: string) => {
    const n = Number(s.replace(/[,$%\s]/g, ""));
    return s.replace(/[,$%\s]/g, "") !== "" && Number.isFinite(n) ? n : null;
  };
  switch (h.when) {
    case "is":
      return v.toLowerCase() === want.toLowerCase();
    case "contains":
      return want !== "" && v.toLowerCase().includes(want.toLowerCase());
    case "above": {
      const a = num(v), b = num(want);
      return a !== null && b !== null && a > b;
    }
    case "below": {
      const a = num(v), b = num(want);
      return a !== null && b !== null && a < b;
    }
  }
}

/** The fill a value shows: the first matching highlight, else the fixed fill. */
export function fillFor(f: Format | null | undefined, value: string): string | undefined {
  return f?.highlight?.find((h) => highlightMatches(h, value))?.fill ?? f?.fill;
}

/** CSS classes for a format: `fmt-b`, `fmt-i`, `fmt-right`, `fill-yellow`, `fmt-wrap`. */
export function formatClasses(f: Format | null | undefined, value = ""): string[] {
  if (!f) return [];
  const out: string[] = [];
  if (f.bold) out.push("fmt-b");
  if (f.italic) out.push("fmt-i");
  if (f.align) out.push(`fmt-${f.align}`);
  const fill = fillFor(f, value);
  if (fill) out.push(`fill-${fill}`);
  if (f.wrap) out.push("fmt-wrap");
  return out;
}

/** A number preset applied to a value that parses; other text is unchanged.
 * Dates render as ISO so they sort and grep. */
export function formatValue(value: string, num: Format["num"] | undefined, opts: { decimals?: number; currency?: string } = {}): string {
  if (!num || num === "text") return value;
  const trimmed = value.trim();
  if (num === "date") {
    const d = parseDate(trimmed);
    return d ? isoDate(d) : value;
  }
  const n = Number(trimmed.replace(/,/g, "").replace(/^\$/, "").replace(/%$/, ""));
  if (trimmed === "" || !Number.isFinite(n)) return value;
  const d = opts.decimals;
  switch (num) {
    case "integer":
      return Math.round(n).toLocaleString(undefined, { maximumFractionDigits: 0 });
    case "number":
      return n.toLocaleString(undefined, { minimumFractionDigits: d ?? 2, maximumFractionDigits: d ?? 2 });
    case "currency": {
      const style = { style: "currency" as const, currency: opts.currency ?? "USD", currencyDisplay: "narrowSymbol" as const };
      try {
        return n.toLocaleString(undefined, d === undefined ? style : { ...style, minimumFractionDigits: d, maximumFractionDigits: d });
      } catch {
        // An ISO code this browser doesn't know: the code, then the amount.
        return `${opts.currency} ${n.toLocaleString(undefined, { minimumFractionDigits: d ?? 2, maximumFractionDigits: d ?? 2 })}`;
      }
    }
    case "percent": {
      // A ratio (0.125) reads as 12.5%; a value already over 1 is taken as
      // a percentage typed as such, so 12.5 also reads as 12.5%.
      const pct = Math.abs(n) <= 1 && !trimmed.endsWith("%") ? n * 100 : n;
      return `${pct.toLocaleString(undefined, d === undefined ? { maximumFractionDigits: 2 } : { minimumFractionDigits: d, maximumFractionDigits: d })}%`;
    }
  }
  return value;
}

/** Does a number preset change how this value reads? Text stays as typed
 * under a number format, and a date format only reads dates. */
export function formatApplies(value: string, num: Format["num"] | undefined): boolean {
  const trimmed = value.trim();
  if (!num || num === "text") return true;
  if (num === "date") return parseDate(trimmed) !== null;
  return trimmed !== "" && Number.isFinite(Number(trimmed.replace(/,/g, "").replace(/^\$/, "").replace(/%$/, "")));
}

function isoDate(d: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

/** ISO dates and a few unambiguous forms; anything else is not a date. */
function parseDate(s: string): Date | null {
  if (/^\d{4}-\d{2}-\d{2}(T.*)?$/.test(s)) {
    const d = new Date(s.length === 10 ? `${s}T00:00:00` : s);
    return Number.isNaN(d.getTime()) ? null : d;
  }
  if (/^[A-Za-z]{3,9} \d{1,2}, \d{4}$/.test(s) || /^\d{1,2} [A-Za-z]{3,9} \d{4}$/.test(s)) {
    const d = new Date(s);
    return Number.isNaN(d.getTime()) ? null : d;
  }
  return null;
}

/** Numbers sit right by default; a format's explicit alignment wins. */
export function defaultAlign(f: Format | null | undefined): "left" | "center" | "right" {
  if (f?.align) return f.align;
  if (f?.num && f.num !== "date" && f.num !== "text") return "right";
  return "left";
}
