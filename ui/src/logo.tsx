// Scout's logo in the variants Settings > Appearance offers. Classic is the app's own icon; the rest are drawn from the
// same mark, so they stay sharp at any size. The one picked also becomes the taskbar icon (drawn here, sent to Rust).
import { invoke } from './api';
import { getTheme, subscribeTheme, themeColor, useTheme } from './theme';
import classic from '../../assets/icon.png';

/** The mark (a < and a > that interlock into an S), on a 32×32 tile. */
const MARK = 'M18.65 4.84V8.79L10.74 13.32 16.03 16.57V20.21L7.48 15.07V11.37ZM13.35 27.16V23.21L21.26 18.68 15.97 15.43V11.79L24.52 16.93V20.63Z';

export type LogoVariant = {
  id: string;
  name: string;
  /** Tile fill: one color or a gradient from top left to bottom right. "primary" follows the theme's buttons. */
  tile: string[];
  ink: string;
  /** An edge around the tile, for the outline and glass looks. */
  edge?: string;
  /** A soft light along the top, for glass. */
  sheen?: boolean;
};

export const LOGOS: LogoVariant[] = [
  { id: 'classic', name: 'Classic', tile: ['#f5a623'], ink: '#ffffff' },
  { id: 'theme', name: 'Theme', tile: ['primary'], ink: 'primary-ink' },
  { id: 'graphite', name: 'Graphite', tile: ['#3a3a3a', '#1c1c1c'], ink: '#f5a623' },
  { id: 'paper', name: 'Paper', tile: ['#f7f7f4', '#e4e3dc'], ink: '#26251e' },
  { id: 'sunset', name: 'Sunset', tile: ['#ffb347', '#f2546d'], ink: '#ffffff' },
  { id: 'ocean', name: 'Ocean', tile: ['#4fb3ff', '#5b4dff'], ink: '#ffffff' },
  { id: 'mint', name: 'Mint', tile: ['#5ee0a8', '#0f9e9b'], ink: '#ffffff' },
  { id: 'outline', name: 'Outline', tile: ['transparent'], ink: '#f5a623', edge: '#f5a623' },
  { id: 'glass', name: 'Glass', tile: ['rgb(255 255 255 / 0.22)', 'rgb(255 255 255 / 0.06)'], ink: '#ffffff', edge: 'rgb(255 255 255 / 0.35)', sheen: true },
];

export const logoById = (id: string) => LOGOS.find((l) => l.id === id) ?? LOGOS[0];

/** A theme color name ("primary") as its value now; anything else as it is. */
const color = (c: string) => (/^[a-z-]+$/.test(c) && c !== 'transparent' && c !== 'white' ? themeColor(c) || c : c);

/** The variant as an SVG document, with every color resolved, for an <img> or the taskbar icon. */
export function logoSvg(v: LogoVariant): string {
  const [a, b] = v.tile.map(color);
  const fill = b ? 'url(#g)' : a;
  const edge = v.edge ? `stroke="${color(v.edge)}" stroke-width="1.2"` : '';
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" width="32" height="32">
<defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="${a}"/><stop offset="1" stop-color="${b ?? a}"/></linearGradient>
<linearGradient id="s" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#fff" stop-opacity=".35"/><stop offset=".5" stop-color="#fff" stop-opacity="0"/></linearGradient></defs>
<rect x="${v.edge ? 0.6 : 0}" y="${v.edge ? 0.6 : 0}" width="${v.edge ? 30.8 : 32}" height="${v.edge ? 30.8 : 32}" rx="7.5" fill="${fill}" ${edge}/>
${v.sheen ? '<rect x="1" y="1" width="30" height="15" rx="7" fill="url(#s)"/>' : ''}
<path d="${MARK}" fill="${color(v.ink)}" stroke="${color(v.ink)}" stroke-width="1" stroke-linejoin="round" transform="translate(16 16) scale(0.92) translate(-16 -16)"/>
</svg>`;
}

const dataUrl = (svg: string) => `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;

/** Scout's logo as the user picked it, or a specific variant when `id` is given. */
export function ScoutLogo({ size, className, drag, id }: { size: number; className?: string; drag?: boolean; id?: string }) {
  const theme = useTheme();
  const v = logoById(id ?? theme.logo);
  // Theme colors are read when it draws, so it follows a change of theme too.
  const src = v.id === 'classic' ? classic : dataUrl(logoSvg(v));
  return <img className={className} src={src} width={size} height={size} alt="" {...(drag ? { 'data-tauri-drag-region': true } : {})} />;
}

let drawn = '';

// The taskbar icon follows the pick (and, for Theme, the button color), from the start.
queueMicrotask(() => updateAppIcon(getTheme().logo));
subscribeTheme(() => updateAppIcon(getTheme().logo));

/** Makes the taskbar icon match the logo. Only redraws when it would look different. */
export function updateAppIcon(id: string) {
  const v = logoById(id);
  const src = v.id === 'classic' ? classic : dataUrl(logoSvg(v));
  if (src === drawn) return;
  drawn = src;
  const size = 64;
  const img = new Image();
  img.crossOrigin = 'anonymous';

  const drawAndSend = (imageSource: HTMLImageElement) => {
    try {
      const canvas = document.createElement('canvas');
      canvas.width = canvas.height = size;
      const ctx = canvas.getContext('2d');
      if (!ctx) return;
      ctx.drawImage(imageSource, 0, 0, size, size);
      const rgba = Array.from(ctx.getImageData(0, 0, size, size).data);
      invoke('set_app_icon', { rgba, size }).catch((e) => {
        console.error('set_app_icon', e);
        drawn = '';
      });
    } catch (e) {
      console.error('Failed to draw icon to canvas', e);
      // If an external image (e.g. classic PNG) throws due to cross-origin taint, fall back to SVG
      if (imageSource.src !== dataUrl(logoSvg(v))) {
        const svgImg = new Image();
        svgImg.onload = () => drawAndSend(svgImg);
        svgImg.src = dataUrl(logoSvg(v));
      } else {
        drawn = '';
      }
    }
  };

  img.onload = () => drawAndSend(img);
  img.onerror = () => {
    console.warn('Failed to load icon image', src);
    if (src !== dataUrl(logoSvg(v))) {
      const svgImg = new Image();
      svgImg.onload = () => drawAndSend(svgImg);
      svgImg.src = dataUrl(logoSvg(v));
    } else {
      drawn = '';
    }
  };
  img.src = src;
}
