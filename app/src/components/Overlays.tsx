import { useEffect, useRef, useState } from 'react';
import { useStore, setState } from '../store';
import { dismissToast } from '../actions';

export function Toasts() {
  const toasts = useStore((s) => s.toasts);
  return (
    <div className="toasts" aria-live="polite">
      {toasts.map((t) => <div key={t.id} className={`toast ${t.kind}`} onClick={() => dismissToast(t.id)}>{t.text}</div>)}
    </div>
  );
}

export function ContextMenu() {
  const menu = useStore((s) => s.menu);
  const [hl, setHl] = useState(0);
  const [kb, setKb] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const items = menu?.items ?? [];
  const enabled = items.map((it, i) => (it.sep || it.disabled ? -1 : i)).filter((i) => i >= 0);
  useEffect(() => { setHl(enabled[0] ?? 0); setKb(false); }, [menu]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    if (!menu) return;
    const close = () => setState({ menu: null });
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); close(); }
      else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault(); e.stopPropagation(); setKb(true);
        const pos = enabled.indexOf(hl);
        setHl(enabled[(pos + (e.key === 'ArrowDown' ? 1 : -1) + enabled.length) % enabled.length]);
      } else if (e.key === 'Enter') { e.preventDefault(); e.stopPropagation(); const it = items[hl]; close(); it?.run?.(); }
    };
    const down = (e: MouseEvent) => { if (!ref.current?.contains(e.target as Node)) close(); };
    window.addEventListener('keydown', key, true);
    window.addEventListener('mousedown', down, true);
    window.addEventListener('blur', close);
    return () => { window.removeEventListener('keydown', key, true); window.removeEventListener('mousedown', down, true); window.removeEventListener('blur', close); };
  });
  if (!menu) return null;
  const h = items.length * 30 + 16;
  const x = Math.min(menu.x, window.innerWidth - 240);
  const y = Math.min(menu.y, window.innerHeight - h - 30);
  return (
    <div className="menu" ref={ref} style={{ left: x, top: y }} role="menu" onContextMenu={(e) => e.preventDefault()}>
      {items.map((it, i) => it.sep ? <hr key={i} /> : (
        <button key={i} role="menuitem" disabled={it.disabled} className={`${it.danger ? 'danger' : ''} ${kb && hl === i ? 'hl' : ''}`}
          onMouseEnter={() => { setKb(false); setHl(i); }} onClick={() => { setState({ menu: null }); it.run?.(); }}>
          <span>{it.label}</span>{it.hint && <span className="dim small">{it.hint}</span>}
        </button>
      ))}
    </div>
  );
}
