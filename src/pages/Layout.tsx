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


/* ── Drag-to-arrange canvas (ShareMouse / macOS-Displays style) ──
   Local monitor is fixed in the center of a canvas. The remote monitor
   starts on the configured edge and can be dragged to any of the 4 sides;
   on release, it snaps to the nearest edge and saves to settings. */
const ArrangementCanvas = ({
  localName,
  localScreen,
  remoteName,
  remoteScreen,
  edge,
  onEdge,
}: {
  localName: string;
  localScreen?: { width: number; height: number };
  remoteName?: string;
  remoteScreen?: { width: number; height: number };
  edge: Edge;
  onEdge: (e: Edge) => void;
}) => {
  const canvasRef = useRef<HTMLDivElement>(null);
  const CANVAS_W = 360;
  const CANVAS_H = 220;

  // Compute monitor display dimensions at real aspect ratios
  const localAspect = localScreen ? localScreen.width / localScreen.height : 16 / 9;
  const remoteAspect = remoteScreen ? remoteScreen.width / remoteScreen.height : 16 / 9;
  const LOCAL_MAX = 128;
  const REMOTE_MAX = 100;
  const lw = localAspect >= 1 ? LOCAL_MAX : Math.round(LOCAL_MAX * localAspect);
  const lh = localAspect >= 1 ? Math.round(LOCAL_MAX / localAspect) : LOCAL_MAX;
  const rw = remoteAspect >= 1 ? REMOTE_MAX : Math.round(REMOTE_MAX * remoteAspect);
  const rh = remoteAspect >= 1 ? Math.round(REMOTE_MAX / remoteAspect) : REMOTE_MAX;

  // Local monitor is centered
  const lx = (CANVAS_W - lw) / 2;
  const ly = (CANVAS_H - lh) / 2;

  // Remote monitor target position based on current edge
  const targetForEdge = (e: Edge) => {
    const gap = 12;
    switch (e) {
      case 'left':   return { x: lx - rw - gap,                            y: ly + (lh - rh) / 2 };
      case 'right':  return { x: lx + lw + gap,                            y: ly + (lh - rh) / 2 };
      case 'top':    return { x: lx + (lw - rw) / 2,                       y: ly - rh - gap };
      case 'bottom': return { x: lx + (lw - rw) / 2,                       y: ly + lh + gap };
    }
  };
  const snapped = targetForEdge(edge);

  // Figure out closest edge based on where the remote tile was dropped
  const handleDragEnd = (_e: unknown, info: { offset: { x: number; y: number }; point: { x: number; y: number } }) => {
    if (!canvasRef.current) return;
    const rect = canvasRef.current.getBoundingClientRect();
    // Remote tile center in canvas coordinates
    const cx = info.point.x - rect.left;
    const cy = info.point.y - rect.top;
    const localCx = lx + lw / 2;
    const localCy = ly + lh / 2;
    const dx = cx - localCx;
    const dy = cy - localCy;
    // Pick the axis with the largest absolute displacement
    let next: Edge;
    if (Math.abs(dx) > Math.abs(dy)) next = dx > 0 ? 'right' : 'left';
    else                              next = dy > 0 ? 'bottom' : 'top';
    onEdge(next);
  };

  return (
    <div
      ref={canvasRef}
      className="relative rounded-xl mx-auto select-none overflow-hidden"
      style={{
        width: CANVAS_W,
        height: CANVAS_H,
        background: 'var(--bg-subtle)',
        border: '1px dashed var(--border-subtle)',
      }}
    >
      {/* Drop-zone hints: subtle edge strips that highlight while dragging near */}
      <div className="absolute inset-0 pointer-events-none">
        {(['left', 'right', 'top', 'bottom'] as Edge[]).map((e) => {
          const isActive = edge === e;
          const common = {
            position: 'absolute' as const,
            background: isActive ? 'rgba(99,102,241,0.10)' : 'transparent',
          };
          if (e === 'left')   return <div key={e} style={{ ...common, left: 0, top: 0, bottom: 0, width: 40 }} />;
          if (e === 'right')  return <div key={e} style={{ ...common, right: 0, top: 0, bottom: 0, width: 40 }} />;
          if (e === 'top')    return <div key={e} style={{ ...common, left: 0, right: 0, top: 0, height: 40 }} />;
          return                    <div key={e} style={{ ...common, left: 0, right: 0, bottom: 0, height: 40 }} />;
        })}
      </div>

      {/* Local monitor — fixed center, labeled */}
      <div
        className="absolute rounded-md flex items-center justify-center"
        style={{
          left: lx,
          top: ly,
          width: lw,
          height: lh,
          background: 'linear-gradient(135deg, rgba(99,102,241,0.22), rgba(99,102,241,0.10))',
          border: '1.5px solid rgba(99,102,241,0.55)',
          boxShadow: '0 4px 18px rgba(99,102,241,0.22)',
        }}
      >
        <div className="text-center px-1">
          <p className="text-[10px] font-bold truncate" style={{ color: 'var(--accent-primary)' }}>
            {localName.split(' ')[0]}
          </p>
          {localScreen && (
            <p className="text-[8px] font-mono mt-0.5" style={{ color: 'var(--text-faint)' }}>
              {Math.round(localScreen.width)}×{Math.round(localScreen.height)}
            </p>
          )}
        </div>
      </div>

      {/* Remote monitor — draggable */}
      {remoteName && (
        <motion.div
          key={`${edge}-${remoteName}`}
          className="absolute rounded-md flex items-center justify-center cursor-grab active:cursor-grabbing"
          drag
          dragMomentum={false}
          dragElastic={0}
          dragConstraints={canvasRef}
          onDragEnd={handleDragEnd}
          initial={{ left: snapped.x, top: snapped.y }}
          animate={{ left: snapped.x, top: snapped.y }}
          transition={{ type: 'spring', stiffness: 320, damping: 28 }}
          whileDrag={{ scale: 1.05, boxShadow: '0 10px 28px rgba(249,115,22,0.35)' }}
          style={{
            width: rw,
            height: rh,
            background: 'linear-gradient(135deg, rgba(249,115,22,0.22), rgba(249,115,22,0.10))',
            border: '1.5px solid rgba(249,115,22,0.55)',
            boxShadow: '0 3px 14px rgba(249,115,22,0.22)',
            touchAction: 'none',
          }}
        >
          <div className="text-center px-1 pointer-events-none">
            <p className="text-[10px] font-bold truncate" style={{ color: '#f97316' }}>
              {remoteName.split(' ')[0]}
            </p>
            {remoteScreen && (
              <p className="text-[8px] font-mono mt-0.5" style={{ color: 'var(--text-faint)' }}>
                {Math.round(remoteScreen.width)}×{Math.round(remoteScreen.height)}
              </p>
            )}
          </div>
          {/* Drag handle hint */}
          <div
            className="absolute top-1 right-1 flex flex-col gap-[1.5px] opacity-40 pointer-events-none"
            style={{ color: '#f97316' }}
          >
            <div className="flex gap-[1.5px]"><span className="w-[2px] h-[2px] rounded-full bg-current"/><span className="w-[2px] h-[2px] rounded-full bg-current"/></div>
            <div className="flex gap-[1.5px]"><span className="w-[2px] h-[2px] rounded-full bg-current"/><span className="w-[2px] h-[2px] rounded-full bg-current"/></div>
          </div>
        </motion.div>
      )}

      {/* Empty state when no peer */}
      {!remoteName && (
        <div className="absolute inset-0 flex items-end justify-center pb-3 pointer-events-none">
          <p className="text-[10px]" style={{ color: 'var(--text-faint)' }}>
            Connect to a device to arrange screens
          </p>
        </div>
      )}
    </div>
  );
};

