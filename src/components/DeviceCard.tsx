import { useState } from 'react';
import { motion } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { PeerInfo } from '../types';
import { useStore } from '../store/useStore';
import { cmpSemver } from '../App';

// TODO: display a "last seen" timestamp for offline/paired-but-absent peers.
// Planned: backend needs to track and expose a `last_seen` field on PeerInfo/KnownDevice
// before we can render it here. Not implemented in this pass.

interface Props {
  peer: PeerInfo;
  onConnect: (id: string) => void;
  isConnecting: boolean;
}

const PALETTES = [
  { a: '#f472b6', b: '#ec4899' },
  { a: '#fb923c', b: '#f97316' },
  { a: '#34d399', b: '#059669' },
  { a: '#38bdf8', b: '#0ea5e9' },
  { a: '#a78bfa', b: '#7c3aed' },
  { a: '#e879f9', b: '#c026d3' },
  { a: '#fbbf24', b: '#d97706' },
  { a: '#4ade80', b: '#16a34a' },
];

function pickPalette(name: string) {
  const idx = name.split('').reduce((s, c) => s + c.charCodeAt(0), 0) % PALETTES.length;
  return PALETTES[idx];
}

const PingBadge = ({ ms }: { ms?: number }) => {
  if (ms == null) return null;
  const color = ms < 20 ? '#4ade80' : ms < 60 ? '#fbbf24' : '#f87171';
  const label = ms < 20 ? 'Excellent' : ms < 60 ? 'Good' : 'Slow';
  return (
    <span
      className="text-[10px] font-semibold px-1.5 py-0.5 rounded-full"
      style={{ color, background: color + '18' }}
    >
      {ms}ms · {label}
    </span>
  );
};

const StatusText = ({
  peer,
  isConnecting,
}: {
  peer: PeerInfo;
  isConnecting: boolean;
}) => {
  if (peer.status === 'connected') return <span style={{ color: 'var(--accent-primary)' }}>Connected</span>;
  if (isConnecting || peer.status === 'pairing')
    return <span style={{ color: 'var(--warn)' }}>Awaiting approval…</span>;
  if (peer.status === 'error') return <span style={{ color: 'var(--danger)' }}>Failed to connect</span>;
  return <span style={{ color: 'var(--text-muted)' }}>Tap to connect</span>;
};

