import { call } from './api';
import { isMac } from './ui';

// Set 1 scan codes by KeyboardEvent.code: [code, extended]
const SC: Record<string, [number, boolean]> = {
  Escape: [0x01, false], Minus: [0x0c, false], Equal: [0x0d, false], Backspace: [0x0e, false], Tab: [0x0f, false],
  BracketLeft: [0x1a, false], BracketRight: [0x1b, false], Enter: [0x1c, false], ControlLeft: [0x1d, false],
  Semicolon: [0x27, false], Quote: [0x28, false], Backquote: [0x29, false], ShiftLeft: [0x2a, false], Backslash: [0x2b, false],
  Comma: [0x33, false], Period: [0x34, false], Slash: [0x35, false], ShiftRight: [0x36, false], NumpadMultiply: [0x37, false],
  AltLeft: [0x38, false], Space: [0x39, false], CapsLock: [0x3a, false], NumLock: [0x45, false], ScrollLock: [0x46, false],
  Numpad7: [0x47, false], Numpad8: [0x48, false], Numpad9: [0x49, false], NumpadSubtract: [0x4a, false], Numpad4: [0x4b, false],
  Numpad5: [0x4c, false], Numpad6: [0x4d, false], NumpadAdd: [0x4e, false], Numpad1: [0x4f, false], Numpad2: [0x50, false],
  Numpad3: [0x51, false], Numpad0: [0x52, false], NumpadDecimal: [0x53, false], IntlBackslash: [0x56, false], F11: [0x57, false], F12: [0x58, false],
  NumpadEnter: [0x1c, true], ControlRight: [0x1d, true], NumpadDivide: [0x35, true], PrintScreen: [0x37, true], AltRight: [0x38, true],
  Home: [0x47, true], ArrowUp: [0x48, true], PageUp: [0x49, true], ArrowLeft: [0x4b, true], ArrowRight: [0x4d, true], End: [0x4f, true],
  ArrowDown: [0x50, true], PageDown: [0x51, true], Insert: [0x52, true], Delete: [0x53, true], MetaLeft: [0x5b, true], MetaRight: [0x5c, true], ContextMenu: [0x5d, true],
};
'1234567890'.split('').forEach((d, i) => { SC[`Digit${d}`] = [0x02 + i, false]; });
'QWERTYUIOP'.split('').forEach((l, i) => { SC[`Key${l}`] = [0x10 + i, false]; });
'ASDFGHJKL'.split('').forEach((l, i) => { SC[`Key${l}`] = [0x1e + i, false]; });
'ZXCVBNM'.split('').forEach((l, i) => { SC[`Key${l}`] = [0x2c + i, false]; });
for (let i = 1; i <= 10; i++) SC[`F${i}`] = [0x3a + i, false];

export function scanFor(e: KeyboardEvent): [number, boolean] | null {
  if (isMac && (e.code === 'MetaLeft' || e.code === 'MetaRight')) return [0x1d, false]; // Command acts as Left Control
  return SC[e.code] ?? null;
}

export const isNormalClose = (r: string) => /^(closed|server|user|logged|disconnected)/i.test(r);

export class RdpController {
  el = document.createElement('div');
  canvas = document.createElement('canvas');
  private ctx = this.canvas.getContext('2d', { alpha: false })!;
  id: string | null = null;
  rw = 0; rh = 0;
  private lastSent = '';
  private resizeTimer: number | undefined;
  private pendingMove: { x: number; y: number } | null = null;
  private raf = 0;
  private ro: ResizeObserver;

  constructor(private onClosed: (reason: string) => void, private onSize: (w: number, h: number) => void) {
    this.el.style.cssText = 'position:absolute;inset:0;background:#000;display:flex;align-items:center;justify-content:center;overflow:hidden';
    this.canvas.tabIndex = 0;
    this.canvas.style.outline = 'none';
    this.canvas.style.imageRendering = 'auto';
    this.el.appendChild(this.canvas);
    this.attachInput();
    this.ro = new ResizeObserver(() => this.layout());
    this.ro.observe(this.el);
  }

  size() { return { width: Math.max(640, this.el.clientWidth), height: Math.max(480, this.el.clientHeight) }; }
  mount(parent: HTMLElement) { if (this.el.parentElement !== parent) parent.appendChild(this.el); this.layout(); }
  focus() { this.canvas.focus(); }
  refit() { this.layout(); }

  /** Scales the picture to the largest integer pixel size that fits, keeping the aspect ratio. */
  private layout() {
    if (!this.rw || !this.rh || !this.el.clientWidth) return;
    const s = Math.min(this.el.clientWidth / this.rw, this.el.clientHeight / this.rh);
    this.canvas.style.width = `${Math.max(1, Math.floor(this.rw * s))}px`;
    this.canvas.style.height = `${Math.max(1, Math.floor(this.rh * s))}px`;
    this.scheduleResize();
  }