/* ── Directional arrow connector ── */

export const Layout = () => {
  const { peers, status, settings, setSettings } = useStore();
  const [uptimeS, setUptimeS] = useState(0);
  const startRef = useRef<number>(Date.now());

  const connectedPeer = peers.find((p) => p.id === status?.connected_peer);
  const relaying = status?.relaying ?? false;
  const isControlled = status?.is_controlled ?? false;
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
            title="Screen Arrangement"
            icon={
              <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M4 5a1 1 0 011-1h4a1 1 0 010 2H5a1 1 0 01-1-1zm0 8a1 1 0 011-1h4a1 1 0 010 2H5a1 1 0 01-1-1zm6-8a1 1 0 011-1h8a1 1 0 010 2h-8a1 1 0 01-1-1zm0 8a1 1 0 011-1h8a1 1 0 010 2h-8a1 1 0 01-1-1z" />
              </svg>
            }
          >
            <p className="text-[12px] mb-3 leading-relaxed" style={{ color: 'var(--text-muted)' }}>
              Drag <em>{connectedPeer?.name ?? 'the other computer'}</em> next to your screen.
              The edge where you drop it is where your cursor will switch.
            </p>

            <ArrangementCanvas
              localName={status?.device_name ?? 'You'}
              localScreen={status?.local_screen}
              remoteName={connectedPeer?.name}
              remoteScreen={status?.remote_screen ?? undefined}
              edge={edge}
              onEdge={setEdge}
            />

            {/* Current selection label */}
            <p className="text-center text-xs mt-3" style={{ color: 'var(--text-muted)' }}>
              Cursor switches on the{' '}
              <span style={{ color: 'var(--accent-primary)', fontWeight: 600 }}>
                {edge}
              </span>{' '}
              edge
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
