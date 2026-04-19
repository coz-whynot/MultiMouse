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
  size?: 'sm' | 'md' | 'lg';
}) => {
  const w = size === 'sm' ? 56 : size === 'lg' ? 88 : 72;
  const h = size === 'sm' ? 36 : size === 'lg' ? 56 : 48;
  return (
    <div className="flex flex-col items-center gap-1.5">
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
        <span className="text-[10px] font-bold" style={{ color: `${color}bb` }}>
          {label.split(' ')[0]}
        </span>
      </div>
      {/* Stand */}
      <div className="flex flex-col items-center gap-0" style={{ opacity: 0.5 }}>
        <div style={{ width: 12, height: 4, background: color + '40', borderRadius: 2 }} />
        <div style={{ width: 24, height: 2, background: color + '30', borderRadius: 1 }} />
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
          width: isVertical ? 56 : 36,
          height: isVertical ? 32 : 52,
          background: active ? 'rgba(99,102,241,0.22)' : 'var(--bg-subtle)',
          border: `1.5px solid ${active ? 'rgba(99,102,241,0.55)' : 'var(--border-subtle)'}`,
          boxShadow: active ? '0 0 16px rgba(99,102,241,0.25)' : 'none',
        }}
      >
        <svg
          className="w-4 h-4 transition-colors"
          fill="none"
          viewBox="0 0 24 24"
          stroke={active ? 'var(--accent-primary)' : 'var(--text-faint)'}
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
            className="text-[9px] font-bold truncate max-w-[70px] text-center"
            style={{ color: 'var(--accent-primary)' }}
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

  // Pick the primary mode text
  const modeText = relaying ? 'Sending' : isControlled ? 'Receiving' : 'Idle';
  const modeColor = relaying ? 'var(--accent-secondary)' : isControlled ? 'var(--success)' : 'var(--text-muted)';

  return (
    <div className="flex flex-col flex-1 overflow-y-auto px-5 py-4 gap-4 pb-3">

      {/* ── Two-column dashboard: Left = Edge Picker + Take Control · Right = Flow + Stats ── */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">

        {/* ── LEFT COLUMN — Spatial monitor view + Take Control ── */}
        <div className="flex flex-col gap-4">
          <Card
            title="Screen Edge"
            icon={
              <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M4 5a1 1 0 011-1h4a1 1 0 010 2H5a1 1 0 01-1-1zm0 8a1 1 0 011-1h4a1 1 0 010 2H5a1 1 0 01-1-1zm6-8a1 1 0 011-1h8a1 1 0 010 2h-8a1 1 0 01-1-1zm0 8a1 1 0 011-1h8a1 1 0 010 2h-8a1 1 0 01-1-1z" />
              </svg>
            }
          >
            <p className="text-[12px] mb-5 leading-relaxed" style={{ color: 'var(--text-muted)' }}>
              Which edge of <em>your screen</em> should switch control to the other computer?
            </p>

            {/* Visual monitor layout */}
            <div className="flex flex-col items-center gap-2 select-none">
              {/* Top edge button */}
              <EdgeArrow edge="top" active={edge === 'top'} peerName={connectedPeer?.name} onSet={setEdge} />

              {/* Middle row: left · monitors · right */}
              <div className="flex items-center gap-3">
                <EdgeArrow edge="left" active={edge === 'left'} peerName={connectedPeer?.name} onSet={setEdge} />

                {/* Monitor pair */}
                <div className="flex items-end gap-3 px-2 py-2">
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
                    size="lg"
                  />

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

                <EdgeArrow edge="right" active={edge === 'right'} peerName={connectedPeer?.name} onSet={setEdge} />
              </div>

              {/* Bottom edge button */}
              <EdgeArrow edge="bottom" active={edge === 'bottom'} peerName={connectedPeer?.name} onSet={setEdge} />
            </div>

            {/* Current selection label */}
            <p className="text-center text-xs mt-4" style={{ color: 'var(--text-muted)' }}>
              Push cursor to{' '}
              <span style={{ color: 'var(--accent-primary)', fontWeight: 600 }}>
                {edge} edge
              </span>{' '}
              to switch
            </p>
          </Card>

          {/* Take Control button — prominent */}
          <Card
            title="Control Actions"
            icon={
              <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M15 15l-2 5L9 9l11 4-5 2zm0 0l5 5M7.188 2.239l.777 2.897M5.136 7.965l-2.898-.777M13.95 4.05l-2.122 2.122m-5.657 5.656l-2.12 2.122" />
              </svg>
            }
          >
            {!connectedPeer ? (
              <EmptyConnection />
            ) : (
              <div className="flex flex-col gap-3">
                <button
                  onClick={takeControl}
                  disabled={relaying}
                  className="w-full py-3.5 rounded-2xl text-sm font-bold transition-all active:scale-[0.98] disabled:cursor-default"
                  style={{
                    background: relaying
                      ? 'linear-gradient(135deg, rgba(99,102,241,0.28), rgba(168,85,247,0.2))'
                      : 'linear-gradient(135deg, #6366f1, #a855f7)',
                    border: `1.5px solid ${relaying ? 'rgba(99,102,241,0.5)' : 'rgba(255,255,255,0.12)'}`,
                    color: relaying ? 'var(--accent-primary)' : 'white',
                    boxShadow: relaying ? '0 0 18px rgba(99,102,241,0.25)' : '0 4px 18px rgba(99,102,241,0.3)',
                  }}
                >
                  {relaying ? (
                    <span className="flex items-center justify-center gap-2">
                      <motion.span
                        animate={{ scale: [1, 1.5, 1] }}
                        transition={{ repeat: Infinity, duration: 1.2 }}
                        className="w-2 h-2 rounded-full inline-block"
                        style={{ background: 'var(--accent-secondary)' }}
                      />
                      Controlling {connectedPeer.name.split(' ')[0]}
                    </span>
                  ) : (
                    'Take Control'
                  )}
                </button>
                <button
                  onClick={releaseControl}
                  disabled={!relaying && !isControlled}
                  className="w-full py-2.5 rounded-xl text-xs font-bold transition-all active:scale-[0.97] disabled:opacity-25"
                  style={{
                    background: 'var(--bg-subtle)',
                    border: '1.5px solid var(--border-subtle)',
                    color: 'var(--text-body)',
                  }}
                >
                  Release
                </button>
                <p className="text-[11px] text-center mt-1" style={{ color: 'var(--text-faint)' }}>
                  Or push cursor to the <span style={{ color: 'var(--accent-primary)' }}>{edge} edge</span>
                </p>
              </div>
            )}
          </Card>
        </div>

        {/* ── RIGHT COLUMN — Flow + Stats + Trackpad ── */}
        <div className="flex flex-col gap-4">
          <Card
            title="Connection"
            icon={
              <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M13.828 10.172a4 4 0 00-5.656 0l-4 4a4 4 0 105.656 5.656l1.102-1.101m-.758-4.899a4 4 0 005.656 0l4-4a4 4 0 00-5.656-5.656l-1.1 1.1" />
              </svg>
            }
          >
            {!connectedPeer ? (
              <EmptyConnection />
            ) : (
              <div className="space-y-4">
                {/* Flow diagram */}
                <div className="flex items-center justify-center gap-4 py-2">
                  <DeviceBlob
                    name={status?.device_name ?? 'You'}
                    color="#6366f1"
                    active={relaying}
                    label="This Mac"
                  />

                  {/* Arrow */}
                  <div className="flex flex-col items-center gap-1 w-20">
                    <AnimatePresence mode="wait">
                      {relaying ? (
                        <motion.div key="r" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}>
                          <motion.svg
                            className="w-6 h-6"
                            fill="none"
                            viewBox="0 0 24 24"
                            stroke="#818cf8"
                            strokeWidth={2.5}
                            animate={{ x: [0, 6, 0] }}
                            transition={{ repeat: Infinity, duration: 0.8 }}
                          >
                            <path strokeLinecap="round" strokeLinejoin="round" d="M13 7l5 5m0 0l-5 5m5-5H6" />
                          </motion.svg>
                        </motion.div>
                      ) : isControlled ? (
                        <motion.div key="c" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}>
                          <motion.svg
                            className="w-6 h-6"
                            fill="none"
                            viewBox="0 0 24 24"
                            stroke="#34d399"
                            strokeWidth={2.5}
                            animate={{ x: [0, -6, 0] }}
                            transition={{ repeat: Infinity, duration: 0.8 }}
                          >
                            <path strokeLinecap="round" strokeLinejoin="round" d="M11 17l-5-5m0 0l5-5m-5 5h12" />
                          </motion.svg>
                        </motion.div>
                      ) : (
                        <motion.div key="i" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}>
                          <div className="w-12 h-px" style={{ background: 'var(--divider)' }} />
                        </motion.div>
                      )}
                    </AnimatePresence>
                    <span
                      className="text-[10px] uppercase tracking-wide text-center leading-tight font-semibold"
                      style={{ color: modeColor }}
                    >
                      {modeText}
                    </span>
                  </div>

                  <DeviceBlob
                    name={connectedPeer.name}
                    color="#f97316"
                    active={isControlled}
                    label={connectedPeer.name.split(' ')[0]}
                  />
                </div>

                {/* Stats — expanded to 5 tiles */}
                <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-5 gap-2">
                  <StatTile
                    label="Ping"
                    value={connectedPeer.ping_ms != null ? `${connectedPeer.ping_ms}ms` : '—'}
                    good={connectedPeer.ping_ms != null && connectedPeer.ping_ms < 30}
                    warn={connectedPeer.ping_ms != null && connectedPeer.ping_ms >= 30 && connectedPeer.ping_ms < 80}
                  />
                  <StatTile
                    label="Latency"
                    value={connectedPeer.ping_ms != null
                      ? connectedPeer.ping_ms < 30
                        ? 'Low'
                        : connectedPeer.ping_ms < 80
                          ? 'Med'
                          : 'High'
                      : '—'}
                    good={connectedPeer.ping_ms != null && connectedPeer.ping_ms < 30}
                    warn={connectedPeer.ping_ms != null && connectedPeer.ping_ms >= 30 && connectedPeer.ping_ms < 80}
                  />
                  <StatTile label="Uptime" value={formatUptime(uptimeS)} />
                  <StatTile
                    label="Mode"
                    value={modeText}
                    good={relaying || isControlled}
                  />
                  <StatTile
                    label="Edge"
                    value={edge.charAt(0).toUpperCase() + edge.slice(1)}
                  />
                </div>
              </div>
            )}
          </Card>

          {/* Phone trackpad — always render; it handles its own empty state */}
          <TrackpadCard />

          {/* File transfer hint */}
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
              className="rounded-xl p-4 flex items-start gap-3"
              style={{
                background: 'var(--bg-subtle-2)',
                border: '1px dashed var(--border-subtle)',
              }}
            >
              <svg
                className="w-5 h-5 flex-shrink-0 mt-0.5"
                fill="none"
                viewBox="0 0 24 24"
                stroke="var(--text-ghost)"
                strokeWidth={1.5}
              >
                <path strokeLinecap="round" strokeLinejoin="round"
                  d="M7 16a4 4 0 01-.88-7.903A5 5 0 1115.9 6L16 6a5 5 0 011 9.9M15 13l-3-3m0 0l-3 3m3-3v12" />
              </svg>
              <div>
                <p className="text-xs font-medium mb-0.5" style={{ color: 'var(--text-body)' }}>
                  {connectedPeer ? `Send to ${connectedPeer.name}` : 'No device connected'}
                </p>
                <p className="text-[11px] leading-relaxed" style={{ color: 'var(--text-faint)' }}>
                  {connectedPeer
                    ? 'Drag any file onto this window to send it. It lands in their Downloads folder.'
                    : 'Connect to a device first, then drag files onto this window to transfer them.'}
                </p>
              </div>
            </div>
          </Card>
        </div>
      </div>

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
    className="rounded-2xl p-5"
    style={{
      background: 'var(--bg-subtle-2)',
      border: '1px solid var(--border-subtle)',
    }}
  >
    <div className="flex items-center gap-2 mb-4">
      {icon && (
        <span style={{ color: 'var(--text-muted)' }}>{icon}</span>
      )}
      <p
        className="text-[10px] font-bold uppercase tracking-widest"
        style={{ color: 'var(--text-muted)' }}
      >
        {title}
      </p>
    </div>
    {children}
  </div>
);

