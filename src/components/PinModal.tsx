import { motion } from 'framer-motion';
import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface PinDisplayProps {
  peerName: string;
  onClose: () => void;
}

interface PinEntryProps {
  peerName: string;
  onSubmit: (pin: string) => void;
  onCancel: () => void;
  isLoading?: boolean;
}

export const PinDisplay = ({ peerName, onClose }: PinDisplayProps) => {
  const [busy, setBusy] = useState(false);

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

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      className="fixed inset-0 flex items-center justify-center z-50"
      style={{ background: 'rgba(8,8,18,0.85)', backdropFilter: 'blur(12px)' }}
    >
      <motion.div
        initial={{ scale: 0.9, opacity: 0 }}
        animate={{ scale: 1, opacity: 1 }}
        exit={{ scale: 0.9, opacity: 0 }}
        className="w-72 rounded-3xl p-6 border border-white/10"
        style={{ background: 'rgba(19,19,42,0.95)' }}
      >
        <div className="text-center mb-5">
          <div className="w-12 h-12 rounded-2xl bg-gradient-to-br from-accent-500 to-purple-500 mx-auto mb-3
            flex items-center justify-center">
            <svg className="w-6 h-6 text-white" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2}
                d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z" />
            </svg>
          </div>
          <p className="text-white/60 text-xs mb-1">Connection request from</p>
          <p className="text-white font-semibold">{peerName}</p>
        </div>

        <p className="text-center text-white/50 text-xs mb-5">
          Allow <span className="text-white">{peerName}</span> to control this device?
        </p>

        <div className="flex gap-2">
          <button
            onClick={() => respond(false)}
            disabled={busy}
            className="flex-1 py-2.5 rounded-xl text-sm text-white/50 hover:text-white
              hover:bg-white/5 border border-white/10 transition-all disabled:opacity-40"
          >
            Reject
          </button>
          <button
            onClick={() => respond(true)}
            disabled={busy}
            className="flex-1 py-2.5 rounded-xl text-sm font-semibold text-white
              bg-gradient-to-r from-accent-500 to-purple-500
              disabled:opacity-40 hover:opacity-90 transition-all"
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
      style={{ background: 'rgba(8,8,18,0.85)', backdropFilter: 'blur(12px)' }}
    >
      <motion.div
        initial={{ scale: 0.9, opacity: 0 }}
        animate={{ scale: 1, opacity: 1 }}
        exit={{ scale: 0.9, opacity: 0 }}
        className="w-72 rounded-3xl p-6 border border-white/10"
        style={{ background: 'rgba(19,19,42,0.95)' }}
      >
        <div className="text-center mb-5">
          <div className="w-12 h-12 rounded-2xl bg-gradient-to-br from-accent-500 to-purple-500 mx-auto mb-3
            flex items-center justify-center">
            <svg className="w-6 h-6 text-white" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2}
                d="M8 11V7a4 4 0 118 0m-4 8v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2z" />
            </svg>
          </div>
          <p className="text-white font-semibold">Connect to {peerName}</p>
          <p className="text-white/40 text-xs mt-1">Enter the PIN shown on {peerName}</p>
        </div>

        <input
          type="text"
          maxLength={8}
          value={pin}
          onChange={(e) => setPin(e.target.value.replace(/\D/g, ''))}
          onKeyDown={(e) => e.key === 'Enter' && handleSubmit()}
          placeholder="000000"
          autoFocus
          className="w-full text-center text-2xl font-mono font-bold tracking-[0.3em]
            bg-white/[0.05] border border-white/10 rounded-xl py-3 px-4 text-white
            placeholder:text-white/20 focus:outline-none focus:border-accent-500/50
            transition-all mb-4"
        />

        <div className="flex gap-2">
          <button
            onClick={onCancel}
            className="flex-1 py-2.5 rounded-xl text-sm text-white/50 hover:text-white
              hover:bg-white/5 border border-white/10 transition-all"
          >
            Cancel
          </button>
          <button
            onClick={handleSubmit}
            disabled={pin.length < 4 || isLoading}
            className="flex-1 py-2.5 rounded-xl text-sm font-semibold text-white
              bg-gradient-to-r from-accent-500 to-purple-500
              disabled:opacity-40 disabled:cursor-not-allowed
              hover:opacity-90 transition-all"
          >
            {isLoading ? 'Connecting…' : 'Connect'}
          </button>
        </div>
      </motion.div>
    </motion.div>
  );
};
