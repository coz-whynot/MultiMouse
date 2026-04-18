import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { MonitorInfo } from '../types';

const EDGES = ['left', 'right', 'top', 'bottom'] as const;
type Edge = (typeof EDGES)[number];

interface Props {
  activeEdge: Edge;
  onChange: (edge: Edge) => void;
}

const CANVAS_W = 260;
const CANVAS_H = 130;

function scaleMonitors(monitors: MonitorInfo[]): {
  monitors: Array<MonitorInfo & { sx: number; sy: number; sw: number; sh: number }>;
  minX: number;
  minY: number;
  totalW: number;
  totalH: number;
  scale: number;
} {
  if (monitors.length === 0) {
    return {
      monitors: [],
      minX: 0,
      minY: 0,
      totalW: 1920,
      totalH: 1080,
      scale: 1,
    };
  }
  const minX = Math.min(...monitors.map((m) => m.x));
  const minY = Math.min(...monitors.map((m) => m.y));
  const maxX = Math.max(...monitors.map((m) => m.x + m.width));
  const maxY = Math.max(...monitors.map((m) => m.y + m.height));
  const totalW = maxX - minX;
  const totalH = maxY - minY;
  const scaleX = (CANVAS_W - 40) / totalW;
  const scaleY = (CANVAS_H - 40) / totalH;
  const scale = Math.min(scaleX, scaleY);
  const offsetX = (CANVAS_W - totalW * scale) / 2;
  const offsetY = (CANVAS_H - totalH * scale) / 2;

  return {
    monitors: monitors.map((m) => ({
      ...m,
      sx: (m.x - minX) * scale + offsetX,
      sy: (m.y - minY) * scale + offsetY,
      sw: m.width * scale,
      sh: m.height * scale,
    })),
    minX,
    minY,
    totalW,
    totalH,
    scale,
  };
}

const EDGE_ARROW: Record<Edge, string> = {
  right: '→',
  left: '←',
  top: '↑',
  bottom: '↓',
};

export const LayoutEditor = ({ activeEdge, onChange }: Props) => {
  const [monitors, setMonitors] = useState<MonitorInfo[]>([]);

  useEffect(() => {
    invoke<MonitorInfo[]>('get_monitors')
      .then(setMonitors)
      .catch(() => {});
  }, []);

  const scaled = scaleMonitors(monitors);

  return (
    <div className="space-y-3">
      {/* Visual canvas */}
      <div
        className="relative rounded-xl bg-white/[0.03] border border-white/[0.07] overflow-hidden"
        style={{ width: CANVAS_W, height: CANVAS_H }}
      >
        {scaled.monitors.map((m, i) => (
          <div
            key={i}
            className="absolute rounded border border-white/20 bg-white/[0.06] flex items-center justify-center"
            style={{ left: m.sx, top: m.sy, width: m.sw, height: m.sh }}
          >
            <span className="text-[8px] text-white/40 font-mono truncate px-1 text-center leading-tight">
              {m.is_primary ? '●' : '○'} {m.name.replace(/[^\w\s]/g, '') || `Display ${i + 1}`}
              {'\n'}
              {m.width}×{m.height}
            </span>
          </div>
        ))}

        {/* Edge indicator overlay */}
        {activeEdge === 'right' && (
          <div className="absolute right-0 top-0 bottom-0 w-3 bg-gradient-to-l from-accent-500/40 to-transparent flex items-center justify-end pr-0.5">
            <span className="text-accent-400 text-[9px]">▶</span>
          </div>
        )}
        {activeEdge === 'left' && (
          <div className="absolute left-0 top-0 bottom-0 w-3 bg-gradient-to-r from-accent-500/40 to-transparent flex items-center justify-start pl-0.5">
            <span className="text-accent-400 text-[9px]">◀</span>
          </div>
        )}
        {activeEdge === 'top' && (
          <div className="absolute top-0 left-0 right-0 h-3 bg-gradient-to-b from-accent-500/40 to-transparent flex justify-center items-start pt-0.5">
            <span className="text-accent-400 text-[9px]">▲</span>
          </div>
        )}
        {activeEdge === 'bottom' && (
          <div className="absolute bottom-0 left-0 right-0 h-3 bg-gradient-to-t from-accent-500/40 to-transparent flex justify-center items-end pb-0.5">
            <span className="text-accent-400 text-[9px]">▼</span>
          </div>
        )}

        {monitors.length === 0 && (
          <div className="absolute inset-0 flex items-center justify-center">
            <span className="text-[10px] text-white/20">Loading display info…</span>
          </div>
        )}
      </div>

      {/* Edge buttons */}
      <div className="grid grid-cols-4 gap-2">
        {EDGES.map((edge) => (
          <button
            key={edge}
            onClick={() => onChange(edge)}
            className={`py-2 rounded-xl text-xs font-medium transition-all ${
              activeEdge === edge
                ? 'bg-accent-500 text-white shadow-lg shadow-accent-500/20'
                : 'bg-white/[0.04] text-white/50 hover:bg-white/[0.08] border border-white/[0.07]'
            }`}
          >
            {EDGE_ARROW[edge]} {edge}
          </button>
        ))}
      </div>

      {monitors.length > 1 && (
        <p className="text-[10px] text-white/25">
          {monitors.length} displays detected — transition fires at the outer edge of the combined desktop.
        </p>
      )}
    </div>
  );
};
