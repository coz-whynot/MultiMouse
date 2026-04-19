import { useState, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useStore } from '../store/useStore';
import { DeviceCard } from '../components/DeviceCard';
import { ConnectionBanner } from '../components/ConnectionBanner';
import { InternetModal } from '../components/InternetModal';

/* Smooth ripple: opacity fades in + out in the same cycle so there's no pop
   when the animation loops (scale starts from 0.2, opacity stays at 0 at the
   boundary for seamless continuation). */
const RadarRing = ({ delay, size }: { delay: number; size: number }) => (
  <motion.div
    className="absolute rounded-full pointer-events-none will-change-transform"
    style={{
      width: size,
      height: size,
      border: '1.5px solid rgba(99,102,241,0.5)',
      top: '50%',
      left: '50%',
      marginLeft: -size / 2,
      marginTop: -size / 2,
    }}
    animate={{ scale: [0.2, 1.4], opacity: [0, 0.7, 0] }}
    transition={{
      repeat: Infinity,
      duration: 3.6,
      delay,
      ease: 'linear',
      times: [0, 0.5, 1],
    }}
  />
);

/* Rotating sweep line — transform: rotate() on a GPU layer stays smooth at
   60fps (use linear ease; any curve adds per-frame CPU recalculation). */
const RadarSweep = () => (
  <motion.div
    className="absolute top-1/2 left-1/2 origin-left pointer-events-none will-change-transform"
    style={{ width: 90, height: 1.5, marginTop: -0.75, marginLeft: 0 }}
    animate={{ rotate: 360 }}
    transition={{ repeat: Infinity, duration: 4, ease: 'linear' }}
  >
    <div
      className="w-full h-full rounded-full"
      style={{ background: 'linear-gradient(90deg, rgba(99,102,241,0.75), rgba(99,102,241,0))' }}
    />
  </motion.div>
);

