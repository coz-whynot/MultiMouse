import { useState, useEffect, useRef } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { useStore } from '../store/useStore';

type Edge = 'left' | 'right' | 'top' | 'bottom';

const EDGE_LABELS: Record<Edge, string> = {
  left: '← Left',
  right: 'Right →',
  top: '↑ Top',
  bottom: '↓ Bottom',
};

function formatUptime(s: number) {
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  const sec = s % 60;
  return `${m}m ${sec < 10 ? '0' : ''}${sec}s`;
}

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
    const t = setInterval(() => setUptimeS(Math.floor((Date.now() - startRef.current) / 1000)), 1000);
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

  const takeControl = async () => {
    try { await invoke('take_control'); } catch (e) { console.error(e); }
  };

  const releaseControl = async () => {
    try { await invoke('release_cursor'); } catch (e) { console.error(e); }
  };

  return (
    <div className="flex flex-col flex-1 overflow-y-auto px-3 py-3 gap-3 pb-4">

      {/* ── Monitor Arrangement ── */}
      <Section title="Monitor Arrangement">
        <div className="flex flex-col items-center gap-2 py-1">
          {/* Top */}
          <EdgeBtn edge="top" current={edge} peer={connectedPeer?.name} onSet={setEdge} />

          <div className="flex items-center gap-3">
            {/* Left */}
            <EdgeBtn edge="left" current={edge} peer={connectedPeer?.name} onSet={setEdge} />

            {/* Your monitor block */}
            <div
              className="w-32 h-20 rounded-2xl flex flex-col items-center justify-center gap-1"
              style={{
                background: 'linear-gradient(135deg, rgba(99,102,241,0.25), rgba(168,85,247,0.18))',
                border: '2px solid rgba(99,102,241,0.5)',
                boxShadow: '0 4px 20px rgba(99,102,241,0.2)',
              }}
            >
              <svg className="w-5 h-5 text-white/60" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5}
                  d="M9.75 17L9 20l-1 1h8l-1-1-.75-3M3 13h18M5 17h14a2 2 0 002-2V5a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z" />
              </svg>
              <span className="text-xs font-bold text-white/70">You</span>
            </div>

            {/* Right */}
            <EdgeBtn edge="right" current={edge} peer={connectedPeer?.name} onSet={setEdge} />
          </div>

          {/* Bottom */}
          <EdgeBtn edge="bottom" current={edge} peer={connectedPeer?.name} onSet={setEdge} />
        </div>

        <p className="text-center text-[11px] mt-2" style={{ color: 'rgba(255,255,255,0.35)' }}>
          Move cursor to the{' '}
          <span style={{ color: '#a78bfa' }}>{edge}</span> edge to switch to{' '}
          {connectedPeer ? <span style={{ color: 'white' }}>{connectedPeer.name}</span> : 'the other device'}
        </p>
      </Section>

      {/* ── Control Direction ── */}
      <Section title="Control">
        {!connectedPeer ? (
          <p className="text-center py-4 text-sm" style={{ color: 'rgba(255,255,255,0.25)' }}>
            Not connected to any device
          </p>
        ) : (
          <div className="space-y-3">
            {/* Flow diagram */}
            <div className="flex items-center justify-center gap-4 py-2">
              <DeviceBlob
                label={status?.device_name?.split(' ')[0] ?? 'You'}
                color="#6366f1"
                active={relaying}
              />

              <div className="flex flex-col items-center gap-0.5 w-14">
                <AnimatePresence mode="wait">
                  {relaying ? (
                    <motion.div key="r" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}
                      className="flex items-center gap-0.5">
                      <motion.span
                        animate={{ x: [0, 5, 0] }}
                        transition={{ repeat: Infinity, duration: 0.8 }}
                        className="text-lg" style={{ color: '#a78bfa' }}>→</motion.span>
                    </motion.div>
                  ) : isControlled ? (
                    <motion.div key="c" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}
                      className="flex items-center gap-0.5">
                      <motion.span
                        animate={{ x: [0, -5, 0] }}
                        transition={{ repeat: Infinity, duration: 0.8 }}
                        className="text-lg" style={{ color: '#34d399' }}>←</motion.span>
                    </motion.div>
                  ) : (
                    <motion.span key="i" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}
                      className="text-lg" style={{ color: 'rgba(255,255,255,0.15)' }}>⟷</motion.span>
                  )}
                </AnimatePresence>
                <span className="text-[9px] uppercase tracking-wide" style={{ color: 'rgba(255,255,255,0.25)' }}>
                  {relaying ? 'sending' : isControlled ? 'receiving' : 'idle'}
                </span>
              </div>

              <DeviceBlob
                label={connectedPeer.name.split(' ')[0]}
                color="#f97316"
                active={isControlled}
              />
            </div>

            {/* Status pill */}
            <div className="flex justify-center">
              <div
                className="px-3 py-1 rounded-full text-xs font-semibold"
                style={{
                  background: relaying ? 'rgba(99,102,241,0.15)' : isControlled ? 'rgba(52,211,153,0.12)' : 'rgba(255,255,255,0.05)',
                  color: relaying ? '#a78bfa' : isControlled ? '#34d399' : 'rgba(255,255,255,0.3)',
                }}
              >
                {relaying
                  ? `You → ${connectedPeer.name}`
                  : isControlled
                  ? `${connectedPeer.name} → You`
                  : 'Idle'}
              </div>
            </div>

            {/* Buttons */}
            <div className="flex gap-2 pt-1">
              <button
                onClick={takeControl}
                disabled={relaying}
                className="flex-1 py-2.5 rounded-xl text-xs font-bold transition-all active:scale-95"
                style={{
                  background: relaying ? 'rgba(99,102,241,0.3)' : 'rgba(99,102,241,0.18)',
                  border: `1.5px solid ${relaying ? 'rgba(99,102,241,0.6)' : 'rgba(99,102,241,0.35)'}`,
                  color: relaying ? '#a78bfa' : 'rgba(167,139,250,0.75)',
                  cursor: relaying ? 'default' : 'pointer',
                }}
              >
                {relaying ? '● Controlling' : 'Take Control'}
              </button>
              <button
                onClick={releaseControl}
                disabled={!relaying && !isControlled}
                className="flex-1 py-2.5 rounded-xl text-xs font-bold transition-all active:scale-95 disabled:opacity-30"
                style={{
                  background: 'rgba(255,255,255,0.05)',
                  border: '1.5px solid rgba(255,255,255,0.1)',
                  color: 'rgba(255,255,255,0.55)',
                }}
              >
                Release
              </button>
            </div>
          </div>
        )}
      </Section>

      {/* ── Connection Stats ── */}
      {connectedPeer && (
        <Section title="Connection">
          <div className="grid grid-cols-3 gap-2">
            <StatTile
              label="Ping"
              value={connectedPeer.ping_ms != null ? `${connectedPeer.ping_ms}ms` : '—'}
              good={connectedPeer.ping_ms != null && connectedPeer.ping_ms < 30}
              warn={connectedPeer.ping_ms != null && connectedPeer.ping_ms >= 30 && connectedPeer.ping_ms < 80}
            />
            <StatTile label="Uptime" value={formatUptime(uptimeS)} />
            <StatTile
              label="Mode"
              value={relaying ? 'Control' : isControlled ? 'Receiver' : 'Idle'}
              good={relaying || isControlled}
            />
          </div>
        </Section>
      )}

      {/* ── File Transfer hint ── */}
      <Section title="File Transfer">
        <div
          className="rounded-xl p-3 flex items-center gap-3"
          style={{ background: 'rgba(255,255,255,0.03)', border: '1px dashed rgba(255,255,255,0.1)' }}
        >
          <svg className="w-5 h-5 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="rgba(255,255,255,0.3)">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5}
              d="M7 16a4 4 0 01-.88-7.903A5 5 0 1115.9 6L16 6a5 5 0 011 9.9M15 13l-3-3m0 0l-3 3m3-3v12" />
          </svg>
          <p className="text-xs" style={{ color: 'rgba(255,255,255,0.35)' }}>
            {connectedPeer
              ? `Drop files anywhere on this window to send to ${connectedPeer.name}`
              : 'Connect to a device to enable file transfer'}
          </p>
        </div>
      </Section>

    </div>
  );
};

