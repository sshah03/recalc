// The slice of the xterm.js API we use, declared as UMD globals (loaded via
// <script> in index.html). Only what terminal.ts touches.

interface XtermOptions {
  fontFamily?: string;
  fontSize?: number;
  lineHeight?: number;
  cursorBlink?: boolean;
  theme?: Record<string, string>;
}

interface XtermDisposable {
  dispose(): void;
}

interface XtermBufferLine {
  translateToString(trimRight?: boolean): string;
}

interface XtermBuffer {
  readonly viewportY: number;
  getLine(y: number): XtermBufferLine | undefined;
}

declare class Terminal {
  readonly buffer: { readonly active: XtermBuffer };
  constructor(options?: XtermOptions);
  options: XtermOptions;
  readonly cols: number;
  readonly rows: number;
  open(parent: HTMLElement): void;
  write(data: string): void;
  onData(handler: (data: string) => void): XtermDisposable;
  onResize(handler: (size: { cols: number; rows: number }) => void): XtermDisposable;
  loadAddon(addon: unknown): void;
  focus(): void;
  dispose(): void;
}

interface FitAddonInstance {
  fit(): void;
}

declare const FitAddon: { FitAddon: new () => FitAddonInstance };