const EmptyConnection = () => (
  <div className="flex flex-col items-center gap-2 py-6">
    <div
      className="w-12 h-12 rounded-2xl flex items-center justify-center"
      style={{ background: 'var(--bg-subtle)', border: '1px solid var(--border-subtle)' }}
    >
      <svg className="w-6 h-6" fill="none" viewBox="0 0 24 24" stroke="var(--text-faint)" strokeWidth={1.5}>
        <path strokeLinecap="round" strokeLinejoin="round"
          d="M13.828 10.172a4 4 0 00-5.656 0l-4 4a4 4 0 105.656 5.656l1.102-1.101m-.758-4.899a4 4 0 005.656 0l4-4a4 4 0 00-5.656-5.656l-1.1 1.1" />
      </svg>
    </div>
    <p className="text-sm font-medium" style={{ color: 'var(--text-muted)' }}>No active connection</p>
    <p className="text-xs text-center" style={{ color: 'var(--text-faint)' }}>
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
  <div className="flex flex-col items-center gap-2" style={{ width: 64 }}>
    <motion.div
      animate={
        active
          ? { boxShadow: [`0 0 0 0px ${color}55`, `0 0 0 12px ${color}00`] }
          : { boxShadow: `0 3px 10px ${color}25` }
      }
      transition={{ repeat: active ? Infinity : 0, duration: 1.5 }}
      className="w-14 h-14 rounded-2xl flex items-center justify-center text-lg font-black text-white"
      style={{
        background: `linear-gradient(135deg, ${color}ee, ${color}88)`,
        opacity: active ? 1 : 0.55,
      }}
    >
      {name.charAt(0).toUpperCase()}
    </motion.div>
    <span className="text-[10px] font-medium text-center truncate w-full" style={{ color: 'var(--text-muted)' }}>
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
    className="flex flex-col items-center gap-1 rounded-xl py-3 px-2"
    style={{
      background: 'var(--bg-subtle)',
      border: '1px solid var(--border-subtle)',
    }}
  >
    <span className="text-[9px] uppercase tracking-widest font-bold" style={{ color: 'var(--text-faint)' }}>
      {label}
    </span>
    <span
      className="text-xs font-bold truncate max-w-full"
      style={{ color: good ? 'var(--success)' : warn ? 'var(--warn)' : 'var(--text-secondary)' }}
    >
      {value}
    </span>
  </div>
);
