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

function FileIcon({ direction }: { direction: 'sending' | 'receiving' }) {
  return direction === 'sending' ? (
    <svg className="w-4 h-4 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="var(--accent-primary)" strokeWidth={1.8}>
      <path strokeLinecap="round" strokeLinejoin="round" d="M7 16a4 4 0 01-.88-7.903A5 5 0 1115.9 6L16 6a5 5 0 011 9.9M15 13l-3-3m0 0l-3 3m3-3v12" />
    </svg>
  ) : (
    <svg className="w-4 h-4 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="var(--success)" strokeWidth={1.8}>
      <path strokeLinecap="round" strokeLinejoin="round" d="M7 16a4 4 0 01-.88-7.903A5 5 0 1115.9 6L16 6a5 5 0 011 9.9M9 19l3 3m0 0l3-3m-3 3V10" />
    </svg>
  );
}

function TransferRow({ t }: { t: TransferInfo }) {
  const pct = t.size > 0 ? Math.round((t.transferred / t.size) * 100) : 0;
  const isActive = t.status === 'active' || t.status === 'pending';

  const statusColor =
    t.status === 'done'
      ? 'var(--success)'
      : t.status === 'error'
      ? 'var(--danger)'
      : t.status === 'active'
      ? 'var(--accent-primary)'
      : 'var(--text-faint)';

  const statusLabel =
    t.status === 'pending'
      ? t.direction === 'receiving'
        ? 'Waiting…'
        : 'Offering…'
      : t.status === 'active'
      ? `${pct}%`
      : t.status === 'done'
      ? 'Done'
      : t.status === 'error'
      ? 'Error'
      : 'Declined';

  return (
    <div className="flex flex-col gap-1.5 py-2.5 border-b last:border-0" style={{ borderColor: 'var(--divider)' }}>
      <div className="flex items-center gap-2.5">
        <FileIcon direction={t.direction} />
        <div className="flex-1 min-w-0">
          <p className="text-xs font-medium truncate" style={{ color: 'var(--text-strong)' }}>{t.name}</p>
          <p className="text-[10px] mt-0.5" style={{ color: 'var(--text-muted)' }}>
            {t.peer_name} · {fmt(t.size)}
          </p>
        </div>
        <span className="text-xs font-mono font-semibold flex-shrink-0" style={{ color: statusColor }}>
          {statusLabel}
        </span>
      </div>

      {isActive && (
        <div className="w-full h-1 rounded-full overflow-hidden" style={{ background: 'var(--bg-subtle)' }}>
          <motion.div
            className="h-full rounded-full"
            style={{ background: 'linear-gradient(90deg, #6366f1, #a855f7)' }}
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
      className="fixed inset-0 flex items-end justify-center z-50 p-3 pb-20"
      style={{ background: 'var(--bg-overlay)', backdropFilter: 'blur(10px)' }}
    >
      <motion.div
        initial={{ y: 48, opacity: 0 }}
        animate={{ y: 0, opacity: 1 }}
        exit={{ y: 48, opacity: 0 }}
        transition={{ type: 'spring', stiffness: 400, damping: 34 }}
        className="w-full rounded-2xl p-4"
        style={{
          background: 'var(--bg-card-strong)',
          border: '1px solid var(--border)',
          boxShadow: 'var(--shadow-card)',
        }}
      >
        <div className="flex items-center gap-3 mb-4">
          <div
            className="w-11 h-11 rounded-xl flex items-center justify-center flex-shrink-0"
            style={{
              background: 'var(--accent-soft-bg)',
              border: '1px solid var(--accent-soft-br)',
            }}
          >
            <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="var(--accent-primary)" strokeWidth={1.8}>
              <path strokeLinecap="round" strokeLinejoin="round"
                d="M7 16a4 4 0 01-.88-7.903A5 5 0 1115.9 6L16 6a5 5 0 011 9.9M9 19l3 3m0 0l3-3m-3 3V10" />
            </svg>
          </div>
          <div className="min-w-0">
            <p className="text-sm font-bold truncate" style={{ color: 'var(--text-primary)' }}>{offer.name}</p>
            <p className="text-xs mt-0.5" style={{ color: 'var(--text-muted)' }}>
              {offer.peer_name} wants to send {fmt(offer.size)}
            </p>
          </div>
        </div>

        <div className="flex gap-2">
          <button
            onClick={handleReject}
            className="flex-1 py-2.5 rounded-xl text-sm font-medium transition-all active:scale-[0.97]"
            style={{
              background: 'var(--bg-subtle)',
              border: '1px solid var(--border-subtle)',
              color: 'var(--text-body)',
            }}
          >
            Decline
          </button>
          <button
            onClick={handleAccept}
            className="flex-1 py-2.5 rounded-xl text-sm font-bold transition-all active:scale-[0.97]"
            style={{
              background: 'linear-gradient(135deg, #6366f1, #a855f7)',
              boxShadow: '0 4px 16px rgba(99,102,241,0.3)',
              color: 'white',
            }}
          >
            Save to Downloads
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
            className="mx-3 mb-1.5 rounded-xl px-3 py-1 overflow-hidden"
            style={{
              background: 'var(--bg-subtle-2)',
              border: '1px solid var(--border-subtle)',
            }}
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
