import { motion } from 'framer-motion';
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

  const handleHost = async () => {
    setLoading(true);
    try {
      const code = await invoke<string>('create_internet_session');
      setRoomCode(code);
    } catch (e) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  };

  const handleJoin = async () => {
    if (!joinCode || !pin) return;
    setLoading(true);
    try {
      await invoke('join_internet_session', { code: joinCode.toUpperCase(), pin });
      onClose();
    } catch (e) {
      console.error(e);
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
      style={{ background: 'rgba(8,8,18,0.9)', backdropFilter: 'blur(12px)' }}
      onClick={(e) => e.target === e.currentTarget && onClose()}
    >
      <motion.div
        initial={{ scale: 0.92, opacity: 0 }}
        animate={{ scale: 1, opacity: 1 }}
        exit={{ scale: 0.92, opacity: 0 }}
        className="w-80 rounded-3xl p-5 border border-white/10"
        style={{ background: 'rgba(19,19,42,0.98)' }}
      >
        <div className="flex items-center justify-between mb-4">
          <div className="flex items-center gap-2">
            <div className="w-8 h-8 rounded-xl bg-gradient-to-br from-accent-500 to-purple-500
              flex items-center justify-center">
              <svg className="w-4 h-4 text-white" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2}
                  d="M3.055 11H5a2 2 0 012 2v1a2 2 0 002 2 2 2 0 012 2v2.945M8 3.935V5.5A2.5 2.5 0 0010.5 8h.5a2 2 0 012 2 2 2 0 104 0 2 2 0 012-2h1.064M15 20.488V18a2 2 0 012-2h3.064" />
              </svg>
            </div>
            <span className="font-semibold text-white text-sm">Connect via Internet</span>
          </div>
          <button onClick={onClose} className="text-white/30 hover:text-white/60 transition-colors">
            <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        {/* Tabs */}
        <div className="flex gap-1 p-1 bg-white/[0.04] rounded-xl mb-4">
          {(['host', 'join'] as const).map((t) => (
            <button
              key={t}
              onClick={() => setTab(t)}
              className={`flex-1 py-1.5 rounded-lg text-xs font-medium capitalize transition-all ${
                tab === t
                  ? 'bg-accent-500 text-white shadow'
                  : 'text-white/40 hover:text-white/60'
              }`}
            >
              {t === 'host' ? 'Share Code' : 'Enter Code'}
            </button>
          ))}
        </div>

        {tab === 'host' ? (
          <div>
            <p className="text-xs text-white/40 mb-3">
              Generate a code and share it with the other person.
            </p>
            {roomCode ? (
              <>
                <p className="text-xs text-white/40 text-center mb-2">Your session code</p>
                <div className="flex justify-center gap-1.5 mb-3">
                  {roomCode.split('').map((c, i) => (
                    <div
                      key={i}
                      className="w-9 h-10 rounded-xl bg-white/[0.07] border border-white/10
                        flex items-center justify-center text-lg font-bold text-white font-mono"
                    >
                      {c}
                    </div>
                  ))}
                </div>
                <p className="text-xs text-white/30 text-center mb-3">
                  Waiting for other device to connect…
                </p>
                <motion.div
                  animate={{ rotate: 360 }}
                  transition={{ repeat: Infinity, duration: 2, ease: 'linear' }}
                  className="w-5 h-5 border-2 border-accent-400/20 border-t-accent-400 rounded-full mx-auto"
                />
              </>
            ) : (
              <button
                onClick={handleHost}
                disabled={loading}
                className="w-full py-3 rounded-xl text-sm font-semibold text-white
                  bg-gradient-to-r from-accent-500 to-purple-500
                  hover:opacity-90 disabled:opacity-50 transition-all"
              >
                {loading ? 'Generating…' : 'Generate Code'}
              </button>
            )}
          </div>
        ) : (
          <div className="space-y-3">
            <div>
              <p className="text-xs text-white/40 mb-1.5">Session code from other device</p>
              <input
                type="text"
                maxLength={6}
                value={joinCode}
                onChange={(e) => setJoinCode(e.target.value.toUpperCase().replace(/[^A-Z0-9]/g, ''))}
                placeholder="XXXXXX"
                className="w-full text-center text-xl font-mono font-bold tracking-[0.3em]
                  bg-white/[0.05] border border-white/10 rounded-xl py-3 text-white
                  placeholder:text-white/20 focus:outline-none focus:border-accent-500/50"
              />
            </div>
            <div>
              <p className="text-xs text-white/40 mb-1.5">PIN shown on that device</p>
              <input
                type="text"
                maxLength={6}
                value={pin}
                onChange={(e) => setPin(e.target.value.replace(/\D/g, ''))}
                placeholder="000000"
                className="w-full text-center text-xl font-mono font-bold tracking-[0.3em]
                  bg-white/[0.05] border border-white/10 rounded-xl py-3 text-white
                  placeholder:text-white/20 focus:outline-none focus:border-accent-500/50"
              />
            </div>
            <button
              onClick={handleJoin}
              disabled={joinCode.length < 6 || pin.length < 4 || loading}
              className="w-full py-3 rounded-xl text-sm font-semibold text-white
                bg-gradient-to-r from-accent-500 to-purple-500
                hover:opacity-90 disabled:opacity-40 transition-all"
            >
              {loading ? 'Connecting…' : 'Connect'}
            </button>
          </div>
        )}
      </motion.div>
    </motion.div>
  );
};
