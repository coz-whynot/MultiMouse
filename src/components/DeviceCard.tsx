import { motion } from 'framer-motion';
import { PeerInfo } from '../types';

interface Props {
  peer: PeerInfo;
  onConnect: (id: string) => void;
  isConnecting: boolean;
}

const statusColors = {
  available: 'bg-emerald-400',
  connected: 'bg-accent-400',
  pairing: 'bg-amber-400',
  error: 'bg-red-400',
};

const statusLabels = {
  available: 'Available',
  connected: 'Connected',
  pairing: 'Pairing…',
  error: 'Error',
};

const PingBadge = ({ ms }: { ms?: number }) => {
  if (ms == null) return null;
  const color = ms < 20 ? 'text-emerald-400' : ms < 60 ? 'text-amber-400' : 'text-red-400';
  return (
    <span className={`text-xs font-mono ${color}`}>{ms}ms</span>
  );
};

export const DeviceCard = ({ peer, onConnect, isConnecting }: Props) => {
  const isConnected = peer.status === 'connected';

  return (
    <motion.div
      layout
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, y: -8 }}
      className="group relative flex items-center gap-3 rounded-2xl p-4 cursor-pointer
        bg-white/[0.04] border border-white/[0.07]
        hover:bg-white/[0.08] hover:border-white/[0.14]
        transition-all duration-200"
      onClick={() => !isConnected && onConnect(peer.id)}
    >
      {/* Device icon */}
      <div className="relative flex-shrink-0">
        <div className="w-11 h-11 rounded-xl bg-gradient-to-br from-accent-500/20 to-purple-500/20
          border border-accent-500/20 flex items-center justify-center">
          <svg className="w-5 h-5 text-accent-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5}
              d="M9.75 17L9 20l-1 1h8l-1-1-.75-3M3 13h18M5 17h14a2 2 0 002-2V5a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z" />
          </svg>
        </div>
        <span className={`absolute -bottom-0.5 -right-0.5 w-3 h-3 rounded-full border-2 border-surface-900 ${statusColors[peer.status]}`} />
      </div>

      {/* Info */}
      <div className="flex-1 min-w-0">
        <p className="text-sm font-semibold text-white truncate">{peer.name}</p>
        <div className="flex items-center gap-2 mt-0.5">
          <span className="text-xs text-white/40">{statusLabels[peer.status]}</span>
          {peer.is_known && peer.status === 'available' && (
            <span className="text-[10px] text-accent-400/70 font-medium">Paired</span>
          )}
          <PingBadge ms={peer.ping_ms} />
        </div>
      </div>

      {/* Action */}
      {isConnected ? (
        <div className="flex items-center gap-1.5 text-accent-400">
          <motion.span
            animate={{ scale: [1, 1.3, 1] }}
            transition={{ repeat: Infinity, duration: 2 }}
            className="w-2 h-2 rounded-full bg-accent-400"
          />
          <span className="text-xs font-medium">Active</span>
        </div>
      ) : (
        <motion.div
          initial={{ opacity: 0 }}
          whileHover={{ opacity: 1 }}
          className="opacity-0 group-hover:opacity-100 transition-opacity"
        >
          {isConnecting ? (
            <motion.div
              animate={{ rotate: 360 }}
              transition={{ repeat: Infinity, duration: 1, ease: 'linear' }}
              className="w-5 h-5 border-2 border-accent-400/30 border-t-accent-400 rounded-full"
            />
          ) : (
            <svg className="w-5 h-5 text-accent-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
            </svg>
          )}
        </motion.div>
      )}
    </motion.div>
  );
};
