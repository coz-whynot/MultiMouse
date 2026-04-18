import { motion } from 'framer-motion';
import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface PinDisplayProps {
  peerName: string;
  pin?: string;
  onClose: () => void;
}

interface PinEntryProps {
  peerName: string;
  onSubmit: (pin: string) => void;
  onCancel: () => void;
  isLoading?: boolean;
}

const TIMEOUT_SECONDS = 30;

export const PinDisplay = ({ peerName, pin, onClose }: PinDisplayProps) => {
  const [busy, setBusy] = useState(false);
  const [secondsLeft, setSecondsLeft] = useState(TIMEOUT_SECONDS);

  useEffect(() => {
    const t = setInterval(() => {
      setSecondsLeft((s) => {
        if (s <= 1) {
          clearInterval(t);
          onClose();
          return 0;
        }
        return s - 1;
      });
    }, 1000);
    return () => clearInterval(t);
  }, [onClose]);

  const respond = async (accept: boolean) => {
    setBusy(true);
    try {
      await invoke(accept ? 'accept_pairing' : 'reject_pairing');
    } catch (e) {
      console.error('pairing response error', e);
    } finally {
      onClose();
    }
  };

  const progress = secondsLeft / TIMEOUT_SECONDS;
  const circumference = 2 * Math.PI * 18;

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      className="fixed inset-0 flex items-center justify-center z-50"
      style={{ background: 'rgba(6,6,16,0.88)', backdropFilter: 'blur(16px)' }}
    >
      <motion.div
        initial={{ scale: 0.88, opacity: 0, y: 12 }}
        animate={{ scale: 1, opacity: 1, y: 0 }}
        exit={{ scale: 0.88, opacity: 0, y: 12 }}
        transition={{ type: 'spring', stiffness: 420, damping: 32 }}
        className="w-[280px] rounded-3xl p-6"
        style={{
          background: 'linear-gradient(160deg, rgba(22,20,50,0.98) 0%, rgba(16,14,38,0.98) 100%)',
          border: '1px solid rgba(99,102,241,0.2)',
          boxShadow: '0 24px 64px rgba(0,0,0,0.6), 0 0 0 1px rgba(255,255,255,0.04) inset',
        }}
      >
        {/* Header */}
        <div className="flex flex-col items-center mb-5">
          {/* Animated ring with icon */}
          <div className="relative w-16 h-16 mb-3">
            <svg className="absolute inset-0 w-full h-full -rotate-90" viewBox="0 0 44 44">
              <circle cx="22" cy="22" r="18" fill="none" stroke="rgba(99,102,241,0.12)" strokeWidth="2.5" />
              <motion.circle
                cx="22" cy="22" r="18"
                fill="none"
                stroke="#6366f1"
                strokeWidth="2.5"
                strokeLinecap="round"
                strokeDasharray={circumference}
                animate={{ strokeDashoffset: circumference * (1 - progress) }}
                transition={{ ease: 'linear', duration: 0.9 }}
              />
            </svg>
            <div
              className="absolute inset-1.5 rounded-full flex items-center justify-center"
              style={{
                background: 'linear-gradient(135deg, rgba(99,102,241,0.25), rgba(168,85,247,0.18))',
                border: '1px solid rgba(99,102,241,0.3)',
              }}
            >
              <svg className="w-6 h-6" fill="none" viewBox="0 0 24 24" stroke="rgba(167,139,250,0.9)" strokeWidth={1.8}>
                <path strokeLinecap="round" strokeLinejoin="round"
                  d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z" />
              </svg>
            </div>
          </div>

          <p className="text-[11px] font-semibold uppercase tracking-widest mb-1" style={{ color: 'rgba(167,139,250,0.55)' }}>
            Connection Request
          </p>
          <p className="text-white font-bold text-base leading-tight text-center">{peerName}</p>
        </div>

        <p className="text-center text-[13px] mb-3 leading-relaxed" style={{ color: 'rgba(255,255,255,0.45)' }}>
          Allow <span className="text-white/80 font-semibold">{peerName}</span> to control this device?
        </p>

        {/* PIN for visual verification */}
        {pin && (
          <div
            className="mb-3 rounded-2xl py-3 px-4"
            style={{
              background: 'rgba(99,102,241,0.08)',
              border: '1px solid rgba(99,102,241,0.22)',
            }}
          >
            <p
              className="text-center text-[10px] font-semibold uppercase tracking-widest mb-1.5"
              style={{ color: 'rgba(167,139,250,0.65)' }}
            >
              Verify this code matches
            </p>
            <p
              className="text-center text-white font-mono font-bold tracking-[0.3em] text-2xl"
              style={{ textShadow: '0 2px 8px rgba(99,102,241,0.4)' }}
            >
              {pin}
            </p>
          </div>
        )}

        {/* Countdown text */}
        <p className="text-center text-[11px] mb-4" style={{ color: 'rgba(255,255,255,0.25)' }}>
          Auto-dismissing in {secondsLeft}s
        </p>

        <div className="flex gap-2">
          <button
            onClick={() => respond(false)}
            disabled={busy}
            className="flex-1 py-2.5 rounded-xl text-sm font-medium transition-all active:scale-[0.97] disabled:opacity-40"
            style={{
              background: 'rgba(255,255,255,0.05)',
              border: '1px solid rgba(255,255,255,0.1)',
              color: 'rgba(255,255,255,0.55)',
            }}
          >
            Reject
          </button>
          <button
            onClick={() => respond(true)}
            disabled={busy}
            className="flex-1 py-2.5 rounded-xl text-sm font-bold transition-all active:scale-[0.97] disabled:opacity-40"
            style={{
              background: 'linear-gradient(135deg, #6366f1, #a855f7)',
              boxShadow: '0 4px 16px rgba(99,102,241,0.35)',
              color: 'white',
            }}
          >
            {busy ? 'Accepting…' : 'Accept'}
          </button>
        </div>
      </motion.div>
    </motion.div>
  );
};

