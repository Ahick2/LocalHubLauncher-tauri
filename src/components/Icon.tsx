import type { CSSProperties } from 'react';

const paths = {
  grid: 'M3 3h7v7H3z M14 3h7v7h-7z M3 14h7v7H3z M14 14h7v7h-7z',
  activity: 'M3 12h4l3-8 4 16 3-8h4',
  bolt: 'M13 2 4 14h7l-1 8 10-12h-7z',
  terminal: 'm5 7 5 5-5 5 M13 17h6',
  settings: 'M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8 M9 3h6l1 3 3 1 2 5-2 5-3 1-1 3H9l-1-3-3-1-2-5 2-5 3-1z',
  plus: 'M12 5v14 M5 12h14',
  play: 'm8 4 12 8-12 8z',
  stop: 'M6 6h12v12H6z',
  restart: 'M20 7v5h-5 M20 12a8 8 0 1 0-2 6',
  search: 'M10.5 3a7.5 7.5 0 1 0 0 15 7.5 7.5 0 0 0 0-15 M16 16l5 5',
  chevron: 'm8 10 4 4 4-4',
  close: 'm6 6 12 12 M6 18 18 6',
  folder: 'M3 7V5h7l2 3h9v11H3z',
  link: 'M10 13a4 4 0 0 0 6 0l4-4a4 4 0 0 0-6-6l-2 2 M14 11a4 4 0 0 0-6 0l-4 4a4 4 0 0 0 6 6l2-2',
  copy: 'M8 8h13v13H8z M16 8V3H3v13h5',
  trash: 'M3 6h18 M9 6V3h6v3 M6 6l1 15h10l1-15 M10 10v7 M14 10v7',
  edit: 'm15 4 5 5 M4 20l5-1L21 7l-5-5L4 14z',
  refresh: 'M20 4v6h-6 M4 20v-6h6 M20 10a8 8 0 0 0-14-5 M4 14a8 8 0 0 0 14 5',
  check: 'm5 12 4 4 10-10',
  info: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18 M12 11v6 M12 7v.1',
  warning: 'm12 3 10 18H2z M12 9v5 M12 17v.1',
  down: 'M12 4v16 m-6-6 6 6 6-6',
  more: 'M5 12h.01 M12 12h.01 M19 12h.01',
  exit: 'M9 4H4v16h5 M10 12h11 m-5-5 5 5-5 5',
  server: 'M3 3h18v7H3z M3 14h18v7H3z M7 6.5h.01 M7 17.5h.01',
  upload: 'M12 16V3 m-5 5 5-5 5 5 M4 16v5h16v-5',
} as const;

export type IconName = keyof typeof paths;
export function Icon({ name, size = 18, className = '', style }: {
  name: IconName; size?: number; className?: string; style?: CSSProperties;
}) {
  return <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor"
    strokeWidth={1.65} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"
    className={'icon ' + className} style={style}><path d={paths[name]} /></svg>;
}
