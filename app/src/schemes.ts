export interface Scheme { name: string; dark: boolean; background: string; foreground: string; selectionBackground: string; cursor?: string; ansi: string[] }

const S = (name: string, dark: boolean, bg: string, fg: string, ansi: string, sel?: string): Scheme => ({
  name, dark, background: bg, foreground: fg, selectionBackground: sel ?? (dark ? '#3b4b6b' : '#b4d5fe'), ansi: ansi.split(' '),
});

export const BUILTIN_SCHEMES: Scheme[] = [
  S('OkRemote Dark', true, '#181818', '#dcdcdc', '#2a2a2a #f0717a #8bd17c #e8c46a #6aa7f8 #c792ea #5fd4d8 #dcdcdc #6b6b6b #ff8f97 #a6e59a #ffd98a #8fbfff #dcb0ff #86eaee #ffffff'),
  S('OkRemote Light', false, '#faf9f7', '#1f1e1d', '#1f2937 #c62828 #2e7d32 #9a6b00 #1d4ed8 #8e24aa #00838f #e5e7eb #6b7280 #e53935 #43a047 #b58900 #2563eb #ab47bc #0097a7 #ffffff'),
  S('Solarized Dark', true, '#002b36', '#839496', '#073642 #dc322f #859900 #b58900 #268bd2 #d33682 #2aa198 #eee8d5 #586e75 #cb4b16 #586e75 #657b83 #839496 #6c71c4 #93a1a1 #fdf6e3'),
  S('Solarized Light', false, '#fdf6e3', '#657b83', '#073642 #dc322f #859900 #b58900 #268bd2 #d33682 #2aa198 #eee8d5 #002b36 #cb4b16 #586e75 #657b83 #839496 #6c71c4 #93a1a1 #fdf6e3', '#eee8d5'),
  S('Dracula', true, '#282a36', '#f8f8f2', '#21222c #ff5555 #50fa7b #f1fa8c #bd93f9 #ff79c6 #8be9fd #f8f8f2 #6272a4 #ff6e6e #69ff94 #ffffa5 #d6acff #ff92df #a4ffff #ffffff', '#44475a'),
  S('Nord', true, '#2e3440', '#d8dee9', '#3b4252 #bf616a #a3be8c #ebcb8b #81a1c1 #b48ead #88c0d0 #e5e9f0 #4c566a #bf616a #a3be8c #ebcb8b #81a1c1 #b48ead #8fbcbb #eceff4', '#434c5e'),
  S('Gruvbox Dark', true, '#282828', '#ebdbb2', '#282828 #cc241d #98971a #d79921 #458588 #b16286 #689d6a #a89984 #928374 #fb4934 #b8bb26 #fabd2f #83a598 #d3869b #8ec07c #ebdbb2', '#504945'),
  S('Campbell', true, '#0c0c0c', '#cccccc', '#0c0c0c #c50f1f #13a10e #c19c00 #0037da #881798 #3a96dd #cccccc #767676 #e74856 #16c60c #f9f1a5 #3b78ff #b4009e #61d6d6 #f2f2f2'),
  S('One Light', false, '#fafafa', '#383a42', '#383a42 #e45649 #50a14f #c18401 #0184bc #a626a4 #0997b3 #fafafa #4f525e #e06c75 #98c379 #e5c07b #61afef #c678dd #56b6c2 #ffffff', '#e5e5e6'),
];

const ANSI_KEYS = ['black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white', 'brightBlack', 'brightRed', 'brightGreen', 'brightYellow', 'brightBlue', 'brightMagenta', 'brightCyan', 'brightWhite'];

export function findScheme(name: string, custom: { name: string; colors: Record<string, string> }[]): Scheme {
  const b = BUILTIN_SCHEMES.find((s) => s.name === name);
  if (b) return b;
  const c = custom.find((s) => s.name === name);
  if (c) {
    const col = c.colors;
    return { name: c.name, dark: true, background: col.background, foreground: col.foreground, cursor: col.cursor,
      selectionBackground: col.selectionBackground ?? '#3b4b6b', ansi: ANSI_KEYS.map((k) => col[k] ?? '#888888') };
  }
  return BUILTIN_SCHEMES[0];
}

export function xtermTheme(s: Scheme) {
  const t: Record<string, string> = { background: s.background, foreground: s.foreground, cursor: s.cursor ?? s.foreground, selectionBackground: s.selectionBackground };
  ANSI_KEYS.forEach((k, i) => { t[k] = s.ansi[i]; });
  return t;
}