export const DeviceCard = ({ peer, onConnect, isConnecting }: Props) => {
  const isConnected = peer.status === 'connected';
  const pal = pickPalette(peer.name);
  const initial = peer.name.charAt(0).toUpperCase();
  const clickable = !isConnected && !isConnecting;
  const ownVersion = useStore((s) => s.appVersion);
  const setErrorMsg = useStore((s) => s.setErrorMsg);
  const peerVersion = peer.app_version ?? null;
  const peerIsNewer =
    ownVersion != null && peerVersion != null && cmpSemver(peerVersion, ownVersion) > 0;
  const [updating, setUpdating] = useState(false);

  const runUpdate = async (e: React.MouseEvent) => {
    e.stopPropagation();
    if (updating) return;
    setUpdating(true);
    try {
      const { check } = await import('@tauri-apps/plugin-updater');
      const { relaunch } = await import('@tauri-apps/plugin-process');
      const result = await check();
      if (result?.available) {
        await result.downloadAndInstall();
        await relaunch();
      } else {
        setErrorMsg('No release available yet — try again in a minute.');
      }
    } catch (err) {
      setErrorMsg(`Update failed: ${String(err)}`);
    } finally {
      setUpdating(false);
    }
  };

  return (
    <motion.div
      layout
      initial={{ opacity: 0, y: 14 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, y: -8, scale: 0.95 }}
      whileTap={clickable ? { scale: 0.975 } : undefined}
      transition={{ type: 'spring', stiffness: 380, damping: 28 }}
      className="relative flex items-center gap-3.5 rounded-2xl px-4 py-3.5"
      style={{
        background: isConnected
          ? 'linear-gradient(135deg, rgba(99,102,241,0.18) 0%, rgba(168,85,247,0.13) 100%)'
          : `linear-gradient(135deg, ${pal.a}14 0%, ${pal.b}09 100%)`,
        border: `1.5px solid ${isConnected ? 'rgba(99,102,241,0.35)' : pal.a + '30'}`,
        boxShadow: isConnected
          ? '0 4px 20px rgba(99,102,241,0.1)'
          : `0 2px 12px ${pal.a}10`,
        cursor: clickable ? 'pointer' : 'default',
      }}
      onClick={() => clickable && onConnect(peer.id)}
    >
      {/* Avatar */}
      <div className="relative flex-shrink-0">
        <div
          className="w-12 h-12 rounded-xl flex items-center justify-center text-xl font-black text-white"
          style={{
            background: `linear-gradient(135deg, ${pal.a}, ${pal.b})`,
            boxShadow: `0 4px 16px ${pal.a}50`,
          }}
        >
          {initial}
        </div>

        {/* Pulsing ring when connected */}
        {isConnected && (
          <motion.div
            animate={{ scale: [1, 1.22, 1], opacity: [0.5, 0, 0.5] }}
            transition={{ repeat: Infinity, duration: 2.2 }}
            className="absolute inset-0 rounded-xl pointer-events-none"
            style={{ border: '2px solid #818cf8' }}
          />
        )}

        {/* Status dot */}
        <div
          className="absolute -bottom-1 -right-1 w-3.5 h-3.5 rounded-full border-2"
          style={{
            background: isConnected
              ? '#818cf8'
              : peer.status === 'pairing' || isConnecting
              ? '#fbbf24'
              : peer.status === 'error'
              ? '#ef4444'
              : '#10b981',
            borderColor: 'var(--bg-base)',
          }}
        />
      </div>

      {/* Info */}
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2 mb-1">
          <p className="text-sm font-bold truncate leading-tight" style={{ color: 'var(--text-primary)' }}>{peer.name}</p>
          {peer.is_known && !isConnected && (
            <span
              className="text-[9px] font-bold px-1.5 py-0.5 rounded-full uppercase tracking-wide flex-shrink-0"
              style={{ background: pal.a + '22', color: pal.a, border: `1px solid ${pal.a}30` }}
            >
              Paired
            </span>
          )}
          {peerVersion && (
            <span
              className="text-[9px] font-semibold px-1.5 py-0.5 rounded-full flex-shrink-0"
              title={peerIsNewer ? 'Peer is on a newer build than this device' : 'Peer app version'}
              style={{
                background: peerIsNewer ? 'rgba(251,191,36,0.15)' : 'rgba(148,163,184,0.14)',
                color: peerIsNewer ? '#fbbf24' : 'var(--text-muted)',
                border: `1px solid ${peerIsNewer ? 'rgba(251,191,36,0.35)' : 'rgba(148,163,184,0.22)'}`,
              }}
            >
              v{peerVersion}
            </span>
          )}
        </div>
        <div className="flex items-center gap-2 flex-wrap">
          <span className="text-xs">
            <StatusText peer={peer} isConnecting={isConnecting} />
          </span>
          {isConnected && peer.ping_ms != null && <PingBadge ms={peer.ping_ms} />}
        </div>
      </div>

      {/* Right action */}
      {isConnected ? (
        <div className="flex-shrink-0 flex items-center gap-1.5" onClick={(e) => e.stopPropagation()}>
          <motion.div
            className="w-2 h-2 rounded-full"
            animate={{ scale: [1, 1.6, 1], opacity: [1, 0.4, 1] }}
            transition={{ repeat: Infinity, duration: 1.8 }}
            style={{ background: 'var(--accent-secondary)' }}
          />
          <button
            onClick={(e) => {
              e.stopPropagation();
              invoke('disconnect').catch(console.error);
            }}
            className="text-[10px] font-bold px-2 py-1 rounded-lg transition-all active:scale-95"
            style={{
              background: 'rgba(239,68,68,0.12)',
              color: '#f87171',
              border: '1px solid rgba(239,68,68,0.22)',
            }}
            title="End session with this device"
          >
            End
          </button>
        </div>
      ) : peerIsNewer && !isConnected ? (
        <div className="flex-shrink-0" onClick={(e) => e.stopPropagation()}>
          <button
            onClick={runUpdate}
            disabled={updating}
            className="text-[10px] font-bold px-2.5 py-1.5 rounded-lg transition-all active:scale-95 flex items-center gap-1 disabled:opacity-50"
            style={{
              background: 'rgba(251,191,36,0.18)',
              color: '#fbbf24',
              border: '1px solid rgba(251,191,36,0.35)',
            }}
            title={`Peer is on v${peerVersion}. Tap to update this device.`}
          >
            <svg className="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2.5}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M12 4v12m0 0l-4-4m4 4l4-4M4 20h16" />
            </svg>
            {updating ? 'Updating…' : 'Update'}
          </button>
        </div>
      ) : peer.is_known && peer.status === 'available' ? (
        <div className="flex-shrink-0" onClick={(e) => e.stopPropagation()}>
          <button
            onClick={(e) => {
              e.stopPropagation();
              onConnect(peer.id);
            }}
            className="text-[10px] font-bold px-2.5 py-1.5 rounded-lg transition-all active:scale-95 flex items-center gap-1"
            style={{
              background: pal.a + '22',
              color: pal.a,
              border: `1px solid ${pal.a}40`,
            }}
            title="Reconnect to this paired device"
          >
            <svg className="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2.5}>
              <path strokeLinecap="round" strokeLinejoin="round"
                d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
            </svg>
            Reconnect
          </button>
        </div>
      ) : isConnecting ? (
        <motion.div
          animate={{ rotate: 360 }}
          transition={{ repeat: Infinity, duration: 0.85, ease: 'linear' }}
          className="flex-shrink-0 w-5 h-5 rounded-full border-2"
          style={{ borderColor: pal.a + '30', borderTopColor: pal.a }}
        />
      ) : (
        <div
          className="flex-shrink-0 w-8 h-8 rounded-xl flex items-center justify-center"
          style={{ background: pal.a + '18', border: `1px solid ${pal.a}28` }}
        >
          <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke={pal.a} strokeWidth={2.5}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M9 5l7 7-7 7" />
          </svg>
        </div>
      )}
    </motion.div>
  );
};
