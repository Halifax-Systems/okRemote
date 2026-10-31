import { useEffect } from 'react';
import { useStore } from './store';
import { boot, handleShortcut, updateSettings } from './actions';
import { Sidebar } from './components/Sidebar';
import { Sessions, FirstRun } from './components/Sessions';
import { Editor } from './components/Editor';
import { StatusBar, TopBar } from './components/Chrome';
import { Dialogs, Prompts } from './components/Dialogs';
import { Palette } from './components/Palette';
import { ContextMenu, Toasts } from './components/Overlays';

export function App() {
  const ready = useStore((s) => s.ready);
  const collapsed = useStore((s) => s.settings.sidebar_collapsed);
  const palette = useStore((s) => s.palette);
  const firstRun = useStore((s) => s.firstRun);

  useEffect(() => {
    void boot();
    const key = (e: KeyboardEvent) => { handleShortcut(e); };
    const ctx = (e: MouseEvent) => { if (!(e.target as HTMLElement).closest('input, textarea')) e.preventDefault(); };
    window.addEventListener('keydown', key, true);
    window.addEventListener('contextmenu', ctx);
    return () => { window.removeEventListener('keydown', key, true); window.removeEventListener('contextmenu', ctx); };
  }, []);

  const dragSidebar = (e: React.MouseEvent) => {
    e.preventDefault();
    const move = (ev: MouseEvent) => updateSettings({ sidebar_width: Math.round(Math.min(600, Math.max(200, ev.clientX))) });
    const up = () => { window.removeEventListener('mousemove', move); window.removeEventListener('mouseup', up); };
    window.addEventListener('mousemove', move);
    window.addEventListener('mouseup', up);
  };

  if (!ready) return <div style={{ padding: 20 }}>Starting…</div>;
  return (
    <div id="app">
      <TopBar />
      <div className="main">
        {!collapsed && <><Sidebar /><div className="handle" onMouseDown={dragSidebar} /></>}
        {firstRun ? <FirstRun /> : <Sessions />}
        <Editor />
      </div>
      <StatusBar />
      <Dialogs />
      <Prompts />
      {palette && <Palette />}
      <ContextMenu />
      <Toasts />
    </div>
  );
}
