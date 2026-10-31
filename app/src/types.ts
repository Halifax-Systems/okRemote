export type Protocol = 'ssh' | 'rdp' | 'vnc' | 'http' | 'https' | 'external';
export type Opts = Record<string, unknown>;

export interface Folder {
  id: string; parent_id: string | null; name: string; color: string | null; icon: string | null;
  defaults: Opts; has_secret: boolean; sort: number; version: number;
}
export interface Conn {
  id: string; folder_id: string | null; name: string; protocol: Protocol; host: string; port: number | null;
  options: Opts; tags: string[]; notes: string; icon: string | null; has_secret: boolean; sort: number; version: number;
}
export interface TunnelProfile { id: string; name: string; chain: string[]; forwards: Record<string, unknown>[]; auto_start: boolean; version: number }
export interface TermProfile { id: string; name: string; spec: Record<string, unknown>; version: number }
export interface Snapshot { folders: Folder[]; connections: Conn[]; tunnels: TunnelProfile[]; terminal_profiles: TermProfile[] }
export interface WsInfo { id: string; name: string; path: string; locked: boolean; read_only: boolean }
export interface HostKey { host: string; port: number; key_type: string; fingerprint: string; first_seen: string; trusted_by: string }
export interface AuditEntry { id: string; at: string; actor: string; action: string; target_id: string; detail: unknown }
export interface QuickTarget { protocol: Protocol; host: string; port: number | null; user: string | null; url: string | null; name: string }

export interface TermSettings {
  fontFamily: string; fontSize: number; fontWeight: string; lineHeight: number; scheme: string;
  cursorStyle: 'block' | 'underline' | 'bar'; cursorBlink: boolean; scrollback: number; opacity: number;
  bell: 'off' | 'visual' | 'audible'; copyOnSelect: boolean; rightClickPaste: boolean; confirmMultilinePaste: boolean;
  highlightRules: unknown[];
}
export interface Settings {
  theme: 'system' | 'light' | 'dark'; accent: string; density: 'comfortable' | 'compact'; ui_scale: number;
  startup_mode: 'last' | 'pinned' | 'chooser'; pinned_workspace: string | null; recent_workspaces: string[];
  last_open_workspaces: string[]; auto_lock_minutes: number; lock_on_sleep: boolean; confirm_tab_close: boolean;
  backup_retention: number; clipboard_clear_seconds: number; terminal: TermSettings;
  custom_schemes: { name: string; colors: Record<string, string> }[]; keybindings: Record<string, string>;
  sidebar_width: number; sidebar_collapsed: boolean; recents: { ws: string; id: string; at: number }[];
  last_tabs: { ws: string | null; id: string | null; quick: string | null }[];
  restore_tabs: 'ask' | 'always' | 'never'; expanded: string[];
}

export interface Target { ws: string | null; conn_id: string | null; quick: string | null }
export type TabState = 'connecting' | 'connected' | 'closed' | 'error';
export interface Tab {
  id: string; title: string; protocol: Protocol; target: Target; sessionId: string | null; state: TabState;
  error: string | null; color: string | null; route: string | null; zoom: number; termProfile: string | null;
  osc52: boolean; fixedCols: number | null; fixedRows: number | null; logFile: string | null; cols: number; rows: number;
}
export type PaneNode =
  | { kind: 'leaf'; id: string; tabs: string[]; active: string | null }
  | { kind: 'split'; id: string; dir: 'row' | 'column'; ratio: number; a: PaneNode; b: PaneNode };

export interface PromptMsg { id: string; kind: string; [k: string]: any }
export interface Toast { id: number; kind: 'info' | 'success' | 'error'; text: string }
export interface ApiError { code: string; message: string; detail?: any }
