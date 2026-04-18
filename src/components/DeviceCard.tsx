import { motion } from 'framer-motion';
import { PeerInfo } from '../types';

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
  if (peer.status === 'connected') return <span style={{ color: '#a78bfa' }}>Connected</span>;
  if (isConnecting || peer.status === 'pairing')
    return <span style={{ color: '#fbbf24' }}>Awaiting approval…</span>;
  if (peer.status === 'error') return <span style={{ color: '#f87171' }}>Failed to connect</span>;
  return <span style={{ color: 'rgba(255,255,255,0.4)' }}>Tap to connect</span>;
};

export const DeviceCard = ({ peer, onConnect, isConnecting }: Props) => {
  const isConnected = peer.status === 'connected';
  const pal = pickPalette(peer.name);
  const initial = peer.name.charAt(0).toUpperCase();
  const clickable = !isConnected && !isConnecting;

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
            borderColor: '#100c2a',
          }}
        />
      </div>

      {/* Info */}
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2 mb-1">
          <p className="text-sm font-bold text-white truncate leading-tight">{peer.name}</p>
          {peer.is_known && !isConnected && (
            <span
              className="text-[9px] font-bold px-1.5 py-0.5 rounded-full uppercase tracking-wide flex-shrink-0"
              style={{ background: pal.a + '22', color: pal.a, border: `1px solid ${pal.a}30` }}
            >
              Paired
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
        <div className="flex-shrink-0 flex items-center gap-1.5">
          <motion.div
            className="w-2 h-2 rounded-full"
            animate={{ scale: [1, 1.6, 1], opacity: [1, 0.4, 1] }}
            transition={{ repeat: Infinity, duration: 1.8 }}
            style={{ background: '#818cf8' }}
          />
          <span className="text-xs font-semibold" style={{ color: '#a78bfa' }}>Live</span>
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
