import { useState, useEffect, useRef } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { useStore } from '../store/useStore';
import { TrackpadCard } from '../components/TrackpadCard';

type Edge = 'left' | 'right' | 'top' | 'bottom';

function formatUptime(s: number) {
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  const sec = s % 60;
  return `${m}m ${sec < 10 ? '0' : ''}${sec}s`;
}

/* ── Monitor SVG shapes ── */
const MonitorShape = ({
  label,
  color,
  size = 'md',
}: {
  label: string;
  color: string;
  size?: 'sm' | 'md';
}) => {
  const w = size === 'sm' ? 48 : 60;
  const h = size === 'sm' ? 32 : 40;
  return (
    <div className="flex flex-col items-center gap-1">
      <div
        className="rounded flex items-center justify-center"
        style={{
          width: w,
          height: h,
          background: `linear-gradient(135deg, ${color}22, ${color}14)`,
          border: `1.5px solid ${color}50`,
          boxShadow: `0 2px 10px ${color}20`,
        }}
      >
        <span className="text-[9px] font-bold" style={{ color: `${color}bb` }}>
          {label.split(' ')[0]}
        </span>
      </div>
      {/* Stand */}
      <div className="flex flex-col items-center gap-0" style={{ opacity: 0.5 }}>
        <div style={{ width: 10, height: 4, background: color + '40', borderRadius: 2 }} />
        <div style={{ width: 20, height: 2, background: color + '30', borderRadius: 1 }} />
      </div>
    </div>
  );
};