/* ── Helpers ── */

const Section = ({ title, children }: { title: string; children: React.ReactNode }) => (
  <div
    className="rounded-2xl p-4"
    style={{ background: 'rgba(255,255,255,0.035)', border: '1px solid rgba(255,255,255,0.07)' }}
  >
    <p className="text-[10px] font-bold uppercase tracking-widest mb-3" style={{ color: 'rgba(255,255,255,0.35)' }}>
      {title}
    </p>
    {children}
  </div>
);

const EdgeBtn = ({
  edge, current, peer, onSet,
}: {
  edge: Edge; current: Edge; peer?: string; onSet: (e: Edge) => void;
}) => {
  const active = current === edge;
  const isH = edge === 'left' || edge === 'right';
  return (
    <button
      onClick={() => onSet(edge)}
      className="flex flex-col items-center gap-1 transition-all active:scale-90"
      style={{ minWidth: isH ? 52 : 80 }}
    >
      {active && peer && (
        <motion.div
          initial={{ opacity: 0, scale: 0.8 }}
          animate={{ opacity: 1, scale: 1 }}
          className="text-[9px] font-semibold truncate max-w-[80px] text-center"
          style={{ color: '#a78bfa' }}
        >
          {peer}
        </motion.div>
      )}
      <div
        className="rounded-xl flex items-center justify-center text-sm font-bold"
        style={{
          width: isH ? 44 : 72,
          height: isH ? 32 : 28,
          background: active ? 'rgba(99,102,241,0.35)' : 'rgba(255,255,255,0.06)',
          border: `1.5px solid ${active ? 'rgba(99,102,241,0.7)' : 'rgba(255,255,255,0.1)'}`,
          color: active ? '#a78bfa' : 'rgba(255,255,255,0.35)',
          boxShadow: active ? '0 0 12px rgba(99,102,241,0.3)' : 'none',
        }}
      >
        {EDGE_LABELS[edge]}
      </div>
    </button>
  );
};

