import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { SearchAddon } from '@xterm/addon-search';
import { Unicode11Addon } from '@xterm/addon-unicode11';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { WebglAddon } from '@xterm/addon-webgl';
import '@xterm/xterm/css/xterm.css';
import { openUrl } from '@tauri-apps/plugin-opener';
import { findScheme, xtermTheme } from './schemes';
import { isMac } from './ui';
import type { Settings, TermSettings } from './types';

export interface TermHooks {
  onInput(data: string): void;
  onResize(cols: number, rows: number): void;
  copyText(text: string): void;
  readText(): Promise<string>;
  confirmPaste(text: string): Promise<boolean>;
  appKey(e: KeyboardEvent): boolean;
  toggleSearch(): void;
  zoom(delta: number | 'reset'): void;
}

const APP_KEYS = new Set(['k', 'l', 'w', 'n', 'b', 'd', 'Tab']);

export class TermController {
  term: Terminal;
  fit = new FitAddon();
  search = new SearchAddon();
  el: HTMLDivElement;
  fixed = false;
  private resizeTimer: number | undefined;
  private lastSize = '';
  private osc52 = false;
  private cfg: TermSettings;
  private bellTimer: number | undefined;

  constructor(private hooks: TermHooks, cfg: TermSettings, private settings: () => Settings) {
    this.cfg = cfg;
    this.el = document.createElement('div');
    this.el.style.cssText = 'position:absolute;inset:0';
    this.term = new Terminal({
      allowProposedApi: true,
      macOptionIsMeta: isMac,
      rightClickSelectsWord: false,
      ...this.options(cfg, 0),
    });
    this.term.loadAddon(this.fit);
    this.term.loadAddon(this.search);
    this.term.loadAddon(new Unicode11Addon());
    this.term.unicode.activeVersion = '11';
    this.term.loadAddon(new WebLinksAddon((_e, uri) => { if (/^https?:/i.test(uri)) void openUrl(uri); }));
    this.term.parser.registerOscHandler(8, () => false);
    this.term.parser.registerOscHandler(52, (data) => {
      const i = data.indexOf(';');
      const payload = i >= 0 ? data.slice(i + 1) : '';
      if (this.osc52 && payload && payload !== '?') {
        try { hooks.copyText(new TextDecoder().decode(Uint8Array.from(atob(payload), (c) => c.charCodeAt(0)))); } catch { /* ignore */ }
      }
      return true;
    });
    this.term.onData((d) => hooks.onInput(d));
    this.term.onBell(() => this.bell());
    this.term.onSelectionChange(() => { if (this.cfg.copyOnSelect && this.term.hasSelection()) hooks.copyText(this.term.getSelection()); });
    this.term.attachCustomKeyEventHandler((e) => this.key(e));
    this.el.addEventListener('contextmenu', (e) => {
      e.preventDefault();
      if (!this.cfg.rightClickPaste) return;
      if (this.term.hasSelection()) { hooks.copyText(this.term.getSelection()); this.term.clearSelection(); } else void this.paste();
    });
    this.el.addEventListener('paste', (e) => { e.preventDefault(); const t = e.clipboardData?.getData('text'); if (t) void this.pasteText(t); });
  }

  private options(cfg: TermSettings, zoom: number) {
    const scheme = findScheme(cfg.scheme, this.settings?.().custom_schemes ?? []);
    const weight = ({ light: 300, normal: 400, medium: 500, semibold: 600, bold: 700 } as Record<string, number>)[cfg.fontWeight] ?? 400;
    return {
      fontFamily: `${cfg.fontFamily}, monospace`, fontSize: Math.max(6, cfg.fontSize + zoom), fontWeight: weight as any,
      lineHeight: cfg.lineHeight, cursorStyle: cfg.cursorStyle, cursorBlink: cfg.cursorBlink, scrollback: cfg.scrollback,
      theme: xtermTheme(scheme),
    };
  }

  apply(cfg: TermSettings, zoom: number) {
    this.cfg = cfg;
    const o = this.options(cfg, zoom);
    for (const [k, v] of Object.entries(o)) (this.term.options as any)[k] = v;
    this.refit();
  }

