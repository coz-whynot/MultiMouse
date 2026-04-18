import { motion } from 'framer-motion';
import { PeerInfo } from '../types';

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
  return <span className="text-xs font-mono" style={{ color }}>{ms}ms</span>;
};

export const DeviceCard = ({ peer, onConnect, isConnecting }: Props) => {
  const isConnected = peer.status === 'connected';
  const pal = pickPalette(peer.name);
  const initial = peer.name.charAt(0).toUpperCase();

  return (
    <motion.div
      layout
      initial={{ opacity: 0, y: 14 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, y: -8, scale: 0.95 }}
      whileTap={!isConnected ? { scale: 0.975 } : undefined}
      transition={{ type: 'spring', stiffness: 380, damping: 28 }}
      className="relative flex items-center gap-4 rounded-2xl px-4 py-3.5 cursor-pointer"
      style={{
        background: isConnected
          ? 'linear-gradient(135deg, rgba(99,102,241,0.2) 0%, rgba(168,85,247,0.15) 100%)'
          : `linear-gradient(135deg, ${pal.a}18 0%, ${pal.b}0c 100%)`,
        border: `1.5px solid ${isConnected ? 'rgba(99,102,241,0.38)' : pal.a + '35'}`,
        boxShadow: `0 4px 16px ${isConnected ? 'rgba(99,102,241,0.12)' : pal.a + '12'}`,
      }}
      onClick={() => !isConnected && onConnect(peer.id)}
    >
      {/* Circular avatar */}
      <div className="relative flex-shrink-0">
        <div
          className="w-14 h-14 rounded-2xl flex items-center justify-center"
          style={{
            background: `linear-gradient(135deg, ${pal.a}, ${pal.b})`,
            boxShadow: `0 6px 22px ${pal.a}55`,
          }}
        >
          <span className="text-2xl font-black text-white">{initial}</span>
        </div>

        {/* Status ring when connected */}
        {isConnected && (
          <motion.div
            animate={{ scale: [1, 1.18, 1], opacity: [0.6, 0.2, 0.6] }}
            transition={{ repeat: Infinity, duration: 2 }}
            className="absolute inset-0 rounded-2xl"
            style={{ boxShadow: `0 0 0 3px #818cf8` }}
          />
        )}

        {/* Status dot */}
        <div
          className="absolute -bottom-1 -right-1 w-4 h-4 rounded-full border-2"
          style={{
            background: isConnected ? '#6366f1' : peer.status === 'pairing' ? '#fbbf24' : peer.status === 'error' ? '#ef4444' : '#10b981',
            borderColor: '#100c2a',
          }}
        />
      </div>

      {/* Info */}
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2 mb-0.5">
          <p className="text-base font-bold text-white truncate leading-tight">{peer.name}</p>
          {peer.is_known && !isConnected && (
            <span
              className="text-[10px] font-bold px-1.5 py-0.5 rounded-full uppercase tracking-wide flex-shrink-0"
              style={{ background: pal.a + '28', color: pal.a }}
            >
              Paired
            </span>
          )}
        </div>
        <div className="flex items-center gap-2">
          <span className="text-sm" style={{ color: 'rgba(255,255,255,0.48)' }}>
            {isConnected ? 'Connected' : peer.status === 'pairing' ? 'Pairing…' : peer.status === 'error' ? 'Error' : 'Tap to connect'}
          </span>
          <PingBadge ms={peer.ping_ms} />
        </div>
      </div>

      {/* Right action */}
      {isConnected ? (
        <div className="flex-shrink-0 flex items-center gap-1.5">
          <motion.div
            className="w-2.5 h-2.5 rounded-full"
            animate={{ scale: [1, 1.5, 1] }}
            transition={{ repeat: Infinity, duration: 1.6 }}
            style={{ background: '#818cf8' }}
          />
          <span className="text-xs font-semibold" style={{ color: '#818cf8' }}>Active</span>
        </div>
      ) : isConnecting ? (
        <motion.div
          animate={{ rotate: 360 }}
          transition={{ repeat: Infinity, duration: 0.85, ease: 'linear' }}
          className="flex-shrink-0 w-5 h-5 rounded-full border-2"
          style={{ borderColor: pal.a + '35', borderTopColor: pal.a }}
        />
      ) : (
        <div
          className="flex-shrink-0 w-9 h-9 rounded-xl flex items-center justify-center"
          style={{ background: pal.a + '22' }}
        >
          <svg className="w-4.5 h-4.5" fill="none" viewBox="0 0 24 24" stroke={pal.a}>
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2.5} d="M9 5l7 7-7 7" />
          </svg>
        </div>
      )}
    </motion.div>
  );
};