export const Home = () => {
  const { peers, status, connectingTo, setConnectingTo, shownPin } = useStore();
  const [showInternet, setShowInternet] = useState(false);
  const [accessibilityNeeded, setAccessibilityNeeded] = useState(false);

  const connectedPeer = peers.find((p) => p.id === status?.connected_peer);
  const deviceName = status?.device_name ?? 'This Device';
  const deviceInitial = deviceName.charAt(0).toUpperCase();
  const isEmpty = peers.length === 0;

  useEffect(() => {
    const unsubs = Promise.all([
      listen('accessibility-needed', () => setAccessibilityNeeded(true)),
      listen('connected', () => setConnectingTo(null)),
      listen('pin-rejected', () => setConnectingTo(null)),
      listen('connection-failed', () => setConnectingTo(null)),
      listen('disconnected', () => setConnectingTo(null)),
    ]);
    return () => { unsubs.then((fns) => fns.forEach((fn) => fn())); };
  }, []);

  const handleConnect = async (peerId: string) => {
    setConnectingTo(peerId);
    try {
      await invoke('connect_to_device', { peerId, pin: '' });
    } catch (e) {
      console.error(e);
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
            className="mx-5 mt-1 overflow-hidden"
          >
            <div
              className="rounded-xl px-4 py-3 flex gap-3 mb-3"
              style={{ background: 'rgba(245,158,11,0.12)', border: '1px solid rgba(245,158,11,0.28)' }}
            >
              <svg className="w-4 h-4 flex-shrink-0 mt-0.5" fill="none" viewBox="0 0 24 24" stroke="#d97706">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2}
                  d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
              </svg>
              <div className="flex-1">
                <p className="text-xs font-semibold leading-tight" style={{ color: '#d97706' }}>Accessibility permission needed</p>
                <p className="text-[10px] mt-0.5 leading-relaxed" style={{ color: 'rgba(217,119,6,0.8)' }}>
                  System Settings → Privacy & Security → Accessibility → enable MultiMouse
                </p>
              </div>
              <button
                onClick={() => setAccessibilityNeeded(false)}
                className="flex-shrink-0 w-5 h-5 rounded flex items-center justify-center transition-colors"
                style={{ color: 'rgba(251,191,36,0.45)' }}
              >
                <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2.5}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
                </svg>
              </button>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      <div className="px-5">
        <ConnectionBanner connectedPeer={connectedPeer} relaying={status?.relaying ?? false} />
      </div>

      {/* ── Two-column dashboard: Left = This Device + Internet · Right = Nearby list ── */}
      <div className="flex-1 overflow-y-auto px-5 pb-4">
        <div className="grid grid-cols-1 md:grid-cols-[360px_1fr] gap-4">

          {/* ── LEFT COLUMN ── */}
          <div className="flex flex-col gap-4">
            {/* This Device hero card */}
            <div
              className="rounded-3xl px-6 py-6 flex flex-col gap-4 relative overflow-hidden"
              style={{
                background: 'linear-gradient(135deg, rgba(99,102,241,0.18) 0%, rgba(168,85,247,0.13) 100%)',
                border: '1.5px solid rgba(99,102,241,0.26)',
                boxShadow: '0 6px 24px rgba(99,102,241,0.14)',
                minHeight: 220,
              }}
            >
              {/* Decorative blob */}
              <div
                className="absolute right-0 top-0 w-40 h-40 pointer-events-none"
                style={{
                  background: 'radial-gradient(circle, rgba(168,85,247,0.22) 0%, transparent 70%)',
                  transform: 'translate(28%, -28%)',
                }}
              />

              <div className="flex items-center gap-4 relative">
                {/* Avatar */}
                <motion.div
                  animate={{
                    boxShadow: [
                      '0 0 0 0px rgba(99,102,241,0.4)',
                      '0 0 0 9px rgba(99,102,241,0)',
                      '0 0 0 0px rgba(99,102,241,0)',
                    ],
                  }}
                  transition={{ repeat: Infinity, duration: 3.2, ease: 'easeInOut' }}
                  className="w-16 h-16 rounded-2xl flex items-center justify-center flex-shrink-0 relative"
                  style={{
                    background: 'linear-gradient(135deg, #6366f1, #a855f7)',
                    boxShadow: '0 6px 20px rgba(99,102,241,0.45)',
                  }}
                >
                  <span className="text-3xl font-black text-white">{deviceInitial}</span>
                  {/* Online dot */}
                  <div
                    className="absolute -bottom-1 -right-1 w-4 h-4 rounded-full border-2"
                    style={{ background: 'var(--success)', borderColor: 'var(--bg-base)' }}
                  />
                </motion.div>

                {/* Info */}
                <div className="flex-1 min-w-0 relative">
                  <p
                    className="text-[9px] font-bold uppercase tracking-widest mb-1"
                    style={{ color: 'var(--accent-muted)' }}
                  >
                    This Device
                  </p>
                  <p className="text-lg font-black leading-tight truncate" style={{ color: 'var(--text-primary)' }}>{deviceName}</p>
                  <p className="text-xs mt-1" style={{ color: 'var(--text-muted)' }}>
                    {connectedPeer ? (
                      <span style={{ color: 'var(--accent-primary)' }}>
                        Connected to {connectedPeer.name}
                      </span>
                    ) : (
                      'Ready to share'
                    )}
                  </p>
                </div>
              </div>

              {/* Device stats / status line */}
              <div className="relative grid grid-cols-2 gap-2 mt-auto">
                <div
                  className="rounded-xl px-3 py-2.5"
                  style={{
                    background: 'var(--bg-subtle)',
                    border: '1px solid var(--border-subtle)',
                  }}
                >
                  <p className="text-[9px] font-bold uppercase tracking-widest mb-0.5" style={{ color: 'var(--accent-muted)' }}>
                    Nearby
                  </p>
                  <p className="text-base font-black" style={{ color: 'var(--text-primary)' }}>{peers.length}</p>
                </div>
                <div
                  className="rounded-xl px-3 py-2.5"
                  style={{
                    background: 'var(--bg-subtle)',
                    border: '1px solid var(--border-subtle)',
                  }}
                >
                  <p className="text-[9px] font-bold uppercase tracking-widest mb-0.5" style={{ color: 'var(--accent-muted)' }}>
                    Status
                  </p>
                  <p className="text-sm font-bold" style={{ color: connectedPeer ? 'var(--success)' : 'var(--text-secondary)' }}>
                    {connectedPeer ? 'Linked' : 'Idle'}
                  </p>
                </div>
              </div>
            </div>

            {/* Connect via Internet button card */}
            <button
              onClick={() => setShowInternet(true)}
              className="w-full flex items-center gap-3 px-5 py-4 rounded-2xl text-sm font-semibold transition-all active:scale-[0.98]"
              style={{
                background: 'var(--accent-soft-bg)',
                border: '1.5px solid var(--accent-soft-br)',
                color: 'var(--accent-primary)',
              }}
            >
              <div
                className="w-10 h-10 rounded-xl flex items-center justify-center flex-shrink-0"
                style={{
                  background: 'linear-gradient(135deg, rgba(99,102,241,0.25), rgba(168,85,247,0.18))',
                  border: '1px solid rgba(99,102,241,0.3)',
                }}
              >
                <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.8}>
                  <path strokeLinecap="round" strokeLinejoin="round"
                    d="M3.055 11H5a2 2 0 012 2v1a2 2 0 002 2 2 2 0 012 2v2.945M8 3.935V5.5A2.5 2.5 0 0010.5 8h.5a2 2 0 012 2 2 2 0 104 0 2 2 0 012-2h1.064M15 20.488V18a2 2 0 012-2h3.064" />
                </svg>
              </div>
              <div className="flex-1 text-left min-w-0">
                <p className="text-sm font-bold leading-tight" style={{ color: 'var(--text-primary)' }}>Connect via Internet</p>
                <p className="text-[11px] mt-0.5" style={{ color: 'var(--text-muted)' }}>
                  Link to a device beyond your Wi-Fi
                </p>
              </div>
              <svg className="w-4 h-4 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="var(--accent-muted)" strokeWidth={2.5}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M9 5l7 7-7 7" />
              </svg>
            </button>
          </div>

          {/* ── RIGHT COLUMN — Nearby devices ── */}
          <div
            className="rounded-3xl p-5 flex flex-col"
            style={{
              background: 'var(--bg-subtle-2)',
              border: '1px solid var(--border-subtle)',
              minHeight: 360,
            }}
          >
            {/* Section header */}
            <div className="flex items-center justify-between mb-4">
              <div className="flex items-center gap-2">
                <span className="text-[10px] font-bold uppercase tracking-widest" style={{ color: 'var(--text-muted)' }}>
                  Nearby Devices
                </span>
                {peers.length > 0 && (
                  <span
                    className="text-[10px] font-bold px-2 py-0.5 rounded-full"
                    style={{
                      background: 'var(--accent-soft-bg)',
                      border: '1px solid var(--accent-soft-br)',
                      color: 'var(--accent-primary)',
                    }}
                  >
                    {peers.length}
                  </span>
                )}
              </div>
              <div className="flex items-center gap-1.5">
                <motion.div
                  animate={{ opacity: [0.4, 1, 0.4] }}
                  transition={{ repeat: Infinity, duration: 2 }}
                  className="w-1.5 h-1.5 rounded-full"
                  style={{ background: 'var(--success)' }}
                />
                <span className="text-[10px]" style={{ color: 'var(--text-muted)' }}>Scanning</span>
              </div>
            </div>

            <AnimatePresence mode="wait">
              {isEmpty ? (
                /* ── Radar empty state ── */
                <motion.div
                  key="empty"
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  exit={{ opacity: 0 }}
                  className="flex-1 flex flex-col items-center justify-center py-8"
                >
                  {/* Radar */}
                  <div
                    className="relative flex items-center justify-center mb-6"
                    style={{ width: 200, height: 200 }}
                  >
                    {/* Static rings */}
                    {[72, 130, 190].map((d) => (
                      <div
                        key={d}
                        className="absolute rounded-full pointer-events-none"
                        style={{
                          width: d,
                          height: d,
                          border: '1px solid rgba(99,102,241,0.1)',
                          top: '50%',
                          left: '50%',
                          marginLeft: -d / 2,
                          marginTop: -d / 2,
                        }}
                      />
                    ))}

                    {/* Animated ripple rings */}
                    <RadarRing delay={0} size={180} />
                    <RadarRing delay={0.8} size={180} />
                    <RadarRing delay={1.6} size={180} />
                    <RadarRing delay={2.4} size={180} />

                    {/* Sweep line */}
                    <RadarSweep />

                    {/* Center icon */}
                    <div
                      className="relative z-10 w-16 h-16 rounded-2xl flex items-center justify-center"
                      style={{
                        background: 'linear-gradient(135deg, rgba(99,102,241,0.25), rgba(168,85,247,0.18))',
                        border: '1.5px solid rgba(99,102,241,0.35)',
                        boxShadow: '0 4px 20px rgba(99,102,241,0.22)',
                      }}
                    >
                      <svg className="w-8 h-8" fill="none" viewBox="0 0 24 24" stroke="var(--accent-primary)" strokeWidth={1.4}>
                        <path strokeLinecap="round" strokeLinejoin="round"
                          d="M8.111 16.404a5.5 5.5 0 017.778 0M12 20h.01m-7.08-7.071c3.904-3.905 10.236-3.905 14.14 0M1.394 9.393c5.857-5.857 15.355-5.857 21.213 0" />
                      </svg>
                    </div>
                  </div>

                  <p className="text-base font-bold mb-2" style={{ color: 'var(--text-secondary)' }}>
                    Scanning nearby…
                  </p>
                  <p className="text-xs text-center leading-relaxed px-8 max-w-sm" style={{ color: 'var(--text-muted)' }}>
                    Open MultiMouse on another computer on the same Wi-Fi network to see it appear here.
                  </p>
                </motion.div>
              ) : (
                /* ── Device grid ── */
                <motion.div
                  key="list"
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  exit={{ opacity: 0 }}
                  className="grid grid-cols-1 xl:grid-cols-2 gap-3"
                >
                  {peers.map((peer, i) => (
                    <motion.div
                      key={peer.id}
                      initial={{ opacity: 0, y: 10 }}
                      animate={{ opacity: 1, y: 0 }}
                      transition={{ delay: i * 0.04, type: 'spring', stiffness: 360, damping: 28 }}
                    >
                      <DeviceCard
                        peer={peer}
                        onConnect={handleConnect}
                        isConnecting={connectingTo === peer.id}
                      />
                    </motion.div>
                  ))}
                </motion.div>
              )}
            </AnimatePresence>
          </div>
        </div>
      </div>

      {/* ── Connecting overlay ── */}
      <AnimatePresence>
        {connectingTo && !peers.find((p) => p.id === connectingTo && p.status === 'connected') && (
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className="fixed inset-0 flex items-center justify-center z-50"
            style={{ background: 'var(--bg-overlay)', backdropFilter: 'blur(14px)' }}
          >
            <motion.div
              initial={{ scale: 0.88, opacity: 0, y: 12 }}
              animate={{ scale: 1, opacity: 1, y: 0 }}
              exit={{ scale: 0.88, opacity: 0, y: 12 }}
              transition={{ type: 'spring', stiffness: 420, damping: 32 }}
              className="rounded-3xl p-7 flex flex-col items-center gap-4 w-64"
              style={{
                background: 'var(--bg-card-strong)',
                border: '1px solid var(--border)',
                boxShadow: 'var(--shadow-hero)',
              }}
            >
              {/* Spinner */}
              <div className="relative w-14 h-14">
                <motion.div
                  animate={{ rotate: 360 }}
                  transition={{ repeat: Infinity, duration: 1.1, ease: 'linear' }}
                  className="absolute inset-0 rounded-full"
                  style={{ border: '2.5px solid rgba(99,102,241,0.15)', borderTopColor: 'var(--accent-secondary)' }}
                />
                <div
                  className="absolute inset-2 rounded-full flex items-center justify-center"
                  style={{ background: 'rgba(99,102,241,0.12)' }}
                >
                  <span className="text-base font-black" style={{ color: 'var(--text-primary)' }}>
                    {(peers.find((p) => p.id === connectingTo)?.name ?? 'D').charAt(0).toUpperCase()}
                  </span>
                </div>
              </div>

              <div className="text-center">
                <p className="font-bold leading-tight" style={{ color: 'var(--text-primary)' }}>
                  {peers.find((p) => p.id === connectingTo)?.name ?? 'Device'}
                </p>
                <p className="text-xs mt-1.5" style={{ color: 'var(--text-muted)' }}>
                  {shownPin ? 'Verify the code matches on the other device' : 'Waiting for them to accept…'}
                </p>
              </div>

              {/* Shared PIN for visual verification */}
              <AnimatePresence>
                {shownPin && (
                  <motion.div
                    initial={{ opacity: 0, scale: 0.95 }}
                    animate={{ opacity: 1, scale: 1 }}
                    exit={{ opacity: 0, scale: 0.95 }}
                    className="w-full rounded-2xl py-3 px-4"
                    style={{
                      background: 'var(--accent-soft-bg)',
                      border: '1px solid var(--accent-soft-br)',
                    }}
                  >
                    <p
                      className="text-center text-[10px] font-semibold uppercase tracking-widest mb-1"
                      style={{ color: 'var(--accent-muted)' }}
                    >
                      Pairing code
                    </p>
                    <p
                      className="text-center font-mono font-bold tracking-[0.3em] text-2xl"
                      style={{ color: 'var(--text-primary)' }}
                    >
                      {shownPin}
                    </p>
                  </motion.div>
                )}
              </AnimatePresence>

              <button
                onClick={() => { invoke('disconnect').catch(() => {}); setConnectingTo(null); }}
                className="text-xs font-medium transition-colors"
                style={{ color: 'var(--text-faint)' }}
              >
                Cancel
              </button>
            </motion.div>
          </motion.div>
        )}

        {showInternet && <InternetModal onClose={() => setShowInternet(false)} />}
      </AnimatePresence>
    </div>
  );
};