/* ── Directional arrow connector ── */
const EdgeArrow = ({
  edge,
  active,
  peerName,
  onSet,
}: {
  edge: Edge;
  active: boolean;
  peerName?: string;
  onSet: (e: Edge) => void;
}) => {
  const isVertical = edge === 'top' || edge === 'bottom';

  const arrowPath = {
    right: 'M5 12h14M13 6l6 6-6 6',
    left: 'M19 12H5M11 6L5 12l6 6',
    top: 'M12 19V5M6 11l6-6 6 6',
    bottom: 'M12 5v14M6 13l6 6 6-6',
  }[edge];

  return (
    <button
      onClick={() => onSet(edge)}
      className="relative flex flex-col items-center gap-1 group transition-all"
      title={`Switch on ${edge} edge`}
    >
      <div
        className="rounded-xl flex items-center justify-center transition-all"
        style={{
          width: isVertical ? 48 : 32,
          height: isVertical ? 28 : 44,
          background: active ? 'rgba(99,102,241,0.22)' : 'rgba(255,255,255,0.04)',
          border: `1.5px solid ${active ? 'rgba(99,102,241,0.55)' : 'rgba(255,255,255,0.08)'}`,
          boxShadow: active ? '0 0 16px rgba(99,102,241,0.25)' : 'none',
        }}
      >
        <svg
          className="w-4 h-4 transition-colors"
          fill="none"
          viewBox="0 0 24 24"
          stroke={active ? '#a78bfa' : 'rgba(255,255,255,0.25)'}
          strokeWidth={2.5}
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <path d={arrowPath} />
        </svg>
      </div>

      {/* Peer label below active arrow */}
      <AnimatePresence>
        {active && peerName && (
          <motion.p
            initial={{ opacity: 0, scale: 0.8 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={{ opacity: 0, scale: 0.8 }}
            className="text-[9px] font-bold truncate max-w-[60px] text-center"
            style={{ color: '#a78bfa' }}
          >
            {peerName.split(' ')[0]}
          </motion.p>
        )}
      </AnimatePresence>
    </button>
  );
};

export const Layout = () => {
  const { peers, status, settings, setSettings } = useStore();
  const [uptimeS, setUptimeS] = useState(0);
  const startRef = useRef<number>(Date.now());

  const connectedPeer = peers.find((p) => p.id === status?.connected_peer);
  const relaying = status?.relaying ?? false;
  const isControlled = (status as any)?.is_controlled ?? false;
  const edge: Edge = (settings?.transition_edge as Edge) ?? 'right';

  useEffect(() => {
    if (!connectedPeer) { setUptimeS(0); return; }
    startRef.current = Date.now();
    setUptimeS(0);
    const t = setInterval(
      () => setUptimeS(Math.floor((Date.now() - startRef.current) / 1000)),
      1000,
    );
    return () => clearInterval(t);
  }, [connectedPeer?.id]);

  const setEdge = async (e: Edge) => {
    if (!settings) return;
    const next = { ...settings, transition_edge: e };
    try {
      await invoke('update_settings', { settings: next });
      setSettings(next);
    } catch (err) {
      console.error(err);
    }
  };

  const takeControl = () => invoke('take_control').catch(console.error);
  const releaseControl = () => invoke('release_cursor').catch(console.error);

  return (
    <div className="flex flex-col flex-1 overflow-y-auto px-3 py-3 gap-3 pb-2">

      {/* ── Control Card ── */}
      <Card
        title="Control"
        icon={
          <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M15 15l-2 5L9 9l11 4-5 2zm0 0l5 5M7.188 2.239l.777 2.897M5.136 7.965l-2.898-.777M13.95 4.05l-2.122 2.122m-5.657 5.656l-2.12 2.122" />
          </svg>
        }
      >
        {!connectedPeer ? (
          <EmptyConnection />
        ) : (
          <div className="space-y-3">
            {/* Flow diagram */}
            <div className="flex items-center justify-center gap-3 py-1">
              <DeviceBlob
                name={status?.device_name ?? 'You'}
                color="#6366f1"
                active={relaying}
                label="This Mac"
              />

              {/* Arrow */}
              <div className="flex flex-col items-center gap-1 w-14">
                <AnimatePresence mode="wait">
                  {relaying ? (
                    <motion.div key="r" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}>
                      <motion.svg
                        className="w-5 h-5"
                        fill="none"
                        viewBox="0 0 24 24"
                        stroke="#818cf8"
                        strokeWidth={2.5}
                        animate={{ x: [0, 5, 0] }}
                        transition={{ repeat: Infinity, duration: 0.8 }}
                      >
                        <path strokeLinecap="round" strokeLinejoin="round" d="M13 7l5 5m0 0l-5 5m5-5H6" />
                      </motion.svg>
                    </motion.div>
                  ) : isControlled ? (
                    <motion.div key="c" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}>
                      <motion.svg
                        className="w-5 h-5"
                        fill="none"
                        viewBox="0 0 24 24"
                        stroke="#34d399"
                        strokeWidth={2.5}
                        animate={{ x: [0, -5, 0] }}
                        transition={{ repeat: Infinity, duration: 0.8 }}
                      >
                        <path strokeLinecap="round" strokeLinejoin="round" d="M11 17l-5-5m0 0l5-5m-5 5h12" />
                      </motion.svg>
                    </motion.div>
                  ) : (
                    <motion.div key="i" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}>
                      <div className="w-8 h-px" style={{ background: 'rgba(255,255,255,0.12)' }} />
                    </motion.div>
                  )}
                </AnimatePresence>
                <span
                  className="text-[9px] uppercase tracking-wide text-center leading-tight"
                  style={{ color: relaying ? '#818cf8' : isControlled ? '#34d399' : 'rgba(255,255,255,0.2)' }}
                >
                  {relaying ? 'Sending' : isControlled ? 'Receiving' : 'Idle'}
                </span>
              </div>

              <DeviceBlob
                name={connectedPeer.name}
                color="#f97316"
                active={isControlled}
                label={connectedPeer.name.split(' ')[0]}
              />
            </div>

            {/* Action buttons */}
            <div className="flex gap-2">
              <button
                onClick={takeControl}
                disabled={relaying}
                className="flex-1 py-2.5 rounded-xl text-xs font-bold transition-all active:scale-[0.97] disabled:cursor-default"
                style={{
                  background: relaying ? 'rgba(99,102,241,0.22)' : 'rgba(99,102,241,0.12)',
                  border: `1.5px solid ${relaying ? 'rgba(99,102,241,0.5)' : 'rgba(99,102,241,0.25)'}`,
                  color: relaying ? '#a78bfa' : 'rgba(167,139,250,0.6)',
                  boxShadow: relaying ? '0 0 14px rgba(99,102,241,0.2)' : 'none',
                }}
              >
                {relaying ? (
                  <span className="flex items-center justify-center gap-1.5">
                    <motion.span
                      animate={{ scale: [1, 1.5, 1] }}
                      transition={{ repeat: Infinity, duration: 1.2 }}
                      className="w-1.5 h-1.5 rounded-full inline-block"
                      style={{ background: '#818cf8' }}
                    />
                    Controlling
                  </span>
                ) : (
                  'Take Control'
                )}
              </button>
              <button
                onClick={releaseControl}
                disabled={!relaying && !isControlled}
                className="flex-1 py-2.5 rounded-xl text-xs font-bold transition-all active:scale-[0.97] disabled:opacity-25"
                style={{
                  background: 'rgba(255,255,255,0.04)',
                  border: '1.5px solid rgba(255,255,255,0.08)',
                  color: 'rgba(255,255,255,0.45)',
                }}
              >
                Release
              </button>
            </div>

            {/* Stats */}
            <div className="grid grid-cols-3 gap-1.5">
              <StatTile
                label="Ping"
                value={connectedPeer.ping_ms != null ? `${connectedPeer.ping_ms}ms` : '—'}
                good={connectedPeer.ping_ms != null && connectedPeer.ping_ms < 30}
                warn={connectedPeer.ping_ms != null && connectedPeer.ping_ms >= 30 && connectedPeer.ping_ms < 80}
              />
              <StatTile label="Uptime" value={formatUptime(uptimeS)} />
              <StatTile
                label="Mode"
                value={relaying ? 'Sending' : isControlled ? 'Rcvng' : 'Idle'}
                good={relaying || isControlled}
              />
            </div>
          </div>
        )}
      </Card>

      {/* ── Edge Picker ── */}
      <Card
        title="Screen Edge"
        icon={
          <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M4 5a1 1 0 011-1h4a1 1 0 010 2H5a1 1 0 01-1-1zm0 8a1 1 0 011-1h4a1 1 0 010 2H5a1 1 0 01-1-1zm6-8a1 1 0 011-1h8a1 1 0 010 2h-8a1 1 0 01-1-1zm0 8a1 1 0 011-1h8a1 1 0 010 2h-8a1 1 0 01-1-1z" />
          </svg>
        }
      >
        <p className="text-[11px] mb-4 leading-relaxed" style={{ color: 'rgba(255,255,255,0.32)' }}>
          Which edge of <em>your screen</em> should trigger switching control to the other computer?
        </p>

        {/* Visual monitor layout */}
        <div className="flex flex-col items-center gap-1 select-none">
          {/* Top edge button */}
          <EdgeArrow edge="top" active={edge === 'top'} peerName={connectedPeer?.name} onSet={setEdge} />

          {/* Middle row: left · monitors · right */}
          <div className="flex items-center gap-2">
            {/* Left arrow */}
            <EdgeArrow edge="left" active={edge === 'left'} peerName={connectedPeer?.name} onSet={setEdge} />

            {/* Monitor pair */}
            <div className="flex items-end gap-2 px-1 py-1">
              {/* Other machine — positioned based on edge */}
              {(edge === 'left' || edge === 'top' || edge === 'bottom') && (
                <motion.div
                  key={`peer-left-${edge}`}
                  initial={{ opacity: 0, scale: 0.85 }}
                  animate={{ opacity: 1, scale: 1 }}
                  className="opacity-60"
                >
                  <MonitorShape
                    label={connectedPeer?.name ?? 'Other'}
                    color="#f97316"
                    size="sm"
                  />
                </motion.div>
              )}

              {/* This machine — always center */}
              <MonitorShape
                label={status?.device_name ?? 'You'}
                color="#6366f1"
                size="md"
              />

              {/* Other machine on right */}
              {(edge === 'right' || edge === 'top' || edge === 'bottom') && (
                <motion.div
                  key={`peer-right-${edge}`}
                  initial={{ opacity: 0, scale: 0.85 }}
                  animate={{ opacity: 1, scale: 1 }}
                  className="opacity-60"
                >
                  <MonitorShape
                    label={connectedPeer?.name ?? 'Other'}
                    color="#f97316"
                    size="sm"
                  />
                </motion.div>
              )}
            </div>

            {/* Right arrow */}
            <EdgeArrow edge="right" active={edge === 'right'} peerName={connectedPeer?.name} onSet={setEdge} />
          </div>

          {/* Bottom edge button */}
          <EdgeArrow edge="bottom" active={edge === 'bottom'} peerName={connectedPeer?.name} onSet={setEdge} />
        </div>

        {/* Current selection label */}
        <p className="text-center text-xs mt-3" style={{ color: 'rgba(255,255,255,0.28)' }}>
          Push cursor to{' '}
          <span style={{ color: '#a78bfa', fontWeight: 600 }}>
            {edge} edge
          </span>{' '}
          to switch
        </p>
      </Card>

      {/* ── Phone Trackpad ── */}
      <TrackpadCard />

      {/* ── File Transfer ── */}
      <Card
        title="File Transfer"
        icon={
          <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round"
              d="M7 16a4 4 0 01-.88-7.903A5 5 0 1115.9 6L16 6a5 5 0 011 9.9M15 13l-3-3m0 0l-3 3m3-3v12" />
          </svg>
        }
      >
        <div
          className="rounded-xl p-3.5 flex items-start gap-3"
          style={{
            background: 'rgba(255,255,255,0.025)',
            border: '1px dashed rgba(255,255,255,0.08)',
          }}
        >
          <svg
            className="w-5 h-5 flex-shrink-0 mt-0.5"
            fill="none"
            viewBox="0 0 24 24"
            stroke="rgba(255,255,255,0.22)"
            strokeWidth={1.5}
          >
            <path strokeLinecap="round" strokeLinejoin="round"
              d="M7 16a4 4 0 01-.88-7.903A5 5 0 1115.9 6L16 6a5 5 0 011 9.9M15 13l-3-3m0 0l-3 3m3-3v12" />
          </svg>
          <div>
            <p className="text-xs font-medium mb-0.5" style={{ color: 'rgba(255,255,255,0.55)' }}>
              {connectedPeer ? `Send to ${connectedPeer.name}` : 'No device connected'}
            </p>
            <p className="text-[11px] leading-relaxed" style={{ color: 'rgba(255,255,255,0.28)' }}>
              {connectedPeer
                ? 'Drag any file onto this window to send it. It lands in their Downloads folder.'
                : 'Connect to a device first, then drag files onto this window to transfer them.'}
            </p>
          </div>
        </div>
      </Card>

    </div>
  );
};

