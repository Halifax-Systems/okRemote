import { ReactNode, useEffect, useRef } from 'react';
import type { Protocol } from './types';

export const ICONS: Record<string, string> = {
  folder: 'M3 6.5A1.5 1.5 0 0 1 4.5 5H9l2 2h8.5A1.5 1.5 0 0 1 21 8.5v9a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 17.5z',
  folderOpen: 'M3 6.5A1.5 1.5 0 0 1 4.5 5H9l2 2h8.5A1.5 1.5 0 0 1 21 8.5V10H6.2a1.5 1.5 0 0 0-1.4 1L3 17zM3 17l2.2-5.6A1.5 1.5 0 0 1 6.6 10.5H22l-2.4 7.1a1.5 1.5 0 0 1-1.4.9H4.5',
  ssh: 'M4 5h16v14H4zM7 9l3 3-3 3M12 15h5', terminal: 'M4 5h16v14H4zM7 9l3 3-3 3M12 15h5',
  rdp: 'M3 4h18v12H3zM8 20h8M12 16v4', vnc: 'M3 4h18v12H3zM8 20h8M12 16v4M8 8h8v4H8z',
  http: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18M3 12h18M12 3c2.5 2.7 3.5 5.7 3.5 9s-1 6.3-3.5 9c-2.5-2.7-3.5-5.7-3.5-9S9.5 5.7 12 3',
  external: 'M14 4h6v6M20 4l-9 9M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5',
  lock: 'M6 11h12v9H6zM8.5 11V8a3.5 3.5 0 0 1 7 0v3', unlock: 'M6 11h12v9H6zM8.5 11V8a3.5 3.5 0 0 1 6.7-1.4',
  vault: 'M4 4h16v16H4zM12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6M12 9V7M12 17v-2M9 12H7M17 12h-2',
  star: 'M12 3.5l2.6 5.3 5.9.9-4.3 4.1 1 5.8L12 16.9l-5.2 2.7 1-5.8-4.3-4.1 5.9-.9z',
  plus: 'M12 5v14M5 12h14', close: 'M6 6l12 12M18 6L6 18', search: 'M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14M20 20l-4-4',
  tunnel: 'M3 18V11a9 9 0 0 1 18 0v7M7 18v-6a5 5 0 0 1 10 0v6M3 18h4M17 18h4', chevron: 'M9 6l6 6-6 6',
  menu: 'M4 7h16M4 12h16M4 17h16', split: 'M4 4h16v16H4zM12 4v16', splitDown: 'M4 4h16v16H4zM4 12h16', sidebar: 'M4 4h16v16H4zM9 4v16',
  key: 'M15 7a4 4 0 1 0-3.9 4.9L4 19v2h3v-2h2v-2h2l1.1-1.1A4 4 0 0 0 15 7M16 6.5h.01',
  history: 'M3 12a9 9 0 1 0 3-6.7M3 4v4h4M12 7v5l3 2', importIcon: 'M12 3v12M7 10l5 5 5-5M5 21h14',
  copy: 'M8 8h11v13H8zM5 16H4V3h12v1', eye: 'M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6',
  reset: 'M4 4v6h6M4.5 15a8 8 0 1 0 1.9-8.3L4 10', warn: 'M12 3l10 18H2zM12 10v5M12 18h.01', dots: 'M5 12h.01M12 12h.01M19 12h.01',
  refresh: 'M20 11a8 8 0 0 0-14.9-3M4 4v4h4M4 13a8 8 0 0 0 14.9 3M20 20v-4h-4',
  gear: 'M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z',
};

export function Icon({ name, size = 16, style }: { name: string; size?: number; style?: React.CSSProperties }) {
  const d = ICONS[name === 'https' ? 'http' : name] ?? ICONS.dots;
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.8} strokeLinecap="round" strokeLinejoin="round" style={{ flex: 'none', ...style }} aria-hidden>
      <path d={d} />
    </svg>
  );
}

export const protoColor: Record<Protocol, string> = { ssh: '#14b8a6', rdp: '#3b82f6', vnc: '#8b5cf6', http: '#f97316', https: '#f97316', external: '#64748b' };

export function Dot({ state }: { state: string }) { return <span className={`dot ${state}`} />; }

export function Field({ label, hint, children, mark }: { label?: ReactNode; hint?: ReactNode; children: ReactNode; mark?: ReactNode }) {
  return (
    <div className="field">
      {label != null && <div className="lbl">{label}{mark}</div>}
      {children}
      {hint && <div className="hint">{hint}</div>}
    </div>
  );
}

export function Modal({ title, subtitle, width, onClose, children, footer }: { title: string; subtitle?: ReactNode; width?: 'wide' | 'xwide'; onClose: () => void; children: ReactNode; footer?: ReactNode }) {
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const prev = document.activeElement as HTMLElement | null;
    const first = box.current?.querySelector<HTMLElement>('input:not([type=checkbox]), select, textarea, button.primary');
    first?.focus();
    return () => prev?.focus?.();
  }, []);
  useEffect(() => {
    const k = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.stopPropagation(); onClose(); } };
    window.addEventListener('keydown', k, true);
    return () => window.removeEventListener('keydown', k, true);
  }, [onClose]);
  return (
    <div className="backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className={`modal ${width ?? ''}`} ref={box} role="dialog" aria-label={title}>
        <header>
          <div style={{ flex: 1 }}><h2>{title}</h2>{subtitle && <div className="dim small">{subtitle}</div>}</div>
          <button className="icon" aria-label="Close" onClick={onClose}><Icon name="close" /></button>
        </header>
        <div className="body">{children}</div>
        {footer && <footer>{footer}</footer>}
      </div>
    </div>
  );
}

export const isMac = navigator.platform.toLowerCase().includes('mac');
export function hint(combo: string) {
  return combo.split('+').map((p) => (p === 'Mod' ? (isMac ? '⌘' : 'Ctrl') : p === 'Shift' && isMac ? '⇧' : p === 'Alt' && isMac ? '⌥' : p)).join(isMac ? '' : '+');
}
