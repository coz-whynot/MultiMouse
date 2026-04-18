import { motion, AnimatePresence } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { useStore } from '../store/useStore';
import { TransferInfo, FileOffer } from '../types';

function fmt(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function TransferRow({ t }: { t: TransferInfo }) {
  const pct = t.size > 0 ? Math.round((t.transferred / t.size) * 100) : 0;
  const isActive = t.status === 'active' || t.status === 'pending';

  const statusColor = {
    pending: 'text-white/40',
    active: 'text-accent-400',
    done: 'text-emerald-400',
    error: 'text-red-400',
    rejected: 'text-white/30',
  }[t.status] ?? 'text-white/40';

  const statusLabel = {
    pending: t.direction === 'receiving' ? 'Waiting…' : 'Offering…',
    active: `${pct}%`,
    done: 'Done',
    error: 'Error',
    rejected: 'Rejected',
  }[t.status] ?? '';

  return (
    <div className="flex flex-col gap-1.5 py-2.5 border-b border-white/[0.05] last:border-0">
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-2 min-w-0">
          <span className="text-lg">
            {t.direction === 'sending' ? '↑' : '↓'}
          </span>
          <div className="min-w-0">
            <p className="text-xs font-medium text-white/80 truncate">{t.name}</p>
            <p className="text-[10px] text-white/30">
              {t.peer_name} · {fmt(t.size)}
            </p>
          </div>
        </div>
        <span className={`text-xs font-mono font-semibold flex-shrink-0 ${statusColor}`}>
          {statusLabel}
        </span>
      </div>

      {isActive && (
        <div className="w-full h-1 bg-white/[0.06] rounded-full overflow-hidden">
          <motion.div
            className="h-full bg-gradient-to-r from-accent-500 to-purple-500 rounded-full"
            initial={{ width: 0 }}
            animate={{ width: `${pct}%` }}
            transition={{ ease: 'linear', duration: 0.3 }}
          />
        </div>
      )}
    </div>
  );
}

function FileOfferModal({ offer, onClose }: { offer: FileOffer; onClose: () => void }) {
  const handleAccept = async () => {
    await invoke('accept_transfer', { transferId: offer.id });
    onClose();
  };
  const handleReject = async () => {
    await invoke('reject_transfer', { transferId: offer.id });
    onClose();
  };

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      className="fixed inset-0 flex items-end justify-center z-50 p-3"
      style={{ background: 'rgba(8,8,18,0.6)', backdropFilter: 'blur(8px)' }}
    >
      <motion.div
        initial={{ y: 40, opacity: 0 }}
        animate={{ y: 0, opacity: 1 }}
        exit={{ y: 40, opacity: 0 }}
        className="w-full rounded-2xl p-4 border border-white/10"
        style={{ background: 'rgba(19,19,42,0.98)' }}
      >
        <div className="flex items-center gap-3 mb-3">
          <div className="w-10 h-10 rounded-xl bg-accent-500/10 border border-accent-500/20
            flex items-center justify-center flex-shrink-0">
            <svg className="w-5 h-5 text-accent-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.8}
                d="M7 16a4 4 0 01-.88-7.903A5 5 0 1115.9 6L16 6a5 5 0 011 9.9M9 19l3 3m0 0l3-3m-3 3V10" />
            </svg>
          </div>
          <div className="min-w-0">
            <p className="text-sm font-semibold text-white truncate">{offer.name}</p>
            <p className="text-xs text-white/40">
              {offer.peer_name} wants to send you {fmt(offer.size)}
            </p>
          </div>
        </div>

        <div className="flex gap-2">
          <button
            onClick={handleReject}
            className="flex-1 py-2.5 rounded-xl text-sm text-white/50
              bg-white/[0.04] hover:bg-white/[0.08] border border-white/[0.07]
              transition-all active:scale-[0.97]"
          >
            Decline
          </button>
          <button
            onClick={handleAccept}
            className="flex-1 py-2.5 rounded-xl text-sm font-semibold text-white
              bg-gradient-to-r from-accent-500 to-purple-500
              hover:opacity-90 transition-all active:scale-[0.97]"
          >
            Accept
          </button>
        </div>
      </motion.div>
    </motion.div>
  );
}

export const TransferPanel = () => {
  const { transfers, fileOffer, setFileOffer } = useStore();
  const visible = transfers.filter(
    (t) => t.status === 'active' || t.status === 'pending',
  );

  if (visible.length === 0 && !fileOffer) return null;

  return (
    <>
      <AnimatePresence>
        {fileOffer && (
          <FileOfferModal offer={fileOffer} onClose={() => setFileOffer(null)} />
        )}
      </AnimatePresence>

      <AnimatePresence>
        {visible.length > 0 && (
          <motion.div
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: 'auto' }}
            exit={{ opacity: 0, height: 0 }}
            className="mx-3 mb-1 rounded-xl px-3 py-1
              bg-white/[0.03] border border-white/[0.06]"
          >
            {visible.map((t) => (
              <TransferRow key={t.id} t={t} />
            ))}
          </motion.div>
        )}
      </AnimatePresence>
    </>
  );
};