/* ── Sub-components ── */

const Card = ({
  title,
  icon,
  children,
}: {
  title: string;
  icon?: React.ReactNode;
  children: React.ReactNode;
}) => (
  <div
    className="rounded-2xl p-4"
    style={{
      background: 'rgba(255,255,255,0.03)',
      border: '1px solid rgba(255,255,255,0.07)',
    }}
  >
    <div className="flex items-center gap-2 mb-3">
      {icon && (
        <span style={{ color: 'rgba(255,255,255,0.3)' }}>{icon}</span>
      )}
      <p
        className="text-[10px] font-bold uppercase tracking-widest"
        style={{ color: 'rgba(255,255,255,0.32)' }}
      >
        {title}
      </p>
    </div>
    {children}
  </div>
);

const EmptyConnection = () => (
  <div className="flex flex-col items-center gap-2 py-4">
    <div
      className="w-10 h-10 rounded-2xl flex items-center justify-center"
      style={{ background: 'rgba(255,255,255,0.04)', border: '1px solid rgba(255,255,255,0.07)' }}
    >
      <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="rgba(255,255,255,0.2)" strokeWidth={1.5}>
        <path strokeLinecap="round" strokeLinejoin="round"
          d="M13.828 10.172a4 4 0 00-5.656 0l-4 4a4 4 0 105.656 5.656l1.102-1.101m-.758-4.899a4 4 0 005.656 0l4-4a4 4 0 00-5.656-5.656l-1.1 1.1" />
      </svg>
    </div>
    <p className="text-sm font-medium" style={{ color: 'rgba(255,255,255,0.28)' }}>No active connection</p>
    <p className="text-xs text-center" style={{ color: 'rgba(255,255,255,0.18)' }}>
      Go to Devices tab to connect to another computer
    </p>
  </div>
);