export const PinEntry = ({ peerName, onSubmit, onCancel, isLoading }: PinEntryProps) => {
  const [pin, setPin] = useState('');

  const handleSubmit = () => {
    if (pin.length >= 4) onSubmit(pin);
  };

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      className="fixed inset-0 flex items-center justify-center z-50"
      style={{ background: 'rgba(6,6,16,0.88)', backdropFilter: 'blur(16px)' }}
    >
      <motion.div
        initial={{ scale: 0.88, opacity: 0, y: 12 }}
        animate={{ scale: 1, opacity: 1, y: 0 }}
        exit={{ scale: 0.88, opacity: 0, y: 12 }}
        transition={{ type: 'spring', stiffness: 420, damping: 32 }}
        className="w-[280px] rounded-3xl p-6"
        style={{
          background: 'linear-gradient(160deg, rgba(22,20,50,0.98) 0%, rgba(16,14,38,0.98) 100%)',
          border: '1px solid rgba(99,102,241,0.2)',
          boxShadow: '0 24px 64px rgba(0,0,0,0.6), 0 0 0 1px rgba(255,255,255,0.04) inset',
        }}
      >
        <div className="flex flex-col items-center mb-5">
          <div
            className="w-12 h-12 rounded-2xl flex items-center justify-center mb-3"
            style={{
              background: 'linear-gradient(135deg, rgba(99,102,241,0.25), rgba(168,85,247,0.18))',
              border: '1px solid rgba(99,102,241,0.3)',
            }}
          >
            <svg className="w-6 h-6" fill="none" viewBox="0 0 24 24" stroke="rgba(167,139,250,0.9)" strokeWidth={1.8}>
              <path strokeLinecap="round" strokeLinejoin="round"
                d="M8 11V7a4 4 0 118 0m-4 8v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2z" />
            </svg>
          </div>
          <p className="text-white font-bold text-base">Connect to {peerName}</p>
          <p className="text-[11px] mt-1 text-center" style={{ color: 'rgba(255,255,255,0.38)' }}>
            Enter the PIN shown on {peerName}
          </p>
        </div>

        <input
          type="text"
          inputMode="numeric"
          maxLength={8}
          value={pin}
          onChange={(e) => setPin(e.target.value.replace(/\D/g, ''))}
          onKeyDown={(e) => e.key === 'Enter' && handleSubmit()}
          placeholder="· · · · · ·"
          autoFocus
          className="w-full text-center text-2xl font-mono font-bold tracking-[0.35em]
            rounded-xl py-3 px-4 mb-5 transition-all focus:outline-none"
          style={{
            background: 'rgba(255,255,255,0.05)',
            border: `1.5px solid ${pin.length >= 4 ? 'rgba(99,102,241,0.5)' : 'rgba(255,255,255,0.09)'}`,
            color: 'white',
          }}
        />

        <div className="flex gap-2">
          <button
            onClick={onCancel}
            className="flex-1 py-2.5 rounded-xl text-sm font-medium transition-all active:scale-[0.97]"
            style={{
              background: 'rgba(255,255,255,0.05)',
              border: '1px solid rgba(255,255,255,0.1)',
              color: 'rgba(255,255,255,0.55)',
            }}
          >
            Cancel
          </button>
          <button
            onClick={handleSubmit}
            disabled={pin.length < 4 || isLoading}
            className="flex-1 py-2.5 rounded-xl text-sm font-bold transition-all active:scale-[0.97] disabled:opacity-40 disabled:cursor-not-allowed"
            style={{
              background: pin.length >= 4 ? 'linear-gradient(135deg, #6366f1, #a855f7)' : 'rgba(99,102,241,0.2)',
              boxShadow: pin.length >= 4 ? '0 4px 16px rgba(99,102,241,0.3)' : 'none',
              color: 'white',
            }}
          >
            {isLoading ? 'Connecting…' : 'Connect'}
          </button>
        </div>
      </motion.div>
    </motion.div>
  );
};