const DeviceBlob = ({ label, color, active }: { label: string; color: string; active: boolean }) => (
  <div className="flex flex-col items-center gap-1.5">
    <motion.div
      animate={active ? { boxShadow: [`0 0 0 0px ${color}40`, `0 0 0 8px ${color}00`] } : {}}
      transition={{ repeat: Infinity, duration: 1.5 }}
      className="w-12 h-12 rounded-2xl flex items-center justify-center text-base font-black text-white"
      style={{
        background: `linear-gradient(135deg, ${color}, ${color}99)`,
        boxShadow: active ? `0 4px 16px ${color}60` : `0 4px 12px ${color}30`,
        opacity: active ? 1 : 0.65,
      }}
    >
      {label.charAt(0).toUpperCase()}
    </motion.div>
    <span className="text-[9px] font-semibold text-white/40 truncate max-w-14 text-center">{label}</span>
  </div>
);

const StatTile = ({
  label, value, good, warn,
}: {
  label: string; value: string; good?: boolean; warn?: boolean;
}) => (
  <div
    className="flex flex-col items-center gap-0.5 rounded-xl py-2.5 px-2"
    style={{ background: 'rgba(255,255,255,0.04)' }}
  >
    <span className="text-[9px] uppercase tracking-wide" style={{ color: 'rgba(255,255,255,0.3)' }}>
      {label}
    </span>
    <span
      className="text-sm font-bold"
      style={{ color: good ? '#34d399' : warn ? '#fbbf24' : 'rgba(255,255,255,0.7)' }}
    >
      {value}
    </span>
  </div>
);