  private scheduleResize() {
    window.clearTimeout(this.resizeTimer);
    this.resizeTimer = window.setTimeout(() => {
      const { width, height } = this.size();
      const k = `${width}x${height}`;
      if (this.id && k !== this.lastSent) { this.lastSent = k; this.send({ t: 'resize', width, height }); }
    }, 250);
  }

  start(id: string, w: number, h: number) {
    this.id = id;
    this.lastSent = `${this.size().width}x${this.size().height}`;
    this.setSize(w, h);
    this.focus();
  }

  private setSize(w: number, h: number) {
    this.rw = w; this.rh = h;
    this.canvas.width = w; this.canvas.height = h;
    this.onSize(w, h);
    this.layout();
  }

  /** Handles one tagged binary message from the core (spec 9.4). */
  handle(buf: ArrayBuffer) {
    const v = new DataView(buf);
    const tag = v.getUint8(0);
    switch (tag) {
      case 1: {
        const [x, y, w, h] = [1, 3, 5, 7].map((o) => v.getUint16(o, true));
        this.ctx.putImageData(new ImageData(new Uint8ClampedArray(buf, 9, w * h * 4), w, h), x, y);
        break;
      }
      case 2: this.setSize(v.getUint16(1, true), v.getUint16(3, true)); break;
      case 3: this.canvas.style.cursor = 'default'; break;
      case 4: this.canvas.style.cursor = 'none'; break;
      case 5: {
        const [w, h, hx, hy] = [1, 3, 5, 7].map((o) => v.getUint16(o, true));
        const c = document.createElement('canvas');
        c.width = w; c.height = h;
        c.getContext('2d')!.putImageData(new ImageData(new Uint8ClampedArray(buf.slice(9, 9 + w * h * 4)), w, h), 0, 0);
        this.canvas.style.cursor = `url(${c.toDataURL()}) ${hx} ${hy}, default`;
        break;
      }
      case 7: this.onClosed(new TextDecoder().decode(new Uint8Array(buf, 1))); break;
    }
  }

  private send(event: Record<string, unknown>) { if (this.id) void call('rdp_input', { id: this.id, event }); }

  private pos(e: MouseEvent) {
    const r = this.canvas.getBoundingClientRect();
    const x = Math.round(((e.clientX - r.left) * this.rw) / r.width), y = Math.round(((e.clientY - r.top) * this.rh) / r.height);
    return { x: Math.min(Math.max(x, 0), this.rw - 1), y: Math.min(Math.max(y, 0), this.rh - 1) };
  }

  private attachInput() {
    const c = this.canvas;
    c.addEventListener('mousemove', (e) => {
      this.pendingMove = this.pos(e);
      if (!this.raf) this.raf = requestAnimationFrame(() => { this.raf = 0; if (this.pendingMove) this.send({ t: 'move', ...this.pendingMove }); this.pendingMove = null; });
    });
    const btn = (down: boolean) => (e: MouseEvent) => {
      e.preventDefault();
      if (down) c.focus();
      this.send({ t: 'button', ...this.pos(e), button: e.button, down });
    };
    c.addEventListener('mousedown', btn(true));
    c.addEventListener('mouseup', btn(false));
    c.addEventListener('wheel', (e) => {
      e.preventDefault();
      const unit = e.deltaMode === 1 ? 40 : e.deltaMode === 2 ? 400 : 1;
      const clamp = (n: number) => Math.max(-255, Math.min(255, Math.round(n)));
      const p = this.pos(e);
      if (e.deltaY) this.send({ t: 'wheel', ...p, delta: clamp(-e.deltaY * 1.2 * unit), horizontal: false });
      if (e.deltaX) this.send({ t: 'wheel', ...p, delta: clamp(e.deltaX * 1.2 * unit), horizontal: true });
    }, { passive: false });
    c.addEventListener('contextmenu', (e) => e.preventDefault());
    const key = (down: boolean) => (e: KeyboardEvent) => {
      if (e.isComposing) return;
      e.preventDefault();
      e.stopPropagation();
      const sc = scanFor(e);
      if (sc) this.send({ t: 'key', code: sc[0], extended: sc[1], down });
      else if (e.key.length === 1) this.send({ t: 'unicode', ch: e.key, down });
    };
    c.addEventListener('keydown', key(true));
    c.addEventListener('keyup', key(false));
    c.addEventListener('blur', () => this.send({ t: 'release' }));
  }

  close() { this.send({ t: 'close' }); }
  dispose() { this.ro.disconnect(); window.clearTimeout(this.resizeTimer); this.el.remove(); }
}
