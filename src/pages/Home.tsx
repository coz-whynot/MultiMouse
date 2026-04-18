import { useState, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useStore } from '../store/useStore';
import { DeviceCard } from '../components/DeviceCard';
import { PinEntry } from '../components/PinModal';
import { ConnectionBanner } from '../components/ConnectionBanner';
import { InternetModal } from '../components/InternetModal';

export const Home = () => {
  const { peers, status, connectingTo, setConnectingTo } = useStore();
  const [showInternet, setShowInternet] = useState(false);
  const [accessibilityNeeded, setAccessibilityNeeded] = useState(false);

  const connectedPeer = peers.find((p) => p.id === status?.connected_peer);

  useEffect(() => {
    const unsub = listen('accessibility-needed', () => setAccessibilityNeeded(true));
    return () => { unsub.then((fn) => fn()); };
  }, []);

  const handleConnect = async (peerId: string) => {
    const peer = peers.find((p) => p.id === peerId);
    if (peer?.is_known) {
      // Auto-reconnect — no PIN needed
      try {
        await invoke('connect_to_device', { peerId, pin: '' });
      } catch (e) {
        console.error(e);
      }
    } else {
      setConnectingTo(peerId);
    }
  };

  const handlePinSubmit = async (pin: string) => {
    if (!connectingTo) return;
    try {
      await invoke('connect_to_device', { peerId: connectingTo, pin });
    } catch (e) {
      console.error(e);
    } finally {
      setConnectingTo(null);
    }
  };

  const isEmpty = peers.length === 0;

  return (
    <div className="flex flex-col flex-1 overflow-hidden">
      {/* Accessibility warning */}
      <AnimatePresence>
        {accessibilityNeeded && (
          <motion.div
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: 'auto' }}
            exit={{ opacity: 0, height: 0 }}
            className="mx-3 mt-1 mb-0"
          >
            <div className="rounded-xl p-3 bg-amber-500/10 border border-amber-500/20 flex gap-2">
              <svg className="w-4 h-4 text-amber-400 flex-shrink-0 mt-0.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2}
                  d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
              </svg>
              <div>
                <p className="text-xs font-medium text-amber-300">Accessibility permission needed</p>
                <p className="text-[10px] text-amber-400/70 mt-0.5">
                  System Settings → Privacy & Security → Accessibility → enable MultiMouse
                </p>
              </div>
              <button
                onClick={() => setAccessibilityNeeded(false)}
                className="ml-auto text-amber-400/50 hover:text-amber-300 flex-shrink-0"
              >
                <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                </svg>
              </button>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      <ConnectionBanner connectedPeer={connectedPeer} relaying={status?.relaying ?? false} />

      {/* Device list */}
      <div className="flex-1 overflow-y-auto px-3 pb-3">
        <div className="flex items-center justify-between mb-2 px-1">
          <span className="text-xs font-medium text-white/40 uppercase tracking-wider">
            Nearby Devices
          </span>
          {!isEmpty && (
            <span className="text-xs text-white/30">{peers.length} found</span>
          )}
        </div>

        <AnimatePresence>
          {isEmpty ? (
            <motion.div
              key="empty"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              className="flex flex-col items-center justify-center py-14 text-center"
            >
              <motion.div
                animate={{ scale: [1, 1.05, 1] }}
                transition={{ repeat: Infinity, duration: 3, ease: 'easeInOut' }}
                className="w-16 h-16 rounded-2xl bg-white/[0.04] border border-white/[0.07]
                  flex items-center justify-center mb-4"
              >
                <svg className="w-8 h-8 text-white/20" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5}
                    d="M8.111 16.404a5.5 5.5 0 017.778 0M12 20h.01m-7.08-7.071c3.904-3.905 10.236-3.905 14.14 0M1.394 9.393c5.857-5.857 15.355-5.857 21.213 0" />
                </svg>
              </motion.div>
              <p className="text-white/40 text-sm font-medium">Scanning for devices…</p>
              <p className="text-white/20 text-xs mt-1">
                Make sure MultiMouse is open on other devices
              </p>
            </motion.div>
          ) : (
            <div className="space-y-2">
              {peers.map((peer) => (
                <DeviceCard
                  key={peer.id}
                  peer={peer}
                  onConnect={handleConnect}
                  isConnecting={connectingTo === peer.id}
                />
              ))}
            </div>
          )}
        </AnimatePresence>

        {/* Internet connect */}
        <div className="mt-4">
          <div className="flex items-center gap-3 mb-3">
            <div className="h-px flex-1 bg-white/[0.07]" />
            <span className="text-xs text-white/25">or</span>
            <div className="h-px flex-1 bg-white/[0.07]" />
          </div>
          <button
            onClick={() => setShowInternet(true)}
            className="w-full flex items-center justify-center gap-2 py-3 rounded-2xl
              border border-white/10 text-white/50 text-sm
              hover:border-accent-500/40 hover:text-white/70 hover:bg-accent-500/5
              transition-all active:scale-[0.98]"
          >
            <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.8}
                d="M3.055 11H5a2 2 0 012 2v1a2 2 0 002 2 2 2 0 012 2v2.945M8 3.935V5.5A2.5 2.5 0 0010.5 8h.5a2 2 0 012 2 2 2 0 104 0 2 2 0 012-2h1.064M15 20.488V18a2 2 0 012-2h3.064" />
            </svg>
            Connect via Internet
          </button>
        </div>
      </div>

      {/* Footer */}
      <div className="px-4 py-3 border-t border-white/[0.05] flex items-center justify-between">
        <div className="flex items-center gap-2">
          <div className="w-2 h-2 rounded-full bg-emerald-400" />
          <span className="text-xs text-white/40">{status?.device_name ?? 'This Device'}</span>
        </div>
        <span className="text-xs text-white/20">v0.1.0</span>
      </div>

      <AnimatePresence>
        {connectingTo && (
          <PinEntry
            peerName={peers.find((p) => p.id === connectingTo)?.name ?? 'Device'}
            onSubmit={handlePinSubmit}
            onCancel={() => setConnectingTo(null)}
          />
        )}
        {showInternet && (
          <InternetModal onClose={() => setShowInternet(false)} />
        )}
      </AnimatePresence>
    </div>
  );
};