  setOsc52(v: boolean) { this.osc52 = v; }

  mount(parent: HTMLElement) {
    if (this.el.parentElement !== parent) parent.appendChild(this.el);
    if (!this.term.element) {
      this.term.open(this.el);
      try {
        const gl = new WebglAddon();
        gl.onContextLoss(() => gl.dispose());
        this.term.loadAddon(gl);
      } catch { /* DOM renderer fallback */ }
    }
    this.refit();
  }

  refit() {
    if (this.fixed || !this.el.isConnected || this.el.clientWidth < 10) return;
    window.clearTimeout(this.resizeTimer);
    this.resizeTimer = window.setTimeout(() => {
      try { this.fit.fit(); } catch { return; }
      this.announce();
    }, 50);
  }

  fitNow(): { cols: number; rows: number } {
    if (!this.fixed) { try { this.fit.fit(); } catch { /* not visible yet */ } }
    return { cols: this.term.cols, rows: this.term.rows };
  }

  announce(force = false) {
    const k = `${this.term.cols}x${this.term.rows}`;
    if (force || k !== this.lastSize) { this.lastSize = k; this.hooks.onResize(this.term.cols, this.term.rows); }
  }

  setFixed(cols: number, rows: number) { this.fixed = true; this.term.resize(cols, rows); }
  write(data: Uint8Array | string) { this.term.write(data); }
  focus() { this.term.focus(); }
  dispose() { window.clearTimeout(this.resizeTimer); this.term.dispose(); this.el.remove(); }

  async paste() { const t = await this.hooks.readText(); if (t) await this.pasteText(t); }

  async pasteText(text: string) {
    const trimmed = text.replace(/[\r\n]+$/, '');
    if (this.cfg.confirmMultilinePaste && /[\r\n]/.test(trimmed)) { if (!(await this.hooks.confirmPaste(text))) return; }
    this.term.paste(text);
  }

  private bell() {
    if (this.cfg.bell === 'visual') {
      this.el.parentElement?.classList.add('bell');
      window.clearTimeout(this.bellTimer);
      this.bellTimer = window.setTimeout(() => this.el.parentElement?.classList.remove('bell'), 150);
    } else if (this.cfg.bell === 'audible') {
      try {
        const ctx = new AudioContext(); const o = ctx.createOscillator(); const g = ctx.createGain();
        o.frequency.value = 880; g.gain.value = 0.05; o.connect(g); g.connect(ctx.destination); o.start();
        setTimeout(() => { o.stop(); void ctx.close(); }, 80);
      } catch { /* no audio */ }
    }
  }

  /** Returns false when the key must not reach the terminal. */
  private key(e: KeyboardEvent): boolean {
    if (e.type !== 'keydown') return true;
    const mod = isMac ? e.metaKey : e.ctrlKey;
    if (e.ctrlKey && e.key === 'Tab') return false;
    if (isMac ? (e.metaKey && !e.shiftKey && e.key === 'c') : (e.ctrlKey && e.shiftKey && e.key.toLowerCase() === 'c')) {
      if (this.term.hasSelection()) { this.hooks.copyText(this.term.getSelection()); return false; }
      return isMac ? false : true;
    }
    if (isMac ? (e.metaKey && e.key === 'v') : (e.ctrlKey && e.shiftKey && e.key.toLowerCase() === 'v')) { void this.paste(); return false; }
    if (mod && e.shiftKey && e.key.toLowerCase() === 'f') { this.hooks.toggleSearch(); return false; }
    if (mod && !e.shiftKey && (e.key === '=' || e.key === '+')) { this.hooks.zoom(1); return false; }
    if (mod && !e.shiftKey && e.key === '-') { this.hooks.zoom(-1); return false; }
    if (mod && !e.shiftKey && e.key === '0') { this.hooks.zoom('reset'); return false; }
    if (mod && (APP_KEYS.has(e.key) || APP_KEYS.has(e.key.toLowerCase()))) { return !this.hooks.appKey(e) ? true : false; }
    return true;
  }
}
