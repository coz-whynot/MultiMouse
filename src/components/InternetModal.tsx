import { motion, AnimatePresence } from 'framer-motion';
import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface Props {
  onClose: () => void;
}

export const InternetModal = ({ onClose }: Props) => {
  const [tab, setTab] = useState<'host' | 'join'>('host');
  const [roomCode, setRoomCode] = useState('');
  const [joinCode, setJoinCode] = useState('');
  const [pin, setPin] = useState('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleHost = async () => {
    setLoading(true);
    setError(null);
    try {
      const code = await invoke<string>('create_internet_session');
      setRoomCode(code);
    } catch {
      setError('Could not create session. Check your relay URL in Settings.');
    } finally {
      setLoading(false);
    }
  };

  const handleJoin = async () => {
    if (!joinCode || !pin) return;
    setLoading(true);
    setError(null);
    try {
      await invoke('join_internet_session', { code: joinCode.toUpperCase(), pin });
      onClose();
    } catch {
      setError('Could not join. Check the code and PIN, then try again.');
    } finally {
      setLoading(false);
    }
  };

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      className="fixed inset-0 flex items-center justify-center z-50"
      style={{ background: 'rgba(6,6,16,0.88)', backdropFilter: 'blur(16px)' }}
      onClick={(e) => e.target === e.currentTarget && onClose()}
    >
      <motion.div
        initial={{ scale: 0.9, opacity: 0, y: 10 }}
        animate={{ scale: 1, opacity: 1, y: 0 }}
        exit={{ scale: 0.9, opacity: 0, y: 10 }}
        transition={{ type: 'spring', stiffness: 420, damping: 32 }}
        className="w-[300px] rounded-3xl p-5"
        style={{
          background: 'linear-gradient(160deg, rgba(22,20,50,0.99) 0%, rgba(16,14,38,0.99) 100%)',
          border: '1px solid rgba(99,102,241,0.2)',
          boxShadow: '0 24px 64px rgba(0,0,0,0.6), 0 0 0 1px rgba(255,255,255,0.04) inset',
        }}
      >
        {/* Header */}
        <div className="flex items-center justify-between mb-4">
          <div className="flex items-center gap-2.5">
            <div
              className="w-8 h-8 rounded-xl flex items-center justify-center"
              style={{
                background: 'linear-gradient(135deg, rgba(99,102,241,0.25), rgba(168,85,247,0.18))',
                border: '1px solid rgba(99,102,241,0.3)',
              }}
            >
              <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="rgba(167,139,250,0.9)" strokeWidth={1.8}>
                <path strokeLinecap="round" strokeLinejoin="round"
                  d="M3.055 11H5a2 2 0 012 2v1a2 2 0 002 2 2 2 0 012 2v2.945M8 3.935V5.5A2.5 2.5 0 0010.5 8h.5a2 2 0 012 2 2 2 0 104 0 2 2 0 012-2h1.064M15 20.488V18a2 2 0 012-2h3.064" />
              </svg>
            </div>
            <span className="font-bold text-white text-sm">Connect via Internet</span>
          </div>
          <button
            onClick={onClose}
            className="w-7 h-7 rounded-lg flex items-center justify-center transition-all"
            style={{ background: 'rgba(255,255,255,0.05)', color: 'rgba(255,255,255,0.4)' }}
          >
            <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2.5}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        {/* Tabs */}
        <div
          className="flex p-1 rounded-xl mb-4"
          style={{ background: 'rgba(255,255,255,0.04)', border: '1px solid rgba(255,255,255,0.06)' }}
        >
          {(['host', 'join'] as const).map((t) => (
            <button
              key={t}
              onClick={() => { setTab(t); setError(null); }}
              className="flex-1 py-1.5 rounded-lg text-xs font-semibold transition-all"
              style={{
                background: tab === t ? 'rgba(99,102,241,0.22)' : 'transparent',
                color: tab === t ? '#a78bfa' : 'rgba(255,255,255,0.35)',
                border: tab === t ? '1px solid rgba(99,102,241,0.3)' : '1px solid transparent',
              }}
            >
              {t === 'host' ? 'Share Code' : 'Enter Code'}
            </button>
          ))}
        </div>

        {/* Error */}
        <AnimatePresence>
          {error && (
            <motion.div
              initial={{ opacity: 0, height: 0 }}
              animate={{ opacity: 1, height: 'auto' }}
              exit={{ opacity: 0, height: 0 }}
              className="mb-3 rounded-xl px-3 py-2.5 text-xs"
              style={{
                background: 'rgba(239,68,68,0.08)',
                border: '1px solid rgba(239,68,68,0.2)',
                color: '#fca5a5',
              }}
            >
              {error}
            </motion.div>
          )}
        </AnimatePresence>

        <AnimatePresence mode="wait">
          {tab === 'host' ? (
            <motion.div
              key="host"
              initial={{ opacity: 0, x: -8 }}
              animate={{ opacity: 1, x: 0 }}
              exit={{ opacity: 0, x: 8 }}
              transition={{ duration: 0.15 }}
            >
              <p className="text-xs mb-3 leading-relaxed" style={{ color: 'rgba(255,255,255,0.38)' }}>
                Generate a code and share it with the other person. They'll enter it on their device.
              </p>
              {roomCode ? (
                <div className="text-center">
                  <p className="text-xs mb-2.5" style={{ color: 'rgba(255,255,255,0.35)' }}>Your session code</p>
                  <div className="flex justify-center gap-1.5 mb-3">
                    {roomCode.split('').map((c, i) => (
                      <motion.div
                        key={i}
                        initial={{ scale: 0, opacity: 0 }}
                        animate={{ scale: 1, opacity: 1 }}
                        transition={{ delay: i * 0.06, type: 'spring', stiffness: 500, damping: 28 }}
                        className="w-9 h-10 rounded-xl flex items-center justify-center text-lg font-bold text-white font-mono"
                        style={{
                          background: 'rgba(99,102,241,0.12)',
                          border: '1px solid rgba(99,102,241,0.25)',
                        }}
                      >
                        {c}
                      </motion.div>
                    ))}
                  </div>
                  <p className="text-xs mb-3" style={{ color: 'rgba(255,255,255,0.28)' }}>
                    Waiting for other device…
                  </p>
                  <motion.div
                    animate={{ rotate: 360 }}
                    transition={{ repeat: Infinity, duration: 2, ease: 'linear' }}
                    className="w-5 h-5 rounded-full mx-auto"
                    style={{ border: '2px solid rgba(99,102,241,0.2)', borderTopColor: '#818cf8' }}
                  />
                </div>
              ) : (
                <button
                  onClick={handleHost}
                  disabled={loading}
                  className="w-full py-3 rounded-xl text-sm font-bold transition-all active:scale-[0.98] disabled:opacity-50"
                  style={{
                    background: 'linear-gradient(135deg, #6366f1, #a855f7)',
                    boxShadow: '0 4px 16px rgba(99,102,241,0.3)',
                    color: 'white',
                  }}
                >
                  {loading ? 'Generating…' : 'Generate Session Code'}
                </button>
              )}
            </motion.div>
          ) : (
            <motion.div
              key="join"
              initial={{ opacity: 0, x: 8 }}
              animate={{ opacity: 1, x: 0 }}
              exit={{ opacity: 0, x: -8 }}
              transition={{ duration: 0.15 }}
              className="space-y-3"
            >
              <div>
                <p className="text-xs mb-1.5" style={{ color: 'rgba(255,255,255,0.38)' }}>
                  Session code from other device
                </p>
                <input
                  type="text"
                  maxLength={6}
                  value={joinCode}
                  onChange={(e) => setJoinCode(e.target.value.toUpperCase().replace(/[^A-Z0-9]/g, ''))}
                  placeholder="A B C 1 2 3"
                  className="w-full text-center text-xl font-mono font-bold tracking-[0.3em]
                    rounded-xl py-3 transition-all focus:outline-none"
                  style={{
                    background: 'rgba(255,255,255,0.05)',
                    border: `1.5px solid ${joinCode.length === 6 ? 'rgba(99,102,241,0.5)' : 'rgba(255,255,255,0.09)'}`,
                    color: 'white',
                  }}
                />
              </div>
              <div>
                <p className="text-xs mb-1.5" style={{ color: 'rgba(255,255,255,0.38)' }}>
                  PIN shown on that device
                </p>
                <input
                  type="text"
                  inputMode="numeric"
                  maxLength={6}
                  value={pin}
                  onChange={(e) => setPin(e.target.value.replace(/\D/g, ''))}
                  placeholder="· · · · · ·"
                  className="w-full text-center text-xl font-mono font-bold tracking-[0.3em]
                    rounded-xl py-3 transition-all focus:outline-none"
                  style={{
                    background: 'rgba(255,255,255,0.05)',
                    border: `1.5px solid ${pin.length >= 4 ? 'rgba(99,102,241,0.5)' : 'rgba(255,255,255,0.09)'}`,
                    color: 'white',
                  }}
                />
              </div>
              <button
                onClick={handleJoin}
                disabled={joinCode.length < 6 || pin.length < 4 || loading}
                className="w-full py-3 rounded-xl text-sm font-bold transition-all active:scale-[0.98] disabled:opacity-40"
                style={{
                  background: 'linear-gradient(135deg, #6366f1, #a855f7)',
                  boxShadow: '0 4px 16px rgba(99,102,241,0.3)',
                  color: 'white',
                }}
              >
                {loading ? 'Connecting…' : 'Connect'}
              </button>
            </motion.div>
          )}
        </AnimatePresence>
      </motion.div>
    </motion.div>
  );
};
