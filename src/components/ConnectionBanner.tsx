import { motion, AnimatePresence } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { PeerInfo } from '../types';

interface Props {
  connectedPeer?: PeerInfo;
  relaying: boolean;
}

export const ConnectionBanner = ({ connectedPeer, relaying }: Props) => {
  const handleDisconnect = () => invoke('disconnect');
  const handleRelease = () => invoke('release_cursor');

  return (
    <AnimatePresence>
      {connectedPeer && (
        <motion.div
          initial={{ opacity: 0, scale: 0.96, y: -4 }}
          animate={{ opacity: 1, scale: 1, y: 0 }}
          exit={{ opacity: 0, scale: 0.96, y: -4 }}
          transition={{ type: 'spring', stiffness: 400, damping: 30 }}
          className="mx-3 mb-3 rounded-2xl overflow-hidden"
          style={{
            background: relaying
              ? 'linear-gradient(135deg, rgba(99,102,241,0.22) 0%, rgba(168,85,247,0.18) 100%)'
              : 'linear-gradient(135deg, rgba(16,185,129,0.15) 0%, rgba(5,150,105,0.12) 100%)',
            border: `1px solid ${relaying ? 'rgba(99,102,241,0.35)' : 'rgba(16,185,129,0.3)'}`,
            boxShadow: relaying
              ? '0 4px 20px rgba(99,102,241,0.15)'
              : '0 4px 20px rgba(16,185,129,0.1)',
          }}
        >
          <div className="flex items-center gap-3 px-4 py-3">
            {/* Animated status dot */}
            <div className="relative flex-shrink-0">
              <motion.div
                animate={{ scale: [1, 1.6, 1], opacity: [1, 0, 1] }}
                transition={{ repeat: Infinity, duration: 2, ease: 'easeInOut' }}
                className="absolute inset-0 rounded-full"
                style={{ background: relaying ? '#6366f1' : '#10b981' }}
              />
              <div
                className="w-3 h-3 rounded-full relative z-10"
                style={{ background: relaying ? '#818cf8' : '#34d399' }}
              />
            </div>

            {/* Text */}
            <div className="flex-1 min-w-0">
              <p className="text-sm font-semibold text-white truncate">
                {relaying ? `Controlling ${connectedPeer.name}` : `Linked · ${connectedPeer.name}`}
              </p>
              <p className="text-xs text-white/45 mt-0.5">
                {relaying ? 'Mouse & keyboard are forwarding' : 'Move cursor to edge to take control'}
              </p>
            </div>

            {/* Actions */}
            <div className="flex gap-1.5 flex-shrink-0">
              {relaying && (
                <button
                  onClick={handleRelease}
                  className="text-xs px-3 py-1.5 rounded-xl font-medium transition-all
                    text-white/70 hover:text-white"
                  style={{ background: 'rgba(255,255,255,0.08)' }}
                >
                  Release
                </button>
              )}
              <button
                onClick={handleDisconnect}
                className="text-xs px-3 py-1.5 rounded-xl font-medium transition-all
                  text-red-400 hover:text-red-300"
                style={{ background: 'rgba(239,68,68,0.12)' }}
              >
                End
              </button>
            </div>
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  );
};
