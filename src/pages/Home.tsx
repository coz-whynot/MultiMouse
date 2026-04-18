import { useState, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useStore } from '../store/useStore';
import { DeviceCard } from '../components/DeviceCard';
import { PinEntry } from '../components/PinModal';
import { ConnectionBanner } from '../components/ConnectionBanner';
import { InternetModal } from '../components/InternetModal';

const RadarRing = ({ delay, size }: { delay: number; size: number }) => (
  <motion.div
    className="absolute rounded-full"
    style={{
      width: size,
      height: size,
      border: '1.5px solid rgba(99,102,241,0.45)',
      top: '50%',
      left: '50%',
      marginLeft: -size / 2,
      marginTop: -size / 2,
    }}
    animate={{ scale: [0.15, 1.4], opacity: [0.8, 0] }}
    transition={{ repeat: Infinity, duration: 3, delay, ease: 'easeOut' }}
  />
);

export const Home = () => {
  const { peers, status, connectingTo, setConnectingTo } = useStore();
  const [showInternet, setShowInternet] = useState(false);
  const [accessibilityNeeded, setAccessibilityNeeded] = useState(false);

  const connectedPeer = peers.find((p) => p.id === status?.connected_peer);
  const deviceName = status?.device_name ?? 'This Device';
  const deviceInitial = deviceName.charAt(0).toUpperCase();
  const isEmpty = peers.length === 0;

  useEffect(() => {
    const unsub = listen('accessibility-needed', () => setAccessibilityNeeded(true));
    return () => { unsub.then((fn) => fn()); };
  }, []);

  const handleConnect = async (peerId: string) => {
    const peer = peers.find((p) => p.id === peerId);
    if (peer?.is_known) {
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

  return (
    <div className="flex flex-col flex-1 overflow-hidden">
      {/* Accessibility warning */}
      <AnimatePresence>
        {accessibilityNeeded && (
          <motion.div
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: 'auto' }}
            exit={{ opacity: 0, height: 0 }}
            className="mx-3 mt-1"
          >
            <div className="rounded-xl px-3 py-2.5 flex gap-2 mb-2"
              style={{ background: 'rgba(245,158,11,0.1)', border: '1px solid rgba(245,158,11,0.22)' }}>
              <svg className="w-4 h-4 text-amber-400 flex-shrink-0 mt-0.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2}
                  d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
              </svg>
              <div className="flex-1">
                <p className="text-xs font-semibold text-amber-300">Accessibility permission needed</p>
                <p className="text-[10px] text-amber-400/70 mt-0.5">
                  System Settings → Privacy & Security → Accessibility → enable MultiMouse
                </p>
              </div>
              <button onClick={() => setAccessibilityNeeded(false)} className="text-amber-400/50 hover:text-amber-300">
                <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                </svg>
              </button>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      <ConnectionBanner connectedPeer={connectedPeer} relaying={status?.relaying ?? false} />

      {/* ── This Device Hero ── */}
      <div className="mx-3 mb-3 rounded-3xl px-4 py-4 flex items-center gap-4 relative overflow-hidden"
        style={{
          background: 'linear-gradient(135deg, rgba(99,102,241,0.22) 0%, rgba(168,85,247,0.16) 100%)',
          border: '1.5px solid rgba(99,102,241,0.3)',
          boxShadow: '0 8px 28px rgba(99,102,241,0.18)',
        }}
      >
        {/* Decorative glow blob */}
        <div className="absolute right-0 top-0 w-32 h-32 rounded-full pointer-events-none"
          style={{ background: 'radial-gradient(circle, rgba(168,85,247,0.25) 0%, transparent 70%)', transform: 'translate(30%, -30%)' }} />

        {/* Avatar */}
        <div className="relative flex-shrink-0">
          <motion.div
            animate={{ boxShadow: ['0 0 0 0px rgba(99,102,241,0.5)', '0 0 0 8px rgba(99,102,241,0)', '0 0 0 0px rgba(99,102,241,0)'] }}
            transition={{ repeat: Infinity, duration: 3, ease: 'easeInOut' }}
            className="w-16 h-16 rounded-2xl flex items-center justify-center"
            style={{ background: 'linear-gradient(135deg, #6366f1, #a855f7)', boxShadow: '0 8px 24px rgba(99,102,241,0.5)' }}
          >
            <span className="text-3xl font-black text-white">{deviceInitial}</span>
          </motion.div>
          <div className="absolute -bottom-1 -right-1 w-4 h-4 rounded-full border-2"
            style={{ background: '#10b981', borderColor: '#0f0c29' }} />
        </div>

        {/* Info */}
        <div className="flex-1 min-w-0">
          <p className="text-[10px] font-semibold uppercase tracking-widest mb-0.5"
            style={{ color: 'rgba(167,139,250,0.7)' }}>This Device</p>
          <p className="text-lg font-black text-white leading-tight truncate">{deviceName}</p>
          <p className="text-xs mt-0.5" style={{ color: 'rgba(255,255,255,0.45)' }}>
            {connectedPeer ? `Connected to ${connectedPeer.name}` : 'Ready to share'}
          </p>
        </div>
      </div>

      {/* ── Scrollable content ── */}
      <div className="flex-1 overflow-y-auto px-3 pb-3">
        <AnimatePresence mode="wait">
          {isEmpty ? (
            /* Radar empty state */
            <motion.div
              key="empty"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              className="flex flex-col items-center pt-4 pb-2"
            >
              {/* Radar animation */}
              <div className="relative flex items-center justify-center mb-5" style={{ width: 180, height: 180 }}>
                <RadarRing delay={0} size={160} />
                <RadarRing delay={0.75} size={160} />
                <RadarRing delay={1.5} size={160} />
                <RadarRing delay={2.25} size={160} />

                {/* Static rings */}
                {[55, 100, 145].map((d) => (
                  <div key={d} className="absolute rounded-full" style={{
                    width: d, height: d,
                    border: '1px solid rgba(99,102,241,0.12)',
                    top: '50%', left: '50%',
                    marginLeft: -d / 2, marginTop: -d / 2,
                  }} />
                ))}

                {/* Center icon */}
                <div className="relative z-10 w-14 h-14 rounded-2xl flex items-center justify-center"
                  style={{
                    background: 'linear-gradient(135deg, rgba(99,102,241,0.3), rgba(168,85,247,0.2))',
                    border: '1.5px solid rgba(99,102,241,0.4)',
                    boxShadow: '0 4px 20px rgba(99,102,241,0.25)',
                  }}
                >
                  <svg className="w-7 h-7" fill="none" viewBox="0 0 24 24" stroke="rgba(167,139,250,0.9)">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5}
                      d="M8.111 16.404a5.5 5.5 0 017.778 0M12 20h.01m-7.08-7.071c3.904-3.905 10.236-3.905 14.14 0M1.394 9.393c5.857-5.857 15.355-5.857 21.213 0" />
                  </svg>
                </div>
              </div>

              <p className="text-base font-bold text-white/70 mb-1">Scanning nearby…</p>
              <p className="text-sm text-white/35 text-center leading-relaxed px-6">
                Open MultiMouse on another computer on the same network
              </p>
            </motion.div>
          ) : (
            /* Device list */
            <motion.div
              key="list"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
            >
              {/* Section header with subtle scanning indicator */}
              <div className="flex items-center justify-between mb-3 px-0.5">
                <span className="text-xs font-bold uppercase tracking-widest"
                  style={{ color: 'rgba(255,255,255,0.4)' }}>
                  Nearby · {peers.length}
                </span>
                <div className="flex items-center gap-1.5">
                  <motion.div
                    animate={{ opacity: [0.4, 1, 0.4] }}
                    transition={{ repeat: Infinity, duration: 1.8 }}
                    className="w-1.5 h-1.5 rounded-full"
                    style={{ background: '#10b981' }}
                  />
                  <span className="text-xs" style={{ color: 'rgba(255,255,255,0.3)' }}>Scanning</span>
                </div>
              </div>

              <div className="space-y-2.5">
                {peers.map((peer) => (
                  <DeviceCard
                    key={peer.id}
                    peer={peer}
                    onConnect={handleConnect}
                    isConnecting={connectingTo === peer.id}
                  />
                ))}
              </div>
            </motion.div>
          )}
        </AnimatePresence>

        {/* Internet connect */}
        <div className="mt-4">
          <div className="flex items-center gap-3 mb-3">
            <div className="h-px flex-1" style={{ background: 'rgba(255,255,255,0.07)' }} />
            <span className="text-xs" style={{ color: 'rgba(255,255,255,0.22)' }}>or</span>
            <div className="h-px flex-1" style={{ background: 'rgba(255,255,255,0.07)' }} />
          </div>
          <button
            onClick={() => setShowInternet(true)}
            className="w-full flex items-center justify-center gap-2.5 py-3.5 rounded-2xl
              text-sm font-semibold transition-all active:scale-[0.98]"
            style={{
              background: 'rgba(99,102,241,0.08)',
              border: '1.5px solid rgba(99,102,241,0.2)',
              color: 'rgba(167,139,250,0.8)',
            }}
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
      <div className="px-4 py-3 flex items-center justify-between flex-shrink-0"
        style={{ borderTop: '1px solid rgba(255,255,255,0.06)' }}>
        <div className="flex items-center gap-2">
          <div className="w-2 h-2 rounded-full" style={{ background: '#10b981' }} />
          <span className="text-xs" style={{ color: 'rgba(255,255,255,0.35)' }}>{deviceName}</span>
        </div>
        <span className="text-xs" style={{ color: 'rgba(255,255,255,0.18)' }}>v0.1.0</span>
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
