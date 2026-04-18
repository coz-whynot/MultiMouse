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
          initial={{ opacity: 0, y: -8 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: -8 }}
          className="mx-3 mb-2 rounded-2xl p-3 border"
          style={{
            background: relaying
              ? 'linear-gradient(135deg, rgba(99,102,241,0.15) 0%, rgba(168,85,247,0.15) 100%)'
              : 'rgba(255,255,255,0.03)',
            borderColor: relaying ? 'rgba(99,102,241,0.3)' : 'rgba(255,255,255,0.07)',
          }}
        >
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2.5">
              <motion.div
                animate={relaying ? { scale: [1, 1.2, 1] } : {}}
                transition={{ repeat: Infinity, duration: 1.5 }}
                className={`w-2 h-2 rounded-full ${relaying ? 'bg-accent-400' : 'bg-emerald-400'}`}
              />
              <div>
                <p className="text-xs font-semibold text-white">
                  {relaying ? `Controlling ${connectedPeer.name}` : `Linked with ${connectedPeer.name}`}
                </p>
                <p className="text-[10px] text-white/40 mt-0.5">
                  {relaying ? 'Mouse & keyboard forwarding' : 'Move cursor to edge to switch'}
                </p>
              </div>
            </div>
            <div className="flex gap-1.5">
              {relaying && (
                <button
                  onClick={handleRelease}
                  className="text-[10px] px-2 py-1 rounded-lg bg-white/5 hover:bg-white/10
                    text-white/60 hover:text-white transition-all"
                >
                  Release
                </button>
              )}
              <button
                onClick={handleDisconnect}
                className="text-[10px] px-2 py-1 rounded-lg bg-red-500/10 hover:bg-red-500/20
                  text-red-400 hover:text-red-300 transition-all"
              >
                Disconnect
              </button>
            </div>
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  );
};