const DeviceBlob = ({
  name,
  color,
  active,
  label,
}: {
  name: string;
  color: string;
  active: boolean;
  label?: string;
}) => (
  <div className="flex flex-col items-center gap-1.5" style={{ width: 52 }}>
    <motion.div
      animate={
        active
          ? { boxShadow: [`0 0 0 0px ${color}55`, `0 0 0 10px ${color}00`] }
          : { boxShadow: `0 3px 10px ${color}25` }
      }
      transition={{ repeat: active ? Infinity : 0, duration: 1.5 }}
      className="w-11 h-11 rounded-2xl flex items-center justify-center text-sm font-black text-white"
      style={{
        background: `linear-gradient(135deg, ${color}ee, ${color}88)`,
        opacity: active ? 1 : 0.55,
      }}
    >
      {name.charAt(0).toUpperCase()}
    </motion.div>
    <span className="text-[9px] font-medium text-center truncate w-full" style={{ color: 'rgba(255,255,255,0.3)' }}>
      {label ?? name.split(' ')[0]}
    </span>
  </div>
);

const StatTile = ({
  label,
  value,
  good,
  warn,
}: {
  label: string;
  value: string;
  good?: boolean;
  warn?: boolean;
}) => (
  <div
    className="flex flex-col items-center gap-0.5 rounded-xl py-2 px-1"
    style={{ background: 'rgba(255,255,255,0.035)' }}
  >
    <span className="text-[8px] uppercase tracking-wide" style={{ color: 'rgba(255,255,255,0.25)' }}>
      {label}
    </span>
    <span
      className="text-xs font-bold"
      style={{ color: good ? '#34d399' : warn ? '#fbbf24' : 'rgba(255,255,255,0.6)' }}
    >
      {value}
    </span>
  </div>
);
